// src/gpu/wgpu_3d.rs — 3D-конвейер wgpu (draw_mesh_3d, draw_batch_3d, clear_3d).

use super::wgpu_backend::WgpuRenderer;

impl WgpuRenderer {
    pub(crate) fn clear_3d_impl(&mut self, r: u8, g: u8, b: u8) {
        let color = wgpu::Color {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
            a: 1.0,
        };
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.enc.take().unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rlisp-frame"),
                })
        });
        {
            let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rlisp-clear-3d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.enc = Some(enc);
        self.pipeline_3d.reset_frame();
        self.pending += 1;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_mesh_3d_impl(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        rgba: Option<&[f32]>,
        _nrm: Option<&[f32]>,
        idx: &[f32],
        model: &[f32],
        mvp: &[f32],
        tint: (f32, f32, f32),
        texid: i64,
        wire: bool,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String> {
        if pos.len() % 3 != 0 || pos.is_empty() || idx.is_empty() {
            return Ok(0);
        }

        let (vbuf, ibuf, v_bytes_len, idx_count) = self.pipeline_3d.get_or_upload_mesh(
            &self.device,
            &self.queue,
            pos,
            uv,
            rgba,
            idx,
            tint,
        );

        let mut m_model = [0.0f32; 16];
        m_model.copy_from_slice(model);
        let mut m_mvp = [0.0f32; 16];
        m_mvp.copy_from_slice(mvp);

        let raw = crate::gpu_3d::Uniforms3DRaw {
            model: m_model,
            mvp: m_mvp,
            light_dir: r3.light,
            ambient: r3.ambient,
            diffuse: r3.diffuse,
            spec: r3.spec,
            shininess: r3.shininess,
            alpha_test: r3.alpha_test,
            camera_pos: r3.view,
            blend_opacity: r3.blend,
            tiling: r3.tiling,
            fog_density: r3.fog[3],
            _pad0: 0.0,
            fog_color: [r3.fog[0], r3.fog[1], r3.fog[2]],
            _pad1: 0.0,
        };
        let u_offset = self
            .pipeline_3d
            .write_uniforms(&self.queue, &self.device, &raw);

        let is_blend = r3.blend < 1.0
            || rgba
                .map(|c| c.iter().skip(3).step_by(4).any(|&a| a < 1.0))
                .unwrap_or(false);
        let write_depth = r3.blend >= 0.90
            && rgba
                .map(|c| c.iter().skip(3).step_by(4).all(|&a| a >= 0.90))
                .unwrap_or(true);
        let pipe = if wire {
            &self.pipeline_3d.pipeline_wire
        } else if texid != 0 {
            if is_blend {
                if write_depth {
                    &self.pipeline_3d.pipeline_blend_tex_depth
                } else {
                    &self.pipeline_3d.pipeline_blend_tex
                }
            } else {
                &self.pipeline_3d.pipeline_tex
            }
        } else if is_blend {
            &self.pipeline_3d.pipeline_blend_solid
        } else {
            &self.pipeline_3d.pipeline_solid
        };

        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let tex_entry = if texid != 0 {
            self.tex_views.get(&texid)
        } else {
            None
        };

        let mut enc = self.enc.take().unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rlisp-frame"),
                })
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rlisp-draw-3d-mesh"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.pipeline_3d.uniform_bind_group, &[u_offset]);
            if let Some((_, bg)) = tex_entry {
                pass.set_bind_group(1, bg, &[]);
            }
            pass.set_vertex_buffer(0, vbuf.slice(0..v_bytes_len));
            pass.set_index_buffer(
                ibuf.slice(0..(idx_count as u64 * 4)),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..idx_count, 0, 0..1);
        }
        self.enc = Some(enc);
        self.pending += 1;

        let drawn = if wire {
            idx_count as u64 / 2
        } else {
            idx_count as u64 / 3
        };
        Ok(drawn)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_batch_3d_impl(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        _nrm: Option<&[f32]>,
        idx: &[f32],
        insts: &[f32],
        mvp: &[f32],
        texid: i64,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String> {
        if pos.len() % 3 != 0 || pos.is_empty() || idx.is_empty() || insts.len() < 12 {
            return Ok(0);
        }
        let k = insts.len() / 12;

        let (vbuf, ibuf, v_bytes_len, idx_count) = self.pipeline_3d.get_or_upload_mesh(
            &self.device,
            &self.queue,
            pos,
            uv,
            None,
            idx,
            (1.0, 1.0, 1.0),
        );

        let inst_offset = self
            .pipeline_3d
            .write_instances(&self.queue, &self.device, insts);

        let mut m_mvp = [0.0f32; 16];
        m_mvp.copy_from_slice(mvp);
        let mut m_identity = [0.0f32; 16];
        m_identity[0] = 1.0;
        m_identity[5] = 1.0;
        m_identity[10] = 1.0;
        m_identity[15] = 1.0;

        let raw = crate::gpu_3d::Uniforms3DRaw {
            model: m_identity,
            mvp: m_mvp,
            light_dir: r3.light,
            ambient: r3.ambient,
            diffuse: r3.diffuse,
            spec: r3.spec,
            shininess: r3.shininess,
            alpha_test: r3.alpha_test,
            camera_pos: r3.view,
            blend_opacity: r3.blend,
            tiling: r3.tiling,
            fog_density: r3.fog[3],
            _pad0: 0.0,
            fog_color: [r3.fog[0], r3.fog[1], r3.fog[2]],
            _pad1: 0.0,
        };
        let u_offset = self
            .pipeline_3d
            .write_uniforms(&self.queue, &self.device, &raw);

        let is_blend = r3.blend < 1.0;
        let write_depth = r3.blend >= 0.90;
        let pipe = if texid != 0 {
            if is_blend {
                if write_depth {
                    &self.pipeline_3d.pipeline_inst_blend_tex_depth
                } else {
                    &self.pipeline_3d.pipeline_inst_blend_tex
                }
            } else {
                &self.pipeline_3d.pipeline_inst_tex
            }
        } else if is_blend {
            &self.pipeline_3d.pipeline_inst_blend_solid
        } else {
            &self.pipeline_3d.pipeline_inst_solid
        };

        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let tex_entry = if texid != 0 {
            self.tex_views.get(&texid)
        } else {
            None
        };

        let mut enc = self.enc.take().unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rlisp-frame"),
                })
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rlisp-draw-3d-batch"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.pipeline_3d.uniform_bind_group, &[u_offset]);
            if let Some((_, bg)) = tex_entry {
                pass.set_bind_group(1, bg, &[]);
            }
            pass.set_vertex_buffer(0, vbuf.slice(0..v_bytes_len));
            pass.set_vertex_buffer(
                1,
                self.pipeline_3d
                    .inst_buf
                    .slice(inst_offset..inst_offset + (insts.len() * 4) as u64),
            );
            pass.set_index_buffer(
                ibuf.slice(0..(idx_count as u64 * 4)),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..idx_count, 0, 0..k as u32);
        }
        self.enc = Some(enc);
        self.pending += 1;

        let drawn = (idx_count as u64 / 3) * (k as u64);
        Ok(drawn)
    }

    pub(crate) fn clear_mesh_cache_impl(&mut self) {
        self.pipeline_3d.clear_mesh_cache();
    }
}
