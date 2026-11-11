//! src/physics/mod.rs — Этап 10 «Физика»: сферы + heightfield + плавучесть.
//! Zero-dep, f32, детерминизм: обход контактов строго по индексам (без хеш-
//! таблиц), фиксированные подшаги — те же входы дают побитово те же выходы.
//! HAL не знает про физику: солвер живёт здесь, Lisp — через нативы phys-*.

pub mod lisp;
pub mod solver;
pub mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use self::lisp::*;
#[allow(unused_imports)]
pub use self::solver::*;
#[allow(unused_imports)]
pub use self::types::*;
