// src/render3d.rs — Уровень 1 (HAL): «GPU» фиксированного конвейера.
// Философия ТЗ соблюдена: Rust знает только треугольники, глубину и свет —
// ровно то, что в «железе» делает GPU (T&L эпохи GeForce 256 + z-буфер).
// Камера, проекции, матрицы моделей, меши, сцена, игра — Lisp
// (assets/lisp/render3d.lisp, game3d.lisp).
//
// Примитивы:
//   gpu3d-clear     — очистка цвета + z-буфера (+ сброс статистики кадра)
//   gpu3d-light     — направленный свет и фоновая освещённость (fixed-function)
//                     этап 7: + блик Blinn-Phong (spec, shininess — опц. аргументы)
//   gpu3d-view      — позиция камеры в мире (для half-vector блика)
//   gpu3d-mat-tiling— множители UV перед семплингом (repeat-текстуры)
//   gpu3d-alpha-test— порог alpha (cutout): тексель с alpha < порога отбрасывается
//                     ДО записи в z-буфер (семантика discard у GPU)
//   gpu3d-fog       — экспоненциальный туман: 1 - exp(-density * глубина),
//                     смешение ПОСЛЕ шейдинга, только для непрозрачных поверхностей
//   gpu3d-state     — таблица текущего состояния конвейера (свет/блик/туман/…)
//   gpu3d-mesh      — сетка: T&L → near-clip → backface cull → проекция →
//                     z-буфер → flat-затенение (Lambert)
//   gpu3d-mesh-tex  — то же + текстурирование: перспектива-корректный UV
//                     (интерполяция u/w и 1/w, как в железе), nearest-семплинг
//   gpu3d-batch     — БАТЧ-ВЕРШИННИК: много инстансов одним draw call
//                     (12 f32 на инстанс: t.xyz s.xyz yaw pitch roll rgb)
//   gpu3d-batch-tex — батч + текстура + тинт из инстанса
//   gpu3d-wire      — линии (пары индексов) с clip по near-плоскости
//   gpu3d-stats     — (нарисовано треугольников, отброшено куллингом)
//
// Глубина: интерполируется NDC-z линейно в экранных координатах — это
// корректная перспектива для глубины (так же делает железный depth buffer).
// UV: u/w и 1/w аффинны в экранном пространстве → семплинг корректен и после
// клиппинга (атрибуты режутся в однородных координатах вместе с вершиной).

use crate::hal::Hal;
use crate::interp::{env_define, need_args, num_val, Gc, Value};
use crate::simd::to_f32s;
use std::cell::RefCell;
use std::rc::Rc;

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

/// Глубина насыщения LUT тумана: density·depth = 6 → фактор ≈ 0.9975.
const FOG_SPAN_DIV: f32 = 6.0;

/// Ленивая пересборка LUT тумана: вызывается раз на draw-вызов; тесты и
/// gpu3d-fog меняют r3.fog напрямую — снимок-ключ ловит любое изменение.
pub(crate) fn fog_lut_sync(r3: &mut R3) {
    if r3.fog_lut_key == r3.fog {
        return;
    }
    let d = r3.fog[3];
    let span = if d > 0.0 { FOG_SPAN_DIV / d } else { 1.0 };
    for (i, v) in r3.fog_lut.iter_mut().enumerate() {
        *v = fog_factor(d, i as f32 * span / 256.0);
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

    pub fn state(&self) -> crate::gpu_3d::R3State {
        crate::gpu_3d::R3State {
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

fn col(x: f64) -> u8 {
    (x * 255.0).round().clamp(0.0, 255.0) as u8
}

// ——— этап 7 «материалы»: чистые функции ядра шейдинга (покрыты юнит-тестами) ———

/// Фактор экспоненциального тумана: 1 - exp(-density * глубина).
/// density 0 или глубина 0 → 0 (без тумана); растёт монотонно к 1.
#[inline]
pub(crate) fn fog_factor(density: f32, depth: f32) -> f32 {
    (1.0 - (-(density.max(0.0) * depth.max(0.0))).exp()).clamp(0.0, 1.0)
}

/// Предикат alpha-test: тексель с alpha < порога отбрасывается.
/// threshold <= 0 — тест выключен (всё проходит); граница включительная:
/// alpha/255 == порог — тексель ОСТАЁТСЯ (как `if (a >= cutoff) discard` нет:
/// у GPU обычно `alpha < cutoff → discard`, т.е. равный порогу проходит).
#[inline]
pub(crate) fn alpha_keep(a: u8, threshold: f32) -> bool {
    threshold <= 0.0 || (a as f32 / 255.0) >= threshold
}

/// Координата семплинга с учётом тайлинга: tiling == 1 — координата как есть
/// (старое поведение: clamp к краю текстуры), иначе repeat через fract.
#[inline]
pub(crate) fn tiled_coord(u: f32, tiling: f32) -> f32 {
    if tiling == 1.0 {
        u
    } else {
        (u * tiling).fract()
    }
}

/// Интенсивность блика Blinn-Phong: strength * max(N·H, 0)^shininess.
#[inline]
pub(crate) fn spec_highlight(ndh: f32, shininess: f32, strength: f32) -> f32 {
    if strength <= 0.0 || ndh <= 0.0 {
        0.0
    } else {
        strength * ndh.powf(shininess.max(1.0))
    }
}

/// Разбор аргументов gpu3d-light с честным фолбэком:
/// (dx dy dz ambient [diffuse [spec [shininess]]]) —
/// отсутствующие опциональные аргументы сохраняют прежнее состояние
/// (старые вызовы с 4-5 аргументами работают байт-в-байт как раньше).
pub(crate) fn apply_light_args(r3: &mut R3, args: &[Value]) -> Result<(), String> {
    need_args(args, 4)?;
    r3.light = [
        num_val(&args[0])? as f32,
        num_val(&args[1])? as f32,
        num_val(&args[2])? as f32,
    ];
    r3.ambient = num_val(&args[3])? as f32;
    if let Some(v) = args.get(4) {
        r3.diffuse = num_val(v)? as f32;
    }
    if let Some(v) = args.get(5) {
        r3.spec = num_val(v)?.max(0.0) as f32;
    }
    if let Some(v) = args.get(6) {
        r3.shininess = num_val(v)?.max(1.0) as f32;
    }
    Ok(())
}

#[inline]
fn edge(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

#[inline]
fn cross3(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn dot3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Вершина клип-пространства: [x y z w] + атрибуты u/w, v/w, 1/w, wx/w, wy/w,
/// wz/w, a/w, r/w, g/w, b/w (этап 9: per-vertex RGBA для mesh-vc).
/// Клиппинг Sutherland–Hodgman по плоскости z + w >= 0 (near).
/// Атрибуты (u/w, 1/w, мир/w, a/w, rgb/w) линейны вдоль ребра → интерполируются
/// тем же t. Мир/w нужен per-pixel half-vector блику, a/w и rgb/w — прозрачности
/// и цветовым градиентам (вода, небо).
fn clip_near6(poly: &[[f32; 17]], out: &mut Vec<[f32; 17]>) {
    out.clear();
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let da = a[2] + a[3];
        let db = b[2] + b[3];
        let a_in = da >= 0.0;
        let b_in = db >= 0.0;
        if a_in {
            out.push(a);
        }
        if a_in != b_in {
            let t = da / (da - db);
            let mut v = [0.0f32; 17];
            for k in 0..17 {
                v[k] = a[k] + (b[k] - a[k]) * t;
            }
            out.push(v);
        }
    }
}

fn clip_near_pos(poly: &[[f32; 4]], out: &mut Vec<[f32; 4]>) {
    out.clear();
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let da = a[2] + a[3];
        let db = b[2] + b[3];
        let a_in = da >= 0.0;
        let b_in = db >= 0.0;
        if a_in {
            out.push(a);
        }
        if a_in != b_in {
            let t = da / (da - db);
            let mut v = [0.0f32; 4];
            for k in 0..4 {
                v[k] = a[k] + (b[k] - a[k]) * t;
            }
            out.push(v);
        }
    }
}

/// Семплирование карты теней с Percentage Closer Filtering (PCF 2x2).
/// Возвращает коэффициент освещённости: 0.0 (полная тень) .. 1.0 (полный свет).
#[inline]
pub fn shadow_sample(r3: &R3, wx: f32, wy: f32, wz: f32, bias: f32) -> f32 {
    if !r3.shadow_enabled || r3.shadow_map.is_empty() || r3.shadow_w == 0 || r3.shadow_h == 0 {
        return 1.0;
    }
    let m = &r3.light_matrix;
    let lx = m[0] * wx + m[1] * wy + m[2] * wz + m[3];
    let ly = m[4] * wx + m[5] * wy + m[6] * wz + m[7];
    let lz = m[8] * wx + m[9] * wy + m[10] * wz + m[11];
    let lw = m[12] * wx + m[13] * wy + m[14] * wz + m[15];
    if lw <= 1e-6 {
        return 1.0;
    }
    let inv_lw = 1.0 / lw;
    let nx = lx * inv_lw;
    let ny = ly * inv_lw;
    let nz = lz * inv_lw;
    if nx < -1.0 || nx > 1.0 || ny < -1.0 || ny > 1.0 || nz < -1.0 || nz > 1.0 {
        return 1.0;
    }
    let sw = r3.shadow_w;
    let sh = r3.shadow_h;
    let sx = (nx * 0.5 + 0.5) * (sw as f32 - 1.0);
    let sy = (1.0 - (ny * 0.5 + 0.5)) * (sh as f32 - 1.0);
    let sz = nz;

    let x0 = (sx.floor() as usize).min(sw - 1);
    let y0 = (sy.floor() as usize).min(sh - 1);
    let x1 = (x0 + 1).min(sw - 1);
    let y1 = (y0 + 1).min(sh - 1);

    let fx = sx - sx.floor();
    let fy = sy - sy.floor();

    let check = |x: usize, y: usize| -> f32 {
        let depth = r3.shadow_map[y * sw + x];
        if sz - bias > depth {
            0.0
        } else {
            1.0
        }
    };

    let s00 = check(x0, y0);
    let s10 = check(x1, y0);
    let s01 = check(x0, y1);
    let s11 = check(x1, y1);

    let top = s00 * (1.0 - fx) + s10 * fx;
    let bottom = s01 * (1.0 - fx) + s11 * fx;
    top * (1.0 - fy) + bottom * fy
}

pub fn shadow_begin(r3: &mut R3, matrix: [f32; 16], w: usize, h: usize, bias: Option<f32>) {
    r3.light_matrix = matrix;
    r3.shadow_w = w;
    r3.shadow_h = h;
    let total = w * h;
    if r3.shadow_map.len() != total {
        r3.shadow_map = vec![f32::INFINITY; total];
    } else {
        r3.shadow_map.fill(f32::INFINITY);
    }
    r3.shadow_pass = true;
    r3.shadow_enabled = true;
    if let Some(b) = bias {
        r3.shadow_bias = b;
    }
}

pub fn shadow_end(r3: &mut R3) {
    r3.shadow_pass = false;
}

pub fn shadow_clear(r3: &mut R3) {
    r3.shadow_pass = false;
    r3.shadow_enabled = false;
    r3.shadow_map.clear();
    r3.shadow_w = 0;
    r3.shadow_h = 0;
}

pub(crate) fn compute_light_matrix(
    light_dir: [f32; 3],
    target: [f32; 3],
    size: f32,
    near: f32,
    far: f32,
) -> [f32; 16] {
    let len = dot3(&light_dir, &light_dir).sqrt().max(1e-6);
    let ld = [light_dir[0] / len, light_dir[1] / len, light_dir[2] / len];
    let dist = 0.5 * (near + far);
    let eye = [
        target[0] + ld[0] * dist,
        target[1] + ld[1] * dist,
        target[2] + ld[2] * dist,
    ];
    let up = if ld[0].abs() < 0.1 && ld[2].abs() < 0.1 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let f = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let flen = dot3(&f, &f).sqrt().max(1e-6);
    let f = [f[0] / flen, f[1] / flen, f[2] / flen];

    let r = cross3(&f, &up);
    let rlen = dot3(&r, &r).sqrt().max(1e-6);
    let r = [r[0] / rlen, r[1] / rlen, r[2] / rlen];

    let u = cross3(&r, &f);

    let view = [
        r[0],
        r[1],
        r[2],
        -dot3(&r, &eye),
        u[0],
        u[1],
        u[2],
        -dot3(&u, &eye),
        -f[0],
        -f[1],
        -f[2],
        dot3(&f, &eye),
        0.0,
        0.0,
        0.0,
        1.0,
    ];

    let rl = 2.0 * size;
    let tb = 2.0 * size;
    let fn_dist = far - near;
    let proj = [
        2.0 / rl,
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 / tb,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / fn_dist,
        -(far + near) / fn_dist,
        0.0,
        0.0,
        0.0,
        1.0,
    ];

    let mut out = [0.0f32; 16];
    for row in 0..4 {
        for col in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += proj[row * 4 + k] * view[k * 4 + col];
            }
            out[row * 4 + col] = sum;
        }
    }
    out
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_point_lights(
    lights: &[PointLight],
    view: &[f32; 3],
    wx: f32,
    wy: f32,
    wz: f32,
    nrm: &[f32; 3],
    two_sided: bool,
    spec_on: bool,
    shininess: f32,
    spec: f32,
) -> ([f32; 3], [f32; 3]) {
    let mut diff = [0.0f32; 3];
    let mut spec_acc = [0.0f32; 3];
    if lights.is_empty() {
        return (diff, spec_acc);
    }
    let v_norm = if spec_on {
        let v = [view[0] - wx, view[1] - wy, view[2] - wz];
        let vl = dot3(&v, &v).sqrt().max(1e-9);
        [v[0] / vl, v[1] / vl, v[2] / vl]
    } else {
        [0.0, 0.0, 0.0]
    };

    for pl in lights {
        let dx = pl.pos[0] - wx;
        let dy = pl.pos[1] - wy;
        let dz = pl.pos[2] - wz;
        let d2 = dx * dx + dy * dy + dz * dz;
        let r2 = pl.radius * pl.radius;
        if d2 < r2 && d2 > 1e-6 {
            let dist = d2.sqrt();
            let att = ((1.0 - dist / pl.radius).max(0.0) / (1.0 + 0.09 * dist + 0.032 * d2))
                * pl.attenuation;
            let lx = dx / dist;
            let ly = dy / dist;
            let lz = dz / dist;
            let mut p_ndl = nrm[0] * lx + nrm[1] * ly + nrm[2] * lz;
            if two_sided && p_ndl < 0.0 {
                p_ndl = -p_ndl;
            }
            let diff_f = att * p_ndl.max(0.0);
            diff[0] += pl.color[0] * diff_f;
            diff[1] += pl.color[1] * diff_f;
            diff[2] += pl.color[2] * diff_f;

            if spec_on {
                let hv = [lx + v_norm[0], ly + v_norm[1], lz + v_norm[2]];
                let hl = dot3(&hv, &hv).sqrt().max(1e-9);
                let sp = spec_highlight(
                    dot3(nrm, &[hv[0] / hl, hv[1] / hl, hv[2] / hl]),
                    shininess,
                    spec,
                );
                spec_acc[0] += pl.color[0] * sp * att;
                spec_acc[1] += pl.color[1] * sp * att;
                spec_acc[2] += pl.color[2] * sp * att;
            }
        }
    }
    (diff, spec_acc)
}

fn raster_shadow_tris(r3: &mut R3, clip: &[f32], idx: &[f32]) -> Result<u64, String> {
    let sw = r3.shadow_w;
    let sh = r3.shadow_h;
    if sw == 0 || sh == 0 || r3.shadow_map.len() != sw * sh {
        return Ok(0);
    }
    let n = clip.len() / 4;
    let mut poly_buf: Vec<[f32; 4]> = Vec::with_capacity(8);
    let mut clip_buf: Vec<[f32; 4]> = Vec::with_capacity(8);
    let mut drawn = 0u64;

    for f in 0..idx.len() / 3 {
        let mut tri = [0usize; 3];
        for k in 0..3 {
            let vi = idx[f * 3 + k];
            let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
            if !idx_ok {
                return Err(format!("gpu3d-shadow: индекс за границей: {}", vi));
            }
            tri[k] = vi as usize;
        }

        poly_buf.clear();
        for &t in &tri {
            poly_buf.push([
                clip[t * 4],
                clip[t * 4 + 1],
                clip[t * 4 + 2],
                clip[t * 4 + 3],
            ]);
        }

        let poly: &[[f32; 4]] = if poly_buf.iter().any(|v| v[3] < 1e-6) {
            clip_near_pos(&poly_buf, &mut clip_buf);
            if clip_buf.len() < 3 {
                continue;
            }
            &clip_buf
        } else {
            &poly_buf
        };

        let mut pts = Vec::with_capacity(poly.len());
        for v in poly {
            if v[3] <= 1e-6 {
                continue;
            }
            let inv_w = 1.0 / v[3];
            let nx = v[0] * inv_w;
            let ny = v[1] * inv_w;
            let nz = v[2] * inv_w;
            pts.push((
                (nx * 0.5 + 0.5) * sw as f32,
                (1.0 - (ny * 0.5 + 0.5)) * sh as f32,
                nz,
            ));
        }
        if pts.len() < 3 {
            continue;
        }

        for kk in 1..pts.len() - 1 {
            let (p0, p1, p2) = (&pts[0], &pts[kk], &pts[kk + 1]);
            let area = edge(p0.0, p0.1, p1.0, p1.1, p2.0, p2.1);
            if area.abs() < 1e-6 {
                continue;
            }

            let minx = (p0.0.min(p1.0).min(p2.0).floor() as i64).max(0);
            let maxx = (p0.0.max(p1.0).max(p2.0).ceil() as i64).min(sw as i64 - 1);
            let miny = (p0.1.min(p1.1).min(p2.1).floor() as i64).max(0);
            let maxy = (p0.1.max(p1.1).max(p2.1).ceil() as i64).min(sh as i64 - 1);
            if minx > maxx || miny > maxy {
                continue;
            }

            for py in miny..=maxy {
                let row = py as usize * sw;
                let fy = py as f32 + 0.5;
                for px in minx..=maxx {
                    let fx = px as f32 + 0.5;
                    let w0 = edge(p1.0, p1.1, p2.0, p2.1, fx, fy);
                    let w1 = edge(p2.0, p2.1, p0.0, p0.1, fx, fy);
                    let w2 = edge(p0.0, p0.1, p1.0, p1.1, fx, fy);
                    if (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
                        || (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0)
                    {
                        let sum = w0 + w1 + w2;
                        let z = if sum.abs() < 1e-9 {
                            p0.2
                        } else {
                            (w0 * p0.2 + w1 * p1.2 + w2 * p2.2) / sum
                        };
                        let pi = row + px as usize;
                        if z < r3.shadow_map[pi] {
                            r3.shadow_map[pi] = z;
                        }
                    }
                }
            }
            drawn += 1;
        }
    }
    Ok(drawn)
}

#[derive(Clone, Copy)]
pub(crate) struct Vtx {
    sx: f32,
    sy: f32,
    sz: f32,
    uq: f32,  // u/w
    vq: f32,  // v/w
    iw: f32,  // 1/w
    wxq: f32, // мир.x/w (этап 7: блик)
    wyq: f32, // мир.y/w
    wzq: f32, // мир.z/w
    aq: f32,  // альфа/w (этап 9: блендинг; 1/w → альфа 1)
    rq: f32,  // r/w (этап 9: per-vertex цвет)
    gq: f32,  // g/w
    bq: f32,  // b/w
    nxq: f32, // normal.x/w (сглаженный Фонг)
    nyq: f32, // normal.y/w
    nzq: f32, // normal.z/w
}

fn project6(c: &[f32; 17], w: usize, h: usize) -> Option<Vtx> {
    if c[3] <= 1e-6 {
        return None;
    }
    let nx = c[0] / c[3];
    let ny = c[1] / c[3];
    let nz = c[2] / c[3];
    Some(Vtx {
        sx: (nx * 0.5 + 0.5) * w as f32,
        sy: (1.0 - (ny * 0.5 + 0.5)) * h as f32, // y вниз, как в фреймбуфере
        sz: nz,
        uq: c[4],  // u/w
        vq: c[5],  // v/w
        iw: c[6],  // 1/w
        wxq: c[7], // мир/w
        wyq: c[8],
        wzq: c[9],
        aq: c[10],
        rq: c[11],
        gq: c[12],
        bq: c[13],
        nxq: c[14],
        nyq: c[15],
        nzq: c[16],
    })
}

fn project(c: &[f32; 4], w: usize, h: usize) -> Option<Vtx> {
    if c[3] <= 1e-6 {
        return None;
    }
    let inv = 1.0 / c[3];
    let nx = c[0] * inv;
    let ny = c[1] * inv;
    let nz = c[2] * inv;
    Some(Vtx {
        sx: (nx * 0.5 + 0.5) * w as f32,
        sy: (1.0 - (ny * 0.5 + 0.5)) * h as f32,
        sz: nz,
        uq: 0.0,
        vq: 0.0,
        iw: inv,
        wxq: 0.0,
        wyq: 0.0,
        wzq: 0.0,
        aq: inv,
        rq: 0.0,
        gq: 0.0,
        bq: 0.0,
        nxq: 0.0,
        nyq: 0.0,
        nzq: 0.0,
    })
}

enum Prim {
    Tri,
    Wire,
}

#[derive(Clone)]
pub(crate) struct TexRef {
    w: usize,
    h: usize,
    px: Vec<u8>,
    mips: Vec<crate::hal::MipLevel>,
    id: i64,
}

fn tex_lookup(hal: &Hal, id: i64) -> Result<Option<TexRef>, String> {
    if id == 0 {
        return Ok(None);
    }
    let t = hal
        .tex
        .get((id - 1) as usize)
        .ok_or_else(|| format!("gpu3d: нет текстуры {}", id))?;
    if t.w == 0 {
        return Err("gpu3d: текстура освобождена".into());
    }
    let mips = if t.mips.is_empty() && t.w > 0 && t.h > 0 {
        let mut tmp = t.clone();
        tmp.generate_mips();
        tmp.mips
    } else {
        t.mips.clone()
    };
    Ok(Some(TexRef {
        w: t.w,
        h: t.h,
        px: t.px.clone(),
        mips,
        id,
    }))
}

/// Билинейная фильтрация: взвешенное смешивание 4 соседних текселей по дробным координатам fx, fy.
#[inline]
#[allow(dead_code)]
pub(crate) fn sample_bilinear(mip: &crate::hal::MipLevel, u: f32, v: f32) -> [u8; 4] {
    sample_bilinear_raw(&mip.px, mip.w, mip.h, u, v)
}

#[inline]
pub(crate) fn sample_bilinear_raw(px: &[u8], w: usize, h: usize, u: f32, v: f32) -> [u8; 4] {
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return [255, 255, 255, 255];
    }
    if w == 1 && h == 1 {
        return [px[0], px[1], px[2], px[3]];
    }

    let u_coord = u.clamp(0.0, 1.0) * (w - 1) as f32;
    let v_coord = v.clamp(0.0, 1.0) * (h - 1) as f32;

    let x0 = u_coord.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let fx = u_coord - x0 as f32;

    let y0 = v_coord.floor() as usize;
    let y1 = (y0 + 1).min(h - 1);
    let fy = v_coord - y0 as f32;

    let fx_int = (fx * 256.0).round().clamp(0.0, 256.0) as u32;
    let fy_int = (fy * 256.0).round().clamp(0.0, 256.0) as u32;

    let w00 = (256 - fx_int) * (256 - fy_int);
    let w10 = fx_int * (256 - fy_int);
    let w01 = (256 - fx_int) * fy_int;
    let w11 = fx_int * fy_int;

    let idx00 = (y0 * w + x0) * 4;
    let idx10 = (y0 * w + x1) * 4;
    let idx01 = (y1 * w + x0) * 4;
    let idx11 = (y1 * w + x1) * 4;

    [
        ((px[idx00] as u32 * w00
            + px[idx10] as u32 * w10
            + px[idx01] as u32 * w01
            + px[idx11] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 1] as u32 * w00
            + px[idx10 + 1] as u32 * w10
            + px[idx01 + 1] as u32 * w01
            + px[idx11 + 1] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 2] as u32 * w00
            + px[idx10 + 2] as u32 * w10
            + px[idx01 + 2] as u32 * w01
            + px[idx11 + 2] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 3] as u32 * w00
            + px[idx10 + 3] as u32 * w10
            + px[idx01 + 3] as u32 * w01
            + px[idx11 + 3] as u32 * w11
            + 32768)
            >> 16) as u8,
    ]
}

