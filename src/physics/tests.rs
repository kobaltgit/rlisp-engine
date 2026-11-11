//! src/physics/tests.rs — Модульные тесты физического движка.
//! Отскоки, сохранение энергии, неупругие и упругие удары, качение по склону,
//! плавучесть, стабильность стека, детерминизм и NGS-солвер.

use super::solver::World;
use super::types::{HeightField, Sphere, ANGV_MAX};

fn flat_hf() -> HeightField {
    HeightField {
        x0: -10.0,
        z0: -10.0,
        dx: 1.0,
        dz: 1.0,
        nx: 21,
        nz: 21,
        h: vec![0.0; 21 * 21],
    }
}

fn sim(w: &mut World, seconds: f32, dt: f32) {
    let n = (seconds / dt).round() as u32;
    for _ in 0..n {
        w.step(dt);
    }
}

#[test]
fn drop_bounce_restitution() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.add(Sphere::new(0.0, 5.0, 0.0, 0.5, 0.75, 0.4));
    // апекс первого отскока: 0.5 + 0.75²·4.5 ≈ 3.03 (меряем после 1-го удара)
    let mut bounced = false;
    let mut apex = 0.0f32;
    for _ in 0..120 {
        w.step(1.0 / 60.0);
        let s = &w.spheres[0];
        if !bounced {
            if s.vy > 2.0 {
                bounced = true;
            }
        } else {
            apex = apex.max(s.y);
        }
    }
    assert!(bounced, "шар должен коснуться земли за 2 c");
    assert!((apex - 3.03).abs() < 0.35, "apex {}", apex);
    sim(&mut w, 6.0, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert!((s.y - 0.5).abs() < 0.04, "rest y {}", s.y);
    assert!(s.vy.abs() < 0.35, "rest vy {}", s.vy); // джиттер покоя ~ g·h
}

#[test]
fn energy_nonincreasing() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.add(Sphere::new(0.3, 5.0, 0.0, 0.5, 0.6, 0.35));
    let e0 = 19.6 * 5.0;
    for _ in 0..240 {
        w.step(1.0 / 60.0);
        let s = &w.spheres[0];
        let e = 0.5 * (s.vx * s.vx + s.vy * s.vy + s.vz * s.vz) + 19.6 * s.y;
        assert!(e <= e0 + 0.6, "энергия выросла: {} > {}", e, e0);
    }
}

#[test]
fn exchange_elastic_headon() {
    let mut w = World::new();
    w.gravity = 0.0;
    let mut a = Sphere::new(0.0, 0.0, 0.0, 0.5, 1.0, 0.0);
    a.vx = 2.0;
    let mut b = Sphere::new(3.0, 0.0, 0.0, 0.5, 1.0, 0.0);
    b.vx = -2.0;
    w.add(a);
    w.add(b);
    sim(&mut w, 1.0, 1.0 / 60.0);
    assert!(
        (w.spheres[0].vx - (-2.0)).abs() < 1e-3,
        "va {}",
        w.spheres[0].vx
    );
    assert!(
        (w.spheres[1].vx - 2.0).abs() < 1e-3,
        "vb {}",
        w.spheres[1].vx
    );
}

#[test]
fn stick_inelastic_headon() {
    let mut w = World::new();
    w.gravity = 0.0;
    let mut a = Sphere::new(0.0, 0.0, 0.0, 0.5, 0.0, 0.0);
    a.vx = 2.0;
    let mut b = Sphere::new(3.0, 0.0, 0.0, 0.5, 0.0, 0.0);
    b.vx = -2.0;
    w.add(a);
    w.add(b);
    sim(&mut w, 1.0, 1.0 / 60.0);
    assert!(w.spheres[0].vx.abs() < 1e-3, "va {}", w.spheres[0].vx);
    assert!(w.spheres[1].vx.abs() < 1e-3, "vb {}", w.spheres[1].vx);
}

