// src/simd/lisp.rs — Lisp-нативы, конвертеры типов и функции бенчмарка

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::interp::{env_define, fmt_value, need_args, num_val, Gc, Value};

use super::kernels::{level_add, scalar, vadd, vdot, vmax, vmin, vmul, vsub};
use super::types::{active_level, detect, level_name};

// ——— бенчмарк ———

fn timed<F: FnMut()>(iters: usize, mut f: F) -> f64 {
    let t0 = Instant::now();
    for _ in 0..iters {
        f();
    }
    t0.elapsed().as_secs_f64() * 1000.0
}

pub fn bench_run() {
    println!(
        "обнаружено по флагам: {}; выбрано калибровкой: {}",
        level_name(detect()),
        level_name(active_level())
    );

    // Тест A: большой массив — упор в пропускную способность памяти
    // (честно показываем, где SIMD не помогает: риск из ТЗ §13).
    {
        let n = 1usize << 20;
        let a: Vec<f32> = (0..n).map(|i| (i as f64 * 0.001) as f32).collect();
        let b: Vec<f32> = (0..n).map(|i| (i as f64).sqrt() as f32).collect();
        let mut out = vec![0f32; n];
        let iters = 100usize;
        let t_sc = timed(iters, || scalar::add(&a, &b, &mut out));
        let lvl = active_level();
        let t_sim = timed(iters, || level_add(lvl, &a, &b, &mut out));
        println!(
            "[simd] тест A — vec-add 1M f32 × {} (вне кэша): скаляр {:.1} мс, {} {:.1} мс → {:.2}×  (упор в память)",
            iters,
            t_sc,
            level_name(lvl),
            t_sim,
            t_sc / t_sim.max(1e-9)
        );
    }

    // Тест B/C: данные в L1 — сценарий физики (критерий ТЗ §4.5: ≥2×).
    {
        let n = 2048usize;
        let a: Vec<f32> = (0..n).map(|i| (i % 97) as f32 * 0.5).collect();
        let b: Vec<f32> = (0..n).map(|i| (i % 89) as f32 * 0.25).collect();
        let mut out = vec![0f32; n];
        let iters = 20000usize;
        // прогрев + защита от выбросов
        level_add(active_level(), &a, &b, &mut out);
        let t_sc = timed(iters, || scalar::add(&a, &b, &mut out));
        let lvl = active_level();
        let t_sim = timed(iters, || level_add(lvl, &a, &b, &mut out));
        std::hint::black_box(out[0]);
        println!(
            "[simd] тест B — vec-add 2048 f32 (L1) × {}: скаляр {:.2} мс, {} {:.2} мс → {:.2}×",
            iters,
            t_sc,
            level_name(lvl),
            t_sim,
            t_sc / t_sim.max(1e-9)
        );

        let mut sink1 = 0f32;
        let t_sc = timed(iters, || {
            sink1 = scalar::dot(&a, &b);
        });
        let mut sink2 = 0f32;
        let t_sim = timed(iters, || {
            sink2 = vdot(&a, &b);
        });
        println!(
            "[simd] тест C — vec-dot 2048 f32 × {}: скаляр {:.2} мс, {} {:.2} мс → {:.2}×  (контроль сумм: {} / {})",
            iters,
            t_sc,
            level_name(lvl),
            t_sim,
            t_sc / t_sim.max(1e-9),
            sink1, sink2
        );
    }
}

// ——— Lisp-нативы ———

pub fn to_f32s(v: &Value) -> Result<Vec<f32>, String> {
    match v {
        Value::F32Vec(r) => Ok(r.borrow().clone()),
        Value::Vector(r) => {
            let b = r.borrow();
            let mut out = Vec::with_capacity(b.len());
            for x in b.iter() {
                out.push(num_val(x)? as f32);
            }
            Ok(out)
        }
        Value::List(l) => {
            let mut out = Vec::with_capacity(l.len());
            for x in l {
                out.push(num_val(x)? as f32);
            }
            Ok(out)
        }
        other => Err(format!("нужен вектор чисел: {}", fmt_value(other))),
    }
}

fn ret_f32(v: Vec<f32>) -> Value {
    Value::F32Vec(Rc::new(RefCell::new(v)))
}