#[inline]
#[allow(dead_code)]
pub(crate) fn sample_nearest(mip: &crate::hal::MipLevel, u: f32, v: f32) -> [u8; 4] {
    sample_nearest_raw(&mip.px, mip.w, mip.h, u, v)
}

#[inline]
pub(crate) fn sample_nearest_raw(px: &[u8], w: usize, h: usize, u: f32, v: f32) -> [u8; 4] {
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return [255, 255, 255, 255];
    }
    let x = (u.clamp(0.0, 0.99999) * w as f32) as usize;
    let y = (v.clamp(0.0, 0.99999) * h as f32) as usize;
    let ti = (y * w + x) * 4;
    [px[ti], px[ti + 1], px[ti + 2], px[ti + 3]]
}

pub(crate) struct TexGradients {
    pub d_uq_dx: f32,
    pub d_uq_dy: f32,
    pub d_vq_dx: f32,
    pub d_vq_dy: f32,
    pub d_iw_dx: f32,
    pub d_iw_dy: f32,
    pub tw: f32,
    pub th: f32,
    pub tu: f32,
    pub tv: f32,
}

impl TexGradients {
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        p0: &Vtx,
        p1: &Vtx,
        p2: &Vtx,
        sum: f32,
        tw: usize,
        th: usize,
        tu: f32,
        tv: f32,
    ) -> Option<Self> {
        if sum.abs() < 1e-9 || tw == 0 || th == 0 {
            return None;
        }
        let d_w0_dx = p2.sy - p1.sy;
        let d_w0_dy = p1.sx - p2.sx;
        let d_w1_dx = p0.sy - p2.sy;
        let d_w1_dy = p2.sx - p0.sx;
        let d_w2_dx = p1.sy - p0.sy;
        let d_w2_dy = p0.sx - p1.sx;

        let inv_sum = 1.0 / sum;
        let d_uq_dx = (d_w0_dx * p0.uq + d_w1_dx * p1.uq + d_w2_dx * p2.uq) * inv_sum;
        let d_uq_dy = (d_w0_dy * p0.uq + d_w1_dy * p1.uq + d_w2_dy * p2.uq) * inv_sum;
        let d_vq_dx = (d_w0_dx * p0.vq + d_w1_dx * p1.vq + d_w2_dx * p2.vq) * inv_sum;
        let d_vq_dy = (d_w0_dy * p0.vq + d_w1_dy * p1.vq + d_w2_dy * p2.vq) * inv_sum;
        let d_iw_dx = (d_w0_dx * p0.iw + d_w1_dx * p1.iw + d_w2_dx * p2.iw) * inv_sum;
        let d_iw_dy = (d_w0_dy * p0.iw + d_w1_dy * p1.iw + d_w2_dy * p2.iw) * inv_sum;

        Some(TexGradients {
            d_uq_dx,
            d_uq_dy,
            d_vq_dx,
            d_vq_dy,
            d_iw_dx,
            d_iw_dy,
            tw: tw as f32,
            th: th as f32,
            tu,
            tv,
        })
    }

    #[inline]
    pub fn select_lod(&self, uq: f32, vq: f32, iwp: f32, max_lod: usize) -> usize {
        let iwp_sq = (iwp * iwp).max(1e-12);
        let du_dx = (self.d_uq_dx * iwp - uq * self.d_iw_dx) / iwp_sq * self.tu;
        let dv_dx = (self.d_vq_dx * iwp - vq * self.d_iw_dx) / iwp_sq * self.tv;
        let du_dy = (self.d_uq_dy * iwp - uq * self.d_iw_dy) / iwp_sq * self.tu;
        let dv_dy = (self.d_vq_dy * iwp - vq * self.d_iw_dy) / iwp_sq * self.tv;

        let dudx_tex = du_dx * self.tw;
        let dvdx_tex = dv_dx * self.th;
        let dudy_tex = du_dy * self.tw;
        let dvdy_tex = dv_dy * self.th;

        let rho_x_sq = dudx_tex * dudx_tex + dvdx_tex * dvdx_tex;
        let rho_y_sq = dudy_tex * dudy_tex + dvdy_tex * dvdy_tex;
        let rho_sq = rho_x_sq.max(rho_y_sq);

        if rho_sq <= 1.0 {
            0
        } else {
            let lod = (0.5 * rho_sq.log2()).round().max(0.0) as usize;
            lod.min(max_lod)
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_tex(
    tex: &TexRef,
    filter: TexFilter,
    use_mips: bool,
    grad: Option<&TexGradients>,
    uq: f32,
    vq: f32,
    iwp: f32,
    uf: f32,
    vf: f32,
) -> [u8; 4] {
    let (sample_px, sample_w, sample_h) = if use_mips && !tex.mips.is_empty() {
        let max_lod = tex.mips.len() - 1;
        let lod = match grad {
            Some(g) => g.select_lod(uq, vq, iwp, max_lod),
            None => 0,
        };
        let mip = &tex.mips[lod];
        (&mip.px[..], mip.w, mip.h)
    } else {
        (&tex.px[..], tex.w, tex.h)
    };

    match filter {
        TexFilter::Nearest => sample_nearest_raw(sample_px, sample_w, sample_h, uf, vf),
        TexFilter::Bilinear => sample_bilinear_raw(sample_px, sample_w, sample_h, uf, vf),
    }
}

/// T&L: world = M_model · v, clip = M_mvp · world
/// (UV-атрибуты интерполируются отдельно — см. raster_tris/uvq).
fn transform_all(
    pos: &[f32],
    nrm: Option<&[f32]>,
    model: &[f32],
    mvp: &[f32],
    world: &mut Vec<f32>,
    world_nrm: &mut Vec<f32>,
    clip: &mut Vec<f32>,
) {
    let n = pos.len() / 3;
    world.clear();
    world.resize(n * 3, 0.0);
    clip.clear();
    clip.resize(n * 4, 0.0);
    world_nrm.clear();
    if let Some(normals) = nrm {
        world_nrm.resize(n * 3, 0.0);
        for i in 0..n {
            let nx = normals[i * 3];
            let ny = normals[i * 3 + 1];
            let nz = normals[i * 3 + 2];
            let wnx = model[0] * nx + model[1] * ny + model[2] * nz;
            let wny = model[4] * nx + model[5] * ny + model[6] * nz;
            let wnz = model[8] * nx + model[9] * ny + model[10] * nz;
            let l = (wnx * wnx + wny * wny + wnz * wnz).sqrt().max(1e-9);
            world_nrm[i * 3] = wnx / l;
            world_nrm[i * 3 + 1] = wny / l;
            world_nrm[i * 3 + 2] = wnz / l;
        }
    }
    for i in 0..n {
        let x = pos[i * 3];
        let y = pos[i * 3 + 1];
        let z = pos[i * 3 + 2];
        let wx = model[0] * x + model[1] * y + model[2] * z + model[3];
        let wy = model[4] * x + model[5] * y + model[6] * z + model[7];
        let wz = model[8] * x + model[9] * y + model[10] * z + model[11];
        world[i * 3] = wx;
        world[i * 3 + 1] = wy;
        world[i * 3 + 2] = wz;
        clip[i * 4] = mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3];
        clip[i * 4 + 1] = mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7];
        clip[i * 4 + 2] = mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11];
        clip[i * 4 + 3] = mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15];
    }
}

/// M = T · Ryaw · Rpitch · Rroll · S (конвенция render3d.lisp).
fn inst_matrix(ins: &[f32], out: &mut [f32; 16]) {
    let (tx, ty, tz) = (ins[0], ins[1], ins[2]);
    let (sx, sy, sz) = (ins[3], ins[4], ins[5]);
    // ВАЖНО: f32::sin_cos возвращает (sin, cos) — не (cos, sin)!
    // (баг мастер-ветки: перепутанная связка ломала матрицу инстанса —
    // оси подменялись, батчи рисовались «сплющенными»)
    let (syw, cy) = ins[6].sin_cos();
    let (sxi, cx) = ins[7].sin_cos();
    let (szi, cz) = ins[8].sin_cos();
    // R = Ry · Rx · Rz
    // Ry·Rx:
    //   [ cy  sy*sx  sy*cx ]
    //   [ 0   cx    -sx    ]
    //   [ -sy cy*sx  cy*cx ]
    let a00 = cy;
    let a01 = syw * sxi;
    let a02 = syw * cx;
    let a10 = 0.0;
    let a11 = cx;
    let a12 = -sxi;
    let a20 = -syw;
    let a21 = cy * sxi;
    let a22 = cy * cx;
    // · Rz:
    //   [ cz -sz 0 ]
    //   [ sz  cz 0 ]
    //   [ 0   0  1 ]
    let r00 = a00 * cz - a01 * szi;
    let r01 = a00 * szi + a01 * cz;
    let r02 = a02;
    let r10 = a10 * cz - a11 * szi;
    let r11 = a10 * szi + a11 * cz;
    let r12 = a12;
    let r20 = a20 * cz - a21 * szi;
    let r21 = a20 * szi + a21 * cz;
    let r22 = a22;
    // · S и T:
    out.fill(0.0);
    out[0] = r00 * sx;
    out[1] = r01 * sy;
    out[2] = r02 * sz;
    out[3] = tx;
    out[4] = r10 * sx;
    out[5] = r11 * sy;
    out[6] = r12 * sz;
    out[7] = ty;
    out[8] = r20 * sx;
    out[9] = r21 * sy;
    out[10] = r22 * sz;
    out[11] = tz;
    out[12] = 0.0;
    out[13] = 0.0;
    out[14] = 0.0;
    out[15] = 1.0;
}