#[test]
fn slope_roll_downhill() {
    let mut w = World::new();
    let mut hf = HeightField {
        x0: -10.0,
        z0: -10.0,
        dx: 1.0,
        dz: 1.0,
        nx: 21,
        nz: 21,
        h: vec![0.0; 21 * 21],
    };
    // наклон 30°: высота убывает вдоль +x
    for r in 0..21 {
        for c in 0..21 {
            let x = hf.x0 + c as f32 * hf.dx;
            hf.h[r * 21 + c] = -0.5774 * x;
        }
    }
    w.hf = Some(hf);
    let mut s = Sphere::new(0.0, 0.0, 0.0, 0.4, 0.15, 0.12);
    s.y = 0.5774 * 0.0 + 1.0;
    w.add(s);
    sim(&mut w, 1.5, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert!(s.vx > 1.2, "разгон по склону vx {}", s.vx);
    let h = w.hf.as_ref().unwrap().height(s.x, s.z);
    assert!(
        s.y - h > 0.3 && s.y - h < 0.55,
        "на поверхности dy {}",
        s.y - h
    );
}

#[test]
fn buoyancy_floats() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.water = true;
    w.water_y = 0.0;
    w.buoy = 25.0;
    w.drag = 1.6;
    w.gravity = -19.6;
    w.add(Sphere::new(0.0, 3.0, 0.0, 0.5, 0.1, 0.0));
    sim(&mut w, 5.0, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert!(s.y.abs() < 0.6, "плавает y {}", s.y); // buoy>g → сидит высоко
    assert!(s.vy.abs() < 0.15, "скорость мала vy {}", s.vy);
}

#[test]
fn stack_stable_no_nan() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    let r = 0.5;
    let mut id = 0;
    for (row, n) in [(0usize, 3usize), (1, 2), (2, 1)] {
        for k in 0..n {
            let x = (k as f32 - (n as f32 - 1.0) / 2.0) * (2.0 * r + 0.02);
            let mut s = Sphere::new(
                x,
                1.5 + row as f32 * (2.0 * r * 0.87) + 0.05,
                0.0,
                r,
                0.2,
                0.5,
            );
            s.vx = if id % 2 == 0 { 0.05 } else { -0.05 };
            w.add(s);
            id += 1;
        }
    }
    sim(&mut w, 3.0, 1.0 / 60.0);
    for (i, s) in w.spheres.iter().enumerate() {
        assert!(s.x.is_finite() && s.y.is_finite(), "NaN у тела {}", i);
        assert!(s.y > 0.1, "тело {} под землёй y {}", i, s.y);
        assert!(s.vy.abs() < 1.0, "тело {} vy {}", i, s.vy);
    }
}

#[test]
fn determinism_bitwise() {
    let mk = || {
        let mut w = World::new();
        let mut hf = flat_hf();
        for i in 0..hf.h.len() {
            let x = hf.x0 + (i % 21) as f32;
            let z = hf.z0 + (i / 21) as f32;
            hf.h[i] = 0.4 * (x * 0.3).sin() + 0.3 * (z * 0.5).cos();
        }
        w.hf = Some(hf);
        for k in 0..8u32 {
            let mut s = Sphere::new(
                (k as f32) * 0.7 - 2.5,
                2.0 + k as f32 * 0.5,
                (k as f32) * 0.9 - 3.0,
                0.35 + 0.05 * k as f32,
                0.45,
                0.3,
            );
            s.vx = 0.7 - 0.1 * k as f32;
            s.vz = 0.3 + 0.05 * k as f32;
            w.add(s);
        }
        w
    };
    let (mut wa, mut wb) = (mk(), mk());
    for _ in 0..240 {
        wa.step(1.0 / 60.0);
        wb.step(1.0 / 60.0);
    }
    for (a, b) in wa.spheres.iter().zip(wb.spheres.iter()) {
        assert_eq!(a.x.to_bits(), b.x.to_bits());
        assert_eq!(a.y.to_bits(), b.y.to_bits());
        assert_eq!(a.z.to_bits(), b.z.to_bits());
    }
}

#[test]
fn impact_events_recorded() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.add(Sphere::new(0.0, 3.0, 0.0, 0.5, 0.4, 0.4));
    let mut hits = 0;
    for _ in 0..60 {
        w.step(1.0 / 60.0);
        hits += w.events.len();
    }
    assert!(hits >= 1, "удар должен давать событие");
    assert!(w.events.iter().all(|e| e.kind == 0 && e.b == -1));
}

#[test]
fn spin_conserved_in_flight() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.add(Sphere::new(0.0, 20.0, 0.0, 0.5, 0.5, 0.3));
    w.spheres[0].wx = 10.0;
    w.spheres[0].wy = -7.0;
    w.spheres[0].wz = 3.0;
    // 0.5 c свободного падения с 20 м — контактов нет
    sim(&mut w, 0.5, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert_eq!(s.wx.to_bits(), 10.0f32.to_bits());
    assert_eq!(s.wy.to_bits(), (-7.0f32).to_bits());
    assert_eq!(s.wz.to_bits(), 3.0f32.to_bits());
    // поворот накопился: |ω|·t = sqrt(158)·0.5 ≈ 6.28 рад
    assert!(
        (s.ang - 158.0f32.sqrt() * 0.5).abs() < 1e-4,
        "ang {}",
        s.ang
    );
}

