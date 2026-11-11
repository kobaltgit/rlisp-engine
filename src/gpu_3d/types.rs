// src/gpu_3d/types.rs — Типы данных и структуры 3D-конвейера (R3State, Uniforms3DRaw, кэш мешей).

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