fn binop(env: Gc, name: &str, f: for<'a> fn(&'a [f32], &'a [f32], &'a mut [f32])) {
    let name_s: String = name.to_string();
    env_define(
        env,
        name,
        Value::Native(
            Rc::new(move |args: &[Value]| {
                need_args(args, 2)?;
                let a = to_f32s(&args[0])?;
                let b = to_f32s(&args[1])?;
                if a.len() != b.len() {
                    return Err(format!(
                        "{}: длины не равны ({} и {})",
                        name_s,
                        a.len(),
                        b.len()
                    ));
                }
                let mut o = vec![0f32; a.len()];
                f(&a, &b, &mut o);
                Ok(ret_f32(o))
            }),
            name.to_string(),
        ),
    );
}

pub fn register_simd(env: Gc) {
    binop(env, "vec-add", vadd);
    binop(env, "vec-sub", vsub);
    binop(env, "vec-mul", vmul);
    binop(env, "vec-min", vmin);
    binop(env, "vec-max", vmax);

    fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: Gc, name: &str, f: F) {
        env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
    }
    use need_args as need;

    def(env, "vec-scale", |args| {
        need(args, 2)?;
        let a = to_f32s(&args[0])?;
        let s = num_val(&args[1])? as f32;
        let mut o = vec![0f32; a.len()];
        for (i, x) in a.iter().enumerate() {
            o[i] = x * s;
        }
        Ok(ret_f32(o))
    });
    def(env, "vec-dot", |args| {
        need(args, 2)?;
        let a = to_f32s(&args[0])?;
        let b = to_f32s(&args[1])?;
        if a.len() != b.len() {
            return Err("vec-dot: длины не равны".into());
        }
        Ok(Value::Float(vdot(&a, &b) as f64))
    });
    def(env, "vec-length", |args| {
        need(args, 1)?;
        let a = to_f32s(&args[0])?;
        Ok(Value::Float((vdot(&a, &a) as f64).sqrt()))
    });
    def(env, "vec-normalize", |args| {
        need(args, 1)?;
        let a = to_f32s(&args[0])?;
        let len = (vdot(&a, &a) as f64).sqrt() as f32;
        if len < 1e-12 {
            return Ok(ret_f32(vec![0f32; a.len()]));
        }
        let mut o = vec![0f32; a.len()];
        for (i, x) in a.iter().enumerate() {
            o[i] = x / len;
        }
        Ok(ret_f32(o))
    });
    def(env, "vec-clamp", |args| {
        need(args, 3)?;
        let a = to_f32s(&args[0])?;
        let lo = num_val(&args[1])? as f32;
        let hi = num_val(&args[2])? as f32;
        let mut o = vec![0f32; a.len()];
        for (i, x) in a.iter().enumerate() {
            o[i] = x.max(lo).min(hi);
        }
        Ok(ret_f32(o))
    });
    def(env, "mat4-mul", |args| {
        need(args, 2)?;
        let a = to_f32s(&args[0])?;
        let b = to_f32s(&args[1])?;
        if a.len() != 16 || b.len() != 16 {
            return Err("mat4-mul: нужны матрицы 4x4 (16 f32)".into());
        }
        let mut o = [0f32; 16];
        for i in 0..4 {
            for j in 0..4 {
                let mut s = 0f32;
                for k in 0..4 {
                    s += a[i * 4 + k] * b[k * 4 + j];
                }
                o[i * 4 + j] = s;
            }
        }
        Ok(ret_f32(o.to_vec()))
    });
    def(env, "mat4-mul-vec4", |args| {
        need(args, 2)?;
        let m = to_f32s(&args[0])?;
        let v = to_f32s(&args[1])?;
        if m.len() != 16 || v.len() != 4 {
            return Err("mat4-mul-vec4: нужна матрица 4x4 и vec4".into());
        }
        let mut o = [0f32; 4];
        // v0.2: согласовано с mat4-mul — теперь o = M·v (row-major, вектор-столбец).
        // Раньше считалось v·M (транспонированно), что ломало композицию матриц.
        for i in 0..4 {
            o[i] =
                m[i * 4] * v[0] + m[i * 4 + 1] * v[1] + m[i * 4 + 2] * v[2] + m[i * 4 + 3] * v[3];
        }
        Ok(ret_f32(o.to_vec()))
    });
}
