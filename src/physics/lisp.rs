//! src/physics/lisp.rs — Биндинги физического движка в Lisp (нативы phys-*).
//! Управление миром, добавление сфер, шаги симуляции, чтение контактов и пакетная выгрузка данных.

use super::solver::World;
use super::types::{HeightField, Sphere};

/// нативы Lisp: (phys-*) — мир в Vec<World>, id — индекс
pub fn register(env: crate::interp::Gc, worlds: std::rc::Rc<std::cell::RefCell<Vec<World>>>) {
    use crate::interp::{need_args as need, num_val, Gc, Value as V};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn def<F: Fn(&[V]) -> Result<V, String> + 'static>(env: Gc, name: &str, f: F) {
        crate::interp::env_define(env, name, V::Native(Rc::new(f), name.to_string()));
    }

    fn world_of<'a>(
        worlds: &'a Rc<RefCell<Vec<World>>>,
        v: &V,
    ) -> Result<std::cell::RefMut<'a, World>, String> {
        let id = num_val(v)? as usize;
        let ws = worlds.borrow_mut();
        if id >= ws.len() {
            return Err(format!("phys: нет мира {}", id));
        }
        // порядок borrow: worlds.borrow_mut() держим живым через RefMut
        Ok(std::cell::RefMut::map(ws, |ws| &mut ws[id]))
    }

    fn vec_of(v: &V) -> Result<Rc<RefCell<Vec<f32>>>, String> {
        match v {
            V::F32Vec(r) => Ok(r.clone()),
            _ => Err("phys: ожидался f32-вектор".into()),
        }
    }

    let wc = worlds.clone();
    def(env, "phys-new", move |args| {
        need(args, 0)?;
        let mut ws = wc.borrow_mut();
        ws.push(World::new());
        Ok(V::Int((ws.len() - 1) as i64))
    });

    let wc = worlds.clone();
    def(env, "phys-gravity!", move |args| {
        need(args, 2)?;
        let g = num_val(&args[1])? as f32;
        world_of(&wc, &args[0])?.gravity = g;
        Ok(V::Nil)
    });

    let wc = worlds.clone();
    def(env, "phys-water!", move |args| {
        need(args, 4)?;
        let mut w = world_of(&wc, &args[0])?;
        w.water = true;
        w.water_y = num_val(&args[1])? as f32;
        w.buoy = num_val(&args[2])? as f32;
        w.drag = num_val(&args[3])? as f32;
        Ok(V::Nil)
    });

    let wc = worlds.clone();
    def(env, "phys-heightfield!", move |args| {
        need(args, 8)?;
        let x0 = num_val(&args[1])? as f32;
        let z0 = num_val(&args[2])? as f32;
        let dx = num_val(&args[3])? as f32;
        let dz = num_val(&args[4])? as f32;
        let nx = num_val(&args[5])? as u32;
        let nz = num_val(&args[6])? as u32;
        let h = vec_of(&args[7])?;
        let hb = h.borrow();
        if hb.len() < (nx * nz) as usize {
            return Err(format!(
                "phys-heightfield: вектора {} < сетки {}×{}",
                hb.len(),
                nx,
                nz
            ));
        }
        let mut w = world_of(&wc, &args[0])?;
        w.hf = Some(HeightField {
            x0,
            z0,
            dx,
            dz,
            nx,
            nz,
            h: hb[..(nx * nz) as usize].to_vec(),
        });
        Ok(V::Nil)
    });

    let wc = worlds.clone();
    def(env, "phys-add!", move |args| {
        need(args, 7)?;
        let (x, y, z) = (
            num_val(&args[1])? as f32,
            num_val(&args[2])? as f32,
            num_val(&args[3])? as f32,
        );
        let (r, e, mu) = (
            num_val(&args[4])? as f32,
            num_val(&args[5])? as f32,
            num_val(&args[6])? as f32,
        );
        if r <= 0.0 {
            return Err("phys-add!: радиус > 0".into());
        }
        let id = world_of(&wc, &args[0])?.add(Sphere::new(x, y, z, r, e, mu));
        Ok(V::Int(id as i64))
    });

    let wc = worlds.clone();
    def(env, "phys-vel!", move |args| {
        need(args, 5)?;
        let id = num_val(&args[1])? as usize;
        let mut w = world_of(&wc, &args[0])?;
        if id >= w.spheres.len() {
            return Err(format!("phys-vel!: нет тела {}", id));
        }
        let s = &mut w.spheres[id];
        s.vx = num_val(&args[2])? as f32;
        s.vy = num_val(&args[3])? as f32;
        s.vz = num_val(&args[4])? as f32;
        Ok(V::Nil)
    });

    let wc = worlds.clone();
    def(env, "phys-n", move |args| {
        need(args, 1)?;
        Ok(V::Int(world_of(&wc, &args[0])?.spheres.len() as i64))
    });

    let wc = worlds.clone();
    def(env, "phys-clear!", move |args| {
        need(args, 1)?;
        world_of(&wc, &args[0])?.spheres.clear();
        Ok(V::Nil)
    });

    let wc = worlds.clone();
    def(env, "phys-step!", move |args| {
        need(args, 2)?;
        let dt = num_val(&args[1])? as f32;
        let n = world_of(&wc, &args[0])?.step(dt);
        Ok(V::Int(n as i64))
    });

    let wc = worlds.clone();
    def(env, "phys-events!", move |args| {
        need(args, 1)?;
        let mut w = world_of(&wc, &args[0])?;
        let evs: Vec<V> = w
            .events
            .drain(..)
            .map(|c| {
                V::List(vec![
                    V::Int(c.a as i64),
                    V::Int(c.b as i64),
                    V::Int(c.kind as i64),
                    V::Float(c.speed as f64),
                    V::Float(c.x as f64),
                    V::Float(c.y as f64),
                    V::Float(c.z as f64),
                ])
            })
            .collect();
        Ok(V::List(evs))
    });

    // пакетная выгрузка: out ← [x y z r]×n, возвращает n
    let wc = worlds.clone();
    def(env, "phys-positions!", move |args| {
        need(args, 2)?;
        let out = vec_of(&args[1])?;
        let w = world_of(&wc, &args[0])?;
        let n = w.spheres.len();
        {
            let mut ob = out.borrow_mut();
            ob.clear();
            ob.reserve(n * 4);
            for s in &w.spheres {
                ob.extend_from_slice(&[s.x, s.y, s.z, s.r]);
            }
        }
        Ok(V::Int(n as i64))
    });

    // пакетная выгрузка угловой скорости: out ← [wx wy wz]×n
    let wc = worlds.clone();
    def(env, "phys-angvels!", move |args| {
        need(args, 2)?;
        let out = vec_of(&args[1])?;
        let w = world_of(&wc, &args[0])?;
        let n = w.spheres.len();
        {
            let mut ob = out.borrow_mut();
            ob.clear();
            ob.reserve(n * 3);
            for s in &w.spheres {
                ob.extend_from_slice(&[s.wx, s.wy, s.wz]);
            }
        }
        Ok(V::Int(n as i64))
    });

    // ручная раскрутка: (phys-angvel! w id wx wy wz) — рад/с
    let wc = worlds.clone();
    def(env, "phys-angvel!", move |args| {
        need(args, 5)?;
        let mut w = world_of(&wc, &args[0])?;
        let id = num_val(&args[1])? as usize;
        let (wx, wy, wz) = (
            num_val(&args[2])? as f32,
            num_val(&args[3])? as f32,
            num_val(&args[4])? as f32,
        );
        let s = w
            .spheres
            .get_mut(id)
            .ok_or_else(|| format!("phys-angvel!: нет тела {}", id))?;
        s.wx = wx;
        s.wy = wy;
        s.wz = wz;
        Ok(V::Nil)
    });

    // пакетная выгрузка крена: out ← [ang ax ay az]×n (визуальное вращение)
    let wc = worlds.clone();
    def(env, "phys-spin!", move |args| {
        need(args, 2)?;
        let out = vec_of(&args[1])?;
        let w = world_of(&wc, &args[0])?;
        let n = w.spheres.len();
        {
            let mut ob = out.borrow_mut();
            ob.clear();
            ob.reserve(n * 4);
            for s in &w.spheres {
                ob.extend_from_slice(&[s.ang, s.ax, s.ay, s.az]);
            }
        }
        Ok(V::Int(n as i64))
    });
}