/// Ядро: трансформация вершин + растеризация. Один вызов = один draw call.
#[allow(clippy::too_many_arguments)]
/// скиннинг по весам (этап 21): v' = Σ w_k · J_k · v, до 4 влияний на
/// вершину; веса нормируются по сумме (glTF требует суммарную 1, но файлы
/// бывают с весами вне [0,1]); jmat — финальные матрицы суставов
/// (мир узла × inverseBind), row-major 4×4, нижняя строка 0 0 0 1
pub(crate) fn skin_blend(
    pos: &[f32],
    joints: &[f32],
    weights: &[f32],
    jmat: &[f32],
) -> Result<Vec<f32>, String> {
    let nv = pos.len() / 3;
    if joints.len() != nv * 4 || weights.len() != nv * 4 {
        return Err(format!(
            "skin: joints/weights — по 4 на вершину, вершин {}",
            nv
        ));
    }
    let nj = jmat.len() / 16;
    if nj == 0 {
        return Err("skin: jmat пуст".into());
    }
    let mut out = Vec::with_capacity(nv * 3);
    for i in 0..nv {
        let x = pos[i * 3];
        let y = pos[i * 3 + 1];
        let z = pos[i * 3 + 2];
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        let mut sz = 0.0f32;
        let mut wsum = 0.0f32;
        for k in 0..4 {
            let w = weights[i * 4 + k];
            if w <= 0.0 {
                continue;
            }
            let j = (joints[i * 4 + k] as usize).min(nj - 1);
            let m = &jmat[j * 16..j * 16 + 16];
            sx += w * (m[0] * x + m[1] * y + m[2] * z + m[3]);
            sy += w * (m[4] * x + m[5] * y + m[6] * z + m[7]);
            sz += w * (m[8] * x + m[9] * y + m[10] * z + m[11]);
            wsum += w;
        }
        let (ox, oy, oz) = if wsum > 1e-6 {
            (sx / wsum, sy / wsum, sz / wsum)
        } else {
            (x, y, z)
        };
        out.push(ox);
        out.push(oy);
        out.push(oz);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn mesh_draw(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    vca: Option<&[f32]>, // per-vertex rgba (gpu3d-mesh-vc)
    nrm: Option<&[f32]>, // per-vertex normals (smooth shading)
    idx: &[f32],
    model: &[f32],
    mvp: &[f32],
    r: f64,
    g: f64,
    b: f64,
    texid: i64,
    prim: Prim,
) -> Result<u64, String> {
    // wgpu-режим: 3D-конвейер рисует на GPU, сброс нужен только для софтверного растра
    if !matches!(hal.backend, crate::gpu::Backend::Wgpu(_)) {
        let _ = hal.flush_gpu(true);
    }
    if pos.len() % 3 != 0 || pos.is_empty() {
        return Err("gpu3d: позиции — плоский массив xyz (длина кратна 3)".into());
    }
    let n = pos.len() / 3;
    if let Some(u) = uv {
        if u.len() != n * 2 {
            return Err(format!(
                "gpu3d: uv — пары на вершину ({}), вершин {}",
                u.len(),
                n
            ));
        }
    }
    if let Some(vc) = vca {
        if vc.len() != n * 4 {
            return Err(format!(
                "gpu3d: rgba — четвёрки на вершину ({}), вершин {}",
                vc.len(),
                n
            ));
        }
    }
    if let Some(normals) = nrm {
        if normals.len() != n * 3 {
            return Err(format!(
                "gpu3d: normal — тройки на вершину ({}), вершин {}",
                normals.len(),
                n
            ));
        }
    }
    if let Prim::Wire = prim {
        if idx.len() % 2 != 0 {
            return Err("gpu3d-wire: индексы — пары вершин (длина кратна 2)".into());
        }
    } else if idx.len() % 3 != 0 {
        return Err("gpu3d: индексы — тройки вершин (длина кратна 3)".into());
    }
    if model.len() != 16 || mvp.len() != 16 {
        return Err("gpu3d: матрицы 4x4 = 16 чисел".into());
    }
    let tex = tex_lookup(hal, texid)?;
    if tex.is_some() && uv.is_none() {
        return Err("gpu3d: текстура без uv-массива".into());
    }
    // shadow_pass: запись глубины кастеров тени в shadow_map (depth-only)
    if hal.r3.shadow_pass {
        if matches!(prim, Prim::Wire) {
            return Ok(0);
        }
        let n = pos.len() / 3;
        let mut world = Vec::with_capacity(n * 3);
        let mut world_nrm = Vec::new();
        let mut clip = Vec::with_capacity(n * 4);
        transform_all(pos, None, model, mvp, &mut world, &mut world_nrm, &mut clip);
        let drawn = raster_shadow_tris(&mut hal.r3, &clip, idx)?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    // wgpu-режим: 3D-конвейер рисует на GPU через аппаратный пайплайн
    if let crate::gpu::Backend::Wgpu(gpu) = &mut hal.backend {
        let r3_state = hal.r3.state();
        let drawn = gpu.draw_mesh_3d(
            pos,
            uv,
            vca,
            nrm,
            idx,
            model,
            mvp,
            (r as f32, g as f32, b as f32),
            texid,
            matches!(prim, Prim::Wire),
            &r3_state,
        )?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let Hal { fb, r3, .. } = &mut *hal;

    let mut world = Vec::with_capacity(n * 3);
    let mut world_nrm = Vec::with_capacity(if nrm.is_some() { n * 3 } else { 0 });
    let mut clip = Vec::with_capacity(n * 4);
    transform_all(pos, nrm, model, mvp, &mut world, &mut world_nrm, &mut clip);

    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let mut drawn: u64 = 0;
    let mut culled: u64 = 0;

    if let Prim::Wire = prim {
        // каркас: индексы — ПАРЫ вершин; clip по near-плоскости на сегмент
        for e in 0..idx.len() / 2 {
            let mut seg = [0usize; 2];
            for k in 0..2 {
                let vi = idx[e * 2 + k];
                // через переменную: без !(a >= b) — NaN-семантика прежняя
                let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
                if !idx_ok {
                    return Err(format!("gpu3d: индекс за границей: {}", vi));
                }
                seg[k] = vi as usize;
            }
            let (p, q) = (
                [
                    clip[seg[0] * 4],
                    clip[seg[0] * 4 + 1],
                    clip[seg[0] * 4 + 2],
                    clip[seg[0] * 4 + 3],
                ],
                [
                    clip[seg[1] * 4],
                    clip[seg[1] * 4 + 1],
                    clip[seg[1] * 4 + 2],
                    clip[seg[1] * 4 + 3],
                ],
            );
            let (pw, qw) = (p[3], q[3]);
            if pw <= 1e-6 && qw <= 1e-6 {
                continue;
            }
            let (pa, qa) = if pw < 1e-6 || qw < 1e-6 {
                let t = (1e-6 - pw) / (qw - pw);
                (
                    [
                        p[0] + (q[0] - p[0]) * t,
                        p[1] + (q[1] - p[1]) * t,
                        p[2] + (q[2] - p[2]) * t,
                        1e-6,
                    ],
                    [
                        q[0] + (p[0] - q[0]) * t,
                        q[1] + (p[1] - q[1]) * t,
                        q[2] + (p[2] - q[2]) * t,
                        1e-6,
                    ],
                )
            } else {
                (p, q)
            };
            if let (Some(v0), Some(v1)) = (project(&pa, w, h), project(&qa, w, h)) {
                draw_line(fb, w, h, v0, v1, col(r), col(g), col(b));
                drawn += 1;
            }
        }
    } else {
        raster_tris(
            fb,
            r3,
            w,
            h,
            &clip,
            &world,
            if nrm.is_some() {
                Some(&world_nrm)
            } else {
                None
            },
            uv,
            vca,
            idx,
            None,
            (r as f32, g as f32, b as f32),
            ambient,
            diffuse,
            &light,
            tex.as_ref(),
            false,
            &mut drawn,
            &mut culled,
        )?;
    }
    r3.drawn += drawn;
    r3.culled += culled;
    Ok(drawn)
}

/// Батч-вершинник: инстансы разворачиваются в общие world/clip/uv/tint-массивы,
/// индексы смещаются, затем ОДНА растеризация на все инстансы.
/// pub(crate): доступен бенчмарку 3D-шейдинга (main.rs, этап 7).
#[allow(clippy::too_many_arguments)]
pub(crate) fn batch_draw(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    nrm: Option<&[f32]>,
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
    texid: i64,
) -> Result<u64, String> {
    batch_draw_impl(hal, pos, uv, nrm, idx, insts, mvp, texid, false)
}

/// Этап 16: батч с ГОТОВЫМИ матрицами инстансов (скелетная анимация).
/// Формат инстанса: 20 f32 = матрица 4×4 (строчная, аффинная) + r g b a.
pub(crate) fn batch_draw_mat(
    hal: &mut Hal,
    pos: &[f32],
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
) -> Result<u64, String> {
    batch_draw_impl(hal, pos, None, None, idx, insts, mvp, 0, true)
}

#[allow(clippy::too_many_arguments)] // формат батча — 9 параметров по дизайну HAL
fn batch_draw_impl(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    nrm: Option<&[f32]>,
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
    texid: i64,
    mat_mode: bool,
) -> Result<u64, String> {
    // wgpu-режим: сброс нужен только перед программным 3D-батчем
    if !matches!(hal.backend, crate::gpu::Backend::Wgpu(_)) {
        let _ = hal.flush_gpu(true);
    }
    if pos.len() % 3 != 0 || pos.is_empty() {
        return Err("gpu3d-batch: позиции — плоский xyz".into());
    }
    let n = pos.len() / 3;
    if let Some(normals) = nrm {
        if normals.len() != n * 3 {
            return Err(format!(
                "gpu3d-batch: normal — тройки на вершину ({}), вершин {}",
                normals.len(),
                n
            ));
        }
    }
    if idx.len() % 3 != 0 {
        return Err("gpu3d-batch: индексы — тройки вершин".into());
    }
    let stride = if mat_mode { 20 } else { 12 };
    if insts.len() % stride != 0 {
        return Err(if mat_mode {
            "gpu3d-batch-mat: инстансы — по 20 f32 (матрица 4×4 + r g b a)".into()
        } else {
            "gpu3d-batch: инстансы — по 12 f32 (t.xyz s.xyz yaw pitch roll r g b)".into()
        });
    }
    if insts.is_empty() {
        return Ok(0); // пустой батч — ничего не рисуем (врагов пока нет)
    }
    if mvp.len() != 16 {
        return Err("gpu3d-batch: mvp 4x4 = 16 чисел".into());
    }
    let k = insts.len() / stride;
    if k * n * 3 > 8_000_000 {
        return Err("gpu3d-batch: слишком много вершин".into());
    }
    let tex = tex_lookup(hal, texid)?;
    if tex.is_some() && uv.is_none() {
        return Err("gpu3d-batch-tex: текстура без uv".into());
    }
    // shadow_pass: запись глубины батча в shadow_map (depth-only)
    if hal.r3.shadow_pass {
        let mut clip = Vec::with_capacity(k * n * 4);
        let mut idx_all: Vec<f32> = Vec::with_capacity(k * idx.len());
        let mut m: [f32; 16] = [0.0; 16];
        for i in 0..k {
            let ins = &insts[i * stride..(i + 1) * stride];
            if mat_mode {
                m.copy_from_slice(&ins[0..16]);
            } else {
                inst_matrix(ins, &mut m);
            }
            let base = i * n;
            for vi in 0..n {
                let x = pos[vi * 3];
                let y = pos[vi * 3 + 1];
                let z = pos[vi * 3 + 2];
                let wx = m[0] * x + m[1] * y + m[2] * z + m[3];
                let wy = m[4] * x + m[5] * y + m[6] * z + m[7];
                let wz = m[8] * x + m[9] * y + m[10] * z + m[11];
                clip.push(mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3]);
                clip.push(mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7]);
                clip.push(mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11]);
                clip.push(mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15]);
            }
            for &j in idx {
                idx_all.push(j + base as f32);
            }
        }
        let drawn = raster_shadow_tris(&mut hal.r3, &clip, &idx_all)?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    // wgpu-режим: батч на GPU через аппаратный instancing (только стандартные 12-f32 инстансы)
    if !mat_mode {
        if let crate::gpu::Backend::Wgpu(gpu) = &mut hal.backend {
            let r3_state = hal.r3.state();
            let drawn = gpu.draw_batch_3d(pos, uv, nrm, idx, insts, mvp, texid, &r3_state)?;
            hal.r3.drawn += drawn;
            return Ok(drawn);
        }
    }
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let Hal { fb, r3, .. } = &mut *hal;

    // единые буферы на все инстансы
    let mut world = Vec::with_capacity(k * n * 3);
    let mut world_nrm = Vec::with_capacity(if nrm.is_some() { k * n * 3 } else { 0 });
    let mut clip = Vec::with_capacity(k * n * 4);
    let mut uvq: Vec<f32> = Vec::with_capacity(k * n * 2);
    let mut tint: Vec<f32> = Vec::with_capacity(k * n * 3);
    let mut idx_all: Vec<f32> = Vec::with_capacity(k * idx.len());
    let mut m: [f32; 16] = [0.0; 16];

    for i in 0..k {
        let ins = &insts[i * stride..(i + 1) * stride];
        if mat_mode {
            m.copy_from_slice(&ins[0..16]);
        } else {
            inst_matrix(ins, &mut m);
        }
        let base = i * n;
        for vi in 0..n {
            let x = pos[vi * 3];
            let y = pos[vi * 3 + 1];
            let z = pos[vi * 3 + 2];
            let wx = m[0] * x + m[1] * y + m[2] * z + m[3];
            let wy = m[4] * x + m[5] * y + m[6] * z + m[7];
            let wz = m[8] * x + m[9] * y + m[10] * z + m[11];
            world.push(wx);
            world.push(wy);
            world.push(wz);
            if let Some(normals) = nrm {
                let nx = normals[vi * 3];
                let ny = normals[vi * 3 + 1];
                let nz = normals[vi * 3 + 2];
                let wnx = m[0] * nx + m[1] * ny + m[2] * nz;
                let wny = m[4] * nx + m[5] * ny + m[6] * nz;
                let wnz = m[8] * nx + m[9] * ny + m[10] * nz;
                let l = (wnx * wnx + wny * wny + wnz * wnz).sqrt().max(1e-9);
                world_nrm.push(wnx / l);
                world_nrm.push(wny / l);
                world_nrm.push(wnz / l);
            }
            clip.push(mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3]);
            clip.push(mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7]);
            clip.push(mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11]);
            clip.push(mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15]);
            if let Some(u) = uv {
                uvq.push(u[vi * 2]);
                uvq.push(u[vi * 2 + 1]);
            }
            if mat_mode {
                tint.push(ins[16]);
                tint.push(ins[17]);
                tint.push(ins[18]);
            } else {
                tint.push(ins[9]);
                tint.push(ins[10]);
                tint.push(ins[11]);
            }
        }
        for &j in idx {
            idx_all.push(j + base as f32);
        }
    }

    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let mut drawn: u64 = 0;
    let mut culled: u64 = 0;
    raster_tris(
        fb,
        r3,
        w,
        h,
        &clip,
        &world,
        if nrm.is_some() {
            Some(&world_nrm)
        } else {
            None
        },
        if uv.is_some() { Some(&uvq) } else { None },
        None,
        &idx_all,
        Some(&tint),
        (0.0, 0.0, 0.0),
        ambient,
        diffuse,
        &light,
        tex.as_ref(),
        false,
        &mut drawn,
        &mut culled,
    )?;
    r3.drawn += drawn;
    r3.culled += culled;
    Ok(drawn)
}

/// Прозрачный проход (этап 9): сортируем очередь far→near по видовой глубине
/// центра и рисуем с src-alpha блендингом (z-тест есть, z-записи нет,
/// двусторонне). Порядок в очереди задаёт художник-видимость для крон, воды
/// и облаков — как у настоящего GPU с painter's sort по треугольникам.
pub(crate) fn flush_transparent(hal: &mut Hal) -> Result<u64, String> {
    if hal.r3.transq.is_empty() {
        return Ok(0);
    }
    let _ = hal.flush_gpu(true);
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let mut q = std::mem::take(&mut hal.r3.transq);
    q.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // текстуры резолвим один раз на id (клон px дорог) — до разбора hal
    let mut texcache: Vec<(i64, TexRef)> = Vec::new();
    {
        let mut ids: Vec<i64> = q.iter().map(|t| t.tex).collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            if id != 0 {
                texcache.push((
                    id,
                    tex_lookup(hal, id)?
                        .ok_or_else(|| format!("gpu3d-flush-alpha: нет текстуры {}", id))?,
                ));
            }
        }
    }
    let mut drawn: u64 = 0;
    let Hal { fb, r3, .. } = &mut *hal;
    fog_lut_sync(r3);
    for t in &q {
        let tex = texcache
            .iter()
            .find(|(id, _)| *id == t.tex)
            .map(|(_, tr)| tr);
        blend_tri(fb, r3, w, h, t, tex, &mut drawn)?;
    }
    r3.drawn += drawn;
    Ok(drawn)
}

