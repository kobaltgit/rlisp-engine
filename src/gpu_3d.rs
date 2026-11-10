// src/gpu_3d.rs — Аппаратный 3D-конвейер на WebGPU / WGSL (фича `wgpu`).
//
// Выполняет T&L, z-буфер, Blinn-Phong, экспоненциальный туман и аппаратный
// батчинг инстансов прямо на GPU.
// Zero-Core Hardcoding: Lisp-скрипты (render3d.lisp, island.lisp) вызывают те же
// gpu3d-примитивы, но под капотом данные рендерятся на GPU без пересчёта на CPU.

#[cfg(feature = "wgpu")]
use std::collections::HashMap;
#[cfg(feature = "wgpu")]
use std::num::NonZeroU64;
#[cfg(feature = "wgpu")]
use std::sync::Arc;

#[cfg(feature = "wgpu")]
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct MeshKey {
    pub pos_ptr: usize,
    pub pos_len: usize,
    pub idx_ptr: usize,
    pub idx_len: usize,
    pub uv_ptr: usize,
    pub rgba_ptr: usize,
    pub tint_bits: [u32; 3],
}

#[cfg(feature = "wgpu")]
pub struct CachedGpuMesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub ibuf: Arc<wgpu::Buffer>,
    pub v_bytes_len: u64,
    pub idx_count: u32,
    pub fingerprint: u64,
}

#[cfg(feature = "wgpu")]
pub fn calc_mesh_fingerprint(pos: &[f32], idx: &[f32], rgba: Option<&[f32]>) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let sample = |h: &mut u64, val: f32| {
        *h ^= val.to_bits() as u64;
        *h = h.wrapping_mul(0x100000001b3);
    };
    sample(&mut h, pos.len() as f32);
    sample(&mut h, idx.len() as f32);
    if !pos.is_empty() {
        sample(&mut h, pos[0]);
        sample(&mut h, pos[pos.len() / 4]);
        sample(&mut h, pos[pos.len() / 2]);
        sample(&mut h, pos[pos.len() * 3 / 4]);
        sample(&mut h, pos[pos.len() - 1]);
    }
    if let Some(r) = rgba {
        if !r.is_empty() {
            sample(&mut h, r[0]);
            sample(&mut h, r[r.len() / 2]);
            sample(&mut h, r[r.len() - 1]);
        }
    }
    h
}

#[derive(Clone, Debug)]
pub struct R3State {
    pub light: [f32; 3],
    pub ambient: f32,
    pub diffuse: f32,
    pub spec: f32,
    pub shininess: f32,
    pub view: [f32; 3],
    pub tiling: [f32; 2],
    pub alpha_test: f32,
    pub fog: [f32; 4],
    pub blend: f32,
    pub smooth: bool,
    pub filter: crate::render3d::TexFilter,
    pub mipmaps: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "wgpu"), allow(dead_code))]
pub struct Uniforms3DRaw {
    pub model: [f32; 16],
    pub mvp: [f32; 16],
    pub light_dir: [f32; 3],
    pub ambient: f32,
    pub diffuse: f32,
    pub spec: f32,
    pub shininess: f32,
    pub alpha_test: f32,
    pub camera_pos: [f32; 3],
    pub blend_opacity: f32,
    pub tiling: [f32; 2],
    pub fog_density: f32,
    pub _pad0: f32,
    pub fog_color: [f32; 3],
    pub _pad1: f32,
}

#[cfg(feature = "wgpu")]
pub const SHADER_3D: &str = r#"
struct Uniforms3D {
    model: mat4x4<f32>,
    mvp: mat4x4<f32>,
    light_dir: vec3<f32>,
    ambient: f32,
    diffuse: f32,
    spec: f32,
    shininess: f32,
    alpha_test: f32,
    camera_pos: vec3<f32>,
    blend_opacity: f32,
    tiling: vec2<f32>,
    fog_density: f32,
    _pad0: f32,
    fog_color: vec3<f32>,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms3D;
@group(1) @binding(0) var samp: sampler;
@group(1) @binding(1) var tex: texture_2d<f32>;

struct VertexInput {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) rgba: vec4<f32>,
};

struct InstanceInput {
    @location(3) inst_trans: vec3<f32>,
    @location(4) inst_scale: vec3<f32>,
    @location(5) inst_rot: vec3<f32>,
    @location(6) inst_tint: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) depth: f32,
};

