// src/gpu_3d/shaders.rs — Шейдеры WGSL для 3D-конвейера WebGPU.

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
