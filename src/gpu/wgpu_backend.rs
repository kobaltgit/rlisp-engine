// src/gpu/wgpu_backend.rs — wgpu-бэкенд: offscreen-таргет, 2D-растр и readback в Hal::fb.

use std::collections::HashMap;

use super::types::{composite_readback, GpuBackend};
use super::wgpu_quad::{block_on, bytemuck_bytes, quad_verts, Quad, SHADER};

pub struct WgpuRenderer {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) adapter: String,

    // offscreen-таргет: RGB — цвет, alpha — маска покрытия GPU-командами
    pub(crate) target: wgpu::Texture,
    pub(crate) target_w: usize,
    pub(crate) target_h: usize,
    pub(crate) readback: wgpu::Buffer,
    pub(crate) readback_pad: usize,

    pub(crate) depth_target: wgpu::Texture,
    pub(crate) depth_view: wgpu::TextureView,
    pub(crate) pipeline_3d: crate::gpu_3d::Pipeline3D,

    pub(crate) vbuf: wgpu::Buffer,
    pub(crate) vcap: usize, // байт

    pub(crate) pipeline_solid: wgpu::RenderPipeline,
    pub(crate) pipeline_tex: wgpu::RenderPipeline,
    pub(crate) tex_bind_layout: wgpu::BindGroupLayout,
    pub(crate) sampler: wgpu::Sampler,

    /// реестр текстур бэкенда: id → (текстура, w, h)
    pub(crate) texs: HashMap<i64, (wgpu::Texture, u32, u32)>,
    pub(crate) tex_views: HashMap<i64, (wgpu::TextureView, wgpu::BindGroup)>,
    /// скрэтч-текстура для gpu-blit (переиспользуется при том же размере)
    pub(crate) scratch: Option<(usize, usize, wgpu::Texture)>,

    pub(crate) enc: Option<wgpu::CommandEncoder>,
    /// байт вершин, уже записанных в vbuf в текущем кадре
    pub(crate) vused: usize,
    /// pending-команды в кадре (для gpu_frames статистики)
    pub(crate) pending: u64,
}

