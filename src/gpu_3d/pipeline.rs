// src/gpu_3d/pipeline.rs — Pipeline3D (WebGPU рендер-конвейеры, буферы юниформов и инстансов, кэш мешей).

#[cfg(feature = "wgpu")]
use std::collections::HashMap;
#[cfg(feature = "wgpu")]
use std::num::NonZeroU64;
#[cfg(feature = "wgpu")]
use std::sync::Arc;

#[cfg(feature = "wgpu")]
use super::shaders::SHADER_3D;
#[cfg(feature = "wgpu")]
use super::types::{calc_mesh_fingerprint, CachedGpuMesh, MeshKey, Uniforms3DRaw};

#[cfg(feature = "wgpu")]
pub struct Pipeline3D {
    pub pipeline_solid: wgpu::RenderPipeline,
    pub pipeline_tex: wgpu::RenderPipeline,
    pub pipeline_inst_solid: wgpu::RenderPipeline,
    pub pipeline_inst_tex: wgpu::RenderPipeline,
    pub pipeline_inst_blend_solid: wgpu::RenderPipeline,
    pub pipeline_inst_blend_tex: wgpu::RenderPipeline,
    pub pipeline_inst_blend_tex_depth: wgpu::RenderPipeline,
    pub pipeline_wire: wgpu::RenderPipeline,
    pub pipeline_blend_solid: wgpu::RenderPipeline,
    pub pipeline_blend_tex: wgpu::RenderPipeline,
    pub pipeline_blend_tex_depth: wgpu::RenderPipeline,

    pub uniform_bind_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
    pub uniform_offset: usize,
    pub uniform_cap: usize,

    pub inst_buf: wgpu::Buffer,
    pub inst_buf_used: usize,
    pub inst_buf_cap: usize,

    pub mesh_cache: HashMap<MeshKey, CachedGpuMesh>,
}

