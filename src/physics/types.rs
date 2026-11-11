//! src/physics/types.rs — Базовые структуры данных физического движка.
//! Сферы, сетка высот (HeightField), события контакта и буфер событий для WASM.

/// подшаг не длиннее 1/90 c (анти-туннель и стабильность покоя)
pub const MAX_SUB_DT: f32 = 1.0 / 90.0;
pub const MAX_SUBSTEPS: u32 = 6;
pub const SLOP: f32 = 0.005; // допустимое проникновение (Baumgarte)
pub const CORR: f32 = 0.8; // доля коррекции проникновения за подшаг
pub const MAX_CORR: f32 = 0.05; // потолок выталкивания за подшаг (иначе «телепорт»
                                // после глубокого закапывания даёт гигантские прыжки)
pub const BOUNCE_THRESH: f32 = 0.4; // |vn| ниже — отскок гасится (покой)
pub const VEL_MAX: f32 = 60.0; // страховка от разгона
pub const ITER: usize = 8; // NGS-итерации (накопительные импульсы: больше — точнее сходимость)
pub const ANGV_MAX: f32 = 60.0; // страховка от раскрутки, рад/с
pub const PAIR_COOLDOWN: f32 = 0.09; // повторный стук пары — не чаще

/// контакт-кортеж солвера
pub type Contacts = Vec<(u32, i32, f32, f32, f32, f32, f32, f32, f32, f32)>;

/// событие контакта (для звука в песочнице и ряби в сценах)
#[derive(Clone, Copy)]
pub struct Contact {
    pub a: u32,
    /// -1 — ландшафт, иначе id второй сферы
    pub b: i32,
    /// 0 — ландшафт, 1 — сфера-сфера
    pub kind: u8,
    /// скорость удара (>0), м/с
    pub speed: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone)]
pub struct Sphere {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    pub r: f32,
    pub inv_m: f32,
    /// коэффициент восстановления
    pub e: f32,
    /// трение Кулона
    pub mu: f32,
    /// накопленный поворот (угол + ось) — интегрируется из угловой скорости
    pub ang: f32,
    pub ax: f32,
    pub ay: f32,
    pub az: f32,
    /// угловая скорость, рад/с (сфера изотропна — скалярная инерция на все оси);
    /// контактная динамика спина — NGS-солвер (этап 12): трение в точке
    /// контакта связывает скольжение и вращение (честное качение)
    pub wx: f32,
    pub wy: f32,
    pub wz: f32,
    /// обратная moments инерции I = 2/5·m·r² → inv_I = 5/(2·m·r²)
    pub inv_i: f32,
}

impl Sphere {
    pub fn new(x: f32, y: f32, z: f32, r: f32, e: f32, mu: f32) -> Self {
        // плотность 1: масса шара 4/3 π r³
        let m = 4.1887902 * r * r * r;
        Sphere {
            x,
            y,
            z,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            r,
            inv_m: 1.0 / m,
            e,
            mu,
            ang: 0.0,
            ax: 0.0,
            ay: 1.0,
            az: 0.0,
            wx: 0.0,
            wy: 0.0,
            wz: 0.0,
            inv_i: 2.5 * (1.0 / m) / (r * r),
        }
    }
}

/// сетка высот: h[row*nx+col], row → z, col → x; вне сетки — край
#[derive(Clone)]
pub struct HeightField {
    pub x0: f32,
    pub z0: f32,
    pub dx: f32,
    pub dz: f32,
    pub nx: u32,
    pub nz: u32,
    pub h: Vec<f32>,
}

impl HeightField {
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let fx = ((x - self.x0) / self.dx).clamp(0.0, (self.nx - 1) as f32);
        let fz = ((z - self.z0) / self.dz).clamp(0.0, (self.nz - 1) as f32);
        let c = fx.floor() as u32;
        let r = fz.floor() as u32;
        let c1 = (c + 1).min(self.nx - 1);
        let r1 = (r + 1).min(self.nz - 1);
        let tx = fx - c as f32;
        let tz = fz - r as f32;
        let h00 = self.h[(r * self.nx + c) as usize];
        let h10 = self.h[(r * self.nx + c1) as usize];
        let h01 = self.h[(r1 * self.nx + c) as usize];
        let h11 = self.h[(r1 * self.nx + c1) as usize];
        h00 * (1.0 - tx) * (1.0 - tz)
            + h10 * tx * (1.0 - tz)
            + h01 * (1.0 - tx) * tz
            + h11 * tx * tz
    }

    /// нормаль поверхности центральными разностями (шаг = полквадрата)
    pub fn normal(&self, x: f32, z: f32) -> (f32, f32, f32) {
        let e = self.dx * 0.5;
        let gx = (self.height(x + e, z) - self.height(x - e, z)) / (2.0 * e);
        let gz = (self.height(x, z + e) - self.height(x, z - e)) / (2.0 * e);
        let inv = 1.0 / (gx * gx + 1.0 + gz * gz).sqrt();
        (-gx * inv, inv, -gz * inv)
    }
}

/// скорость точки тела: v + ω×r (r — от центра до точки)
#[inline]
pub(crate) fn point_vel(s: &Sphere, rx: f32, ry: f32, rz: f32) -> (f32, f32, f32) {
    (
        s.vx + s.wy * rz - s.wz * ry,
        s.vy + s.wz * rx - s.wx * rz,
        s.vz + s.wx * ry - s.wy * rx,
    )
}

// буфер для JS-песочницы (звук): плоский формат
// [a, kind, speed, x, y, z] × n; чистится только wasm-фасадом
#[cfg(target_arch = "wasm32")]
use std::sync::Mutex;
#[cfg(target_arch = "wasm32")]
static JS_EVENTS: Mutex<Vec<f32>> = Mutex::new(Vec::new());

#[cfg(target_arch = "wasm32")]
pub(crate) fn push_js_event(c: &Contact) {
    if let Ok(mut q) = JS_EVENTS.lock() {
        if q.len() > 4096 {
            q.clear();
        }
        q.extend_from_slice(&[c.a as f32, c.kind as f32, c.speed, c.x, c.y, c.z]);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn push_js_event(_c: &Contact) {}

/// wasm-фасад: забрать накопленные события (a, kind, speed, x, y, z)×n
#[cfg(target_arch = "wasm32")]
pub fn drain_js_events(out: &mut Vec<f32>) {
    if let Ok(mut q) = JS_EVENTS.lock() {
        std::mem::swap(&mut *q, out);
        q.clear();
    }
}