impl WgpuRenderer {
    pub fn new(w: usize, h: usize) -> Result<WgpuRenderer, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| {
            "wgpu-адаптер недоступен (headless/CI/VM без GPU: ни Vulkan, ни GL)".to_string()
        })?;
        let info = adapter.get_info();
        let adapter_desc = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("rlisp-hal"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .map_err(|e| format!("wgpu: устройство не создано: {}", e))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rlisp-hal-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let tex_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rlisp-tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let vlayout = wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        };
        let make_pipeline = |fs: &str, layout: &wgpu::PipelineLayout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: "vs_main",
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vlayout),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: fs,
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        // блендинг выключен: покрытие учитывается на композите
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        let layout_solid = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rlisp-solid"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        let layout_tex = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rlisp-tex-pipe"),
            bind_group_layouts: &[&tex_bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline_solid = make_pipeline("fs_solid", &layout_solid);
        let pipeline_tex = make_pipeline("fs_tex", &layout_tex);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rlisp-nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // offscreen-таргет и readback создаются до перемещения device в структуру
        let target = device.create_texture(&Self::target_desc(w, h));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-readback"),
            size: (Self::pad_rows(w) * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth_target = device.create_texture(&Self::depth_desc(w, h));
        let depth_view = depth_target.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline_3d = crate::gpu_3d::Pipeline3D::new(&device, &tex_bind_layout);
        let vbuf_dummy = Self::dummy_buffer(&device);
        let readback_pad = Self::pad_rows(w);
        let mut r = WgpuRenderer {
            device,
            queue,
            adapter: adapter_desc,
            target,
            target_w: w,
            target_h: h,
            readback,
            readback_pad,
            depth_target,
            depth_view,
            pipeline_3d,
            vbuf: vbuf_dummy,
            vcap: 0,
            pipeline_solid,
            pipeline_tex,
            tex_bind_layout,
            sampler,
            texs: HashMap::new(),
            tex_views: HashMap::new(),
            scratch: None,
            enc: None,
            vused: 0,
            pending: 0,
        };
        r.vbuf = r.make_vbuf();
        Ok(r)
    }

    fn dummy_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-empty"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_vbuf(&self) -> wgpu::Buffer {
        // стартовая ёмкость: 1 МБ ≈ 32 тыс. вершин (растёт по потребности)
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-quads"),
            size: 1 << 20,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn pad_rows(w: usize) -> usize {
        (w * 4).div_ceil(256) * 256
    }

    fn target_desc(w: usize, h: usize) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rlisp-offscreen"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    fn depth_desc(w: usize, h: usize) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rlisp-depth"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }
    }

    fn make_readback(&self, w: usize, h: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-readback"),
            size: (Self::pad_rows(w) * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn ensure_enc(&mut self) -> &mut wgpu::CommandEncoder {
        if self.enc.is_none() {
            self.enc = Some(
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("rlisp-frame"),
                    }),
            );
        }
        self.enc.as_mut().unwrap()
    }

    fn ensure_vcap(&mut self, need: usize) {
        if self.vused + need <= self.vcap {
            return;
        }
        let mut cap = self.vcap.max(1 << 20); // старт: 1 МБ ≈ 32 тыс. вершин
        while cap < self.vused + need {
            cap *= 2;
        }
        self.vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlisp-quads"),
            size: cap as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.vcap = cap;
        self.vused = 0;
    }

    /// один квад на пайплайне; bind — для текстурного варианта
    fn draw_quad(
        &mut self,
        q: Quad,
        solid: bool,
        bind: Option<&wgpu::BindGroup>,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        self.ensure_vcap(48 * 4);
        self.queue
            .write_buffer(&self.vbuf, self.vused as u64, bytemuck_bytes(&q.data));
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        // ссылки до заимствования энкодера — энкодер забираем из self,
        // чтобы borrow'и не пересекались
        let pipe = if solid {
            &self.pipeline_solid
        } else {
            &self.pipeline_tex
        };
        let range = self.vused as u64..(self.vused + 48 * 4) as u64;
        let mut enc = self.enc.take().unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rlisp-frame"),
                })
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rlisp-quad"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            if let Some(bg) = bind {
                pass.set_bind_group(0, bg, &[]);
            }
            pass.set_vertex_buffer(0, self.vbuf.slice(range));
            pass.draw(0..6, 0..1);
        }
        self.enc = Some(enc);
        self.vused += 48 * 4;
        self.pending += 1;
    }

    /// общая запись draw-команды на GPU
    pub(crate) fn record(
        &mut self,
        target_w: usize,
        target_h: usize,
        q: Quad,
        solid: bool,
        bind: Option<&wgpu::BindGroup>,
    ) {
        self.draw_quad(q, solid, bind, wgpu::LoadOp::Load);
        let _ = (target_w, target_h);
    }
}

impl GpuBackend for WgpuRenderer {
    fn adapter_info(&self) -> String {
        self.adapter.clone()
    }

    fn resize(&mut self, w: usize, h: usize) {
        let (w, h) = (w.max(8), h.max(8));
        if w == self.target_w && h == self.target_h {
            return;
        }
        self.target = self.device.create_texture(&Self::target_desc(w, h));
        self.target_w = w;
        self.target_h = h;
        self.readback_pad = Self::pad_rows(w);
        self.readback = self.make_readback(w, h);
        self.depth_target = self.device.create_texture(&Self::depth_desc(w, h));
        self.depth_view = self
            .depth_target
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.enc = None;
        self.vused = 0;
        self.pending = 0;
    }

