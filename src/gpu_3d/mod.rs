// src/gpu_3d/mod.rs — Аппаратный 3D-конвейер на WebGPU / WGSL (фича `wgpu`).
//
// Выполняет T&L, z-буфер, Blinn-Phong, экспоненциальный туман и аппаратный
// батчинг инстансов прямо на GPU.
// Zero-Core Hardcoding: Lisp-скрипты (render3d.lisp, island.lisp) вызывают те же
// gpu3d-примитивы, но под капотом данные рендерятся на GPU без пересчёта на CPU.

pub mod pipeline;
pub mod shaders;
pub mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use self::pipeline::*;
#[allow(unused_imports)]
pub use self::shaders::*;
pub use self::types::*;
