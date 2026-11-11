//! src/physics/solver.rs — Физический симулятор (NGS-солвер).
//! Интегрирование скоростей, коллизии сфер и ландшафта, последовательные импульсы
//! с накоплением (Gauss-Seidel), коррекция проникновений (split impulse).

use super::types::{
    point_vel, push_js_event, Contact, Contacts, HeightField, Sphere, ANGV_MAX, BOUNCE_THRESH,
    CORR, ITER, MAX_CORR, MAX_SUBSTEPS, MAX_SUB_DT, PAIR_COOLDOWN, SLOP, VEL_MAX,
};

/// контакт NGS-солвера: геометрия, зафиксированная до итераций, + накопленные
/// импульсы (зажим дельт идёт от накопленного — кулоновский лимит не нарушается)
struct NgC {
    i: u32,
    /// -1 — ландшафт
    j: i32,
    nx: f32,
    ny: f32,
    nz: f32,
    /// касательный базис (t1 — по до-солверному скольжению)
    t1x: f32,
    t1y: f32,
    t1z: f32,
    t2x: f32,
    t2y: f32,
    t2z: f32,
    /// плечи от центра до ТОЧКИ контакта на сфере: для пары ±n·r (контакт на
    /// линии центров); для heightfield −n·r (нормаль сферы проходит через
    /// центр — вертикальная точка (x,hy,z) годится только для меры
    /// проникновения, сила же в сфере всегда вдоль радиуса). У нормального
    /// импульса плечо ∥ n → момента нет; трение берёт ω×r в истинной точке.
    rax: f32,
    ray: f32,
    raz: f32,
    rbx: f32,
    rby: f32,
    rbz: f32,
    /// обратные эффективные массы: нормаль inv_mΣ (плечо ∥ n — углового
    /// члена нет); касательные inv_mΣ + inv_I·|r×t|² = 3.5·inv_mΣ для сферы
    k_n: f32,
    k_t1: f32,
    k_t2: f32,
    /// цель по нормальной скорости: −e_eff·vn0, где vn0 — сближение ЦЕНТРОВ
    /// (у пары совпадает со скоростью точки контакта: плечо ∥ n)
    vn_target: f32,
    mu: f32,
    jn_acc: f32,
    jt1_acc: f32,
    jt2_acc: f32,
}

#[derive(Clone)]
pub struct World {
    pub spheres: Vec<Sphere>,
    pub hf: Option<HeightField>,
    pub gravity: f32,
    /// вода: уровень, подъёмная сила (м/с²), линейный.drag (1/с)
    pub water: bool,
    pub water_y: f32,
    pub buoy: f32,
    pub drag: f32,
    pub time: f32,
    /// события, накопленные с последнего drain
    pub events: Vec<Contact>,
    pair_cool: Vec<(u64, f32)>,
}

impl World {
    pub fn new() -> Self {
        World {
            spheres: Vec::new(),
            hf: None,
            gravity: -19.6,
            water: false,
            water_y: 0.0,
            buoy: 0.0,
            drag: 0.0,
            time: 0.0,
            events: Vec::new(),
            pair_cool: Vec::new(),
        }
    }

    pub fn add(&mut self, s: Sphere) -> u32 {
        self.spheres.push(s);
        (self.spheres.len() - 1) as u32
    }

    /// шаг: подшаги по ≤1/90 c; возвращает число событий за шаг
    pub fn step(&mut self, dt: f32) -> usize {
        if dt <= 0.0 || dt > 0.25 {
            return 0;
        }
        let n = ((dt / MAX_SUB_DT).ceil() as u32).clamp(1, MAX_SUBSTEPS);
        let h = dt / n as f32;
        self.events.clear();
        for _ in 0..n {
            self.substep(h);
        }
        self.time += dt;
        self.events.len()
    }

