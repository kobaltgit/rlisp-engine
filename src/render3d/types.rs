// src/render3d/types.rs — Базовые структуры и типы fixed-function 3D конвейера.

use crate::gpu_3d::R3State;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TexFilter {
    Nearest,
    Bilinear,
}

/// Точечный источник света: положение в мире, цвет, радиус действия и затухание.
#[derive(Clone, Debug, PartialEq)]
pub struct PointLight {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub radius: f32,
    pub attenuation: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct Vtx {
    pub(crate) sx: f32,
    pub(crate) sy: f32,
    pub(crate) sz: f32,
    pub(crate) uq: f32,  // u/w
    pub(crate) vq: f32,  // v/w
    pub(crate) iw: f32,  // 1/w
    pub(crate) wxq: f32, // мир.x/w (этап 7: блик)
    pub(crate) wyq: f32, // мир.y/w
    pub(crate) wzq: f32, // мир.z/w
    pub(crate) aq: f32,  // альфа/w (этап 9: блендинг; 1/w → альфа 1)
    pub(crate) rq: f32,  // r/w (этап 9: per-vertex цвет)
    pub(crate) gq: f32,  // g/w
    pub(crate) bq: f32,  // b/w
    pub(crate) nxq: f32, // normal.x/w (сглаженный Фонг)
    pub(crate) nyq: f32, // normal.y/w
    pub(crate) nzq: f32, // normal.z/w
}

/// Треугольник прозрачной очереди: экранные вершины со ВСЕМИ атрибутами
/// (u/w v/w 1/w мир/w a/w r/w g/w b/w — уже спроецированы), мировая нормаль,
/// цвет-множитель и снимок состояния материалов на момент постановки.
pub(crate) struct TransTri {
    /// Видовая глубина центра (w_clip, больше = дальше) — ключ сортировки.
    pub depth: f32,
    pub pts: [Vtx; 3],
    pub nrm: [f32; 3],
    pub tex: i64,
    pub spec: f32,
    pub shininess: f32,
    pub fog: [f32; 4],
    pub filter: TexFilter,
    pub mipmaps: bool,
}

pub struct R3 {
    pub zbuf: Vec<f32>,
    pub w: usize,
    pub h: usize,
    pub light: [f32; 3], // направление К ИСТОЧНИКУ (нормировано)
    pub ambient: f32,
    pub diffuse: f32,
    pub drawn: u64,
    pub culled: u64,
    // — этап 7 «материалы»: состояние конвейера, как у fixed-function GPU —
    /// Порог alpha (cutout): 0 = выкл (дефолт); 0.1..1 — тексели текстурных
    /// граней с alpha < порога отбрасываются целиком (и цвет, и z-запись).
    pub alpha_test: f32,
    /// Сила блика Blinn-Phong: 0 = выкл (дефолт, старое поведение Lambert).
    pub spec: f32,
    /// Показатель степени блика (шершавость): больше — пятно блика уже.
    pub shininess: f32,
    /// Позиция камеры в мире (gpu3d-view) — нужен для half-vector блика.
    pub view: [f32; 3],
    /// Множители UV перед семплингом: (1,1) — как раньше (clamp к краю),
    /// (u,v) ≠ (1,1) — repeat через fract (тайлинг текстуры).
    pub tiling: [f32; 2],
    /// Туман: [r g b density]; density 0 = выкл (дефолт).
    pub fog: [f32; 4],
    /// Равномерная альфа материала (gpu3d-alpha-blend): 1 = непрозрачно
    /// (дефолт); < 1 — треугольники с итоговой альфой < 1 уходят в
    /// прозрачную очередь (src-alpha блендинг, сорт far→near, без z-записи).
    pub blend: f32,
    /// Прозрачная очередь: собирается непрозрачными вызовами, дренируется
    /// gpu3d-flush-alpha (сорт far→near — художник-порядок для крон,
    /// воды, облаков).
    pub(crate) transq: Vec<TransTri>,
    /// Режим затенения: false = flat (дефолт, ретро-стиль), true = smooth (сглаженный Фонг).
    pub smooth: bool,
    /// Режим фильтрации текстур: Nearest (дефолт, ретро-стиль) или Bilinear (сглаживание).
    pub tex_filter: TexFilter,
    /// Использование пирамиды мипмапов: false = только LOD 0 (дефолт), true = авто-выбор уровня LOD.
    pub tex_mipmaps: bool,
    // — этап 4 «освещение и тени»: теневая карта и точечные источники света —
    /// Активные точечные источники света сцены (фонари, снаряды, плазма).
    pub point_lights: Vec<PointLight>,
    /// Буфер теневой карты (глубина в координатах источника света).
    pub shadow_map: Vec<f32>,
    /// Ширина теневой карты.
    pub shadow_w: usize,
    /// Высота теневой карты.
    pub shadow_h: usize,
    /// Матрица вида-проекции источника света (Light View-Proj).
    pub light_matrix: [f32; 16],
    /// Флаг прохода теней: геометрия пишется только в shadow_map (depth-only).
    pub shadow_pass: bool,
    /// Флаг активности теней при основном рендере.
    pub shadow_enabled: bool,
    /// Смещение глубины для устранения артефактов (shadow acne).
    pub shadow_bias: f32,
    /// LUT тумана (этап 9.2): 257 узлов фактора 1-exp(-d·depth) на глубинах
    /// 0..FOG_SPAN (насыщение ~0.9975); между узлами линейная интерполяция —
    /// замена exp() на пиксель в растеризаторе. fog_lut_key — снимок
    /// r3.fog, по которому LUT построена (ленивая пересборка).
    pub(crate) fog_lut: [f32; 257],
    pub(crate) fog_lut_key: [f32; 4],
}

impl Default for R3 {
    fn default() -> Self {
        Self::new()
    }
}

impl R3 {
    pub fn new() -> R3 {
        R3 {
            zbuf: Vec::new(),
            w: 0,
            h: 0,
            light: [0.45, 0.8, 0.35],
            ambient: 0.35,
            diffuse: 0.75,
            drawn: 0,
            culled: 0,
            alpha_test: 0.0,
            spec: 0.0,
            shininess: 32.0,
            view: [0.0, 0.0, 0.0],
            tiling: [1.0, 1.0],
            fog: [0.7, 0.8, 0.9, 0.0],
            blend: 1.0,
            transq: Vec::new(),
            smooth: false,
            tex_filter: TexFilter::Nearest,
            tex_mipmaps: false,
            point_lights: Vec::new(),
            shadow_map: Vec::new(),
            shadow_w: 0,
            shadow_h: 0,
            light_matrix: [0.0; 16],
            shadow_pass: false,
            shadow_enabled: false,
            shadow_bias: 0.005,
            fog_lut: [0.0; 257],
            fog_lut_key: [0.7, 0.8, 0.9, 0.0],
        }
    }

