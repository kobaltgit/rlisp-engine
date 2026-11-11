// src/hal/mod.rs — Уровень 1: Rust HAL (Hardware Abstraction Layer).
// По ТЗ §3: HAL не знает игровых понятий. Никаких draw-sprite / physics-step —
// только сырые команды: окно, растр, ввод, память, файлы, время.
//
// Рендер (этап 1 роадмапа): за фасадом HAL — два бэкенда (src/gpu.rs):
//   Backend::Software — программный растр (дефолт, ноль зависимостей):
//                       команды пишут прямо в fb (как в v0.1–v0.3);
//   Backend::Wgpu     — offscreen-рендер wgpu + readback в fb (фича `wgpu`).
// Lisp-нативы (gpu-clear / gpu-fill-rect / gpu-blit / gpu-blit-tex /
// gpu-present) не меняются — Lisp-уровень не знает о выборе бэкенда.

mod core;
mod lisp;
mod types;

#[cfg(test)]
mod tests;

pub use self::core::*;
pub use self::lisp::*;
pub use self::types::*;