    fn substep(&mut self, h: f32) {
        // 1) интегрирование скоростей: гравитация, плавучесть, drag
        for s in self.spheres.iter_mut() {
            s.vy += self.gravity * h;
            if self.water && s.y < self.water_y {
                s.vy += self.buoy * h;
                let f = (1.0 - self.drag * h).max(0.0);
                s.vx *= f;
                s.vy *= f;
                s.vz *= f;
                s.wx *= f;
                s.wy *= f;
                s.wz *= f;
            }
        }
        // 2) контакты (в порядке индексов — детерминизм)
        // (a, b, nx, ny, nz, pen, cx, cy, cz, vn0 — сближение до солвера)
        let mut cn: Contacts = Vec::new();
        if let Some(hf) = &self.hf {
            for (i, s) in self.spheres.iter().enumerate() {
                let hy = hf.height(s.x, s.z);
                let pen = s.r - (s.y - hy);
                if pen > 0.0 {
                    let (nx, ny, nz) = hf.normal(s.x, s.z);
                    let vn0 = s.vx * nx + s.vy * ny + s.vz * nz;
                    cn.push((i as u32, -1, nx, ny, nz, pen, s.x, hy, s.z, vn0));
                }
            }
        }
        for i in 0..self.spheres.len() {
            for j in (i + 1)..self.spheres.len() {
                let (a, b) = (&self.spheres[i], &self.spheres[j]);
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let dz = b.z - a.z;
                let d2 = dx * dx + dy * dy + dz * dz;
                let rs = a.r + b.r;
                if d2 < rs * rs && d2 > 1e-12 {
                    let d = d2.sqrt();
                    let (nx, ny, nz) = (dx / d, dy / d, dz / d);
                    let pen = rs - d;
                    let cx = a.x + nx * a.r;
                    let cy = a.y + ny * a.r;
                    let cz = a.z + nz * a.r;
                    let vn0 = (b.vx - a.vx) * nx + (b.vy - a.vy) * ny + (b.vz - a.vz) * nz;
                    cn.push((i as u32, j as i32, nx, ny, nz, pen, cx, cy, cz, vn0));
                }
            }
        }
        // 3) NGS-солвер (этап 12): последовательные импульсы с НАКОПЛЕНИЕМ.
        //    Нормаль — чистое ограничение скорости (цель vn = −e·vn0, где vn0 —
        //    до-солверная скорость): проникновение правит только позиционная
        //    фаза (шаг 5) — velocity-bias, накачивавший энергию, отсутствует
        //    (split impulse). Трение — в точке контакта (v + ω×r): дельта
        //    зажата кулоновским лимитом |jt| ≤ μ·jn_нак, поэтому трение строго
        //    диссипативно; спин связан со скольжением — шар переходит на
        //    качение честно. Для сферы плечо r ∥ n: нормальный импульс спин
        //    не меняет; k_t = inv_mΣ + inv_I·|r×t|² по обе стороны контакта.
        let mut sol: Vec<NgC> = Vec::with_capacity(cn.len());
        for c in &cn {
            let (i, j, nx, ny, nz, .., vn0) = *c;
            let a = &self.spheres[i as usize];
            // плечи вдоль нормали (см. NgC): контакт сферы — всегда на линии
            // «центр − r·n»; вертикальная точка (cx,cy,cz) — только мера
            // проникновения и позиция события
            let sgn = if j < 0 { -1.0 } else { 1.0 };
            let (rax, ray, raz) = (sgn * nx * a.r, sgn * ny * a.r, sgn * nz * a.r);
            let (mut rbx, mut rby, mut rbz) = (0.0f32, 0.0f32, 0.0f32);
            let mut rel = point_vel(a, rax, ray, raz);
            let (inv_sum, inv_i_b) = if j >= 0 {
                let b = &self.spheres[j as usize];
                rbx = -nx * b.r;
                rby = -ny * b.r;
                rbz = -nz * b.r;
                let pb = point_vel(b, rbx, rby, rbz);
                rel = (pb.0 - rel.0, pb.1 - rel.1, pb.2 - rel.2);
                (a.inv_m + b.inv_m, b.inv_i)
            } else {
                (a.inv_m, 0.0)
            };
            // t1 — по до-солверному скольжению, иначе стабильный fallback ⊥ n
            let vnn = rel.0 * nx + rel.1 * ny + rel.2 * nz;
            let (tx, ty, tz) = (rel.0 - nx * vnn, rel.1 - ny * vnn, rel.2 - nz * vnn);
            let tl = (tx * tx + ty * ty + tz * tz).sqrt();
            let (t1x, t1y, t1z) = if tl > 1e-6 {
                (tx / tl, ty / tl, tz / tl)
            } else if nx.abs() < 0.7 {
                let inv = (ny * ny + nz * nz).sqrt();
                (0.0, -nz / inv, ny / inv)
            } else {
                let inv = (nz * nz + nx * nx).sqrt();
                (-nz / inv, 0.0, nx / inv)
            };
            let (t2x, t2y, t2z) = (
                ny * t1z - nz * t1y,
                nz * t1x - nx * t1z,
                nx * t1y - ny * t1x,
            );
            let sq = |ax: f32, ay: f32, az: f32| ax * ax + ay * ay + az * az;
            let k_n = inv_sum;
            let k_t1 = inv_sum
                + a.inv_i
                    * sq(
                        ray * t1z - raz * t1y,
                        raz * t1x - rax * t1z,
                        rax * t1y - ray * t1x,
                    )
                + inv_i_b
                    * sq(
                        rby * t1z - rbz * t1y,
                        rbz * t1x - rbx * t1z,
                        rbx * t1y - rby * t1x,
                    );
            let k_t2 = inv_sum
                + a.inv_i
                    * sq(
                        ray * t2z - raz * t2y,
                        raz * t2x - rax * t2z,
                        rax * t2y - ray * t2x,
                    )
                + inv_i_b
                    * sq(
                        rby * t2z - rbz * t2y,
                        rbz * t2x - rbx * t2z,
                        rbx * t2y - rby * t2x,
                    );
            let (e, mu) = if j < 0 {
                (a.e, a.mu)
            } else {
                let b = &self.spheres[j as usize];
                (a.e.max(b.e) * 0.5 + a.e.min(b.e) * 0.5, a.mu.max(b.mu))
            };
            let e_eff = if -vn0 < BOUNCE_THRESH { 0.0 } else { e };
            sol.push(NgC {
                i,
                j,
                nx,
                ny,
                nz,
                t1x,
                t1y,
                t1z,
                t2x,
                t2y,
                t2z,
                rax,
                ray,
                raz,
                rbx,
                rby,
                rbz,
                k_n,
                k_t1,
                k_t2,
                vn_target: -e_eff * vn0,
                mu,
                jn_acc: 0.0,
                jt1_acc: 0.0,
                jt2_acc: 0.0,
            });
        }
        for _ in 0..ITER {
            for s in sol.iter_mut() {
                // скорость в точке контакта на текущей итерации
                let a = &self.spheres[s.i as usize];
                let pv = point_vel(a, s.rax, s.ray, s.raz);
                let rel = if s.j < 0 {
                    pv
                } else {
                    let b = &self.spheres[s.j as usize];
                    let pb = point_vel(b, s.rbx, s.rby, s.rbz);
                    (pb.0 - pv.0, pb.1 - pv.1, pb.2 - pv.2)
                };
                // — нормаль: накопительный импульс, зажим ≥ 0 (без липкости)
                let vn = rel.0 * s.nx + rel.1 * s.ny + rel.2 * s.nz;
                let jn_new = (s.jn_acc + (s.vn_target - vn) / s.k_n).max(0.0);
                let djn = jn_new - s.jn_acc;
                if djn != 0.0 {
                    s.jn_acc = jn_new;
                    let (px, py, pz) = (s.nx * djn, s.ny * djn, s.nz * djn);
                    match s.j {
                        -1 => {
                            let a = &mut self.spheres[s.i as usize];
                            a.vx += px * a.inv_m;
                            a.vy += py * a.inv_m;
                            a.vz += pz * a.inv_m;
                        }
                        jb => {
                            let (l, r) = self.spheres.split_at_mut(jb as usize);
                            let (a, b) = (&mut l[s.i as usize], &mut r[0]);
                            b.vx += px * b.inv_m;
                            b.vy += py * b.inv_m;
                            b.vz += pz * b.inv_m;
                            a.vx -= px * a.inv_m;
                            a.vy -= py * a.inv_m;
                            a.vz -= pz * a.inv_m;
                        }
                    }
                }
                // нормальный импульс ⊥ касательным и без крутящего момента —
                // vt1/vt2 не меняет: трение можно решать по тому же rel
                let lim = s.mu * s.jn_acc;
                if lim > 0.0 {
                    let vt1 = rel.0 * s.t1x + rel.1 * s.t1y + rel.2 * s.t1z;
                    let jt1_new = (s.jt1_acc - vt1 / s.k_t1).clamp(-lim, lim);
                    let djt = jt1_new - s.jt1_acc;
                    if djt != 0.0 {
                        s.jt1_acc = jt1_new;
                        let (px, py, pz) = (s.t1x * djt, s.t1y * djt, s.t1z * djt);
                        match s.j {
                            -1 => {
                                let a = &mut self.spheres[s.i as usize];
                                a.vx += px * a.inv_m;
                                a.vy += py * a.inv_m;
                                a.vz += pz * a.inv_m;
                                a.wx += a.inv_i * (s.ray * pz - s.raz * py);
                                a.wy += a.inv_i * (s.raz * px - s.rax * pz);
                                a.wz += a.inv_i * (s.rax * py - s.ray * px);
                            }
                            jb => {
                                let (l, r) = self.spheres.split_at_mut(jb as usize);
                                let (a, b) = (&mut l[s.i as usize], &mut r[0]);
                                b.vx += px * b.inv_m;
                                b.vy += py * b.inv_m;
                                b.vz += pz * b.inv_m;
                                b.wx += b.inv_i * (s.rby * pz - s.rbz * py);
                                b.wy += b.inv_i * (s.rbz * px - s.rbx * pz);
                                b.wz += b.inv_i * (s.rbx * py - s.rby * px);
                                a.vx -= px * a.inv_m;
                                a.vy -= py * a.inv_m;
                                a.vz -= pz * a.inv_m;
                                a.wx -= a.inv_i * (s.ray * pz - s.raz * py);
                                a.wy -= a.inv_i * (s.raz * px - s.rax * pz);
                                a.wz -= a.inv_i * (s.rax * py - s.ray * px);
                            }
                        }
                    }
                    let vt2 = rel.0 * s.t2x + rel.1 * s.t2y + rel.2 * s.t2z;
                    let jt2_new = (s.jt2_acc - vt2 / s.k_t2).clamp(-lim, lim);
                    let djt = jt2_new - s.jt2_acc;
                    if djt != 0.0 {
                        s.jt2_acc = jt2_new;
                        let (px, py, pz) = (s.t2x * djt, s.t2y * djt, s.t2z * djt);
                        match s.j {
                            -1 => {
                                let a = &mut self.spheres[s.i as usize];
                                a.vx += px * a.inv_m;
                                a.vy += py * a.inv_m;
                                a.vz += pz * a.inv_m;
                                a.wx += a.inv_i * (s.ray * pz - s.raz * py);
                                a.wy += a.inv_i * (s.raz * px - s.rax * pz);
                                a.wz += a.inv_i * (s.rax * py - s.ray * px);
                            }
                            jb => {
                                let (l, r) = self.spheres.split_at_mut(jb as usize);
                                let (a, b) = (&mut l[s.i as usize], &mut r[0]);
                                b.vx += px * b.inv_m;
                                b.vy += py * b.inv_m;
                                b.vz += pz * b.inv_m;
                                b.wx += b.inv_i * (s.rby * pz - s.rbz * py);
                                b.wy += b.inv_i * (s.rbz * px - s.rbx * pz);
                                b.wz += b.inv_i * (s.rbx * py - s.rby * px);
                                a.vx -= px * a.inv_m;
                                a.vy -= py * a.inv_m;
                                a.vz -= pz * a.inv_m;
                                a.wx -= a.inv_i * (s.ray * pz - s.raz * py);
                                a.wy -= a.inv_i * (s.raz * px - s.rax * pz);
                                a.wz -= a.inv_i * (s.rax * py - s.ray * px);
                            }
                        }
                    }
                }
            }
        }
        // 4) интегрирование позиций + крен качением
        for s in self.spheres.iter_mut() {
            s.x += s.vx * h;
            s.y += s.vy * h;
            s.z += s.vz * h;
            // угловая скорость: страховка + накопление поворота из ω; на земле
            // ω ведёт трение качения (NGS, шаг 3) — крен согласован со скольжением
            let w2 = s.wx * s.wx + s.wy * s.wy + s.wz * s.wz;
            if w2 > ANGV_MAX * ANGV_MAX {
                let k = ANGV_MAX / w2.sqrt();
                s.wx *= k;
                s.wy *= k;
                s.wz *= k;
            }
            let wh = w2.sqrt();
            if wh > 1e-4 {
                s.ang += wh * h;
                s.ax = s.wx / wh;
                s.ay = s.wy / wh;
                s.az = s.wz / wh;
            } else {
                let vh = (s.vx * s.vx + s.vz * s.vz).sqrt();
                if vh > 1e-4 {
                    s.ang += vh * h / s.r;
                    s.ax = -s.vz / vh;
                    s.az = s.vx / vh;
                    s.ay = 0.0;
                }
            }
            // страховка
            let v2 = s.vx * s.vx + s.vy * s.vy + s.vz * s.vz;
            if v2 > VEL_MAX * VEL_MAX {
                let k = VEL_MAX / v2.sqrt();
                s.vx *= k;
                s.vy *= k;
                s.vz *= k;
            }
            if !(s.x.is_finite() && s.y.is_finite() && s.z.is_finite()) {
                s.vx = 0.0;
                s.vy = 0.0;
                s.vz = 0.0;
            }
            if !(s.wx.is_finite() && s.wy.is_finite() && s.wz.is_finite()) {
                s.wx = 0.0;
                s.wy = 0.0;
                s.wz = 0.0;
            }
        }
        // 5) коррекция проникновений (позиционная, статика — вся доля шару)
        for c in &cn {
            let (_, j, nx, ny, nz, pen, _, _, _, _) = *c;
            if pen <= SLOP {
                continue;
            }
            let push = ((pen - SLOP) * CORR).min(MAX_CORR);
            match j {
                -1 => {
                    let s = &mut self.spheres[c.0 as usize];
                    s.x += nx * push;
                    s.y += ny * push;
                    s.z += nz * push;
                }
                _ => {
                    let (i, jb) = (c.0 as usize, j as usize);
                    let wa =
                        self.spheres[i].inv_m / (self.spheres[i].inv_m + self.spheres[jb].inv_m);
                    let a = &mut self.spheres[i];
                    a.x -= nx * push * wa;
                    a.y -= ny * push * wa;
                    a.z -= nz * push * wa;
                    let b = &mut self.spheres[jb];
                    b.x += nx * push * (1.0 - wa);
                    b.y += ny * push * (1.0 - wa);
                    b.z += nz * push * (1.0 - wa);
                }
            }
        }
        // 6) события ударов: порог по ДО-солверному vn0 (после импульсов vn≈0)
        for c in &cn {
            let (i, j, _, _, _, _, cx, cy, cz, vn0) = *c;
            let (kind, b) = if j < 0 { (0u8, -1i32) } else { (1u8, j) };
            if vn0 < -BOUNCE_THRESH {
                let key = ((i as u64) << 20) | ((b.max(0) as u64) & 0xFFFFF);
                let last = self
                    .pair_cool
                    .iter()
                    .find(|p| p.0 == key)
                    .map(|p| p.1)
                    .unwrap_or(-10.0);
                if self.time - last > PAIR_COOLDOWN {
                    if let Some(p) = self.pair_cool.iter_mut().find(|p| p.0 == key) {
                        p.1 = self.time;
                    } else {
                        self.pair_cool.push((key, self.time));
                    }
                    let ev = Contact {
                        a: i,
                        b,
                        kind,
                        speed: -vn0,
                        x: cx,
                        y: cy,
                        z: cz,
                    };
                    push_js_event(&ev);
                    self.events.push(ev);
                }
            }
        }
        if self.pair_cool.len() > 256 {
            self.pair_cool.drain(..128);
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
