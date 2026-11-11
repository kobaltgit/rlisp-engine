// src/jit/mod.rs — Этап 3: zero-dependency copy-and-patch JIT для числовых ядер.
//
// Идея (CPython-style): маленькие Lisp-лямбды, тело которых — чистая арифметика
// над числами (+,-,*,/,<,>,<=,>=,=, if, and, or, let, литералы, параметры),
// компилируются в линейный IR (JitIr), а затем в НАТИВНЫЙ машинный код x86-64
// из заранее собранных байтовых шаблонов с пропатченными константами и
// смещениями переходов (copy-and-patch). Код кладётся в RW-память (mmap),
// переводится mprotect'ом в RX (W^X) и вызывается через указатель на функцию.
//
// Модель гардов (стражей):
//   1) КОМПИЛЯЦИОННЫЕ: тело должно целиком состоять из поддержанных форм
//      над числами; любое неизвестное — деоптимизация (compile → None).
//   2) ВХОДНЫЕ: арность и типы аргументов (только Int/Float) проверяются
//      на каждый вызов в Rust-обёртке call_jitted(); провал → интерпретатор.
// Никаких частично-вычисленных результатов: либо JIT целиком, либо interp.
//
// Хард-требование репозитория — ноль зависимостей: никаких cranelift/llvm.
// mmap/mprotect — прямые Linux-сисвызвы через core::arch::asm! (x86_64-linux).
//
// Точная семантика и границы — в LIMITATIONS.md и docs/architecture.md.

pub mod compiler;
pub mod emit;
pub mod ir;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use self::compiler::{compile, Compiler};
pub use self::ir::IrOp;
pub use self::runtime::{call_jitted, guard_args, try_call_lambda, JitFn};
