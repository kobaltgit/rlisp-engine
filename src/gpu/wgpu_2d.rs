// src/gpu/wgpu_2d.rs — 2D-текстурные операции wgpu (blit, blit_tex, tex_upload, tex_free).

use super::wgpu_backend::WgpuRenderer;
use super::wgpu_quad::quad_verts;

impl WgpuRenderer {
    pub(crate) fn blit_impl(
        &mut self,
        x: i64,
        y: i64,
        w: usize,
        h: usize,
        bytes: &[u8],
    ) -> Result<(), String> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        // скрэтч-текстура под размер спрайта (переиспользуем)
        let reuse = matches!(&self.scratch, Some((sw, sh, _)) if *sw == w && *sh == h);
        if !reuse {
            let t = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("rlisp-blit-scratch"),
                size: wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.scratch = Some((w, h, t));
        }
        let Some((_, _, t)) = &self.scratch else {
            return Err("blit: нет scratch".into());
        };
        // короткий буфер дописываем прозрачным — как программный blit
        let need = w * h * 4;
        let owned: Vec<u8>;
        let px: &[u8] = if bytes.len() >= need {
            &bytes[..need]
        } else {
            owned = {
                let mut v = bytes.to_vec();
                v.resize(need, 0);
                v
            };
            &owned
        };
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            px,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some((w * 4) as u32),
                rows_per_image: Some(h as u32),
            },
            wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = t.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rlisp-blit-bind"),
            layout: &self.tex_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        // клип деста + сдвиг UV (1:1, как программный blit)
        let (tw, th) = (self.target_w, self.target_h);
        let dx0 = x.max(0);
        let dy0 = y.max(0);
        let dx1 = (x + w as i64).min(tw as i64);
        let dy1 = (y + h as i64).min(th as i64);
        if dx0 >= dx1 || dy0 >= dy1 {
            return Ok(());
        }
        let u0 = (dx0 - x) as f32 / w as f32;
        let v0 = (dy0 - y) as f32 / h as f32;
        let u1 = (dx1 - x) as f32 / w as f32;
        let v1 = (dy1 - y) as f32 / h as f32;
        let q = quad_verts(
            dx0,
            dy0,
            dx1 - dx0,
            dy1 - dy0,
            tw,
            th,
            [u0, v0],
            [u1, v1],
            [1.0, 1.0, 1.0],
        );
        self.record(tw, th, q, false, Some(&bg));
        Ok(())
    }

    pub(crate) fn blit_tex_impl(
        &mut self,
        id: i64,
        x: i64,
        y: i64,
        dw: i64,
        dh: i64,
        tint: [f32; 3],
    ) -> Result<(), String> {
        let Some((t, tex_w, tex_h)) = self.texs.get(&id) else {
            return Err(format!("gpu-blit-tex: нет текстуры {} на GPU", id));
        };
        let _ = (tex_w, tex_h); // размеры текстуры — uv уже нормирован по dw/dh
        let view = t.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rlisp-blit-tex-bind"),
            layout: &self.tex_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let (dx0, dy0) = (x.max(0), y.max(0));
        let (dx1, dy1) = (
            (x + dw).min(self.target_w as i64),
            (y + dh).min(self.target_h as i64),
        );
        if dx0 >= dx1 || dy0 >= dy1 || dw <= 0 || dh <= 0 {
            return Ok(());
        }
        let u0 = (dx0 - x) as f32 / dw as f32;
        let v0 = (dy0 - y) as f32 / dh as f32;
        let u1 = (dx1 - x) as f32 / dw as f32;
        let v1 = (dy1 - y) as f32 / dh as f32;
        let q = quad_verts(
            dx0,
            dy0,
            dx1 - dx0,
            dy1 - dy0,
            self.target_w,
            self.target_h,
            [u0, v0],
            [u1, v1],
            tint,
        );
        self.record(self.target_w, self.target_h, q, false, Some(&bg));
        Ok(())
    }

    pub(crate) fn tex_upload_impl(&mut self, id: i64, w: usize, h: usize, px: &[u8]) {
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rlisp-tex"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            px,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some((w * 4) as u32),
                rows_per_image: Some(h as u32),
            },
            wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = t.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rlisp-tex-bg"),
            layout: &self.tex_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        self.tex_views.insert(id, (view, bg));
        self.texs.insert(id, (t, w as u32, h as u32));
    }

    pub(crate) fn tex_free_impl(&mut self, id: i64) {
        self.texs.remove(&id); // wgpu::Texture удаляется вместе со слотом
        self.tex_views.remove(&id);
    }
}