    fn clear(&mut self, r: u8, g: u8, b: u8) {
        let color = wgpu::Color {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
            a: 1.0, // полное покрытие — как программный gpu-clear
        };
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let enc = self.ensure_enc();
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rlisp-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        }); // пустой pass с loadOp=Clear — и есть очистка
        self.pending += 1;
    }

    fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, r: u8, g: u8, b: u8) {
        // тот же клип, что у программного растра
        let x0 = (x.max(0) as usize).min(self.target_w);
        let y0 = (y.max(0) as usize).min(self.target_h);
        let x1 = ((x + w).max(0) as usize).min(self.target_w);
        let y1 = ((y + h).max(0) as usize).min(self.target_h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let q = quad_verts(
            x0 as i64,
            y0 as i64,
            (x1 - x0) as i64,
            (y1 - y0) as i64,
            self.target_w,
            self.target_h,
            [0.0, 0.0],
            [0.0, 0.0],
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
        );
        self.record(self.target_w, self.target_h, q, true, None);
    }

    fn blit(&mut self, x: i64, y: i64, w: usize, h: usize, bytes: &[u8]) -> Result<(), String> {
        self.blit_impl(x, y, w, h, bytes)
    }

    fn blit_tex(
        &mut self,
        id: i64,
        x: i64,
        y: i64,
        dw: i64,
        dh: i64,
        tint: [f32; 3],
    ) -> Result<(), String> {
        self.blit_tex_impl(id, x, y, dw, dh, tint)
    }

    fn tex_upload(&mut self, id: i64, w: usize, h: usize, px: &[u8]) {
        self.tex_upload_impl(id, w, h, px);
    }

    fn tex_free(&mut self, id: i64) {
        self.tex_free_impl(id);
    }

    fn flush(&mut self, fb: &mut [u8], w: usize, h: usize, readback: bool) -> (u128, u128) {
        let t0 = std::time::Instant::now();
        if !readback {
            if let Some(enc) = self.enc.take() {
                self.queue.submit([enc.finish()]);
            }
            let submit_ns = t0.elapsed().as_nanos();
            self.pipeline_3d.reset_frame();
            self.vused = 0;
            self.pending = 0;
            return (submit_ns, 0);
        }
        let mut enc = self.enc.take().unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rlisp-readback"),
                })
        });
        enc.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &self.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.readback_pad as u32),
                    rows_per_image: Some(self.target_h as u32),
                },
            },
            wgpu::Extent3d {
                width: self.target_w as u32,
                height: self.target_h as u32,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let submit_ns = t0.elapsed().as_nanos();
        let t1 = std::time::Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });
        self.device.poll(wgpu::Maintain::Wait);
        // callback кладёт Ok(()) или Err(BufferAsyncError); канал закрывается
        // только если поток умер — в обоих случаях fb не трогаем
        let mapped = match rx.recv() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[wgpu] readback: map-канал закрылся");
                self.vused = 0;
                self.pending = 0;
                return (submit_ns, t1.elapsed().as_nanos());
            }
        };
        if let Err(e) = mapped {
            // readback не удался — кадр отдаём как есть (fb не трогаем)
            eprintln!("[wgpu] readback: {}", e);
            self.vused = 0;
            self.pending = 0;
            return (submit_ns, t1.elapsed().as_nanos());
        }
        {
            let view = self.readback.slice(..).get_mapped_range();
            composite_readback(
                fb,
                w.min(self.target_w),
                h.min(self.target_h),
                self.readback_pad,
                &view,
            );
        }
        self.readback.unmap();
        let readback_ns = t1.elapsed().as_nanos();
        self.pipeline_3d.reset_frame();
        self.vused = 0;
        self.pending = 0;
        (submit_ns, readback_ns)
    }

    fn clear_3d(&mut self, r: u8, g: u8, b: u8) {
        self.clear_3d_impl(r, g, b);
    }

    fn draw_mesh_3d(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        rgba: Option<&[f32]>,
        nrm: Option<&[f32]>,
        idx: &[f32],
        model: &[f32],
        mvp: &[f32],
        tint: (f32, f32, f32),
        texid: i64,
        wire: bool,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String> {
        self.draw_mesh_3d_impl(pos, uv, rgba, nrm, idx, model, mvp, tint, texid, wire, r3)
    }

    fn draw_batch_3d(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        nrm: Option<&[f32]>,
        idx: &[f32],
        insts: &[f32],
        mvp: &[f32],
        texid: i64,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String> {
        self.draw_batch_3d_impl(pos, uv, nrm, idx, insts, mvp, texid, r3)
    }

    fn clear_mesh_cache(&mut self) {
        self.clear_mesh_cache_impl();
    }
}
