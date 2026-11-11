// src/gpu/mod.rs — GPU-бэкенд за фасадом HAL (фасадный модуль).
//
// Философия соблюдена: Lisp-код НЕ ЗНАЕТ о бэкендах. Нативы gpu-clear /
// gpu-fill-rect / gpu-blit / gpu-blit-tex / gpu-present идут в Hal, а Hal
// направляет их в активный бэкенд (enum Backend + trait GpuBackend):
//
//   Backend::Software — программный растр (дефолт, НОЛЬ зависимостей):
//                       команды пишут прямо в Hal::fb, как в v0.1–v0.3;
//   Backend::Wgpu     — wgpu (только с фичей `wgpu`, опциональная зависимость):
//                       команды уходят в offscreen-текстуру (RGBA8), при
//                       flush/present кадр копируется с GPU (map-read) в
//                       Hal::fb — поэтому PNG/ASCII/редактор/MCP-вывод работают
//                       без единого изменения.
//
// Честно о покрытии (этап 1): wgpu закрывает 2D-растр (clear / fill-rect /
// blit / blit-tex / present). 3D-конвейер (render3d.rs: T&L, z-буфер, батчи)
// остаётся программным — он рисует в Hal::fb. Чтобы порядок художника совпадал
// с программным растром пиксель-в-пиксель, перед любой 3D-командой отложенные
// 2D-результаты wgpu сбрасываются в fb (Hal::flush_gpu), а flush композитит
// GPU-слой поверх fb по альфе-покрытию (см. composite_readback).
//
// Ресурсы: GpuResources — «книга хэндлов» без устройства (счётчики для тестов
// на утечки: 100 create/reload циклов без роста). Hot-reload вызывает
// Hal::invalidate_textures_except — старые текстуры освобождаются (в v0.3 они
// утекали до рестарта, см. README «Честные отличия» — теперь исправлено).

pub mod resources;
pub mod types;

#[cfg(feature = "wgpu")]
mod wgpu_quad;
#[cfg(feature = "wgpu")]
mod wgpu_2d;
#[cfg(feature = "wgpu")]
mod wgpu_3d;
#[cfg(feature = "wgpu")]
pub mod wgpu_backend;

#[cfg(test)]
mod tests;

pub use self::resources::*;
pub use self::types::*;
#[cfg(feature = "wgpu")]
#[allow(unused_imports)]
pub use self::wgpu_backend::*;

/// Создать wgpu-бэкенд. Без фичи `wgpu` — понятная ошибка (откат на софт).
#[cfg(not(feature = "wgpu"))]
pub fn create_wgpu_backend(_w: usize, _h: usize) -> Result<Box<dyn GpuBackend>, String> {
    Err("бинарь собран без GPU-бэкенда: cargo build --release --features wgpu".into())
}

#[cfg(feature = "wgpu")]
pub fn create_wgpu_backend(w: usize, h: usize) -> Result<Box<dyn GpuBackend>, String> {
    Ok(Box::new(wgpu_backend::WgpuRenderer::new(w.max(8), h.max(8))?))
}
