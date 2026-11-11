#![allow(clippy::map_identity)]

// src/simd/mod.rs — SAL (SIMD Abstraction Layer) по ТЗ §4.3–4.5.
// Портативный API: vec-add / vec-dot / mat4-mul … с runtime-диспетчеризацией:
//   AVX-512 → AVX2 → SSE2 → скалярный fallback.
//
// Диспетчеризация двуступенчатая (по ТЗ §4.4 и честнее):
//   1) по флагам CPU (is_x86_feature_detected!),
//   2) калибровка micro-kernel на старте: на виртуалках флаг есть, а быстрого
//      исполнения может не быть — поэтому уровень выбирается по замеру.
// Скалярный fallback компилируется с авто-векторизацией LLVM (SSE2 baseline) —
// это и есть честный «без нашего SIMD» бенчмарк.

pub mod kernels;
pub mod lisp;
pub mod scanner;
pub mod types;

#[cfg(test)]
mod tests;

pub use self::kernels::*;
pub use self::lisp::*;
pub use self::scanner::*;
pub use self::types::*;