/// Один треугольник прозрачного прохода: блендинг, z-тест без z-записи.
#[allow(clippy::too_many_arguments)]
fn blend_tri(
    fb: &mut [u8],
    r3: &mut R3,
    w: usize,
    h: usize,
    t: &TransTri,
    tex: Option<&TexRef>,
    drawn: &mut u64,
) -> Result<(), String> {
    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let spec_on = t.spec > 0.0;
    let fog_on = t.fog[3] > 0.0;
    let (fog_density, fog_r, fog_g, fog_b) = (t.fog[3], t.fog[0], t.fog[1], t.fog[2]);
    let (tu, tv) = (r3.tiling[0], r3.tiling[1]);
    let nnrm = t.nrm;
    let nl_ok = dot3(&nnrm, &nnrm) > 0.5;
    let _shade = if !nl_ok {
        ambient
    } else {
        let ndl = dot3(&nnrm, &light).abs(); // двустороннее освещение
        (ambient + diffuse * ndl.max(0.0)).clamp(0.0, 1.0)
    };
    let (p0, p1, p2) = (&t.pts[0], &t.pts[1], &t.pts[2]);
    let has_smooth = r3.smooth && (p0.nxq.abs() + p0.nyq.abs() + p0.nzq.abs() > 1e-6);
    let (tw, th) = tex.map(|x| (x.w, x.h)).unwrap_or((0, 0));

    let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
    let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(w as i64 - 1);
    let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
    let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(h as i64 - 1);
    if minx > maxx || miny > maxy {
        return Ok(());
    }
    let tri_area_2 = edge(p1.sx, p1.sy, p2.sx, p2.sy, p0.sx, p0.sy);
    let tex_grad = if tex.is_some() && t.mipmaps {
        TexGradients::compute(p0, p1, p2, tri_area_2, tw, th, tu, tv)
    } else {
        None
    };
    for py in miny..=maxy {
        let row = py as usize * w;
        let fy = py as f32 + 0.5;
        for px in minx..=maxx {
            let fx = px as f32 + 0.5;
            let w0 = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy);
            let w1 = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy);
            let w2 = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
            // двусторонне: внутри полигона веса одного знака
            if (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) || (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) {
                let sum = w0 + w1 + w2;
                if sum.abs() < 1e-9 {
                    continue;
                }
                let z = (w0 * p0.sz + w1 * p1.sz + w2 * p2.sz) / sum;
                let pi = row + px as usize;
                if z < r3.zbuf[pi] {
                    let iwp = (w0 * p0.iw + w1 * p1.iw + w2 * p2.iw) / sum;
                    if iwp.abs() <= 1e-12 {
                        continue;
                    }
                    let (pix_nnrm, pix_ndl) = if has_smooth {
                        let nx = ((w0 * p0.nxq + w1 * p1.nxq + w2 * p2.nxq) / sum) / iwp;
                        let ny = ((w0 * p0.nyq + w1 * p1.nyq + w2 * p2.nyq) / sum) / iwp;
                        let nz = ((w0 * p0.nzq + w1 * p1.nzq + w2 * p2.nzq) / sum) / iwp;
                        let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
                        if nlen < 1e-9 {
                            (
                                nnrm,
                                if nl_ok {
                                    dot3(&nnrm, &light).abs()
                                } else {
                                    0.0
                                },
                            )
                        } else {
                            let inv_nlen = 1.0 / nlen;
                            let norm = [nx * inv_nlen, ny * inv_nlen, nz * inv_nlen];
                            let ndl = dot3(&norm, &light).abs();
                            (norm, ndl)
                        }
                    } else {
                        (
                            nnrm,
                            if nl_ok {
                                dot3(&nnrm, &light).abs()
                            } else {
                                0.0
                            },
                        )
                    };
                    let need_world_pos =
                        spec_on || r3.shadow_enabled || !r3.point_lights.is_empty();
                    let (wx, wy, wz) = if need_world_pos {
                        (
                            ((w0 * p0.wxq + w1 * p1.wxq + w2 * p2.wxq) / sum) / iwp,
                            ((w0 * p0.wyq + w1 * p1.wyq + w2 * p2.wyq) / sum) / iwp,
                            ((w0 * p0.wzq + w1 * p1.wzq + w2 * p2.wzq) / sum) / iwp,
                        )
                    } else {
                        (0.0, 0.0, 0.0)
                    };
                    let shadow_factor = if r3.shadow_enabled {
                        shadow_sample(r3, wx, wy, wz, r3.shadow_bias)
                    } else {
                        1.0
                    };
                    let dir_diffuse = diffuse * pix_ndl.max(0.0) * shadow_factor;
                    let dir_shade = (ambient + dir_diffuse).clamp(0.0, 1.0);
                    let s = if spec_on {
                        let v = [r3.view[0] - wx, r3.view[1] - wy, r3.view[2] - wz];
                        let vl = dot3(&v, &v).sqrt().max(1e-9);
                        let hv = [
                            light[0] + v[0] / vl,
                            light[1] + v[1] / vl,
                            light[2] + v[2] / vl,
                        ];
                        let hl = dot3(&hv, &hv).sqrt().max(1e-9);
                        spec_highlight(
                            dot3(&pix_nnrm, &[hv[0] / hl, hv[1] / hl, hv[2] / hl]),
                            t.shininess,
                            t.spec,
                        ) * shadow_factor
                    } else {
                        0.0
                    };
                    let (pt_diff, pt_spec) = compute_point_lights(
                        &r3.point_lights,
                        &r3.view,
                        wx,
                        wy,
                        wz,
                        &pix_nnrm,
                        true,
                        spec_on,
                        t.shininess,
                        t.spec,
                    );
                    let fogf = if fog_on {
                        fog_factor(fog_density, 1.0 / iwp)
                    } else {
                        0.0
                    };
                    // per-vertex rgb/alpha (градиент воды/листвы) — всегда
                    // интерполяция: у mesh-vc цвет живёт в вершинах, тинт
                    // для прозрачного прохода не используется
                    let f = 1.0 / iwp;
                    let rq = ((w0 * p0.rq + w1 * p1.rq + w2 * p2.rq) / sum) * f;
                    let gq = ((w0 * p0.gq + w1 * p1.gq + w2 * p2.gq) / sum) * f;
                    let bq = ((w0 * p0.bq + w1 * p1.bq + w2 * p2.bq) / sum) * f;
                    let (mut c0, mut c1, mut c2) = (
                        rq * dir_shade + rq * pt_diff[0],
                        gq * dir_shade + gq * pt_diff[1],
                        bq * dir_shade + bq * pt_diff[2],
                    );
                    let mut al =
                        (((w0 * p0.aq + w1 * p1.aq + w2 * p2.aq) / sum) * f).clamp(0.0, 1.0);
                    if let Some(tx) = tex {
                        let uq = (w0 * p0.uq + w1 * p1.uq + w2 * p2.uq) / sum;
                        let vq = (w0 * p0.vq + w1 * p1.vq + w2 * p2.vq) / sum;
                        let uf = tiled_coord(uq / iwp, tu);
                        let vf = tiled_coord(vq / iwp, tv);
                        let texel = sample_tex(
                            tx,
                            t.filter,
                            t.mipmaps,
                            tex_grad.as_ref(),
                            uq,
                            vq,
                            iwp,
                            uf,
                            vf,
                        );
                        al *= texel[3] as f32 / 255.0;
                        if al <= 0.004 {
                            continue;
                        }
                        c0 *= texel[0] as f32 / 255.0;
                        c1 *= texel[1] as f32 / 255.0;
                        c2 *= texel[2] as f32 / 255.0;
                    } else if al <= 0.004 {
                        continue;
                    }
                    if spec_on || pt_spec[0] > 0.0 || pt_spec[1] > 0.0 || pt_spec[2] > 0.0 {
                        c0 = (c0 + s + pt_spec[0]).min(1.0);
                        c1 = (c1 + s + pt_spec[1]).min(1.0);
                        c2 = (c2 + s + pt_spec[2]).min(1.0);
                    }
                    if fog_on {
                        let g = 1.0 - fogf;
                        c0 = c0 * g + fog_r * fogf;
                        c1 = c1 * g + fog_g * fogf;
                        c2 = c2 * g + fog_b * fogf;
                    }
                    let o = pi * 4;
                    let dr = fb[o] as f32 / 255.0;
                    let dg = fb[o + 1] as f32 / 255.0;
                    let db = fb[o + 2] as f32 / 255.0;
                    let iv = 1.0 - al;
                    fb[o] = col((c0 * al + dr * iv) as f64);
                    fb[o + 1] = col((c1 * al + dg * iv) as f64);
                    fb[o + 2] = col((c2 * al + db * iv) as f64);
                    fb[o + 3] = 255;
                }
            }
        }
    }
    *drawn += 1;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn raster_tris(
    fb: &mut [u8],
    r3: &mut R3,
    w: usize,
    h: usize,
    clip: &[f32],
    world: &[f32],
    world_nrm: Option<&[f32]>,
    uvq: Option<&[f32]>,
    vca: Option<&[f32]>, // per-vertex rgba (gpu3d-mesh-vc): n*4 f32
    idx: &[f32],
    tint: Option<&[f32]>,   // rgb на вершину (батч: цвет инстанса)
    solid: (f32, f32, f32), // один цвет (обычный mesh)
    ambient: f32,
    diffuse: f32,
    light: &[f32; 3],
    tex: Option<&TexRef>,
    blend_pass: bool, // true = прозрачный проход (блендинг, без z-записи)
    drawn: &mut u64,
    culled: &mut u64,
) -> Result<(), String> {
    let n = world.len() / 3;
    // LUT тумана актуализируется раз на draw-вызов (fog меняется и напрямую
    // из тестов, и через gpu3d-fog — снимок-ключ ловит любое изменение)
    fog_lut_sync(r3);
    // скретч-буферы переиспользуются всеми треугольниками (без аллокаций
    // на треугольник в горячем цикле — цена тесселяции ревизии 4)
    let mut poly_buf: Vec<[f32; 17]> = Vec::with_capacity(8);
    let mut clip_buf: Vec<[f32; 17]> = Vec::with_capacity(8);
    let mut pts_buf: Vec<Vtx> = Vec::with_capacity(8);
    // кэш проекций: вершина регулярной сетки переиспользуется до 4 квад,
    // project6 с перспективным делением — самая дорогая часть цикла
    let mut vcache: Vec<Option<Vtx>> = vec![None; n];
    for f in 0..idx.len() / 3 {
        let mut tri = [0usize; 3];
        for k in 0..3 {
            let vi = idx[f * 3 + k];
            // через переменную: без !(a >= b) — NaN-семантика прежняя
            let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
            if !idx_ok {
                return Err(format!("gpu3d: индекс за границей: {}", vi));
            }
            tri[k] = vi as usize;
        }
        // полигон в клип-пространстве + атрибуты u/w, v/w, 1/w, мир/w, a/w, rgb/w, nx/w, ny/w, nz/w
        let blend_base = if r3.blend > 0.0 {
            r3.blend.min(1.0)
        } else {
            1.0
        };
        let base_rgb = match tint {
            Some(t) => [t[tri[0] * 3], t[tri[0] * 3 + 1], t[tri[0] * 3 + 2]],
            None => [solid.0, solid.1, solid.2],
        };
        poly_buf.clear();
        for &t in &tri {
            let cw = clip[t * 4 + 3];
            let iw = if cw > 1e-6 { 1.0 / cw } else { 0.0 };
            // перспективно-корректные атрибуты: в полигон идут u/w, v/w,
            // мировая позиция /w, альфа/w, rgb/w и нормали/w (аффинны на ребре)
            let (uq, vq) = if let Some(u) = uvq {
                (u[t * 2] * iw, u[t * 2 + 1] * iw)
            } else {
                (0.0, 0.0)
            };
            let (a, rr, gg, bb) = if let Some(vc) = vca {
                (vc[t * 4 + 3], vc[t * 4], vc[t * 4 + 1], vc[t * 4 + 2])
            } else {
                (blend_base, base_rgb[0], base_rgb[1], base_rgb[2])
            };
            let (nxq, nyq, nzq) = if let Some(wn) = world_nrm {
                (wn[t * 3] * iw, wn[t * 3 + 1] * iw, wn[t * 3 + 2] * iw)
            } else {
                (0.0, 0.0, 0.0)
            };
            poly_buf.push([
                clip[t * 4],
                clip[t * 4 + 1],
                clip[t * 4 + 2],
                cw,
                uq,
                vq,
                iw,
                world[t * 3] * iw,
                world[t * 3 + 1] * iw,
                world[t * 3 + 2] * iw,
                a * iw,
                rr * iw,
                gg * iw,
                bb * iw,
                nxq,
                nyq,
                nzq,
            ]);
        }

        let poly: &[[f32; 17]] = if poly_buf.iter().any(|v| v[3] < 1e-6) {
            clip_near6(&poly_buf, &mut clip_buf);
            if clip_buf.len() < 3 {
                *culled += 1;
                continue;
            }
            &clip_buf
        } else {
            &poly_buf
        };

        let pts = &mut pts_buf;
        pts.clear();
        let mut ok = true;
        if poly.len() == 3 {
            // без клипа: проекции берём из кэша вершины (порядок tri[])
            for (k, &vi) in tri.iter().enumerate() {
                let c = &mut vcache[vi];
                if c.is_none() {
                    *c = project6(&poly[k], w, h);
                }
                match c {
                    Some(p) => pts.push(*p),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        } else {
            for v in poly {
                match project6(v, w, h) {
                    Some(p) => pts.push(p),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            *culled += 1;
            continue;
        }

        // — состояние материалов (этап 7): выключенные фичи — ноль расходов —
        let spec_on = r3.spec > 0.0;
        let (spec, shininess) = (r3.spec, r3.shininess);
        let fog_on = r3.fog[3] > 0.0;
        let (fog_density, fog_r, fog_g, fog_b) = (r3.fog[3], r3.fog[0], r3.fog[1], r3.fog[2]);
        let fog_snap = r3.fog;
        let alpha_on = r3.alpha_test > 0.0;
        let alpha_thr = r3.alpha_test;
        let smooth_shading = r3.smooth && world_nrm.is_some();
        let (tu, tv) = (r3.tiling[0], r3.tiling[1]);

        // LUT тумана — локальная копия (без займа r3 в горячем цикле)
        let lut = r3.fog_lut;
        // вентилятор: (0, k, k+1)
        for kk in 1..pts.len() - 1 {
            let (p0, p1, p2) = (&pts[0], &pts[kk], &pts[kk + 1]);
            // backface cull: фронт = CCW в NDC → после y-flip площадь < 0.
            // Куллинг ПОЗЖЕ — прозрачные грани двусторонни, задние тоже
            // должны попасть в прозрачную очередь (решение ниже).
            let area = edge(p0.sx, p0.sy, p1.sx, p1.sy, p2.sx, p2.sy);
            // нормаль грани в мире (flat shading)
            let (wa, wb, wc) = (
                [
                    world[tri[0] * 3],
                    world[tri[0] * 3 + 1],
                    world[tri[0] * 3 + 2],
                ],
                [
                    world[tri[1] * 3],
                    world[tri[1] * 3 + 1],
                    world[tri[1] * 3 + 2],
                ],
                [
                    world[tri[2] * 3],
                    world[tri[2] * 3 + 1],
                    world[tri[2] * 3 + 2],
                ],
            );
            let e1 = [wb[0] - wa[0], wb[1] - wa[1], wb[2] - wa[2]];
            let e2 = [wc[0] - wa[0], wc[1] - wa[1], wc[2] - wa[2]];
            let nrm = cross3(&e1, &e2);
            let nl = dot3(&nrm, &nrm).sqrt();
            let nnrm = if nl < 1e-9 {
                [0.0f32; 3]
            } else {
                [nrm[0] / nl, nrm[1] / nl, nrm[2] / nl]
            };
            // прозрачный проход двусторонний: нормаль разворачиваем к свету
            let _shade = if nl < 1e-9 {
                ambient
            } else {
                let mut ndl = dot3(&nnrm, light);
                if blend_pass && ndl < 0.0 {
                    ndl = -ndl;
                }
                (ambient + diffuse * ndl.max(0.0)).clamp(0.0, 1.0)
            };

            // — прозрачная очередь (этап 9): альфа треугольника = средняя
            // перспектива-корректная альфа вершин; < 1 → ставим в очередь
            // (сорт far→near и блендинг — в gpu3d-flush-alpha) —
            let va = [
                if p0.iw > 1e-12 { p0.aq / p0.iw } else { 1.0 },
                if p1.iw > 1e-12 { p1.aq / p1.iw } else { 1.0 },
                if p2.iw > 1e-12 { p2.aq / p2.iw } else { 1.0 },
            ];
            let al_avg = (va[0] + va[1] + va[2]) / 3.0;
            if !blend_pass && al_avg < 0.999 {
                let depth = 1.0 / 3.0
                    * (if p0.iw > 1e-12 { 1.0 / p0.iw } else { 0.0 }
                        + if p1.iw > 1e-12 { 1.0 / p1.iw } else { 0.0 }
                        + if p2.iw > 1e-12 { 1.0 / p2.iw } else { 0.0 });
                r3.transq.push(TransTri {
                    depth,
                    pts: [*p0, *p1, *p2],
                    nrm: nnrm,
                    tex: tex.map(|t| t.id).unwrap_or(0),
                    spec,
                    shininess,
                    fog: fog_snap,
                    filter: r3.tex_filter,
                    mipmaps: r3.tex_mipmaps,
                });
                continue;
            }
            // непрозрачный проход: задняя грань — кулим (прозрачные прошли выше)
            if !blend_pass && area >= -1e-6 {
                *culled += 1;
                continue;
            }
            // цвет грани: per-vertex RGB (вода/небо) или тинт первой вершины
            let vc_rgb = !blend_pass
                && p0.rq == p1.rq
                && p0.rq == p2.rq
                && p0.gq == p1.gq
                && p0.gq == p2.gq
                && p0.bq == p1.bq
                && p0.bq == p2.bq;
            let (tr, tg, tb) = if vc_rgb {
                (p0.rq / p0.iw, p0.gq / p0.iw, p0.bq / p0.iw)
            } else {
                match tint {
                    Some(t) => (t[tri[0] * 3], t[tri[0] * 3 + 1], t[tri[0] * 3 + 2]),
                    None => solid,
                }
            };

            let (tw, th) = tex.map(|t| (t.w, t.h)).unwrap_or((0, 0));

            // bbox
            let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
            let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(w as i64 - 1);
            let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
            let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(h as i64 - 1);
            if minx > maxx || miny > maxy {
                continue;
            }
            let tri_area_2 = edge(p1.sx, p1.sy, p2.sx, p2.sy, p0.sx, p0.sy);
            let tex_grad = if tex.is_some() && r3.tex_mipmaps {
                TexGradients::compute(p0, p1, p2, tri_area_2, tw, th, tu, tv)
            } else {
                None
            };

            // — инкрементальный сканлайн (этап 9.2): вместо перебора bbox с
            // тремя edge-функциями на каждый пиксель — спан по строке из
            // полулиний wi ≤ 0 и пошаговые атрибуты: числители N_q = Σ wi·qi
            // аффинны вдоль строки вместе с wi, знаменатель Σ wi инвариантен
            // (площадь) — пиксель = одно деление вместо семи —
            let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
            const A_SZ: usize = 0;
            const A_IW: usize = 1;
            const A_U: usize = 2;
            const A_V: usize = 3;
            const A_WX: usize = 4;
            const A_WY: usize = 5;
            const A_WZ: usize = 6;
            const A_R: usize = 7;
            const A_G: usize = 8;
            const A_B: usize = 9;
            const A_A: usize = 10;
            const A_NX: usize = 11;
            const A_NY: usize = 12;
            const A_NZ: usize = 13;
            const NA: usize = 14;
            let q: [[f32; 3]; NA] = [
                [p0.sz, p1.sz, p2.sz],
                [p0.iw, p1.iw, p2.iw],
                [p0.uq, p1.uq, p2.uq],
                [p0.vq, p1.vq, p2.vq],
                [p0.wxq, p1.wxq, p2.wxq],
                [p0.wyq, p1.wyq, p2.wyq],
                [p0.wzq, p1.wzq, p2.wzq],
                [p0.rq, p1.rq, p2.rq],
                [p0.gq, p1.gq, p2.gq],
                [p0.bq, p1.bq, p2.bq],
                [p0.aq, p1.aq, p2.aq],
                [p0.nxq, p1.nxq, p2.nxq],
                [p0.nyq, p1.nyq, p2.nyq],
                [p0.nzq, p1.nzq, p2.nzq],
            ];
            for py in miny..=maxy {
                let fy = py as f32 + 0.5;
                let fx0 = minx as f32 + 0.5;
                // edge-функции строки в опорной точке (левый край bbox)
                let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
                let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
                let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
                let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
                if from > to {
                    continue;
                }
                let inv_d = 1.0 / (w0r + w1r + w2r);
                let wr = [w0r, w1r, w2r];
                let mut n = [0.0f32; NA];
                let mut dn = [0.0f32; NA];
                for a in 0..NA {
                    n[a] = wr[0] * q[a][0] + wr[1] * q[a][1] + wr[2] * q[a][2];
                    dn[a] = ax[0] * q[a][0] + ax[1] * q[a][1] + ax[2] * q[a][2];
                }
                let skip = (from - minx) as f32;
                if skip != 0.0 {
                    for a in 0..NA {
                        n[a] += dn[a] * skip;
                    }
                }
                let row = py as usize * w;
                for px in from..=to {
                    // z-тест ДО атрибутов: окклюдированный пиксель платит
                    // только за z, шаг и continue (шаг обязателен — следующий
                    // пиксель строки требует шагнувших числителей)
                    let z = n[A_SZ] * inv_d;
                    let pi = row + px as usize;
                    if z >= r3.zbuf[pi] {
                        for a in 0..NA {
                            n[a] += dn[a];
                        }
                        continue;
                    }
                    // видимый: значения ЭТОГО пикселя — чтение ДО шага
                    // (без «отката» n−dn: на клипнутых вершинах атрибуты
                    // ~1e4-1e5, вычитание близких чисел топит точность f32)
                    let iwp = n[A_IW] * inv_d;
                    let uq = n[A_U] * inv_d;
                    let vq = n[A_V] * inv_d;
                    let wxq = n[A_WX] * inv_d;
                    let wyq = n[A_WY] * inv_d;
                    let wzq = n[A_WZ] * inv_d;
                    let rq = n[A_R] * inv_d;
                    let gq = n[A_G] * inv_d;
                    let bq = n[A_B] * inv_d;
                    let alq = n[A_A] * inv_d;
                    let nxq = n[A_NX] * inv_d;
                    let nyq = n[A_NY] * inv_d;
                    let nzq = n[A_NZ] * inv_d;
                    for a in 0..NA {
                        n[a] += dn[a];
                    }
                    if iwp.abs() <= 1e-12 {
                        // деградированный iw: z пишется (opaque), цвет нет
                        if !blend_pass {
                            r3.zbuf[pi] = z;
                        }
                        continue;
                    }
                    // истинная глубина = 1/(1/w) = w_clip — она же глубина тумана
                    let f = 1.0 / iwp;

                    // сглаженные нормали и шейдинг Фонга (этап: per-pixel normal)
                    let (pix_nnrm, pix_ndl) = if smooth_shading {
                        let nx = nxq * f;
                        let ny = nyq * f;
                        let nz = nzq * f;
                        let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
                        if nlen < 1e-9 {
                            (nnrm, if nl < 1e-9 { 0.0 } else { dot3(&nnrm, light) })
                        } else {
                            let inv_nlen = 1.0 / nlen;
                            let norm = [nx * inv_nlen, ny * inv_nlen, nz * inv_nlen];
                            let mut ndl = dot3(&norm, light);
                            if blend_pass && ndl < 0.0 {
                                ndl = -ndl;
                            }
                            (norm, ndl)
                        }
                    } else {
                        let mut ndl = if nl < 1e-9 { 0.0 } else { dot3(&nnrm, light) };
                        if blend_pass && ndl < 0.0 {
                            ndl = -ndl;
                        }
                        (nnrm, ndl)
                    };

                    let need_world_pos =
                        spec_on || r3.shadow_enabled || !r3.point_lights.is_empty();
                    let (wx, wy, wz) = if need_world_pos {
                        (wxq * f, wyq * f, wzq * f)
                    } else {
                        (0.0, 0.0, 0.0)
                    };

                    let shadow_factor = if r3.shadow_enabled {
                        shadow_sample(r3, wx, wy, wz, r3.shadow_bias)
                    } else {
                        1.0
                    };

                    let dir_diffuse = diffuse * pix_ndl.max(0.0) * shadow_factor;
                    let dir_shade = (ambient + dir_diffuse).clamp(0.0, 1.0);

                    // блик Blinn-Phong (этап 7): half-vector на
                    // перспектива-корректной мировой позиции пикселя
                    let s = if spec_on {
                        let v = [r3.view[0] - wx, r3.view[1] - wy, r3.view[2] - wz];
                        let vl = dot3(&v, &v).sqrt().max(1e-9);
                        let hv = [
                            light[0] + v[0] / vl,
                            light[1] + v[1] / vl,
                            light[2] + v[2] / vl,
                        ];
                        let hl = dot3(&hv, &hv).sqrt().max(1e-9);
                        spec_highlight(
                            dot3(&pix_nnrm, &[hv[0] / hl, hv[1] / hl, hv[2] / hl]),
                            shininess,
                            spec,
                        ) * shadow_factor
                    } else {
                        0.0
                    };

                    let (pt_diff, pt_spec) = compute_point_lights(
                        &r3.point_lights,
                        &r3.view,
                        wx,
                        wy,
                        wz,
                        &pix_nnrm,
                        blend_pass,
                        spec_on,
                        shininess,
                        spec,
                    );

                    // туман (этап 7): фактор по LUT (интерполяция между
                    // узлами), смешение ПОСЛЕ шейдинга и блика
                    let fogf = if fog_on {
                        fog_lut_lookup(&lut, fog_density, f)
                    } else {
                        0.0
                    };

                    // базовый цвет пикселя: per-vertex градиент (этап 9)
                    // или равномерный цвет грани × Lambert / Phong + point lights
                    let (mut c0, mut c1, mut c2) = if !vc_rgb {
                        (
                            rq * f * dir_shade + rq * f * pt_diff[0],
                            gq * f * dir_shade + gq * f * pt_diff[1],
                            bq * f * dir_shade + bq * f * pt_diff[2],
                        )
                    } else {
                        (
                            tr * dir_shade + tr * pt_diff[0],
                            tg * dir_shade + tg * pt_diff[1],
                            tb * dir_shade + tb * pt_diff[2],
                        )
                    };
                    let mut al = if blend_pass { alq.clamp(0.0, 1.0) } else { 1.0 };
                    if let Some(t) = tex {
                        // перспектива-корректный UV: (u/w)/(1/w)
                        // тайлинг (этап 7): (1,1) — clamp к краю,
                        // иначе repeat через fract
                        let uf = tiled_coord(uq * f, tu);
                        let vf = tiled_coord(vq * f, tv);
                        let texel = sample_tex(
                            t,
                            r3.tex_filter,
                            r3.tex_mipmaps,
                            tex_grad.as_ref(),
                            uq,
                            vq,
                            iwp,
                            uf,
                            vf,
                        );
                        // alpha-test (этап 7, только opaque): discard
                        // ДО записи z — за дыркой видно то, что позади
                        if alpha_on && !blend_pass && !alpha_keep(texel[3], alpha_thr) {
                            continue;
                        }
                        if blend_pass {
                            al *= texel[3] as f32 / 255.0;
                            if al <= 0.004 {
                                continue; // полностью прозрачно
                            }
                        }
                        c0 *= texel[0] as f32 / 255.0;
                        c1 *= texel[1] as f32 / 255.0;
                        c2 *= texel[2] as f32 / 255.0;
                    } else if blend_pass && al <= 0.004 {
                        continue;
                    }
                    if spec_on || pt_spec[0] > 0.0 || pt_spec[1] > 0.0 || pt_spec[2] > 0.0 {
                        c0 = (c0 + s + pt_spec[0]).min(1.0);
                        c1 = (c1 + s + pt_spec[1]).min(1.0);
                        c2 = (c2 + s + pt_spec[2]).min(1.0);
                    }
                    if fog_on {
                        let g = 1.0 - fogf;
                        c0 = c0 * g + fog_r * fogf;
                        c1 = c1 * g + fog_g * fogf;
                        c2 = c2 * g + fog_b * fogf;
                    }
                    let o = pi * 4;
                    if blend_pass {
                        // src-alpha блендинг поверх кадра; z-буфер
                        // НЕ пишется (порядок даёт сорт far→near)
                        let dr = fb[o] as f32 / 255.0;
                        let dg = fb[o + 1] as f32 / 255.0;
                        let db = fb[o + 2] as f32 / 255.0;
                        let iv = 1.0 - al;
                        fb[o] = col((c0 * al + dr * iv) as f64);
                        fb[o + 1] = col((c1 * al + dg * iv) as f64);
                        fb[o + 2] = col((c2 * al + db * iv) as f64);
                    } else {
                        r3.zbuf[pi] = z;
                        fb[o] = col(c0 as f64);
                        fb[o + 1] = col(c1 as f64);
                        fb[o + 2] = col(c2 as f64);
                    }
                    fb[o + 3] = 255;
                }
            }
            *drawn += 1;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_line(fb: &mut [u8], w: usize, h: usize, a: Vtx, b: Vtx, r: u8, g: u8, bl: u8) {
    let steps = ((b.sx - a.sx).abs().max((b.sy - a.sy).abs()).ceil() as usize).clamp(1, 4096);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (a.sx + (b.sx - a.sx) * t).round() as i64;
        let y = (a.sy + (b.sy - a.sy) * t).round() as i64;
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            continue;
        }
        let o = (y as usize * w + x as usize) * 4;
        fb[o] = r;
        fb[o + 1] = g;
        fb[o + 2] = bl;
        fb[o + 3] = 255;
    }
}

fn check_tex_uv(uv: Option<&[f32]>, n: usize) -> Result<(), String> {
    if let Some(u) = uv {
        if u.len() != n * 2 {
            return Err(format!(
                "gpu3d: uv — пары на вершину ({}), вершин {}",
                u.len(),
                n
            ));
        }
    }
    Ok(())
}

fn val_to_mat4(val: &Value) -> Result<[f32; 16], String> {
    let list = to_f32s(val)?;
    if list.len() != 16 {
        return Err(format!(
            "матрица 4x4 должна содержать 16 элементов, получено {}",
            list.len()
        ));
    }
    let mut m = [0.0f32; 16];
    m.copy_from_slice(&list[..16]);
    Ok(m)
}

pub fn register_render3d(env: Gc, hal: &Rc<RefCell<Hal>>) {
    fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: Gc, name: &str, f: F) {
        env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
    }
    use need_args as need;

    let hc = hal.clone();
    def(env, "gpu3d-clear", move |args| {
        need(args, 3)?;
        let r = col(num_val(&args[0])?);
        let g = col(num_val(&args[1])?);
        let b = col(num_val(&args[2])?);
        let mut h = hc.borrow_mut();
        h.clear_3d(r, g, b);
        Ok(Value::Nil)
    });

    // свет: (gpu3d-light dx dy dz ambient [diffuse [spec [shininess]]]) —
    // опц. аргументы этапа 7: сила блика Blinn-Phong и его показатель.
    // Прежние вызовы (4-5 аргументов) ведут себя байт-в-байт как раньше.
    let hc = hal.clone();
    def(env, "gpu3d-light", move |args| {
        let mut h = hc.borrow_mut();
        apply_light_args(&mut h.r3, args)?;
        Ok(Value::Nil)
    });

    // позиция камеры в мире (для half-vector блика Blinn-Phong)
    let hc = hal.clone();
    def(env, "gpu3d-view", move |args| {
        need(args, 3)?;
        let mut h = hc.borrow_mut();
        h.r3.view = [
            num_val(&args[0])? as f32,
            num_val(&args[1])? as f32,
            num_val(&args[2])? as f32,
        ];
        Ok(Value::Nil)
    });

    // тайлинг UV: (gpu3d-mat-tiling u v) — множители перед семплингом
    // (repeat-текстуры: трава, стены). (gpu3d-mat-tiling 1 1) — сброс/выкл.
    let hc = hal.clone();
    def(env, "gpu3d-mat-tiling", move |args| {
        need(args, 2)?;
        let mut h = hc.borrow_mut();
        h.r3.tiling = [
            num_val(&args[0])?.clamp(0.0, 1024.0) as f32,
            num_val(&args[1])?.clamp(0.0, 1024.0) as f32,
        ];
        Ok(Value::Nil)
    });

    // alpha-test (cutout): (gpu3d-alpha-test threshold) —
    // 0 = выкл (дефолт); 0.1..1 — тексели с alpha < порога отбрасываются
    // ДО записи в z-буфер (за дыркой в билборде видно то, что позади).
    let hc = hal.clone();
    def(env, "gpu3d-alpha-test", move |args| {
        need(args, 1)?;
        let mut h = hc.borrow_mut();
        h.r3.alpha_test = num_val(&args[0])?.clamp(0.0, 1.0) as f32;
        Ok(Value::Nil)
    });

    // туман: (gpu3d-fog r g b density) — экспоненциальный, по истинной
    // глубине (1/w), смешение после шейдинга; density 0 = выкл (дефолт).
    let hc = hal.clone();
    def(env, "gpu3d-fog", move |args| {
        need(args, 4)?;
        let mut h = hc.borrow_mut();
        h.r3.fog = [
            num_val(&args[0])?.clamp(0.0, 1.0) as f32,
            num_val(&args[1])?.clamp(0.0, 1.0) as f32,
            num_val(&args[2])?.clamp(0.0, 1.0) as f32,
            num_val(&args[3])?.clamp(0.0, 100.0) as f32,
        ];
        Ok(Value::Nil)
    });

    // инспекция состояния конвейера (материалы/редактор/отладка):
    // таблица с ключами light/ambient/diffuse/spec/shininess/view/tiling/
    // alpha-test/fog
    let hc = hal.clone();
    def(env, "gpu3d-state", move |_: &[Value]| {
        let h = hc.borrow();
        let r3 = &h.r3;
        let v3 = |a: [f32; 3]| {
            Value::List(vec![
                Value::Float(a[0] as f64),
                Value::Float(a[1] as f64),
                Value::Float(a[2] as f64),
            ])
        };
        let mut t = std::collections::HashMap::new();
        t.insert("light".into(), v3(r3.light));
        t.insert("ambient".into(), Value::Float(r3.ambient as f64));
        t.insert("diffuse".into(), Value::Float(r3.diffuse as f64));
        t.insert("spec".into(), Value::Float(r3.spec as f64));
        t.insert("shininess".into(), Value::Float(r3.shininess as f64));
        t.insert("view".into(), v3(r3.view));
        t.insert(
            "tiling".into(),
            Value::List(vec![
                Value::Float(r3.tiling[0] as f64),
                Value::Float(r3.tiling[1] as f64),
            ]),
        );
        t.insert("alpha-test".into(), Value::Float(r3.alpha_test as f64));
        t.insert("blend".into(), Value::Float(r3.blend as f64));
        t.insert(
            "shading".into(),
            Value::Sym(if r3.smooth {
                "smooth".into()
            } else {
                "flat".into()
            }),
        );
        t.insert(
            "filter".into(),
            Value::Sym(match r3.tex_filter {
                TexFilter::Nearest => "nearest".into(),
                TexFilter::Bilinear => "bilinear".into(),
            }),
        );
        t.insert("mipmaps".into(), Value::Bool(r3.tex_mipmaps));
        t.insert(
            "fog".into(),
            Value::List(vec![
                Value::Float(r3.fog[0] as f64),
                Value::Float(r3.fog[1] as f64),
                Value::Float(r3.fog[2] as f64),
                Value::Float(r3.fog[3] as f64),
            ]),
        );
        t.insert("shadow-enabled".into(), Value::Bool(r3.shadow_enabled));
        t.insert("shadow-bias".into(), Value::Float(r3.shadow_bias as f64));
        t.insert(
            "point-lights".into(),
            Value::Int(r3.point_lights.len() as i64),
        );
        Ok(Value::Table(Rc::new(RefCell::new(t))))
    });

    // — этап 4 «освещение и тени»: Shadow Mapping и Point Lights —

    // подготовка камеры источника света и прохода теней:
    // (gpu3d-shadow-begin light-matrix [w [h [bias]]]) или (gpu3d-shadow-begin dx dy dz size [res [bias]])
    let hc = hal.clone();
    def(env, "gpu3d-shadow-begin", move |args| {
        need(args, 1)?;
        let mut h = hc.borrow_mut();
        if args.len() >= 4 && matches!(&args[0], Value::Float(_) | Value::Int(_)) {
            let dx = num_val(&args[0])? as f32;
            let dy = num_val(&args[1])? as f32;
            let dz = num_val(&args[2])? as f32;
            let size = num_val(&args[3])? as f32;
            let res = if let Some(v) = args.get(4) {
                crate::interp::int_val(v)? as usize
            } else {
                512
            };
            let bias = if let Some(v) = args.get(5) {
                Some(num_val(v)? as f32)
            } else {
                None
            };
            let m = compute_light_matrix([dx, dy, dz], [0.0, 0.0, 0.0], size, 1.0, 200.0);
            shadow_begin(&mut h.r3, m, res, res, bias);
        } else {
            let m = val_to_mat4(&args[0])?;
            let w = if let Some(v) = args.get(1) {
                crate::interp::int_val(v)? as usize
            } else {
                512
            };
            let h_res = if let Some(v) = args.get(2) {
                crate::interp::int_val(v)? as usize
            } else {
                w
            };
            let bias = if let Some(v) = args.get(3) {
                Some(num_val(v)? as f32)
            } else {
                None
            };
            shadow_begin(&mut h.r3, m, w, h_res, bias);
        }
        Ok(Value::Nil)
    });

    // завершение записи теневой карты
    let hc = hal.clone();
    def(env, "gpu3d-shadow-end", move |_: &[Value]| {
        let mut h = hc.borrow_mut();
        shadow_end(&mut h.r3);
        Ok(Value::Nil)
    });

    // сброс/выключение теней
    let hc = hal.clone();
    def(env, "gpu3d-shadow-clear", move |_: &[Value]| {
        let mut h = hc.borrow_mut();
        shadow_clear(&mut h.r3);
        Ok(Value::Nil)
    });

    // управление отображением теней: (gpu3d-shadow-enable [bool]) -> bool
    let hc = hal.clone();
    def(env, "gpu3d-shadow-enable", move |args| {
        let mut h = hc.borrow_mut();
        if !args.is_empty() {
            h.r3.shadow_enabled = match &args[0] {
                Value::Bool(b) => *b,
                Value::Int(i) => *i != 0,
                Value::Nil => false,
                _ => true,
            };
        }
        Ok(Value::Bool(h.r3.shadow_enabled))
    });

    // проверка попадания точки в тень (для тестирования или геймплея на Lisp)
    let hc = hal.clone();
    def(env, "gpu3d-shadow-sample", move |args| {
        need(args, 1)?;
        let h = hc.borrow();
        let (wx, wy, wz, bias) = if args.len() == 1
            || (args.len() == 2 && matches!(&args[0], Value::List(_) | Value::F32Vec(_)))
        {
            let v = to_f32s(&args[0])?;
            if v.len() < 3 {
                return Err("gpu3d-shadow-sample: массив должен содержать xyz".into());
            }
            let b = if let Some(bv) = args.get(1) {
                num_val(bv)? as f32
            } else {
                h.r3.shadow_bias
            };
            (v[0], v[1], v[2], b)
        } else {
            need(args, 3)?;
            let x = num_val(&args[0])? as f32;
            let y = num_val(&args[1])? as f32;
            let z = num_val(&args[2])? as f32;
            let b = if let Some(bv) = args.get(3) {
                num_val(bv)? as f32
            } else {
                h.r3.shadow_bias
            };
            (x, y, z, b)
        };
        let s = shadow_sample(&h.r3, wx, wy, wz, bias);
        Ok(Value::Float(s as f64))
    });

    // смещение глубины тени (shadow bias)
    let hc = hal.clone();
    def(env, "gpu3d-shadow-bias", move |args| {
        let mut h = hc.borrow_mut();
        if !args.is_empty() {
            h.r3.shadow_bias = num_val(&args[0])? as f32;
        }
        Ok(Value::Float(h.r3.shadow_bias as f64))
    });

    // добавление точечного источника: (gpu3d-add-light ['point] x y z r g b radius [attenuation])
    let hc = hal.clone();
    def(env, "gpu3d-add-light", move |args| {
        let offset = if !args.is_empty() && matches!(&args[0], Value::Sym(_) | Value::Str(_)) {
            1
        } else {
            0
        };
        if args.len() < offset + 7 {
            return Err("gpu3d-add-light: ['point] x y z r g b radius [attenuation]".into());
        }
        let x = num_val(&args[offset])? as f32;
        let y = num_val(&args[offset + 1])? as f32;
        let z = num_val(&args[offset + 2])? as f32;
        let r = num_val(&args[offset + 3])? as f32;
        let g = num_val(&args[offset + 4])? as f32;
        let b = num_val(&args[offset + 5])? as f32;
        let radius = num_val(&args[offset + 6])? as f32;
        let attenuation = if let Some(av) = args.get(offset + 7) {
            num_val(av)? as f32
        } else {
            1.0
        };
        let mut h = hc.borrow_mut();
        h.r3.point_lights.push(PointLight {
            pos: [x, y, z],
            color: [r, g, b],
            radius,
            attenuation,
        });
        Ok(Value::Int(h.r3.point_lights.len() as i64))
    });

    let hc = hal.clone();
    def(env, "gpu3d-add-point-light", move |args| {
        if args.len() < 7 {
            return Err("gpu3d-add-point-light: x y z r g b radius [attenuation]".into());
        }
        let x = num_val(&args[0])? as f32;
        let y = num_val(&args[1])? as f32;
        let z = num_val(&args[2])? as f32;
        let r = num_val(&args[3])? as f32;
        let g = num_val(&args[4])? as f32;
        let b = num_val(&args[5])? as f32;
        let radius = num_val(&args[6])? as f32;
        let attenuation = if let Some(av) = args.get(7) {
            num_val(av)? as f32
        } else {
            1.0
        };
        let mut h = hc.borrow_mut();
        h.r3.point_lights.push(PointLight {
            pos: [x, y, z],
            color: [r, g, b],
            radius,
            attenuation,
        });
        Ok(Value::Int(h.r3.point_lights.len() as i64))
    });

    // очистка точечных источников: (gpu3d-clear-lights)
    let hc = hal.clone();
    def(env, "gpu3d-clear-lights", move |_: &[Value]| {
        let mut h = hc.borrow_mut();
        let count = h.r3.point_lights.len();
        h.r3.point_lights.clear();
        Ok(Value::Int(count as i64))
    });

    // alpha-blend (этап 9): (gpu3d-alpha-blend opacity) — равномерная альфа
    // материала; всё с итоговой альфой < 1 уходит в прозрачную очередь
    // (gpu3d-flush-alpha: сорт far→near, src-alpha блендинг, без z-записи).
    // 1 (или 0) = непрозрачно, прежнее поведение.
    let hc = hal.clone();
    def(env, "gpu3d-alpha-blend", move |args| {
        need(args, 1)?;
        let mut h = hc.borrow_mut();
        h.r3.blend = num_val(&args[0])?.clamp(0.0, 1.0) as f32;
        Ok(Value::Nil)
    });

    // режим шейдинга: (gpu3d-shading ['flat | 'smooth]) —
    // по умолчанию 'flat (ретро-стиль), 'smooth (сглаженный Фонг)
    let hc = hal.clone();
    def(env, "gpu3d-shading", move |args| {
        let mut h = hc.borrow_mut();
        if args.is_empty() {
            let s = if h.r3.smooth { "smooth" } else { "flat" };
            return Ok(Value::Sym(s.into()));
        }
        let smooth = match &args[0] {
            Value::Sym(s) => s == "smooth",
            Value::Str(s) => s == "smooth",
            Value::Bool(b) => *b,
            _ => false,
        };
        h.r3.smooth = smooth;
        Ok(Value::Sym(if smooth { "smooth" } else { "flat" }.into()))
    });

    // режим фильтрации текстур: (gpu3d-filter ['nearest | 'bilinear])
    let hc = hal.clone();
    def(env, "gpu3d-filter", move |args| {
        let mut h = hc.borrow_mut();
        if args.is_empty() {
            let s = match h.r3.tex_filter {
                TexFilter::Nearest => "nearest",
                TexFilter::Bilinear => "bilinear",
            };
            return Ok(Value::Sym(s.into()));
        }
        let filter = match &args[0] {
            Value::Sym(s) if s == "bilinear" => TexFilter::Bilinear,
            Value::Str(s) if s == "bilinear" => TexFilter::Bilinear,
            _ => TexFilter::Nearest,
        };
        h.r3.tex_filter = filter;
        let s = match filter {
            TexFilter::Nearest => "nearest",
            TexFilter::Bilinear => "bilinear",
        };
        Ok(Value::Sym(s.into()))
    });

    // использование пирамиды мипмапов: (gpu3d-mipmaps [#t | #f])
    let hc = hal.clone();
    def(env, "gpu3d-mipmaps", move |args| {
        let mut h = hc.borrow_mut();
        if args.is_empty() {
            return Ok(Value::Bool(h.r3.tex_mipmaps));
        }
        let mm = match &args[0] {
            Value::Bool(b) => *b,
            Value::Sym(s) => s != "nil" && s != "#f" && s != "false",
            Value::Int(i) => *i != 0,
            _ => true,
        };
        h.r3.tex_mipmaps = mm;
        Ok(Value::Bool(mm))
    });

    // расчёт сглаженных нормалей вершин сетки (усреднение нормалей смежных треугольников)
    def(env, "gpu3d-compute-normals", |args| {
        need(args, 2)?;
        let pos = to_f32s(&args[0])?;
        let idx = to_f32s(&args[1])?;
        if pos.len() % 3 != 0 || pos.is_empty() {
            return Err("gpu3d-compute-normals: pos должен быть массивом xyz".into());
        }
        if idx.len() % 3 != 0 {
            return Err("gpu3d-compute-normals: idx должен быть массивом треугольников".into());
        }
        let n = pos.len() / 3;
        let mut nrm = vec![0.0f32; n * 3];
        #[allow(clippy::chunks_exact_to_as_chunks)]
        for tri in idx.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 < n && i1 < n && i2 < n {
                let v0 = [pos[i0 * 3], pos[i0 * 3 + 1], pos[i0 * 3 + 2]];
                let v1 = [pos[i1 * 3], pos[i1 * 3 + 1], pos[i1 * 3 + 2]];
                let v2 = [pos[i2 * 3], pos[i2 * 3 + 1], pos[i2 * 3 + 2]];
                let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
                let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
                let fnrm = cross3(&e1, &e2);
                for &vi in &[i0, i1, i2] {
                    nrm[vi * 3] += fnrm[0];
                    nrm[vi * 3 + 1] += fnrm[1];
                    nrm[vi * 3 + 2] += fnrm[2];
                }
            }
        }
        for i in 0..n {
            let nx = nrm[i * 3];
            let ny = nrm[i * 3 + 1];
            let nz = nrm[i * 3 + 2];
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if len > 1e-9 {
                nrm[i * 3] /= len;
                nrm[i * 3 + 1] /= len;
                nrm[i * 3 + 2] /= len;
            } else {
                nrm[i * 3 + 1] = 1.0;
            }
        }
        Ok(Value::F32Vec(Rc::new(RefCell::new(nrm))))
    });

    // дренаж прозрачной очереди: сорт far→near + блендинг
    let hc = hal.clone();
    def(env, "gpu3d-flush-alpha", move |_: &[Value]| {
        let mut h = hc.borrow_mut();
        let n = flush_transparent(&mut h)?;
        Ok(Value::Int(n as i64))
    });

    // mesh с per-vertex RGBA (этап 9):
    // (gpu3d-mesh-vc pos uv rgba idx model mvp [tex]) — rgba: n*4 f32 0..1;
    // альфа/цвет градиентны (вода: френель/пена/глубина; тени: glow).
    // uv обязателен при tex ≠ 0; tex 0 = без текстуры. Вершины с альфой < 1 —
    // в прозрачную очередь.
    let hc = hal.clone();
    def(env, "gpu3d-mesh-vc", move |args| {
        let n = args.len();
        if n != 7 && n != 8 {
            return Err("gpu3d-mesh-vc: (pos uv rgba idx model mvp [tex])".into());
        }
        let pos = to_f32s(&args[0])?;
        let uv = to_f32s(&args[1])?;
        let rgba = to_f32s(&args[2])?;
        let idx = to_f32s(&args[3])?;
        let model = to_f32s(&args[4])?;
        let mvp = to_f32s(&args[5])?;
        let texid = if n == 8 {
            crate::interp::int_val(&args[6])?
        } else {
            0
        };
        let mut h = hc.borrow_mut();
        let drawn = mesh_draw(
            &mut h,
            &pos,
            Some(&uv),
            Some(&rgba),
            None,
            &idx,
            &model,
            &mvp,
            1.0,
            1.0,
            1.0,
            texid,
            Prim::Tri,
        )?;
        Ok(Value::Int(drawn as i64))
    });

    let hc = hal.clone();
    def(env, "gpu3d-mesh", move |args| {
        let n = args.len();
        if n != 7 && n != 8 {
            return Err("gpu3d-mesh: (pos idx model mvp r g b [nrm])".into());
        }
        let pos = to_f32s(&args[0])?;
        let idx = to_f32s(&args[1])?;
        let model = to_f32s(&args[2])?;
        let mvp = to_f32s(&args[3])?;
        let r = num_val(&args[4])?;
        let g = num_val(&args[5])?;
        let b = num_val(&args[6])?;
        let nrm = if n == 8 {
            Some(to_f32s(&args[7])?)
        } else {
            None
        };
        let mut h = hc.borrow_mut();
        let n = mesh_draw(
            &mut h,
            &pos,
            None,
            None,
            nrm.as_deref(),
            &idx,
            &model,
            &mvp,
            r,
            g,
            b,
            0,
            Prim::Tri,
        )?;
        Ok(Value::Int(n as i64))
    });

    // текстурированный mesh: (gpu3d-mesh-tex pos uv idx tex model mvp r g b [nrm])
    let hc = hal.clone();
    def(env, "gpu3d-mesh-tex", move |args| {
        let n = args.len();
        if n != 9 && n != 10 {
            return Err("gpu3d-mesh-tex: (pos uv idx tex model mvp r g b [nrm])".into());
        }
        let pos = to_f32s(&args[0])?;
        let uv = to_f32s(&args[1])?;
        let idx = to_f32s(&args[2])?;
        let texid = crate::interp::int_val(&args[3])?;
        let model = to_f32s(&args[4])?;
        let mvp = to_f32s(&args[5])?;
        let r = num_val(&args[6])?;
        let g = num_val(&args[7])?;
        let b = num_val(&args[8])?;
        let nrm = if n == 10 {
            Some(to_f32s(&args[9])?)
        } else {
            None
        };
        check_tex_uv(Some(&uv), pos.len() / 3)?;
        let mut h = hc.borrow_mut();
        let n = mesh_draw(
            &mut h,
            &pos,
            Some(&uv),
            None,
            nrm.as_deref(),
            &idx,
            &model,
            &mvp,
            r,
            g,
            b,
            texid,
            Prim::Tri,
        )?;
        Ok(Value::Int(n as i64))
    });

    // скиннинг по весам (этап 21): (gpu3d-mesh-skin pos joints weights jmat
    // model mvp uv texid r g b) — поза суставов уже посчитана на Lisp
    // (мир узла × inverseBind), здесь — блендинг до 4 влияний на вершину
    // (нормировка весов по сумме) и растеризация текстурированного меша
    let hc = hal.clone();
    def(env, "gpu3d-mesh-skin", move |args| {
        need(args, 11)?;
        let pos = to_f32s(&args[0])?;
        let joints = to_f32s(&args[1])?;
        let weights = to_f32s(&args[2])?;
        let jmat = to_f32s(&args[3])?;
        let model = to_f32s(&args[4])?;
        let mvp = to_f32s(&args[5])?;
        let uv = to_f32s(&args[6])?;
        let texid = crate::interp::int_val(&args[7])?;
        let r = num_val(&args[8])?;
        let g = num_val(&args[9])?;
        let b = num_val(&args[10])?;
        let out = skin_blend(&pos, &joints, &weights, &jmat)?;
        let nv = pos.len() / 3;
        // двусторонность: mesh-путь куll'ит backfaces, а скиннед-модель
        // игрок видит с ЛЮБОЙ стороны (нос к камере = все грани CW) —
        // добавляем реверсивные треугольники (2× tris, честно дёшево)
        let mut seq: Vec<f32> = (0..nv).map(|i| i as f32).collect();
        let mut i = 0;
        while i + 2 < nv {
            seq.push((i + 2) as f32);
            seq.push((i + 1) as f32);
            seq.push(i as f32);
            i += 3;
        }
        check_tex_uv(Some(&uv), nv)?;
        let mut h = hc.borrow_mut();
        let n = mesh_draw(
            &mut h,
            &out,
            Some(&uv),
            None,
            None, // нормали не используются (flat-шейдинг по граням)
            &seq,
            &model,
            &mvp,
            r,
            g,
            b,
            texid,
            Prim::Tri,
        )?;
        Ok(Value::Int(n as i64))
    });

    // батч: (gpu3d-batch pos idx insts mvp [nrm]) — цвет/трансформ в инстансах
    let hc = hal.clone();
    def(env, "gpu3d-batch", move |args| {
        let n = args.len();
        if n != 4 && n != 5 {
            return Err("gpu3d-batch: (pos idx insts mvp [nrm])".into());
        }
        let pos = to_f32s(&args[0])?;
        let idx = to_f32s(&args[1])?;
        let insts = to_f32s(&args[2])?;
        let mvp = to_f32s(&args[3])?;
        let nrm = if n == 5 {
            Some(to_f32s(&args[4])?)
        } else {
            None
        };
        let mut h = hc.borrow_mut();
        let n = batch_draw(&mut h, &pos, None, nrm.as_deref(), &idx, &insts, &mvp, 0)?;
        Ok(Value::Int(n as i64))
    });

    // матричный батч (этап 16): (gpu3d-batch-mat pos idx mats mvp) —
    // инстансы по 20 f32: матрица 4×4 + цвет; источник — skel-batch!
    let hc = hal.clone();
    def(env, "gpu3d-batch-mat", move |args| {
        need(args, 4)?;
        let pos = to_f32s(&args[0])?;
        let idx = to_f32s(&args[1])?;
        let mats = to_f32s(&args[2])?;
        let mvp = to_f32s(&args[3])?;
        let mut h = hc.borrow_mut();
        let n = batch_draw_mat(&mut h, &pos, &idx, &mats, &mvp)?;
        Ok(Value::Int(n as i64))
    });

    // текстурный батч: (gpu3d-batch-tex pos uv idx tex insts mvp [nrm])
    let hc = hal.clone();
    def(env, "gpu3d-batch-tex", move |args| {
        let n = args.len();
        if n != 6 && n != 7 {
            return Err("gpu3d-batch-tex: (pos uv idx tex insts mvp [nrm])".into());
        }
        let pos = to_f32s(&args[0])?;
        let uv = to_f32s(&args[1])?;
        let idx = to_f32s(&args[2])?;
        let texid = crate::interp::int_val(&args[3])?;
        let insts = to_f32s(&args[4])?;
        let mvp = to_f32s(&args[5])?;
        let nrm = if n == 7 {
            Some(to_f32s(&args[6])?)
        } else {
            None
        };
        check_tex_uv(Some(&uv), pos.len() / 3)?;
        let mut h = hc.borrow_mut();
        let n = batch_draw(
            &mut h,
            &pos,
            Some(&uv),
            nrm.as_deref(),
            &idx,
            &insts,
            &mvp,
            texid,
        )?;
        Ok(Value::Int(n as i64))
    });

    let hc = hal.clone();
    def(env, "gpu3d-wire", move |args| {
        need(args, 7)?;
        let pos = to_f32s(&args[0])?;
        let idx = to_f32s(&args[1])?;
        let model = to_f32s(&args[2])?;
        let mvp = to_f32s(&args[3])?;
        let r = num_val(&args[4])?;
        let g = num_val(&args[5])?;
        let b = num_val(&args[6])?;
        let mut h = hc.borrow_mut();
        let n = mesh_draw(
            &mut h,
            &pos,
            None,
            None,
            None,
            &idx,
            &model,
            &mvp,
            r,
            g,
            b,
            0,
            Prim::Wire,
        )?;
        Ok(Value::Int(n as i64))
    });

    // пост-обработка (этап 19): (gpu3d-post exposure bloom vignette grain)
    let hc = hal.clone();
    def(env, "gpu3d-post", move |args| {
        need(args, 4)?;
        let p = crate::post::PostParams {
            exposure: num_val(&args[0])? as f32,
            bloom: num_val(&args[1])? as f32,
            vignette: num_val(&args[2])? as f32,
            grain: num_val(&args[3])? as f32,
        };
        let mut h = hc.borrow_mut();
        let _ = h.flush_gpu(true); // отложенный 2D-слой — до поста (HUD рисуется позже)
        crate::post::apply(&mut h, &p);
        Ok(Value::Nil)
    });

    let hc = hal.clone();
    def(env, "gpu3d-stats", move |_: &[Value]| {
        let h = hc.borrow();
        Ok(Value::List(vec![
            Value::Int(h.r3.drawn as i64),
            Value::Int(h.r3.culled as i64),
        ]))
    });
}

// ————————————————————————————————————————————————————————————————————————
// Тесты этапа 7 «материалы»: чистые функции ядра шейдинга + разбор
// аргументов света с фолбэком. Растеризация покрывается интеграционно
// (smoke.sh + --capture в M3), здесь — математика.
// ————————————————————————————————————————————————————————————————————————

#[cfg(test)]
mod skin_tests {
    use super::skin_blend;

    #[test]
    fn identity_поза_не_меняет_вершины() {
        // единичные матрицы на все суставы, вес 1 на сустав 0
        let pos = vec![1.0, 2.0, 3.0, -4.0, 0.5, 7.0];
        let joints = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let mut jmat = Vec::new();
        for _ in 0..3 {
            jmat.extend([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]);
        }
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        for k in 0..pos.len() {
            assert!(
                (out[k] - pos[k]).abs() < 1e-5,
                "v{}: {} != {}",
                k,
                out[k],
                pos[k]
            );
        }
    }

    #[test]
    fn блендинг_двух_суставов_линейный() {
        // J0 = сдвиг (1,0,0), J1 = сдвиг (0,0,1); веса 0.5/0.5 → сдвиг (0.5,0,0.5)
        let pos = vec![2.0, 3.0, 4.0];
        let joints = vec![0.0, 1.0, 0.0, 0.0];
        let weights = vec![0.5, 0.5, 0.0, 0.0];
        let j0 = [
            1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let j1 = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let jmat: Vec<f32> = j0.iter().chain(j1.iter()).cloned().collect();
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[0] - 2.5).abs() < 1e-5, "x {}", out[0]);
        assert!((out[1] - 3.0).abs() < 1e-5, "y {}", out[1]);
        assert!((out[2] - 4.5).abs() < 1e-5, "z {}", out[2]);
    }

    #[test]
    fn веса_нормируются_по_сумме() {
        // веса 2/2 вместо 0.5/0.5 — результат тот же (нормировка по сумме)
        let pos = vec![2.0, 3.0, 4.0];
        let joints = vec![0.0, 1.0, 0.0, 0.0];
        let weights = vec![2.0, 2.0, 0.0, 0.0];
        let j0 = [
            1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let j1 = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let jmat: Vec<f32> = j0.iter().chain(j1.iter()).cloned().collect();
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[0] - 2.5).abs() < 1e-5, "x {}", out[0]);
        assert!((out[2] - 4.5).abs() < 1e-5, "z {}", out[2]);
    }

    #[test]
    fn индекс_за_границей_клампится() {
        // 3 сустава, индекс 99 → клампится к 2; нулевой вес не читает суставы
        let pos = vec![1.0, 1.0, 1.0];
        let joints = vec![99.0, 0.0, 0.0, 0.0];
        let weights = vec![1.0, 0.0, 0.0, 0.0];
        let mut jmat = Vec::new();
        // J2 — третья (индекс 2) = сдвиг (0, 5, 0); 99 клампится к 2
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[1] - 6.0).abs() < 1e-5, "y {}", out[1]);
    }

    #[test]
    fn детерминизм_побитовый() {
        let pos: Vec<f32> = (0..300).map(|i| (i % 37) as f32 - 10.0).collect();
        let joints: Vec<f32> = (0..400).map(|i| (i % 5) as f32).collect();
        let weights: Vec<f32> = (0..400).map(|i| ((i % 7) as f32) / 7.0).collect();
        let mut jmat = Vec::new();
        for j in 0..5 {
            let a = 0.3 + j as f32;
            jmat.extend([
                a.cos(),
                0.0,
                a.sin(),
                j as f32,
                0.0,
                1.0,
                0.0,
                0.0,
                -a.sin(),
                0.0,
                a.cos(),
                j as f32 * 0.5,
                0.0,
                0.0,
                0.0,
                1.0,
            ]);
        }
        let a = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        let b = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert_eq!(a, b, "блендинг недетерминирован");
    }

    #[test]
    fn ошибки_размеров() {
        assert!(skin_blend(&[0.0; 3], &[0.0; 3], &[0.0; 4], &[0.0; 16]).is_err());
        assert!(skin_blend(&[0.0; 3], &[0.0; 4], &[0.0; 4], &[0.0; 0]).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn близко(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn кривая_тумана_экспоненциальная_и_монотонная() {
        // density 0 или глубина 0 — тумана нет
        assert!(близко(fog_factor(0.0, 100.0), 0.0));
        assert!(близко(fog_factor(0.25, 0.0), 0.0));
        // эталонные точки: 1 - exp(-d*x)
        assert!(близко(fog_factor(0.1, 10.0), 1.0 - (-1.0f32).exp()));
        assert!(близко(fog_factor(1.0, 5.0), 1.0 - (-5.0f32).exp()));
        assert!(близко(fog_factor(2.0, 2.0), 1.0 - (-4.0f32).exp()));
        // монотонность по обеим переменным, насыщение ≤ 1
        assert!(fog_factor(0.1, 20.0) > fog_factor(0.1, 10.0));
        assert!(fog_factor(0.2, 10.0) > fog_factor(0.1, 10.0));
        assert!(fog_factor(100.0, 100.0) <= 1.0);
        // отрицательные входы (мусор от скрипта) не паникуют и не дают < 0
        assert!(близко(fog_factor(-1.0, 10.0), 0.0));
        assert!(близко(fog_factor(0.1, -5.0), 0.0));
    }

    #[test]
    fn альфа_тест_граница_включительная() {
        // выкл: порог 0 пропускает всё, включая полностью прозрачное
        assert!(alpha_keep(0, 0.0));
        // граница: alpha/255 == порог — тексель ОСТАЁТСЯ
        assert!(alpha_keep(26, 0.1)); // 26/255 = 0.10196 ≥ 0.1
        assert!(!alpha_keep(25, 0.1)); // 25/255 = 0.09804 < 0.1
                                       // полный альфа-канал проходит даже при пороге 1
        assert!(alpha_keep(255, 1.0));
        assert!(!alpha_keep(254, 1.0));
        // порог 0.5: граница 127/128
        assert!(!alpha_keep(127, 0.5)); // 0.498 < 0.5
        assert!(alpha_keep(128, 0.5)); // 0.502 ≥ 0.5
    }

    #[test]
    fn тайлинг_математика_repeat_и_пасsthrough() {
        // tiling 1 — координата без изменений (старое поведение clamp)
        assert!(близко(tiled_coord(0.75, 1.0), 0.75));
        assert!(близко(tiled_coord(0.0, 1.0), 0.0));
        assert!(близко(tiled_coord(1.0, 1.0), 1.0));
        // repeat: u=0.75 при tiling 2 → 1.5 → fract 0.5
        assert!(близко(tiled_coord(0.75, 2.0), 0.5));
        // u=1.0 при tiling 2 → 2.0 → fract 0 (шов стыкуется с началом)
        assert!(близко(tiled_coord(1.0, 2.0), 0.0));
        // u=1.3 при tiling 3 → 3.9 → fract 0.9
        assert!(близко(tiled_coord(1.3, 3.0), 0.9));
        // нулевой tiling схлопывает координату в 0
        assert!(близко(tiled_coord(0.42, 0.0), 0.0));
    }

    #[test]
    fn блик_блинн_фонга_нулевые_и_граничные() {
        // сила 0 — блика нет (старый Lambert-путь, ноль накладных расходов)
        assert!(близко(spec_highlight(0.9, 32.0, 0.0), 0.0));
        // N·H ≤ 0 — блик нет (half-vector за гранью)
        assert!(близко(spec_highlight(0.0, 32.0, 1.0), 0.0));
        assert!(близко(spec_highlight(-0.5, 32.0, 1.0), 0.0));
        // идеальное отражение: N·H = 1 → ровно strength (любая shininess)
        assert!(близко(spec_highlight(1.0, 32.0, 0.7), 0.7));
        assert!(близко(spec_highlight(1.0, 1.0, 0.7), 0.7));
        // рост показателя сужает блик: при N·H=0.5 и shininess 1 → 0.5*strength
        assert!(близко(spec_highlight(0.5, 1.0, 1.0), 0.5));
        assert!(spec_highlight(0.5, 64.0, 1.0) < spec_highlight(0.5, 8.0, 1.0));
        // shininess < 1 из скрипта поднимается до 1 (powf корректен)
        assert!(близко(spec_highlight(0.5, 0.0, 1.0), 0.5));
    }

    #[test]
    fn свет_разбор_аргументов_фолбэк_прежнее_состояние() {
        let f = |x: f64| Value::Float(x);
        // старый вызов 4 аргументов: spec остаётся прежним (0 по умолчанию)
        let mut r3 = R3::new();
        apply_light_args(&mut r3, &[f(0.1), f(0.2), f(0.3), f(0.4)]).unwrap();
        assert!(близко(r3.light[0], 0.1));
        assert!(близко(r3.ambient, 0.4));
        assert!(близко(r3.diffuse, 0.75)); // дефолт не тронут
        assert!(близко(r3.spec, 0.0));
        assert!(близко(r3.shininess, 32.0));
        // 5 аргументов (как было в v0.2+): diffuse задаётся, spec не трогается
        let mut r3 = R3::new();
        apply_light_args(&mut r3, &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9)]).unwrap();
        assert!(близко(r3.diffuse, 0.9));
        assert!(близко(r3.spec, 0.0));
        // 7 аргументов: включает блик
        apply_light_args(
            &mut r3,
            &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(0.6), f(16.0)],
        )
        .unwrap();
        assert!(близко(r3.spec, 0.6));
        assert!(близко(r3.shininess, 16.0));
        // фолбэк: 6 аргументов — shininess сохраняет прежнее значение
        apply_light_args(&mut r3, &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(0.3)]).unwrap();
        assert!(близко(r3.spec, 0.3));
        assert!(близко(r3.shininess, 16.0));
        // мусорные значения: отрицательный spec/shininess срезаются
        apply_light_args(
            &mut r3,
            &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(-1.0), f(0.0)],
        )
        .unwrap();
        assert!(близко(r3.spec, 0.0));
        assert!(близко(r3.shininess, 1.0));
        // мало аргументов — честная ошибка, состояние не меняется
        let mut r3 = R3::new();
        assert!(apply_light_args(&mut r3, &[f(0.1), f(0.2), f(0.3)]).is_err());
        assert!(близко(r3.light[0], 0.45)); // дефолт
    }

    // ——— этап 9: прозрачная очередь, блендинг, mesh-vc ———

    /// Квадрат-во-весь-экран своего цвета на глубине z (плоский свет).
    fn квадрат_цвета(hal: &mut Hal, z: f32, r: f64, g: f64, b: f64) {
        let pos = [-1.0f32, -1.0, z, 1.0, -1.0, z, 1.0, 1.0, z, -1.0, 1.0, z];
        let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
        let mm = ед_матрицы();
        let (model, mvp) = (&mm[0..16], &mm[16..32]);
        mesh_draw(
            hal,
            &pos,
            None,
            None,
            None,
            &idx,
            model,
            mvp,
            r,
            g,
            b,
            0,
            Prim::Tri,
        )
        .unwrap();
    }

    /// Квадрат с обратной (CW) ориентацией — задняя сторона.
    fn квадрат_тыл(hal: &mut Hal, z: f32, r: f64, g: f64, b: f64) {
        let pos = [-1.0f32, -1.0, z, 1.0, -1.0, z, 1.0, 1.0, z, -1.0, 1.0, z];
        let idx = [0.0f32, 2.0, 1.0, 0.0, 3.0, 2.0];
        let mm = ед_матрицы();
        let (model, mvp) = (&mm[0..16], &mm[16..32]);
        mesh_draw(
            hal,
            &pos,
            None,
            None,
            None,
            &idx,
            model,
            mvp,
            r,
            g,
            b,
            0,
            Prim::Tri,
        )
        .unwrap();
    }

    #[test]
    fn блендинг_сорт_far_near_сквозной() {
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        hal.r3.blend = 0.5;
        // порядок постановки НЕ важен: далёкий зелёный, близкий красный
        квадрат_цвета(&mut hal, 0.9, 0.0, 1.0, 0.0);
        квадрат_цвета(&mut hal, 0.5, 1.0, 0.0, 0.0);
        flush_transparent(&mut hal).unwrap();
        // far→near: зелёный 0.5 поверх чёрного → (0, 128, 0);
        // красный 0.5 поверх → (128, 64, 0). Перепутанный порядок дал бы
        // (128, 128, 0).
        let c = пиксель(&hal, 4, 4);
        assert_eq!(c, [128, 64, 0, 255], "сорт far→near сломан: {:?}", c);
        // z-записи нет: z-буфер в точке остался бесконечным
        assert!(hal.r3.zbuf[4 * 8 + 4].is_infinite());
    }

    #[test]
    fn блендинг_z_тест_в_обе_стороны() {
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        // непрозрачный КРАСНЫЙ вписывается первым на z=0.6
        hal.r3.blend = 1.0;
        квадрат_цвета(&mut hal, 0.6, 1.0, 0.0, 0.0);
        // прозрачный ЗЕЛЁНЫЙ ПЕРЕД ним (z=0.4): блендинг поверх
        hal.r3.blend = 0.5;
        квадрат_цвета(&mut hal, 0.4, 0.0, 1.0, 0.0);
        // прозрачный СИНИЙ ЗА ним (z=0.8): z-тест не пропускает
        квадрат_цвета(&mut hal, 0.8, 0.0, 0.0, 1.0);
        flush_transparent(&mut hal).unwrap();
        let c = пиксель(&hal, 4, 4);
        // 0.5 зелёного поверх красного
        assert_eq!(c[0], col(0.5));
        assert_eq!(c[1], col(0.5));
        assert_eq!(c[2], 0, "синий за непрозрачным не должен пройти z-тест");
    }

    #[test]
    fn прозрачный_проход_двусторонний() {
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        hal.r3.blend = 0.6;
        // задняя сторона: opaque-проход её отрисовал бы (куллинг), прозрачный —
        // обязан рисовать (листва видна с обеих сторон)
        квадрат_тыл(&mut hal, 0.5, 1.0, 1.0, 0.0);
        flush_transparent(&mut hal).unwrap();
        let c = пиксель(&hal, 4, 4);
        assert_eq!(c[0], col(0.6));
        assert_eq!(c[1], col(0.6));
    }

    #[test]
    fn mesh_vc_градиент_альфы_перспектива_корректна() {
        // альфа 1 слева → 0 справа, цвет красный: пиксель x=1 (u=-0.625) →
        // a = (1-u)/2 = 0.8125 → красный 207 поверх чёрного фона
        let pos = [
            -1.0f32, -1.0, 0.5, 1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0, 1.0, 0.5,
        ];
        let rgba = [
            1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
        ];
        let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
        let mm = ед_матрицы();
        let (model, mvp) = (&mm[0..16], &mm[16..32]);
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        mesh_draw(
            &mut hal,
            &pos,
            None,
            Some(&rgba),
            None,
            &idx,
            model,
            mvp,
            1.0,
            1.0,
            1.0,
            0,
            Prim::Tri,
        )
        .unwrap();
        flush_transparent(&mut hal).unwrap();
        let c = пиксель(&hal, 1, 4);
        assert_eq!(c[0], col(0.8125), "градиент альфы: {:?}", c);
        assert_eq!(c[1], 0);
        // правый край: центр последнего пикселя u=0.875 → a=(1-u)/2=0.0625
        let c2 = пиксель(&hal, 7, 4);
        assert_eq!(c2, [col(0.0625), 0, 0, 255]);
    }

    #[test]
    fn очередь_чистится_clear_и_flush() {
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        hal.r3.blend = 0.5;
        квадрат_цвета(&mut hal, 0.5, 1.0, 1.0, 1.0);
        assert_eq!(hal.r3.transq.len(), 2); // 2 треугольника квада
        flush_transparent(&mut hal).unwrap();
        assert!(hal.r3.transq.is_empty());
        // clear тоже чистит (забытый flush не портит следующий кадр)
        квадрат_цвета(&mut hal, 0.5, 1.0, 1.0, 1.0);
        hal.r3.reset(8, 8);
        hal.r3.transq.clear();
        assert!(hal.r3.transq.is_empty());
    }

    // ——— интеграционные: растр этапа 7 через mesh_draw (программный путь) ——

    use crate::hal::{Hal, OutputMode, Tex};

    /// Единичные матрицы (model, mvp): клип = мир, w = 1.
    fn ед_матрицы() -> Vec<f32> {
        let mut m = vec![0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        let mut out = m.clone();
        out.extend_from_slice(&m);
        out
    }

    /// Плоский свет для растровых тестов: ambient 1, diffuse 0 → цвет = тексель·тинт.
    fn свет_плоский(hal: &mut Hal) {
        сброс_света(hal);
        hal.r3.light = [0.0, 0.0, 1.0];
        hal.r3.ambient = 1.0;
        hal.r3.diffuse = 0.0;
    }

    /// Квадрат на глубине z во весь экран 8x8 (CCW-лицевой после y-flip).
    fn рисуем_квадрат(hal: &mut Hal, tex: i64, z: f32, uv: &[f32]) {
        let pos = [-1.0f32, -1.0, z, 1.0, -1.0, z, 1.0, 1.0, z, -1.0, 1.0, z];
        let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
        let mm = ед_матрицы();
        let (model, mvp) = (&mm[0..16], &mm[16..32]);
        mesh_draw(
            hal,
            &pos,
            Some(uv),
            None,
            None,
            &idx,
            model,
            mvp,
            1.0,
            1.0,
            1.0,
            tex,
            Prim::Tri,
        )
        .unwrap();
    }

    fn пиксель(hal: &Hal, x: usize, y: usize) -> [u8; 4] {
        let o = (y * hal.w + x) * 4;
        [hal.fb[o], hal.fb[o + 1], hal.fb[o + 2], hal.fb[o + 3]]
    }

    fn сброс_света(hal: &mut Hal) {
        hal.r3.light = [0.45, 0.8, 0.35];
        hal.r3.ambient = 0.35;
        hal.r3.diffuse = 0.75;
        hal.r3.alpha_test = 0.0;
        hal.r3.spec = 0.0;
        hal.r3.shininess = 32.0;
        hal.r3.view = [0.0, 0.0, 0.0];
        hal.r3.tiling = [1.0, 1.0];
        hal.r3.fog = [0.7, 0.8, 0.9, 0.0];
        hal.r3.blend = 1.0;
        hal.r3.transq.clear();
    }

    #[test]
    fn матрица_инстанса_yaw_по_конвенции() {
        // Регрессия: f32::sin_cos возвращает (sin, cos) — прежняя связка
        // перепутывала оси и ломала все батчи. Проверка конвенции
        // M = T·Ry·Rx·Rz·S: yaw=π/2 переводит +X в -Z (колоночный вектор).
        let pi2 = std::f32::consts::FRAC_PI_2;
        let mut m = [0.0f32; 16];
        inst_matrix(
            &[5.0, 6.0, 7.0, 1.0, 1.0, 1.0, pi2, 0.0, 0.0, 1.0, 1.0, 1.0],
            &mut m,
        );
        // вершина (1, 0, 0): world = R·v + t = (0, 0, -1) + (5, 6, 7)
        let wx = m[0] * 1.0 + m[3];
        let wy = m[4] * 1.0 + m[7];
        let wz = m[8] * 1.0 + m[11];
        assert!(близко(wx, 5.0), "wx = {}", wx);
        assert!(близко(wy, 6.0), "wy = {}", wy);
        assert!(близко(wz, 7.0 - 1.0), "wz = {}", wz);
        // масштаб: вершина (0, 2, 0) → (2, 0) → world y = 6 + 2
        let wy2 = m[5] * 2.0 + m[7];
        assert!(близко(wy2, 8.0), "wy2 = {}", wy2);
        // нулевые углы — диагональная матрица масштаба
        inst_matrix(
            &[0.0, 0.0, 0.0, 0.4, 0.4, 0.4, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            &mut m,
        );
        assert!(близко(m[0], 0.4) && близко(m[5], 0.4) && близко(m[10], 0.4));
        assert!(близко(m[1] + m[2] + m[4] + m[6] + m[8] + m[9], 0.0));
    }

    #[test]
    fn растр_alpha_test_вырезает_и_z_не_пишет() {
        // 2x2 текстура: 3 текселя маджента непрозрачные, угловой (u=1,v=1)
        // полностью прозрачный (alpha 0). Далёкий зелёный квадрат на z=0.9,
        // близкий текстурный с дыркой на z=0.5.
        let px = vec![
            255u8, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 9, 9, 9, 0,
        ];
        let зелёный = || Tex::new(1, 1, vec![0, 255, 0, 255]);
        let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let плоский_свет = |hal: &mut Hal| {
            сброс_света(hal);
            hal.r3.light = [0.0, 0.0, 1.0];
            hal.r3.ambient = 1.0;
            hal.r3.diffuse = 0.0; // свет плоский: цвет = тексель·тинт
        };

        // без alpha-test близкий квадрат перекрывает ВСЁ маджентой
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.tex.push(Tex::new(2, 2, px.clone()));
        hal.tex.push(зелёный());
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        рисуем_квадрат(&mut hal, 2, 0.9, &uv);
        assert_eq!(пиксель(&hal, 0, 0), [0, 255, 0, 255]);
        рисуем_квадрат(&mut hal, 1, 0.5, &uv);
        assert_eq!(пиксель(&hal, 0, 0), [255, 0, 255, 255]);
        assert_eq!(пиксель(&hal, 7, 7), [255, 0, 255, 255]);

        // с alpha-test 0.5 прозрачный тексель отброшен — сквозь дыру виден
        // зелёный дальний квадрат: discard не пишет ни цвет, ни z.
        // Ориентация: v=1 → верх экрана (y-flip), u=1 → право; дыра (тексель
        // u=1,v=1) — правый верхний квадрант, напр. пиксель (7,0)
        let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
        hal2.tex.push(Tex::new(2, 2, px.clone()));
        hal2.tex.push(зелёный());
        hal2.fill_all(0, 0, 0);
        плоский_свет(&mut hal2);
        рисуем_квадрат(&mut hal2, 2, 0.9, &uv);
        hal2.r3.alpha_test = 0.5;
        рисуем_квадрат(&mut hal2, 1, 0.5, &uv);
        assert_eq!(пиксель(&hal2, 7, 0), [0, 255, 0, 255]);
        assert_eq!(пиксель(&hal2, 4, 1), [0, 255, 0, 255]);
        // непрозрачные тексели рисуются как раньше
        assert_eq!(пиксель(&hal2, 0, 0), [255, 0, 255, 255]);
        assert_eq!(пиксель(&hal2, 7, 7), [255, 0, 255, 255]);
        assert_eq!(пиксель(&hal2, 0, 7), [255, 0, 255, 255]);
    }

    #[test]
    fn растр_туман_смешивает_после_шейдинга() {
        // белый квадрат, свет вдоль (0, 0.6, 0.8): ndl = 0.8 →
        // shade = 0.35 + 0.75·0.8 = 0.95 → 242
        let свет = |hal: &mut Hal| {
            сброс_света(hal);
            hal.r3.light = [0.0, 0.6, 0.8];
        };
        let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];

        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        свет(&mut hal);
        рисуем_квадрат(&mut hal, 0, 0.5, &uv);
        assert_eq!(пиксель(&hal, 4, 4), [242, 242, 242, 255]);

        // красный туман density 50 на глубине 1.0: фактор ≈ 1 → цвет тумана
        let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
        hal2.fill_all(0, 0, 0);
        свет(&mut hal2);
        hal2.r3.fog = [1.0, 0.0, 0.0, 50.0];
        рисуем_квадрат(&mut hal2, 0, 0.5, &uv);
        assert_eq!(пиксель(&hal2, 4, 4), [255, 0, 0, 255]);

        // слабый туман: точное значение по формуле на истинной глубине w=1
        let f = fog_factor(0.1, 1.0);
        let ожид_r = 0.95 * (1.0 - f) + f;
        let ожид_g = 0.95 * (1.0 - f);
        let mut hal3 = Hal::new(OutputMode::Silent, 8, 8);
        hal3.fill_all(0, 0, 0);
        свет(&mut hal3);
        hal3.r3.fog = [1.0, 0.0, 0.0, 0.1];
        рисуем_квадрат(&mut hal3, 0, 0.5, &uv);
        let c = пиксель(&hal3, 4, 4);
        assert_eq!(c[0], col(ожид_r as f64));
        assert_eq!(c[1], col(ожид_g as f64));
    }

    #[test]
    fn растр_блик_блинн_фонга_ярче_lambert() {
        let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        // камера на +Z (view (0,0,5)), квадрат в z=0.5, свет (0, 0.6, 0.8):
        // Lambert = 0.95 → 242; half-vector блик у центра добавляет ~0.16 →
        // 0.95+0.16 > 1 → clamp → белый
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.fill_all(0, 0, 0);
        сброс_света(&mut hal);
        hal.r3.light = [0.0, 0.6, 0.8];
        рисуем_квадрат(&mut hal, 0, 0.5, &uv);
        let ламберт = пиксель(&hal, 4, 4);
        assert_eq!(ламберт, [242, 242, 242, 255]);

        let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
        hal2.fill_all(0, 0, 0);
        сброс_света(&mut hal2);
        hal2.r3.light = [0.0, 0.6, 0.8];
        hal2.r3.view = [0.0, 0.0, 5.0];
        hal2.r3.spec = 1.0;
        hal2.r3.shininess = 32.0;
        рисуем_квадрат(&mut hal2, 0, 0.5, &uv);
        let с_бликом = пиксель(&hal2, 4, 4);
        assert_eq!(с_бликом, [255, 255, 255, 255]);
        assert!(с_бликом[0] > ламберт[0]);
    }

    #[test]
    fn растр_тайлинг_repeat_меняет_семпл() {
        // 4x1 текстура: столбцы красный / зелёный / синий / белый
        let tex = Tex::new(
            4,
            1,
            vec![
                255, 0, 0, 255, // u ∈ [0, 0.25)
                0, 255, 0, 255, // u ∈ [0.25, 0.5)
                0, 0, 255, 255, // u ∈ [0.5, 0.75)
                255, 255, 255, 255, // u ∈ [0.75, 1)
            ],
        );
        let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let плоский_свет = |hal: &mut Hal| {
            сброс_света(hal);
            hal.r3.light = [0.0, 0.0, 1.0];
            hal.r3.ambient = 1.0;
            hal.r3.diffuse = 0.0;
        };

        // без тайлинга пиксель (4,y): fx=4.5 → u≈0.5625 → столбец 2 (синий)
        let mut hal = Hal::new(OutputMode::Silent, 8, 8);
        hal.tex.push(tex.clone());
        hal.fill_all(0, 0, 0);
        свет_плоский(&mut hal);
        рисуем_квадрат(&mut hal, 1, 0.5, &uv);
        assert_eq!(пиксель(&hal, 4, 4), [0, 0, 255, 255]);
        // край u=0 → столбец 0 (красный)
        assert_eq!(пиксель(&hal, 0, 4), [255, 0, 0, 255]);

        // tiling (2,1): u=0.5625 → 1.125 → fract 0.125 → столбец 0 (красный)
        let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
        hal2.tex.push(tex);
        hal2.fill_all(0, 0, 0);
        плоский_свет(&mut hal2);
        hal2.r3.tiling = [2.0, 1.0];
        рисуем_квадрат(&mut hal2, 1, 0.5, &uv);
        assert_eq!(пиксель(&hal2, 4, 4), [255, 0, 0, 255]);
        // шов стыкуется: u≈0.9375 → 1.875 → fract 0.875 → столбец 3 (белый)
        assert_eq!(пиксель(&hal2, 7, 4), [255, 255, 255, 255]);
    }

    #[test]
    fn smooth_normals() {
        // Квадрат во весь экран (8x8) на z=0.5:
        // Вершины:
        // 0: (-1, -1), 1: (1, -1), 2: (1, 1), 3: (-1, 1)
        // Нормали вершин: левые вершины (0, 3) смотрят вбок (-1, 0, 0),
        // правые вершины (1, 2) смотрят вперед (0, 0, 1).
        // Источник света прямо спереди: light = [0, 0, 1].
        // ambient = 0.2, diffuse = 0.8.
        let pos = [
            -1.0f32, -1.0, 0.5, 1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0, 1.0, 0.5,
        ];
        let nrm = [
            -1.0f32, 0.0, 0.0, // v0
            0.0, 0.0, 1.0, // v1
            0.0, 0.0, 1.0, // v2
            -1.0, 0.0, 0.0, // v3
        ];
        let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
        let mm = ед_матрицы();
        let (model, mvp) = (&mm[0..16], &mm[16..32]);

        // 1. В режиме flat: нормаль треугольников берётся от плоскости грани (0, 0, 1),
        // поэтому ndl = 1.0 везде, цвет равномерный 0.2 + 0.8*1.0 = 1.0 (255)
        let mut hal_flat = Hal::new(OutputMode::Silent, 8, 8);
        hal_flat.fill_all(0, 0, 0);
        сброс_света(&mut hal_flat);
        hal_flat.r3.light = [0.0, 0.0, 1.0];
        hal_flat.r3.ambient = 0.2;
        hal_flat.r3.diffuse = 0.8;
        hal_flat.r3.smooth = false;
        mesh_draw(
            &mut hal_flat,
            &pos,
            None,
            None,
            Some(&nrm),
            &idx,
            model,
            mvp,
            1.0,
            1.0,
            1.0,
            0,
            Prim::Tri,
        )
        .unwrap();
        let left_flat = пиксель(&hal_flat, 0, 4);
        let right_flat = пиксель(&hal_flat, 7, 4);
        assert_eq!(left_flat[0], 255);
        assert_eq!(right_flat[0], 255);

        // 2. В режиме smooth: нормали интерполируются!
        // На левом краю нормаль близка к (-1, 0, 0) -> N·L ≈ 0 -> цвет ≈ ambient (0.2 -> ~51)
        // На правом краю нормаль близка к (0, 0, 1) -> N·L ≈ 1 -> цвет ≈ ambient + diffuse (1.0 -> 255)
        let mut hal_smooth = Hal::new(OutputMode::Silent, 8, 8);
        hal_smooth.fill_all(0, 0, 0);
        сброс_света(&mut hal_smooth);
        hal_smooth.r3.light = [0.0, 0.0, 1.0];
        hal_smooth.r3.ambient = 0.2;
        hal_smooth.r3.diffuse = 0.8;
        hal_smooth.r3.smooth = true;
        mesh_draw(
            &mut hal_smooth,
            &pos,
            None,
            None,
            Some(&nrm),
            &idx,
            model,
            mvp,
            1.0,
            1.0,
            1.0,
            0,
            Prim::Tri,
        )
        .unwrap();
        let left_smooth = пиксель(&hal_smooth, 0, 4);
        let right_smooth = пиксель(&hal_smooth, 7, 4);
        assert!(
            left_smooth[0] < right_smooth[0],
            "левый край ({}) должен быть темнее правого ({})",
            left_smooth[0],
            right_smooth[0]
        );
        assert!(
            left_smooth[0] < 120,
            "левый край должен иметь низкую освещенность: {}",
            left_smooth[0]
        );
        assert_eq!(
            right_smooth[0], 255,
            "правый край освещен полностью: {}",
            right_smooth[0]
        );
    }

    #[test]
    fn bilinear_and_mips() {
        // 1. Генерация пирамиды мипмапов (4x4 -> 2x2 -> 1x1)
        // Квадранты 2x2: Red, Green, Blue, White
        let mut px4x4 = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let idx = (y * 4 + x) * 4;
                let color = match (x < 2, y < 2) {
                    (true, true) => [255, 0, 0, 255],       // Верх-лево: Red
                    (false, true) => [0, 255, 0, 255],      // Верх-право: Green
                    (true, false) => [0, 0, 255, 255],      // Низ-лево: Blue
                    (false, false) => [255, 255, 255, 255], // Низ-право: White
                };
                px4x4[idx..idx + 4].copy_from_slice(&color);
            }
        }
        let tex = Tex::new(4, 4, px4x4);
        assert_eq!(
            tex.mips.len(),
            3,
            "4x4 должна дать 3 уровня LOD (0: 4x4, 1: 2x2, 2: 1x1)"
        );
        assert_eq!((tex.mips[0].w, tex.mips[0].h), (4, 4));
        assert_eq!((tex.mips[1].w, tex.mips[1].h), (2, 2));
        assert_eq!((tex.mips[2].w, tex.mips[2].h), (1, 1));

        // LOD 1: усреднение 2x2 блоков сохраняет чистые цвета квадрантов
        let lod1_tl = &tex.mips[1].px[0..4];
        let lod1_tr = &tex.mips[1].px[4..8];
        let lod1_bl = &tex.mips[1].px[8..12];
        let lod1_br = &tex.mips[1].px[12..16];
        assert_eq!(lod1_tl, [255, 0, 0, 255], "LOD 1 верх-лево: Red");
        assert_eq!(lod1_tr, [0, 255, 0, 255], "LOD 1 верх-право: Green");
        assert_eq!(lod1_bl, [0, 0, 255, 255], "LOD 1 низ-лево: Blue");
        assert_eq!(lod1_br, [255, 255, 255, 255], "LOD 1 низ-право: White");

        // LOD 2: усреднение всех 4 цветов: (255+0+0+255)/4 = 128
        let lod2 = &tex.mips[2].px[0..4];
        assert_eq!(
            lod2,
            [128, 128, 128, 255],
            "LOD 2 1x1: среднее всех 4 квадрантов"
        );

        // Непрямоугольная текстура 8x2: 8x2 -> 4x1 -> 2x1 -> 1x1
        let tex_rec = Tex::new(8, 2, vec![100u8; 8 * 2 * 4]);
        assert_eq!(tex_rec.mips.len(), 4);
        assert_eq!((tex_rec.mips[1].w, tex_rec.mips[1].h), (4, 1));
        assert_eq!((tex_rec.mips[2].w, tex_rec.mips[2].h), (2, 1));
        assert_eq!((tex_rec.mips[3].w, tex_rec.mips[3].h), (1, 1));

        // 2. Билинейная интерполяция
        // 2x2 текстура: левый столбец Black, правый столбец White
        let bw_px = vec![
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let bw_mip = crate::hal::MipLevel {
            w: 2,
            h: 2,
            px: bw_px,
        };

        // Крайние точки (u=0 и u=1)
        assert_eq!(sample_bilinear(&bw_mip, 0.0, 0.5), [0, 0, 0, 255]);
        assert_eq!(sample_bilinear(&bw_mip, 1.0, 0.5), [255, 255, 255, 255]);

        // Середина u=0.5 -> ровно 128 (взвешенное 50/50)
        let mid = sample_bilinear(&bw_mip, 0.5, 0.5);
        assert_eq!(mid[0], 128, "середина lerp: {}", mid[0]);

        // Четверти: u=0.25 -> ~64, u=0.75 -> ~191
        let q1 = sample_bilinear(&bw_mip, 0.25, 0.5);
        let q3 = sample_bilinear(&bw_mip, 0.75, 0.5);
        assert_eq!(q1[0], 64, "четверть 0.25: {}", q1[0]);
        assert_eq!(q3[0], 191, "четверть 0.75: {}", q3[0]);

        // Сравнение с Nearest: на 0.25 Nearest даёт ровно 0, на 0.75 ровно 255
        assert_eq!(sample_nearest(&bw_mip, 0.25, 0.5)[0], 0);
        assert_eq!(sample_nearest(&bw_mip, 0.75, 0.5)[0], 255);

        // 3. Интеграционный тест растра: ступенчатый Nearest vs гладкий Bilinear
        let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let mut hal_near = Hal::new(OutputMode::Silent, 8, 8);
        hal_near.tex.push(Tex::new(
            2,
            2,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
        ));
        hal_near.fill_all(0, 0, 0);
        свет_плоский(&mut hal_near);
        hal_near.r3.tex_filter = TexFilter::Nearest;
        рисуем_квадрат(&mut hal_near, 1, 0.5, &uv);

        let mut hal_bili = Hal::new(OutputMode::Silent, 8, 8);
        hal_bili.tex.push(Tex::new(
            2,
            2,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
        ));
        hal_bili.fill_all(0, 0, 0);
        свет_плоский(&mut hal_bili);
        hal_bili.r3.tex_filter = TexFilter::Bilinear;
        рисуем_квадрат(&mut hal_bili, 1, 0.5, &uv);

        // В Nearest: пиксель 3 черный (0), пиксель 4 белый (255) — резкая ступенька
        assert_eq!(пиксель(&hal_near, 0, 4)[0], 0);
        assert_eq!(пиксель(&hal_near, 3, 4)[0], 0);
        assert_eq!(пиксель(&hal_near, 4, 4)[0], 255);
        assert_eq!(пиксель(&hal_near, 7, 4)[0], 255);

        // В Bilinear: плавный градиент с переходом через середину
        let b0 = пиксель(&hal_bili, 0, 4)[0];
        let b3 = пиксель(&hal_bili, 3, 4)[0];
        let b4 = пиксель(&hal_bili, 4, 4)[0];
        let b7 = пиксель(&hal_bili, 7, 4)[0];
        assert!(b0 <= 20, "левый край bilinear: {}", b0);
        assert!(
            b3 > 50 && b3 < 150,
            "пиксель 3 в bilinear имеет промежуточный цвет: {}",
            b3
        );
        assert!(
            b4 > 120 && b4 < 200,
            "пиксель 4 в bilinear имеет промежуточный цвет: {}",
            b4
        );
        assert!(
            b0 < b3 && b3 < b4 && b4 < b7,
            "монотонный рост градиента: {} < {} < {} < {}",
            b0,
            b3,
            b4,
            b7
        );
        assert!(b7 >= 235, "правый край bilinear: {}", b7);
    }

    #[test]
    fn shadow_and_point_lights() {
        // 1. Тест точечного источника света и затухания
        let pl = PointLight {
            pos: [0.0, 1.0, 0.0],
            color: [1.0, 0.5, 0.25],
            radius: 5.0,
            attenuation: 1.0,
        };
        let view = [0.0, 5.0, 5.0];
        let nrm = [0.0, 1.0, 0.0];

        // Точка прямо под источником: (0, 0, 0)
        let (diff0, _) = compute_point_lights(
            std::slice::from_ref(&pl),
            &view,
            0.0,
            0.0,
            0.0,
            &nrm,
            false,
            false,
            32.0,
            0.0,
        );
        assert!(
            diff0[0] > 0.5,
            "свет прямо под источником ярок: {}",
            diff0[0]
        );

        // Точка на расстоянии 3: свет заметно затухает
        let (diff3, _) = compute_point_lights(
            std::slice::from_ref(&pl),
            &view,
            3.0,
            0.0,
            0.0,
            &nrm,
            false,
            false,
            32.0,
            0.0,
        );
        assert!(
            diff3[0] > 0.0 && diff3[0] < diff0[0],
            "затухание с расстоянием: {} < {}",
            diff3[0],
            diff0[0]
        );

        // Точка на расстоянии 6 (> radius 5.0): свет строго равен 0
        let (diff6, _) = compute_point_lights(
            std::slice::from_ref(&pl),
            &view,
            6.0,
            0.0,
            0.0,
            &nrm,
            false,
            false,
            32.0,
            0.0,
        );
        assert_eq!(diff6, [0.0, 0.0, 0.0], "вне радиуса свет нулевой");

        // 2. Тест теневой карты (Shadow Mapping)
        let mut r3 = R3::new();
        let light_dir = [0.0, 1.0, 0.0]; // Свет светит строго сверху вниз
        let light_mat = compute_light_matrix(light_dir, [0.0, 0.0, 0.0], 10.0, 1.0, 100.0);

        shadow_begin(&mut r3, light_mat, 64, 64, Some(0.005));
        assert!(r3.shadow_pass);
        assert!(r3.shadow_enabled);
        assert_eq!(r3.shadow_map.len(), 64 * 64);

        // Рендерим кастер тени: квадрат размером 2x2 на высоте y = 5.0 над началом координат
        let caster_pos = [
            -1.0f32, 5.0, -1.0, 1.0, 5.0, -1.0, 1.0, 5.0, 1.0, -1.0, 5.0, 1.0,
        ];
        let caster_idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
        let mut world = Vec::new();
        let mut world_nrm = Vec::new();
        let mut clip = Vec::new();
        let identity = [
            1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        transform_all(
            &caster_pos,
            None,
            &identity,
            &light_mat,
            &mut world,
            &mut world_nrm,
            &mut clip,
        );
        let drawn = raster_shadow_tris(&mut r3, &clip, &caster_idx).unwrap();
        assert!(drawn > 0, "треугольники кастера записаны в теневую карту");

        shadow_end(&mut r3);
        assert!(!r3.shadow_pass);
        assert!(r3.shadow_enabled);

        // Проверяем семплирование:
        // Точка (0, 0, 0) на земле находится строго под кастером -> коэффициент тени 0.0
        let s_shadow = shadow_sample(&r3, 0.0, 0.0, 0.0, 0.005);
        assert_eq!(s_shadow, 0.0, "точка под кастером полностью в тени");

        // Точка (8, 0, 8) далеко от кастера -> на прямом свету (1.0)
        let s_lit = shadow_sample(&r3, 8.0, 0.0, 8.0, 0.005);
        assert_eq!(s_lit, 1.0, "точка вдали от кастера полностью освещена");

        // Точка на границе кастера (~1.0, 0, 0) с PCF 2x2 даёт мягкий переход
        let s_edge = shadow_sample(&r3, 1.0, 0.0, 0.0, 0.005);
        assert!(
            (0.0..=1.0).contains(&s_edge),
            "мягкая граница тени PCF: {}",
            s_edge
        );
    }

    // ——— этап 9.2: инкрементальный сканлайн + LUT тумана ———

    fn тест_вершина(sx: f32, sy: f32, iw: f32) -> Vtx {
        Vtx {
            sx,
            sy,
            sz: sx * 0.01,
            uq: sx * iw,
            vq: sy * iw,
            iw,
            wxq: sx * iw,
            wyq: 0.0,
            wzq: sy * iw,
            aq: iw,
            rq: sx * iw,
            gq: 0.5 * iw,
            bq: 0.0,
            nxq: 0.0,
            nyq: 0.0,
            nzq: iw,
        }
    }

    #[test]
    fn сканлайн_спан_совпадает_с_перебором_bbox() {
        // прямоугольный, тупой, вертикальное ребро, «игла»; набор пикселей
        // row_span обязан совпадать с перебором edge-функций на пиксель
        let заготовки: Vec<[Vtx; 3]> = vec![
            [
                тест_вершина(2.0, 1.0, 1.0),
                тест_вершина(9.0, 3.0, 1.0),
                тест_вершина(4.0, 7.0, 1.0),
            ],
            [
                тест_вершина(0.0, 0.0, 1.0),
                тест_вершина(7.0, 6.5, 1.0),
                тест_вершина(1.0, 7.0, 1.0),
            ],
            [
                тест_вершина(3.0, 0.0, 1.0),
                тест_вершина(3.0, 7.0, 1.0),
                тест_вершина(8.0, 4.0, 1.0),
            ],
            [
                тест_вершина(1.0, 3.0, 1.0),
                тест_вершина(9.0, 3.4, 1.0),
                тест_вершина(5.0, 3.6, 1.0),
            ],
        ];
        for з in &заготовки {
            let mut tri = *з;
            // лицевая ориентация (area < 0) — конвенция opaque-прохода
            let area = edge(
                tri[0].sx, tri[0].sy, tri[1].sx, tri[1].sy, tri[2].sx, tri[2].sy,
            );
            if area > 0.0 {
                tri.swap(1, 2);
            }
            let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
            let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
            let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
            let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(7);
            let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
            let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(7);
            for py in miny..=maxy {
                let fy = py as f32 + 0.5;
                let fx0 = minx as f32 + 0.5;
                let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
                let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
                let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
                let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
                for px in minx..=maxx {
                    let fx = px as f32 + 0.5;
                    let brute = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy) <= 0.0
                        && edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy) <= 0.0
                        && edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy) <= 0.0;
                    assert_eq!(
                        from <= px && px <= to,
                        brute,
                        "строка {} пиксель {}: span ({},{})",
                        py,
                        px,
                        from,
                        to
                    );
                }
            }
        }
    }

    #[test]
    fn сумма_edge_функций_инвариантна_и_равна_площади() {
        // знаменатель барицентрика — константа треугольника: это даёт один
        // делитель на строку вместо одного на пиксель
        let tri = [
            тест_вершина(1.0, 2.0, 1.0),
            тест_вершина(8.0, 3.0, 1.0),
            тест_вершина(2.5, 7.0, 1.0),
        ];
        let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
        let area = edge(p0.sx, p0.sy, p1.sx, p1.sy, p2.sx, p2.sy);
        for (fx, fy) in [(3.5, 3.5), (0.5, 0.5), (9.5, 8.5), (5.2, 4.9)] {
            let s = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy)
                + edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy)
                + edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
            assert!(близко(s, area), "s={} area={}", s, area);
        }
    }

    #[test]
    fn сканлайн_атрибуты_совпадают_с_барицентрикой() {
        // пошаговые числители N_q·inv_d обязаны давать те же значения, что
        // прямой барицентрик в каждом пикселе
        let mut tri = [
            тест_вершина(1.3, 0.7, 0.9),
            тест_вершина(7.9, 2.2, 1.1),
            тест_вершина(3.4, 6.8, 0.8),
        ];
        let area = edge(
            tri[0].sx, tri[0].sy, tri[1].sx, tri[1].sy, tri[2].sx, tri[2].sy,
        );
        if area > 0.0 {
            tri.swap(1, 2);
        }
        let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
        let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
        let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
        let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(7);
        let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
        let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(7);
        const NA: usize = 11;
        let q: [[f32; 3]; NA] = [
            [p0.sz, p1.sz, p2.sz],
            [p0.iw, p1.iw, p2.iw],
            [p0.uq, p1.uq, p2.uq],
            [p0.vq, p1.vq, p2.vq],
            [p0.wxq, p1.wxq, p2.wxq],
            [p0.wyq, p1.wyq, p2.wyq],
            [p0.wzq, p1.wzq, p2.wzq],
            [p0.rq, p1.rq, p2.rq],
            [p0.gq, p1.gq, p2.gq],
            [p0.bq, p1.bq, p2.bq],
            [p0.aq, p1.aq, p2.aq],
        ];
        let mut проверено = 0;
        for py in miny..=maxy {
            let fy = py as f32 + 0.5;
            let fx0 = minx as f32 + 0.5;
            let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
            let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
            let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
            let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
            if from > to {
                continue;
            }
            let inv_d = 1.0 / (w0r + w1r + w2r);
            let wr = [w0r, w1r, w2r];
            let mut n = [0.0f32; NA];
            let mut dn = [0.0f32; NA];
            for a in 0..NA {
                n[a] = wr[0] * q[a][0] + wr[1] * q[a][1] + wr[2] * q[a][2];
                dn[a] = ax[0] * q[a][0] + ax[1] * q[a][1] + ax[2] * q[a][2];
            }
            let skip = (from - minx) as f32;
            for a in 0..NA {
                n[a] += dn[a] * skip;
            }
            for px in from..=to {
                let fx = px as f32 + 0.5;
                let w0 = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy);
                let w1 = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy);
                let w2 = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
                let sum = w0 + w1 + w2;
                for a in 0..NA {
                    let прямой = (w0 * q[a][0] + w1 * q[a][1] + w2 * q[a][2]) / sum;
                    assert!(
                        близко(n[a] * inv_d, прямой),
                        "атрибут {} пиксель ({},{}): {} против {}",
                        a,
                        px,
                        py,
                        n[a] * inv_d,
                        прямой
                    );
                }
                проверено += 1;
                for a in 0..NA {
                    n[a] += dn[a];
                }
            }
        }
        assert!(проверено > 0, "спан пуст — тест ничего не проверил");
    }

    #[test]
    fn туман_lut_близок_к_точной_формуле() {
        let mut r3 = R3::new();
        r3.fog = [0.7, 0.8, 0.9, 0.05];
        fog_lut_sync(&mut r3);
        assert!(близко(r3.fog_lut[0], 0.0));
        // монотонность и насыщение к 1
        for i in 0..256 {
            assert!(r3.fog_lut[i + 1] >= r3.fog_lut[i]);
        }
        assert!(r3.fog_lut[256] > 0.99);
        // интерполяция между узлами неотличима от точной формулы (зона до
        // последнего узла; на финальном интервале LUT дотягивается до 1.0,
        // тогда как точная формула ползёт к 0.9975 — расхождение ≤ 0.25%
        // блендинга, невизуально)
        let mut max_err = 0.0f32;
        for k in 0..=238 {
            let depth = k as f32 * 0.5;
            let точный = fog_factor(0.05, depth);
            let через_lut = fog_lut_lookup(&r3.fog_lut, 0.05, depth);
            max_err = max_err.max((точный - через_lut).abs());
        }
        assert!(max_err < 1e-3, "max_err = {}", max_err);
        // хвост за span: LUT накрывает точную кривую и не выше 1
        for k in 239..=400 {
            let depth = k as f32 * 0.5;
            let точный = fog_factor(0.05, depth);
            let через_lut = fog_lut_lookup(&r3.fog_lut, 0.05, depth);
            assert!(
                через_lut >= точный - 1e-3 && через_lut <= 1.0,
                "depth {}: {} против {}",
                depth,
                через_lut,
                точный
            );
        }
        // density 0 — тумана нет
        assert!(близко(fog_lut_lookup(&r3.fog_lut, 0.0, 50.0), 0.0));
        // смена fog лениво перестраивает LUT (снимок-ключ)
        r3.fog = [0.1, 0.1, 0.1, 0.2];
        fog_lut_sync(&mut r3);
        // depth 25 — внутри span (depth 30 = ровно span: там LUT = 1.0,
        // точная формула 0.9975 — семантика насыщения, см. хвост выше)
        assert!(близко(
            fog_lut_lookup(&r3.fog_lut, 0.2, 25.0),
            fog_factor(0.2, 25.0)
        ));
    }
}
