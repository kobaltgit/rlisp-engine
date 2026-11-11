// src/gpu/resources.rs — Менеджер GPU-ресурсов (книга хэндлов и счётчики).

use std::collections::HashMap;

use super::types::{GpuBackend, GpuStats, Slot};

/// Выбор бэкенда для --wgpu: без фичи или без адаптера — честный откат на
/// программный растр (все скрипты продолжают работать). Выделено в функцию,
/// чтобы фолбэк-логика покрывалась тестом без устройства.
pub fn fallback_warning(probe: &Result<Box<dyn GpuBackend>, String>) -> Option<String> {
    match probe {
        Ok(_) => None,
        Err(e) => Some(format!("--wgpu: {} — откат на программный растр", e)),
    }
}

/// Книга GPU-ресурсов. Зеркалит реестр hal.tex (id = индекс+1, 0 — «нет»),
/// но не держит пиксели — только метаданные и счётчики. На wgpu-бэкенде
/// рядом с ней живёт HashMap<id, wgpu::Texture> (см. WgpuRenderer).
#[derive(Default)]
pub struct GpuResources {
    slots: HashMap<i64, Slot>,
    pub stats: GpuStats,
}

impl GpuResources {
    /// текстура создана/перезагружена под тем же id. Перезагрузка живого id
    /// замещает старое поколение (free + create) — счётчики честные: в конце
    /// tex_created - tex_freed == число живых хэндлов.
    pub fn register(&mut self, id: i64, bytes: usize) {
        match self.slots.remove(&id) {
            Some(old) => {
                self.stats.tex_freed += 1; // старое поколение замещено
                self.slots.insert(
                    id,
                    Slot {
                        gen: old.gen + 1,
                        bytes,
                    },
                );
            }
            None => {
                self.slots.insert(id, Slot { gen: 1, bytes });
            }
        }
        self.stats.tex_created += 1;
    }

    /// явное освобождение (texture-free / GPU-сторона). Возвращает true,
    /// если хэндл был жив.
    pub fn free(&mut self, id: i64) -> bool {
        if self.slots.remove(&id).is_some() {
            self.stats.tex_freed += 1;
            true
        } else {
            false
        }
    }

    pub fn live(&self) -> usize {
        self.slots.len()
    }

    /// суммарный байт пикселей живых текстур (рост = утечка)
    pub fn bytes(&self) -> usize {
        self.slots.values().map(|s| s.bytes).sum()
    }

    pub fn generation(&self, id: i64) -> u64 {
        self.slots.get(&id).map(|s| s.gen).unwrap_or(0)
    }

    /// hot-reload: освободить всё, КРОМЕ id из keep (id, на которые ссылаются
    /// persistent-значения). Возвращает число освобождённых.
    pub fn invalidate_except(&mut self, keep: &[i64]) -> usize {
        self.stats.reload_cycles += 1;
        let before = self.slots.len();
        self.slots.retain(|id, _| keep.contains(id));
        let freed = (before - self.slots.len()) as u64;
        self.stats.tex_freed += freed;
        self.stats.reload_invalidated += freed;
        freed as usize
    }

    pub fn reset(&mut self) {
        self.slots.clear();
        self.stats = GpuStats::default();
    }
}
