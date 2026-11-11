// src/gpu/types.rs — Базовые типы, трейт бэкенда и утилиты композита/референсов.

/// Фасад: активный GPU-бэкенд
pub enum Backend {
    Software,
    Wgpu(Box<dyn GpuBackend>),
}

/// То, что умеет каждый GPU-бэкенд за фасадом HAL. Профиль API — как у
/// программного растра v0.1 (gpu-clear/fill-rect/blit/present), плюс реестр
/// текстур и resize.
pub trait GpuBackend {
    /// человекочитаемое имя адаптера для баннера/логов
    fn adapter_info(&self) -> String;
    /// пересоздать offscreen-таргет под новый размер (window-create)
    fn resize(&mut self, w: usize, h: usize);
    /// gpu-clear: очистка цветом, alpha = 1 (полное покрытие кадра)
    fn clear(&mut self, r: u8, g: u8, b: u8);
    /// gpu-fill-rect
    #[allow(clippy::too_many_arguments)]
    fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, r: u8, g: u8, b: u8);
    /// gpu-blit: пиксели RGBA 1:1, alpha=0 пропускается
    fn blit(&mut self, x: i64, y: i64, w: usize, h: usize, bytes: &[u8]) -> Result<(), String>;
    /// gpu-blit-tex: текстурный спрайт (nearest, тинт), dest-прямоугольник
    /// считает Hal (те же формулы, что у программного растра)
    fn blit_tex(
        &mut self,
        id: i64,
        x: i64,
        y: i64,
        dw: i64,
        dh: i64,
        tint: [f32; 3],
    ) -> Result<(), String>;
    /// реестр текстур: загрузка/перезагрузка на бэкенд
    fn tex_upload(&mut self, id: i64, w: usize, h: usize, px: &[u8]);
    fn tex_free(&mut self, id: i64);
    /// дослать команды; при readback — композит кадра в fb.
    /// Возвращает (submit_ns, readback_ns) — профилирование --bench.
    fn flush(&mut self, fb: &mut [u8], w: usize, h: usize, readback: bool) -> (u128, u128);
    /// 3D-очистка кадра: цвет + аппаратный z-буфер
    fn clear_3d(&mut self, r: u8, g: u8, b: u8);
    /// Отрисовка 3D-меша на GPU
    #[allow(clippy::too_many_arguments)]
    fn draw_mesh_3d(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        rgba: Option<&[f32]>,
        nrm: Option<&[f32]>,
        idx: &[f32],
        model: &[f32],
        mvp: &[f32],
        tint: (f32, f32, f32),
        texid: i64,
        wire: bool,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String>;
    /// Отрисовка батча инстансов (аппаратный instancing на GPU)
    #[allow(clippy::too_many_arguments)]
    fn draw_batch_3d(
        &mut self,
        pos: &[f32],
        uv: Option<&[f32]>,
        nrm: Option<&[f32]>,
        idx: &[f32],
        insts: &[f32],
        mvp: &[f32],
        texid: i64,
        r3: &crate::gpu_3d::R3State,
    ) -> Result<u64, String>;
    /// сброс кэша мешей (при hot reload)
    fn clear_mesh_cache(&mut self) {}
}

/// Счётчики ресурсов — их assert-ят тесты на отсутствие утечек.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct GpuStats {
    pub tex_created: u64,
    pub tex_freed: u64,
    /// текстур, освобождённых при hot-reload (инвалидация)
    pub reload_invalidated: u64,
    /// hot-reload циклов, прошедших через invalidate
    pub reload_cycles: u64,
}

impl GpuStats {
    /// живые хэндлы: тест «нет утечек» = tex_created == tex_freed в конце
    pub fn tex_live(&self) -> i64 {
        self.tex_created as i64 - self.tex_freed as i64
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Slot {
    /// поколение: +1 при каждой перезагрузке того же id
    pub(crate) gen: u64,
    /// суммарный байт пикселей живой текстуры (рост = утечка)
    pub(crate) bytes: usize,
}

/// Композит GPU-кадра в fb по альфе-покрытию (вызывается из readback wgpu;
/// без фичи `wgpu` живёт для тестов композита):
/// пиксель с alpha==255 заменяет fb, прозрачный — оставляет fb как есть
/// (там, как правило, результат программного 3D-конвейера). `pad` — длина
/// строки в буфере GPU (выравнивание 256 байт), data — весь буфер.
#[cfg_attr(not(feature = "wgpu"), allow(dead_code))]
pub fn composite_readback(fb: &mut [u8], w: usize, h: usize, pad: usize, data: &[u8]) {
    for py in 0..h {
        let src = &data[py * pad..py * pad + w * 4];
        for px in 0..w {
            let s = px * 4;
            let a = src[s + 3] as u32;
            if a == 255 {
                let d = (py * w + px) * 4;
                fb[d] = src[s];
                fb[d + 1] = src[s + 1];
                fb[d + 2] = src[s + 2];
                fb[d + 3] = 255;
            } else if a > 0 {
                let d = (py * w + px) * 4;
                let inv_a = 255 - a;
                fb[d] = ((src[s] as u32 * a + fb[d] as u32 * inv_a) / 255) as u8;
                fb[d + 1] = ((src[s + 1] as u32 * a + fb[d + 1] as u32 * inv_a) / 255) as u8;
                fb[d + 2] = ((src[s + 2] as u32 * a + fb[d + 2] as u32 * inv_a) / 255) as u8;
                fb[d + 3] = 255;
            }
        }
    }
}

/// Собрать keep-список id (все Int в значениях) для invalidate_except:
/// рекурсивный обход persistent-значений (списки/векторы/таблицы).
pub fn collect_int_refs(v: &crate::interp::Value, out: &mut Vec<i64>, depth: u32) {
    if depth > 8 {
        return;
    }
    use crate::interp::Value;
    match v {
        Value::Int(i) => out.push(*i),
        Value::List(l) => {
            for x in l {
                collect_int_refs(x, out, depth + 1);
            }
        }
        Value::Cons(g) => {
            // Этап 2: Cons-цепочка — элементы на одной глубине, cdr — по цепочке
            // (итеративно, чтобы длинный список не съел лимит глубины); цикл — seen
            use std::collections::HashSet;
            let mut seen: HashSet<u32> = HashSet::new();
            let mut cur = *g;
            loop {
                if !seen.insert(cur.idx) {
                    break;
                }
                let (a, d) = crate::gc::pair_get(cur);
                collect_int_refs(&a, out, depth + 1);
                match d {
                    crate::interp::Value::Cons(next) => cur = next,
                    _ => break,
                }
            }
        }
        Value::Vector(r) => {
            for x in r.borrow().iter() {
                collect_int_refs(x, out, depth + 1);
            }
        }
        Value::Table(t) => {
            for x in t.borrow().values() {
                collect_int_refs(x, out, depth + 1);
            }
        }
        _ => {}
    }
}