fn rot_matrix(yaw: f32, pitch: f32, roll: f32) -> mat3x3<f32> {
    let cy = cos(yaw);
    let syw = sin(yaw);
    let cx = cos(pitch);
    let sxi = sin(pitch);
    let cz = cos(roll);
    let szi = sin(roll);

    let a00 = cy;
    let a01 = syw * sxi;
    let a02 = syw * cx;
    let a10 = 0.0;
    let a11 = cx;
    let a12 = -sxi;
    let a20 = -syw;
    let a21 = cy * sxi;
    let a22 = cy * cx;

    let r00 = a00 * cz - a01 * szi;
    let r01 = a00 * szi + a01 * cz;
    let r02 = a02;
    let r10 = a10 * cz - a11 * szi;
    let r11 = a10 * szi + a11 * cz;
    let r12 = a12;
    let r20 = a20 * cz - a21 * szi;
    let r21 = a20 * szi + a21 * cz;
    let r22 = a22;

    return mat3x3<f32>(
        vec3<f32>(r00, r10, r20),
        vec3<f32>(r01, r11, r21),
        vec3<f32>(r02, r12, r22)
    );
}

@vertex
fn vs_3d(v: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model_m = transpose(u.model);
    let mvp_m = transpose(u.mvp);

    let world = (model_m * vec4<f32>(v.pos, 1.0)).xyz;
    var clip = mvp_m * vec4<f32>(world, 1.0);
    clip.z = (clip.z + clip.w) * 0.5;

    out.clip_pos = clip;
    out.world_pos = world;
    out.uv = v.uv;
    out.color = v.rgba;
    out.depth = clip.w;
    return out;
}

@vertex
fn vs_3d_instanced(v: VertexInput, inst: InstanceInput) -> VertexOutput {
    var out: VertexOutput;
    let r = rot_matrix(inst.inst_rot.x, inst.inst_rot.y, inst.inst_rot.z);
    let scaled = v.pos * inst.inst_scale;
    let rotated = r * scaled;
    let world = rotated + inst.inst_trans;

    let mvp_m = transpose(u.mvp);
    var clip = mvp_m * vec4<f32>(world, 1.0);
    clip.z = (clip.z + clip.w) * 0.5;

    out.clip_pos = clip;
    out.world_pos = world;
    out.uv = v.uv;
    out.color = vec4<f32>(inst.inst_tint, 1.0);
    out.depth = clip.w;
    return out;
}

@fragment
fn fs_3d(in: VertexOutput) -> @location(0) vec4<f32> {
    let dpx = dpdx(in.world_pos);
    let dpy = dpdy(in.world_pos);
    var nrm = cross(dpy, dpx);
    let nl = length(nrm);
    if nl > 1e-6 {
        nrm = nrm / nl;
    } else {
        nrm = vec3<f32>(0.0, 1.0, 0.0);
    }

    let light = normalize(u.light_dir);
    let dot_nl = dot(nrm, light);
    var ndl = max(dot_nl, 0.0);
    if u.blend_opacity < 1.0 {
        ndl = abs(dot_nl);
    }
    let shade = clamp(u.ambient + u.diffuse * ndl, 0.0, 1.0);

    var c = in.color.rgb * shade;
    let alpha = in.color.a;

    if u.spec > 0.0 && dot_nl > 0.0 {
        let v = normalize(u.camera_pos - in.world_pos);
        let h = normalize(light + v);
        let ndh = max(dot(nrm, h), 0.0);
        let s = u.spec * pow(ndh, max(u.shininess, 1.0));
        c = min(c + vec3<f32>(s, s, s), vec3<f32>(1.0, 1.0, 1.0));
    }

    if u.fog_density > 0.0 {
        let fog_f = 1.0 - exp(-u.fog_density * in.depth);
        c = mix(c, u.fog_color, fog_f);
    }

    return vec4<f32>(c, alpha * u.blend_opacity);
}