#[test]
fn spin_damped_in_water() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.water = true;
    w.water_y = 5.0;
    w.buoy = 30.0;
    w.drag = 1.5;
    w.add(Sphere::new(0.0, 2.0, 0.0, 0.5, 0.5, 0.3));
    w.spheres[0].wz = 20.0;
    sim(&mut w, 1.0, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert!(s.wz < 20.0 && s.wz.abs() > 0.5, "drag ω: {}", s.wz);
    assert!(s.y > w.water_y - s.r, "не всплыл: {}", s.y);
}

#[test]
fn bitwise_deterministic_with_spin() {
    let mk = || {
        let mut w = World::new();
        w.hf = Some(flat_hf());
        w.add(Sphere::new(0.0, 4.0, 0.0, 0.45, 0.65, 0.45));
        w.add(Sphere::new(0.6, 5.5, 0.2, 0.45, 0.65, 0.45));
        w.spheres[0].wz = 12.0;
        w.spheres[1].wx = -9.0;
        w
    };
    let (mut a, mut b) = (mk(), mk());
    for _ in 0..240 {
        a.step(1.0 / 60.0);
        b.step(1.0 / 60.0);
    }
    for (pa, pb) in a.spheres.iter().zip(b.spheres.iter()) {
        assert_eq!(pa.x.to_bits(), pb.x.to_bits());
        assert_eq!(pa.y.to_bits(), pb.y.to_bits());
        assert_eq!(pa.wx.to_bits(), pb.wx.to_bits());
        assert_eq!(pa.wz.to_bits(), pb.wz.to_bits());
    }
}

#[test]
fn angvel_clamped() {
    let mut w = World::new();
    w.hf = Some(flat_hf());
    w.add(Sphere::new(0.0, 0.5, 0.0, 0.5, 0.0, 0.5));
    w.spheres[0].wx = 1000.0;
    w.step(1.0 / 60.0);
    let s = &w.spheres[0];
    assert!(s.wx.abs() <= ANGV_MAX + 1.0, "ω не зажата: {}", s.wx);
}

// ——— этап 12: NGS-солвер, честное качение ———

fn slope_30() -> HeightField {
    let mut hf = HeightField {
        x0: -10.0,
        z0: -10.0,
        dx: 1.0,
        dz: 1.0,
        nx: 21,
        nz: 21,
        h: vec![0.0; 21 * 21],
    };
    // наклон 30°: высота убывает вдоль +x (tan 0.5774)
    for r in 0..21 {
        for c in 0..21 {
            let x = hf.x0 + c as f32 * hf.dx;
            hf.h[r * 21 + c] = -0.5774 * x;
        }
    }
    hf
}

#[test]
fn spin_converts_to_rolling() {
    // шар крутится (wz=−20), стоит на земле: трение в точке контакта
    // переводит спин в качение без проскальзывания (vx = −wz·r)
    let mut w = World::new();
    w.hf = Some(flat_hf());
    let r = 0.5f32;
    w.add(Sphere::new(0.0, r, 0.0, r, 0.0, 0.5));
    w.spheres[0].wz = -20.0;
    let m = 4.1887902 * r * r * r;
    let e0 = 0.2 * m * r * r * 400.0; // ½·I·ω² = 0.2·m·r²·ω²
    sim(&mut w, 2.5, 1.0 / 60.0);
    let s = &w.spheres[0];
    let slip = s.vx + s.wz * s.r;
    assert!(slip.abs() < 0.1, "качение не установилось: slip {}", slip);
    assert!(
        (2.6..3.1).contains(&s.vx),
        "vx после перекачки: {} (теория ≈2.86)",
        s.vx
    );
    let ef = 0.5 * m * (s.vx * s.vx + s.vy * s.vy + s.vz * s.vz)
        + 0.2 * m * s.r * s.r * (s.wx * s.wx + s.wy * s.wy + s.wz * s.wz);
    assert!(ef <= e0 + 1e-3, "энергия выросла: {} > {}", ef, e0);
}