    pub fn reset(&mut self, w: usize, h: usize) {
        if self.w != w || self.h != h || self.zbuf.len() != w * h {
            self.w = w;
            self.h = h;
            self.zbuf = vec![f32::INFINITY; w * h];
        } else {
            // каждый gpu3d-clear = новый кадр: z-буфер обязан обнуляться,
            // иначе прошлый кадр «экранирует» геометрию (провалы в небо)
            self.zbuf.fill(f32::INFINITY);
        }
    }

    pub fn state(&self) -> R3State {
        R3State {
            light: self.light,
            ambient: self.ambient,
            diffuse: self.diffuse,
            spec: self.spec,
            shininess: self.shininess,
            view: self.view,
            tiling: self.tiling,
            alpha_test: self.alpha_test,
            fog: self.fog,
            blend: self.blend,
            smooth: self.smooth,
            filter: self.tex_filter,
            mipmaps: self.tex_mipmaps,
        }
    }
}

/// Глубина насыщения LUT тумана: density·depth = 6 → фактор ≈ 0.9975.
pub(crate) const FOG_SPAN_DIV: f32 = 6.0;

/// Ленивая пересборка LUT тумана: вызывается раз на draw-вызов; тесты и
/// gpu3d-fog меняют r3.fog напрямую — снимок-ключ ловит любое изменение.
pub(crate) fn fog_lut_sync(r3: &mut R3) {
    if r3.fog_lut_key == r3.fog {
        return;
    }
    let d = r3.fog[3];
    let span = if d > 0.0 { FOG_SPAN_DIV / d } else { 1.0 };
    for (i, v) in r3.fog_lut.iter_mut().enumerate() {
        *v = super::shading::fog_factor(d, i as f32 * span / 256.0);
    }
    // последний узел — ровно 1: дальше span поверхность полностью в цвете
    // тумана (точная формула даёт 0.9975 — разница в четверть процента,
    // зато насыщение байт-в-байт совпадает с exp-путём)
    r3.fog_lut[256] = 1.0;
    r3.fog_lut_key = r3.fog;
}

/// Фактор тумана по LUT (с линейной интерполяцией между узлами).
#[inline]
pub(crate) fn fog_lut_lookup(lut: &[f32; 257], density: f32, depth: f32) -> f32 {
    if density <= 0.0 {
        return 0.0;
    }
    let scale = 256.0 * density / FOG_SPAN_DIV; // depth → узлы
    let t = (depth * scale).min(255.999);
    let i = t as usize;
    lut[i] + (lut[i + 1] - lut[i]) * (t - i as f32)
}

/// Инкрементальный сканлайн (этап 9.2): диапазон пикселей строки [lo_px,
/// hi_px], где все три edge-функции ≤ 0 (лицевая сторона после y-flip).
/// w — значения функций в опорной точке fx0, ax — производные по fx
/// (wi(fx) = w_i + ax_i·(fx − fx0)): каждое ограничение wi ≤ 0 — полулиния,
/// пересечение трёх — отрезок. Возвращает (from, to) включительно; from > to —
/// строка пуста. Набор пикселей совпадает с перебором bbox побитово.
pub(crate) fn row_span(w: [f32; 3], ax: [f32; 3], fx0: f32, lo_px: i64, hi_px: i64) -> (i64, i64) {
    let mut lo = f32::NEG_INFINITY;
    let mut hi = f32::INFINITY;
    for i in 0..3 {
        let a = ax[i];
        if a > 1e-9 {
            hi = hi.min(fx0 - w[i] / a);
        } else if a < -1e-9 {
            lo = lo.max(fx0 - w[i] / a);
        } else if w[i] > 0.0 {
            return (1, 0); // ребро параллельно строке и снаружи
        }
    }
    if lo.is_nan() || hi.is_nan() {
        return (1, 0);
    }
    let from = ((lo - 0.5).ceil() as i64).max(lo_px);
    let to = ((hi - 0.5).floor() as i64).min(hi_px);
    (from, to)
}