@fragment
fn fs_3d_tex(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if u.tiling.x != 1.0 || u.tiling.y != 1.0 {
        uv = fract(uv * u.tiling);
    }

    let texel = textureSample(tex, samp, uv);

    let thr = select(0.01, u.alpha_test, u.alpha_test > 0.0);
    if texel.a <= thr {
        discard;
    }

    let dpx = dpdx(in.world_pos);
    let dpy = dpdy(in.world_pos);
    var nrm = cross(dpy, dpx);
    let nl = length(nrm);
    if nl > 1e-6 {
        nrm = nrm / nl;
    } else {
        nrm = vec3<f32>(0.0, 1.0, 0.0);
    }

    let light = normalize(u.light_dir);
    let dot_nl = dot(nrm, light);
    var ndl = max(dot_nl, 0.0);
    if u.blend_opacity < 1.0 {
        ndl = abs(dot_nl);
    }
    let shade = clamp(u.ambient + u.diffuse * ndl, 0.0, 1.0);

    var c = in.color.rgb * texel.rgb * shade;
    let alpha = in.color.a * texel.a;

    if u.spec > 0.0 && dot_nl > 0.0 {
        let v = normalize(u.camera_pos - in.world_pos);
        let h = normalize(light + v);
        let ndh = max(dot(nrm, h), 0.0);
        let s = u.spec * pow(ndh, max(u.shininess, 1.0));
        c = min(c + vec3<f32>(s, s, s), vec3<f32>(1.0, 1.0, 1.0));
    }

    if u.fog_density > 0.0 {
        let fog_f = 1.0 - exp(-u.fog_density * in.depth);
        c = mix(c, u.fog_color, fog_f);
    }

    return vec4<f32>(c, alpha * u.blend_opacity);
}

@fragment
fn fs_3d_wire(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, 1.0);
}
"#;

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
            std::slice::from_raw_parts(
                insts.as_ptr() as *const u8,
                std::mem::size_of_val(insts),
            )
        };
        if self.inst_buf_used + bytes.len() > self.inst_buf_cap {
            self.inst_buf_cap = (self.inst_buf_cap * 2).max(self.inst_buf_used + bytes.len() + 1024);
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
                    [carr[i * 4], carr[i * 4 + 1], carr[i * 4 + 2], carr[i * 4 + 3]]
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
        (entry.vbuf.clone(), entry.ibuf.clone(), entry.v_bytes_len, entry.idx_count)
    }

    pub fn clear_mesh_cache(&mut self) {
        self.mesh_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn тест_размер_uniforms_raw_кратно_16_байт() {
        assert_eq!(std::mem::size_of::<Uniforms3DRaw>(), 208);
        assert_eq!(std::mem::size_of::<Uniforms3DRaw>() % 16, 0);
    }

    #[test]
    fn тест_состояние_r3_конвертируется_в_uniforms() {
        let r3 = R3State {
            light: [0.1, 0.2, 0.3],
            ambient: 0.4,
            diffuse: 0.5,
            spec: 0.6,
            shininess: 32.0,
            view: [1.0, 2.0, 3.0],
            tiling: [2.0, 3.0],
            alpha_test: 0.5,
            fog: [0.7, 0.8, 0.9, 0.05],
            blend: 0.8,
            smooth: false,
            filter: crate::render3d::TexFilter::Nearest,
            mipmaps: false,
        };
        let raw = Uniforms3DRaw {
            model: [0.0; 16],
            mvp: [0.0; 16],
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
        assert_eq!(raw.ambient, 0.4);
        assert_eq!(raw.shininess, 32.0);
        assert_eq!(raw.fog_density, 0.05);
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn тест_шейдер_3d_wgsl_содержит_все_точки_входа() {
        assert!(SHADER_3D.contains("fn vs_3d("));
        assert!(SHADER_3D.contains("fn vs_3d_instanced("));
        assert!(SHADER_3D.contains("fn fs_3d("));
        assert!(SHADER_3D.contains("fn fs_3d_tex("));
        assert!(SHADER_3D.contains("fn fs_3d_wire("));
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn тест_mesh_fingerprint_детектирует_изменения_вершин() {
        let mut pos = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let idx = vec![0.0, 1.0, 2.0];
        let fp1 = calc_mesh_fingerprint(&pos, &idx, None);
        let fp2 = calc_mesh_fingerprint(&pos, &idx, None);
        assert_eq!(fp1, fp2);

        // Симуляция динамической волны воды: меняется Y вершины в середине
        pos[4] = 9.99;
        let fp3 = calc_mesh_fingerprint(&pos, &idx, None);
        assert_ne!(fp1, fp3);
    }
}