#[cfg(feature = "wgpu")]
impl Pipeline3D {
    pub fn new(device: &wgpu::Device, tex_bind_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rlisp-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_3D.into()),
        });

        let uniform_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rlisp-3d-uniform-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(NonZeroU64::new(208).unwrap()),
                    },
                    count: None,
                }],
            });

        let uniform_cap = 64 * 1024; // 64 KB
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-3d-uniforms"),
            size: uniform_cap as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rlisp-3d-uniform-bg"),
            layout: &uniform_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: Some(NonZeroU64::new(208).unwrap()),
                }),
            }],
        });

        let layout_solid = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rlisp-3d-solid-layout"),
            bind_group_layouts: &[&uniform_bind_layout],
            push_constant_ranges: &[],
        });

        let layout_tex = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rlisp-3d-tex-layout"),
            bind_group_layouts: &[&uniform_bind_layout, tex_bind_layout],
            push_constant_ranges: &[],
        });

        let vlayout = wgpu::VertexBufferLayout {
            array_stride: 36, // 12 + 8 + 16
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 20,
                    shader_location: 2,
                },
            ],
        };

        let inst_layout = wgpu::VertexBufferLayout {
            array_stride: 48, // 12 * 4
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 24,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 36,
                    shader_location: 6,
                },
            ],
        };

        let make_pipe = |vs: &str,
                         fs: &str,
                         layout: &wgpu::PipelineLayout,
                         instanced: bool,
                         topo: wgpu::PrimitiveTopology,
                         depth_write: bool,
                         blend: Option<wgpu::BlendState>| {
            let buffers = if instanced {
                vec![vlayout.clone(), inst_layout.clone()]
            } else {
                vec![vlayout.clone()]
            };

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: vs,
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: fs,
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: topo,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None, // двустороннее освещение как в render3d.rs
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: depth_write,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };

        let pipeline_solid = make_pipe(
            "vs_3d",
            "fs_3d",
            &layout_solid,
            false,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            None,
        );

        let pipeline_tex = make_pipe(
            "vs_3d",
            "fs_3d_tex",
            &layout_tex,
            false,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            None,
        );

        let pipeline_inst_solid = make_pipe(
            "vs_3d_instanced",
            "fs_3d",
            &layout_solid,
            true,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            None,
        );

        let pipeline_inst_tex = make_pipe(
            "vs_3d_instanced",
            "fs_3d_tex",
            &layout_tex,
            true,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            None,
        );

        let pipeline_inst_blend_solid = make_pipe(
            "vs_3d_instanced",
            "fs_3d",
            &layout_solid,
            true,
            wgpu::PrimitiveTopology::TriangleList,
            false,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let pipeline_inst_blend_tex = make_pipe(
            "vs_3d_instanced",
            "fs_3d_tex",
            &layout_tex,
            true,
            wgpu::PrimitiveTopology::TriangleList,
            false,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let pipeline_inst_blend_tex_depth = make_pipe(
            "vs_3d_instanced",
            "fs_3d_tex",
            &layout_tex,
            true,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let pipeline_wire = make_pipe(
            "vs_3d",
            "fs_3d_wire",
            &layout_solid,
            false,
            wgpu::PrimitiveTopology::LineList,
            false,
            None,
        );

        let pipeline_blend_solid = make_pipe(
            "vs_3d",
            "fs_3d",
            &layout_solid,
            false,
            wgpu::PrimitiveTopology::TriangleList,
            false,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let pipeline_blend_tex = make_pipe(
            "vs_3d",
            "fs_3d_tex",
            &layout_tex,
            false,
            wgpu::PrimitiveTopology::TriangleList,
            false,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let pipeline_blend_tex_depth = make_pipe(
            "vs_3d",
            "fs_3d_tex",
            &layout_tex,
            false,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let inst_buf_cap = 512 * 1024; // 512 KB
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-3d-inst-buf"),
            size: inst_buf_cap as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Pipeline3D {
            pipeline_solid,
            pipeline_tex,
            pipeline_inst_solid,
            pipeline_inst_tex,
            pipeline_inst_blend_solid,
            pipeline_inst_blend_tex,
            pipeline_inst_blend_tex_depth,
            pipeline_wire,
            pipeline_blend_solid,
            pipeline_blend_tex,
            pipeline_blend_tex_depth,
            uniform_bind_layout,
            uniform_buffer,
            uniform_bind_group,
            uniform_offset: 0,
            uniform_cap,
            inst_buf,
            inst_buf_used: 0,
            inst_buf_cap,
            mesh_cache: HashMap::new(),
        }
    }

    pub fn reset_frame(&mut self) {
        self.uniform_offset = 0;
        self.inst_buf_used = 0;
    }

    pub fn write_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        raw: &Uniforms3DRaw,
    ) -> u32 {
        let align = 256;
        let offset = self.uniform_offset;
        let needed = offset + 256;
        if needed > self.uniform_cap {
            self.uniform_cap *= 2;
            self.uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rlisp-3d-uniforms-realloc"),
                size: self.uniform_cap as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rlisp-3d-uniform-bg"),
                layout: &self.uniform_bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(208).unwrap()),
                    }),
                }],
            });
            self.uniform_offset = 0;
        }

        let slice = unsafe {
            std::slice::from_raw_parts(
                raw as *const Uniforms3DRaw as *const u8,
                std::mem::size_of::<Uniforms3DRaw>(),
            )
        };
        let cur_offset = self.uniform_offset;
        queue.write_buffer(&self.uniform_buffer, cur_offset as u64, slice);
        self.uniform_offset += align;
        cur_offset as u32
    }

    pub fn write_instances(
        &mut self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        insts: &[f32],
    ) -> u64 {
        let bytes = unsafe {
            std::slice::from_raw_parts(insts.as_ptr() as *const u8, std::mem::size_of_val(insts))
        };
        if self.inst_buf_used + bytes.len() > self.inst_buf_cap {
            self.inst_buf_cap =
                (self.inst_buf_cap * 2).max(self.inst_buf_used + bytes.len() + 1024);
            self.inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rlisp-3d-inst-grow"),
                size: self.inst_buf_cap as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.inst_buf_used = 0;
        }
        let offset = self.inst_buf_used as u64;
        queue.write_buffer(&self.inst_buf, offset, bytes);
        self.inst_buf_used += bytes.len();
        offset
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_or_upload_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pos: &[f32],
        uv: Option<&[f32]>,
        rgba: Option<&[f32]>,
        idx: &[f32],
        tint: (f32, f32, f32),
    ) -> (Arc<wgpu::Buffer>, Arc<wgpu::Buffer>, u64, u32) {
        let key = MeshKey {
            pos_ptr: pos.as_ptr() as usize,
            pos_len: pos.len(),
            idx_ptr: idx.as_ptr() as usize,
            idx_len: idx.len(),
            uv_ptr: uv.map(|u| u.as_ptr() as usize).unwrap_or(0),
            rgba_ptr: rgba.map(|r| r.as_ptr() as usize).unwrap_or(0),
            tint_bits: [tint.0.to_bits(), tint.1.to_bits(), tint.2.to_bits()],
        };

        let fp = calc_mesh_fingerprint(pos, idx, rgba);

        let needs_upload = match self.mesh_cache.get(&key) {
            Some(cached) => cached.fingerprint != fp,
            None => true,
        };

        if needs_upload {
            if self.mesh_cache.len() > 128 {
                self.mesh_cache.clear();
            }

            let n = pos.len() / 3;
            let mut v_bytes = Vec::with_capacity(n * 36);
            for i in 0..n {
                let px = pos[i * 3];
                let py = pos[i * 3 + 1];
                let pz = pos[i * 3 + 2];
                let (u, v) = if let Some(uarr) = uv {
                    (uarr[i * 2], uarr[i * 2 + 1])
                } else {
                    (0.0, 0.0)
                };
                let col = if let Some(carr) = rgba {
                    [
                        carr[i * 4],
                        carr[i * 4 + 1],
                        carr[i * 4 + 2],
                        carr[i * 4 + 3],
                    ]
                } else {
                    [tint.0, tint.1, tint.2, 1.0]
                };
                for f in [px, py, pz, u, v, col[0], col[1], col[2], col[3]] {
                    v_bytes.extend_from_slice(&f.to_ne_bytes());
                }
            }

            let indices: Vec<u32> = idx.iter().map(|&x| x as u32).collect();
            let idx_count = indices.len() as u32;

            let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rlisp-cached-vbuf"),
                size: (v_bytes.len().max(4)) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vbuf, 0, &v_bytes);

            let ibuf_bytes = unsafe {
                std::slice::from_raw_parts(
                    indices.as_ptr() as *const u8,
                    indices.len() * std::mem::size_of::<u32>(),
                )
            };
            let ibuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rlisp-cached-ibuf"),
                size: (ibuf_bytes.len().max(4)) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&ibuf, 0, ibuf_bytes);

            let v_bytes_len = v_bytes.len() as u64;

            self.mesh_cache.insert(
                key,
                CachedGpuMesh {
                    vbuf: Arc::new(vbuf),
                    ibuf: Arc::new(ibuf),
                    v_bytes_len,
                    idx_count,
                    fingerprint: fp,
                },
            );
        }

        let entry = self.mesh_cache.get(&key).unwrap();
        (
            entry.vbuf.clone(),
            entry.ibuf.clone(),
            entry.v_bytes_len,
            entry.idx_count,
        )
    }

    pub fn clear_mesh_cache(&mut self) {
        self.mesh_cache.clear();
    }
}
