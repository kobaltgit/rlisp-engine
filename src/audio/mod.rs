// src/audio/mod.rs — Аудиосистема rlisp-engine: софтверный микшер, WAV кодек,
// процедурный чиптюн-синтезатор (SFX) и WinMM вывод для Windows.
// Ноль сторонних зависимостей.

pub mod codec;
pub mod device;
pub mod mixer;
pub mod synth;

#[cfg(test)]
mod tests;

pub use codec::*;
pub use device::*;
pub use mixer::*;
pub use synth::*;