#[test]
fn slope_no_energy_pump() {
    // прошлый провал: на склоне Baumgarte-импульсы НАКАЧИВАЛИ энергию.
    // NGS: полная энергия (линейная + вращательная + потенциальная)
    // не превышает начальную — ни на одном кадре.
    let mut w = World::new();
    w.hf = Some(slope_30());
    let mut s = Sphere::new(0.0, 1.0, 0.0, 0.4, 0.05, 0.6);
    s.y = 0.5774 * 0.0 + 1.0;
    w.add(s);
    let m = 4.1887902 * 0.4 * 0.4 * 0.4;
    let e0 = 19.6 * m * 1.0;
    let mut rolled = false;
    for _ in 0..180 {
        w.step(1.0 / 60.0);
        let s = &w.spheres[0];
        let e = 0.5 * m * (s.vx * s.vx + s.vy * s.vy + s.vz * s.vz)
            + 0.2 * m * s.r * s.r * (s.wx * s.wx + s.wy * s.wy + s.wz * s.wz)
            + 19.6 * m * s.y;
        assert!(e <= e0 + 0.4, "накачка энергии: {} > {}", e, e0);
        if s.x > 1.5 {
            rolled = true;
        }
    }
    assert!(rolled, "шар должен скатиться по склону");
}

#[test]
fn rolling_not_faster_than_free_slide() {
    // катящийся шар не может разогнаться быстрее скользящего без трения:
    // vx_своб = g·sinθ·cosθ·t = 19.6·0.5·0.866·1.2 ≈ 10.2
    let mut w = World::new();
    w.hf = Some(slope_30());
    let mut s = Sphere::new(0.0, 1.0, 0.0, 0.4, 0.05, 0.6);
    s.y = 1.0;
    w.add(s);
    sim(&mut w, 1.2, 1.0 / 60.0);
    let s = &w.spheres[0];
    assert!(s.vx > 1.5, "скатывается медленно: {}", s.vx);
    assert!(s.vx < 10.0, "быстрее свободного скольжения: {}", s.vx);
}

#[test]
fn sliding_transitions_to_rolling() {
    // скользящий шар: кулоновское трение (μ·g ≈ 7.8 м/с²) гасит
    // проскальзывание и переводит шар на качение; дальше идеальная сфера
    // катится без сопротивления (rolling resistance не моделируется —
    // см. LIMITATIONS), поэтому vx НЕ ноль, а slip = 0
    let mut w = World::new();
    w.hf = Some(flat_hf());
    let mut s = Sphere::new(0.0, 0.5, 0.0, 0.5, 0.0, 0.4);
    s.vx = 6.0;
    w.add(s);
    sim(&mut w, 3.0, 1.0 / 60.0);
    let s = &w.spheres[0];
    let slip = s.vx + s.wz * s.r;
    assert!(slip.abs() < 0.1, "качение не установилось: slip {}", slip);
    // теория: перекачка спина съедает 1/1.4 скорости → vx ≈ 6/1.4 ≈ 4.3
    assert!(
        (3.9..4.7).contains(&s.vx),
        "vx после перекачки: {} (теория ≈4.3)",
        s.vx
    );
}

#[test]
fn bitwise_deterministic_rolling() {
    let mk = || {
        let mut w = World::new();
        let mut hf = slope_30();
        for i in 0..hf.h.len() {
            let z = hf.z0 + (i / 21) as f32;
            hf.h[i] += 0.15 * (z * 0.9).sin();
        }
        w.hf = Some(hf);
        for k in 0..6u32 {
            let mut s = Sphere::new(
                -1.0 + k as f32 * 0.9,
                1.0 + k as f32 * 0.6,
                (k as f32) * 0.5 - 1.2,
                0.3 + 0.04 * k as f32,
                0.1,
                0.55,
            );
            s.wz = 8.0 - 3.0 * k as f32;
            s.vx = 0.4;
            w.add(s);
        }
        w
    };
    let (mut a, mut b) = (mk(), mk());
    for _ in 0..240 {
        a.step(1.0 / 60.0);
        b.step(1.0 / 60.0);
    }
    for (pa, pb) in a.spheres.iter().zip(b.spheres.iter()) {
        assert_eq!(pa.x.to_bits(), pb.x.to_bits());
        assert_eq!(pa.y.to_bits(), pb.y.to_bits());
        assert_eq!(pa.z.to_bits(), pb.z.to_bits());
        assert_eq!(pa.wx.to_bits(), pb.wx.to_bits());
        assert_eq!(pa.wz.to_bits(), pb.wz.to_bits());
    }
}
