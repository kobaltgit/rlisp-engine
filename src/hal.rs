// src/hal.rs — Уровень 1: Rust HAL (Hardware Abstraction Layer).
// По ТЗ §3: HAL не знает игровых понятий. Никаких draw-sprite / physics-step —
// только сырые команды: окно, растр, ввод, память, файлы, время.
//
// Отличие от финального ТЗ (честно, см. README): вместо wgpu — программный
// растр (фреймбуфер RGBA) с тем же профилем API (gpu-clear / gpu-fill-rect /
// gpu-blit / gpu-present). wgpu-бэкенд подключается за этим же фасадом.

use crate::interp::{env_define, fmt_value, load_file, num_val, int_val, need_args, resolve_path, Env, Value};
use crate::png;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Текстура «GPU» (v0.3): RGBA8, загружается из PNG или генерируется на Lisp.
#[derive(Clone)]
pub struct Tex {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>, // rgba
}

#[derive(Clone)]
pub enum OutputMode {
    Silent,
    Ascii,
    Png { dir: PathBuf, every: usize },
}

pub struct Hal {
    pub w: usize,
    pub h: usize,
    pub fb: Vec<u8>,
    pub frame: u64,
    pub title: String,
    pub out: OutputMode,
    pub queue: Vec<Value>,
    pub start: std::time::Instant,
    pub mem: HashMap<u64, Rc<RefCell<Vec<u8>>>>,
    pub next_mem: u64,
    /// «GPU» фиксированного конвейера: z-буфер + свет + статистика (render3d.rs)
    pub r3: crate::render3d::R3,
    /// Реестр текстур (v0.3): id = индекс+1, 0 — «без текстуры»
    pub tex: Vec<Tex>,
}

impl Hal {
    pub fn new(out: OutputMode, w: usize, h: usize) -> Hal {
        let w = w.max(8);
        let h = h.max(8);
        let mut hal = Hal {
            w,
            h,
            fb: vec![0; w * h * 4],
            frame: 0,
            title: String::new(),
            out,
            queue: Vec::new(),
            start: std::time::Instant::now(),
            mem: HashMap::new(),
            next_mem: 1,
            r3: crate::render3d::R3::new(),
            tex: Vec::new(),
        };
        hal.fill_all(8, 8, 18);
        hal
    }

    pub fn fill_all(&mut self, r: u8, g: u8, b: u8) {
        let n = self.w * self.h;
        for i in 0..n {
            let p = i * 4;
            self.fb[p] = r;
            self.fb[p + 1] = g;
            self.fb[p + 2] = b;
            self.fb[p + 3] = 255;
        }
    }

    pub fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, r: u8, g: u8, b: u8) {
        let x0 = (x.max(0) as usize).min(self.w);
        let y0 = (y.max(0) as usize).min(self.h);
        let x1 = ((x + w).max(0) as usize).min(self.w);
        let y1 = ((y + h).max(0) as usize).min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for yy in y0..y1 {
            let row = yy * self.w;
            for xx in x0..x1 {
                let p = (row + xx) * 4;
                self.fb[p] = r;
                self.fb[p + 1] = g;
                self.fb[p + 2] = b;
                self.fb[p + 3] = 255;
            }
        }
    }

    pub fn blit(&mut self, x: i64, y: i64, w: usize, h: usize, bytes: &[u8]) {
        for row in 0..h {
            let py = y + row as i64;
            if py < 0 || py >= self.h as i64 {
                continue;
            }
            for col in 0..w {
                let px = x + col as i64;
                if px < 0 || px >= self.w as i64 {
                    continue;
                }
                let si = (row * w + col) * 4;
                if si + 3 >= bytes.len() {
                    continue;
                }
                if bytes[si + 3] == 0 {
                    continue; // прозрачный пиксель
                }
                let di = (py as usize * self.w + px as usize) * 4;
                self.fb[di] = bytes[si];
                self.fb[di + 1] = bytes[si + 1];
                self.fb[di + 2] = bytes[si + 2];
                self.fb[di + 3] = 255;
            }
        }
    }
}

fn f2b(x: f64) -> u8 {
    (x * 255.0).round().clamp(0.0, 255.0) as u8
}

fn str_val(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Sym(s) => s.clone(),
        other => fmt_value(other),
    }
}

pub fn ascii_frame(hal: &Hal) -> String {
    let cols = 100usize;
    let rows = (((hal.h as f64 / hal.w as f64) * cols as f64) * 0.5).round().max(8.0) as usize;
    let cw = (hal.w / cols).max(1);
    let ch = (hal.h / rows).max(1);
    let mut s = String::from("\x1b[H");
    for ry in 0..rows {
        for rx in 0..cols {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut n = 0u32;
            let y1 = ((ry + 1) * ch).min(hal.h);
            let x1 = ((rx + 1) * cw).min(hal.w);
            for yy in (ry * ch)..y1 {
                for xx in (rx * cw)..x1 {
                    let p = (yy * hal.w + xx) * 4;
                    r += hal.fb[p] as u32;
                    g += hal.fb[p + 1] as u32;
                    b += hal.fb[p + 2] as u32;
                    n += 1;
                }
            }
            if n == 0 {
                s.push(' ');
                continue;
            }
            let r = (r / n) as u8;
            let g = (g / n) as u8;
            let b = (b / n) as u8;
            let lum = r as u32 * 30 + g as u32 * 59 + b as u32 * 11;
            if lum < 40 {
                s.push(' ');
            } else {
                s.push_str(&format!("\x1b[38;2;{};{};{}m\u{2588}", r, g, b));
            }
        }
        s.push_str("\x1b[0m\r\n");
    }
    s.push_str(&format!(
        "\x1b[K  rlisp-engine | кадр {} | {}x{}\r\n",
        hal.frame, hal.w, hal.h
    ));
    s
}

fn to_rgb(v: &Value) -> Result<(f32, f32, f32), String> {
    match v {
        Value::List(l) if l.len() >= 3 => Ok((
            num_val(&l[0])? as f32,
            num_val(&l[1])? as f32,
            num_val(&l[2])? as f32,
        )),
        Value::Vector(r) if r.borrow().len() >= 3 => {
            let b = r.borrow();
            Ok((
                num_val(&b[0])? as f32,
                num_val(&b[1])? as f32,
                num_val(&b[2])? as f32,
            ))
        }
        _ => Err("ожидался цвет (r g b)".into()),
    }
}

fn mem_block(hal: &Hal, v: &Value) -> Result<Rc<RefCell<Vec<u8>>>, String> {
    match v {
        Value::Ptr(id) => hal
            .mem
            .get(id)
            .cloned()
            .ok_or_else(|| format!("mem: неизвестный указатель {}", id)),
        other => Err(format!("mem: нужен указатель из mem-alloc: {}", fmt_value(other))),
    }
}

pub fn register_hal(env: &Rc<Env>, hal: &Rc<RefCell<Hal>>, root: PathBuf) {
    fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: &Rc<Env>, name: &str, f: F) {
        env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
    }
    use need_args as need;

    // — окно (по ТЗ §3.1; «поверхность» здесь — программный фреймбуфер) —
    let h = hal.clone();
    def(env, "window-create", move |args| {
        need(args, 2)?;
        let w = num_val(&args[0])? as usize;
        let hh = num_val(&args[1])? as usize;
        let title = args.get(2).map(str_val).unwrap_or_else(|| "rlisp".into());
        let mut hal = h.borrow_mut();
        if w > 0 && hh > 0 && w * hh <= 4_000_000 {
            hal.w = w;
            hal.h = hh;
            hal.fb = vec![0; w * hh * 4];
        }
        let (cw, ch) = (hal.w, hal.h);
        hal.r3.reset(cw, ch);
        hal.title = title;
        hal.fill_all(8, 8, 18);
        Ok(Value::Int(1))
    });
    let h = hal.clone();
    def(env, "window-size", move |_: &[Value]| {
        let hal = h.borrow();
        Ok(Value::List(vec![Value::Int(hal.w as i64), Value::Int(hal.h as i64)]))
    });

    // — GPU: сырые растровые команды (по ТЗ §3.2, фасад для wgpu) —
    let h = hal.clone();
    def(env, "gpu-clear", move |args| {
        need(args, 3)?;
        let r = f2b(num_val(&args[0])?);
        let g = f2b(num_val(&args[1])?);
        let b = f2b(num_val(&args[2])?);
        h.borrow_mut().fill_all(r, g, b);
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "gpu-fill-rect", move |args| {
        need(args, 7)?;
        let x = num_val(&args[0])? as i64;
        let y = num_val(&args[1])? as i64;
        let w = num_val(&args[2])? as i64;
        let hh = num_val(&args[3])? as i64;
        let r = f2b(num_val(&args[4])?);
        let g = f2b(num_val(&args[5])?);
        let b = f2b(num_val(&args[6])?);
        h.borrow_mut().fill_rect(x, y, w, hh, r, g, b);
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "gpu-blit", move |args| {
        need(args, 5)?;
        let x = num_val(&args[0])? as i64;
        let y = num_val(&args[1])? as i64;
        let w = num_val(&args[2])? as usize;
        let hh = num_val(&args[3])? as usize;
        let bytes: Vec<u8> = match &args[4] {
            Value::Vector(r) => {
                let b = r.borrow();
                let mut out = Vec::with_capacity(b.len());
                for x in b.iter() {
                    out.push(int_val(x)? as u8);
                }
                out
            }
            Value::List(l) => {
                let mut out = Vec::with_capacity(l.len());
                for x in l {
                    out.push(int_val(x)? as u8);
                }
                out
            }
            _ => return Err("gpu-blit: байты должны быть вектором 0-255".into()),
        };
        h.borrow_mut().blit(x, y, w, hh, &bytes);
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "gpu-present", move |_: &[Value]| {
        use std::io::Write;
        let frame = {
            let mut hal = h.borrow_mut();
            hal.frame += 1;
            hal.frame
        };
        let hal = h.borrow();
        match &hal.out {
            OutputMode::Png { dir, every } => {
                let every = (*every as u64).max(1);
                if frame % every == 0 {
                    let path = dir.join(format!("frame_{:05}.png", frame));
                    png::write_png(&path, &hal.fb, hal.w, hal.h)?;
                }
            }
            OutputMode::Ascii => {
                let s = ascii_frame(&hal);
                print!("{}", s);
                std::io::stdout().flush().ok();
            }
            OutputMode::Silent => {}
        }
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "gpu-screenshot", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), std::path::Path::new("."));
        let hal = h.borrow();
        png::write_png(&p, &hal.fb, hal.w, hal.h)?;
        Ok(Value::Bool(true))
    });

    // — ввод (по ТЗ §3.3): очередь сырых событий —
    let h = hal.clone();
    def(env, "input-poll", move |_: &[Value]| {
        let mut hal = h.borrow_mut();
        Ok(Value::List(std::mem::take(&mut hal.queue)))
    });

    // — время (по ТЗ §3.6) —
    let h = hal.clone();
    def(env, "time-now", move |_: &[Value]| {
        Ok(Value::Float(h.borrow().start.elapsed().as_secs_f64()))
    });
    let h = hal.clone();
    def(env, "time-sleep", move |args| {
        need(args, 1)?;
        std::thread::sleep(std::time::Duration::from_secs_f64(num_val(&args[0])?.clamp(0.0, 60.0)));
        Ok(Value::Nil)
    });

    // — файлы (по ТЗ §3.5) —
    let r = root.clone();
    def(env, "file-read", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), &r);
        let s = std::fs::read_to_string(&p)
            .map_err(|e| format!("file-read {}: {}", p.display(), e))?;
        Ok(Value::Str(s))
    });
    let r = root.clone();
    def(env, "file-write", move |args| {
        need(args, 2)?;
        let p = resolve_path(&str_val(&args[0]), &r);
        std::fs::write(&p, str_val(&args[1]).as_bytes())
            .map_err(|e| format!("file-write {}: {}", p.display(), e))?;
        Ok(Value::Bool(true))
    });
    let r = root.clone();
    def(env, "file-exists?", move |args| {
        need(args, 1)?;
        Ok(Value::Bool(resolve_path(&str_val(&args[0]), &r).exists()))
    });
    let r = root.clone();
    def(env, "file-list", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), &r);
        let mut names = vec![];
        for entry in std::fs::read_dir(&p).map_err(|e| format!("file-list: {}", e))? {
            let entry = entry.map_err(|e| format!("file-list: {}", e))?;
            names.push(Value::Str(entry.file_name().to_string_lossy().into_owned()));
        }
        Ok(Value::List(names))
    });

    // — сырая память (по ТЗ §3.4): Lisp-мир живёт под GC, HAL-мир — вручную —
    let h = hal.clone();
    def(env, "mem-alloc", move |args| {
        need(args, 1)?;
        let n = (num_val(&args[0])? as usize).min(64 * 1024 * 1024);
        let mut hal = h.borrow_mut();
        let id = hal.next_mem;
        hal.next_mem += 1;
        hal.mem.insert(id, Rc::new(RefCell::new(vec![0u8; n])));
        Ok(Value::Ptr(id))
    });
    let h = hal.clone();
    def(env, "mem-free", move |args| {
        need(args, 1)?;
        if let Value::Ptr(id) = args[0] {
            h.borrow_mut().mem.remove(&id);
        }
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "mem-size", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let n = b.borrow().len();
        Ok(Value::Int(n as i64))
    });
    let h = hal.clone();
    def(env, "mem-write-f32", move |args| {
        need(args, 3)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let i = (int_val(&args[1])?.max(0) as usize) * 4;
        let v = num_val(&args[2])? as f32;
        let mut bb = b.borrow_mut();
        if i + 4 > bb.len() {
            return Err(format!("mem-write-f32: за границей (len {})", bb.len()));
        }
        bb[i..i + 4].copy_from_slice(&v.to_le_bytes());
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "mem-read-f32", move |args| {
        need(args, 2)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let i = (int_val(&args[1])?.max(0) as usize) * 4;
        let bb = b.borrow();
        if i + 4 > bb.len() {
            return Err(format!("mem-read-f32: за границей (len {})", bb.len()));
        }
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&bb[i..i + 4]);
        Ok(Value::Float(f32::from_le_bytes(buf) as f64))
    });
    let h = hal.clone();
    def(env, "mem-write-u8", move |args| {
        need(args, 3)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let i = int_val(&args[1])?.max(0) as usize;
        let v = int_val(&args[2])? as u8;
        let mut bb = b.borrow_mut();
        let slot = bb.get_mut(i).ok_or("mem-write-u8: за границей")?;
        *slot = v;
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "mem-read-u8", move |args| {
        need(args, 2)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let i = int_val(&args[1])?.max(0) as usize;
        let bb = b.borrow();
        Ok(Value::Int(*bb.get(i).ok_or("mem-read-u8: за границей")? as i64))
    });
    let h = hal.clone();
    def(env, "mem-fill", move |args| {
        need(args, 2)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let v = int_val(&args[1])? as u8;
        b.borrow_mut().fill(v);
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "mem-copy", move |args| {
        need(args, 3)?;
        let hal = h.borrow();
        let src = mem_block(&hal, &args[0])?;
        let dst = mem_block(&hal, &args[1])?;
        let len = int_val(&args[2])?.max(0) as usize;
        let data: Vec<u8> = src.borrow().iter().take(len).copied().collect();
        let mut d = dst.borrow_mut();
        if data.len() > d.len() {
            return Err("mem-copy: приёмник меньше источника".into());
        }
        d[..data.len()].copy_from_slice(&data);
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "mem-search-u8", move |args| {
        need(args, 2)?;
        let hal = h.borrow();
        let b = mem_block(&hal, &args[0])?;
        let needle = int_val(&args[1])? as u8;
        let bb = b.borrow();
        Ok(match bb.iter().position(|&x| x == needle) {
            Some(i) => Value::Int(i as i64),
            None => Value::Int(-1),
        })
    });

    // — текстуры (v0.3): id = индекс+1; 0 = «без текстуры» —
    let r = root.clone();
    let h = hal.clone();
    def(env, "texture-load", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), &r);
        let bytes = std::fs::read(&p).map_err(|e| format!("texture-load {}: {}", p.display(), e))?;
        let (px, w, hh) = png::decode_png(&bytes)?;
        let mut hal = h.borrow_mut();
        hal.tex.push(Tex { w, h: hh, px });
        Ok(Value::Int(hal.tex.len() as i64))
    });
    let h = hal.clone();
    def(env, "texture-upload", move |args| {
        need(args, 3)?;
        let w = int_val(&args[0])?.max(1) as usize;
        let hh = int_val(&args[1])?.max(1) as usize;
        if w * hh > 16_000_000 {
            return Err("texture-upload: слишком большая".into());
        }
        let px: Vec<u8> = match &args[2] {
            Value::Vector(r) => {
                let b = r.borrow();
                let mut out = Vec::with_capacity(b.len());
                for x in b.iter() {
                    out.push(int_val(x)? as u8);
                }
                out
            }
            Value::List(l) => {
                let mut out = Vec::with_capacity(l.len());
                for x in l {
                    out.push(int_val(x)? as u8);
                }
                out
            }
            Value::F32Vec(r) => {
                let b = r.borrow();
                let mut out = Vec::with_capacity(b.len());
                for x in b.iter() {
                    out.push((*x as f64 * 255.0).round().clamp(0.0, 255.0) as u8);
                }
                out
            }
            _ => return Err("texture-upload: байты — вектор/список/f32vec 0-255".into()),
        };
        if px.len() < w * hh * 4 {
            return Err(format!("texture-upload: байт {} < {}x{}x4", px.len(), w, hh));
        }
        let mut hal = h.borrow_mut();
        hal.tex.push(Tex { w, h: hh, px });
        Ok(Value::Int(hal.tex.len() as i64))
    });
    let h = hal.clone();
    def(env, "texture-size", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let id = int_val(&args[0])? as usize;
        let t = hal.tex.get(id.wrapping_sub(1)).ok_or("texture-size: нет такой текстуры")?;
        Ok(Value::List(vec![Value::Int(t.w as i64), Value::Int(t.h as i64)]))
    });
    let h = hal.clone();
    def(env, "texture-pixels", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let id = int_val(&args[0])? as usize;
        let t = hal.tex.get(id.wrapping_sub(1)).ok_or("texture-pixels: нет такой текстуры")?;
        Ok(Value::Vector(Rc::new(RefCell::new(
            t.px.iter().map(|&b| Value::Int(b as i64)).collect(),
        ))))
    });
    let h = hal.clone();
    def(env, "texture-free", move |args| {
        need(args, 1)?;
        let mut hal = h.borrow_mut();
        let id = int_val(&args[0])?;
        if id >= 1 && id <= hal.tex.len() as i64 {
            hal.tex[(id - 1) as usize] = Tex { w: 0, h: 0, px: vec![] };
        }
        Ok(Value::Nil)
    });
    // 2D-спрайт из текстуры: blit с опциональным тинтом (r g b 0..1) и масштабом
    let h = hal.clone();
    def(env, "gpu-blit-tex", move |args| {
        need(args, 4)?;
        let id = int_val(&args[0])?;
        let x = num_val(&args[1])? as i64;
        let y = num_val(&args[2])? as i64;
        let scale = args.get(3).map(|v| num_val(v)).transpose()?.unwrap_or(1.0).max(0.0) as f32;
        let tint = match args.get(4) {
            Some(v) => to_rgb(v)?,
            None => (1.0f32, 1.0, 1.0),
        };
        let mut hal = h.borrow_mut();
        let t = hal.tex.get((id.max(1) - 1) as usize).ok_or("gpu-blit-tex: нет такой текстуры")?.clone();
        let (tw, th) = (t.w, t.h);
        let dw = (tw as f32 * scale).round().max(1.0) as i64;
        let dh = (th as f32 * scale).round().max(1.0) as i64;
        for row in 0..dh {
            let py = y + row;
            if py < 0 || py >= hal.h as i64 {
                continue;
            }
            let sy = ((row as f32 / scale) as usize).min(th - 1);
            for col in 0..dw {
                let pxx = x + col;
                if pxx < 0 || pxx >= hal.w as i64 {
                    continue;
                }
                let sx = ((col as f32 / scale) as usize).min(tw - 1);
                let si = (sy * tw + sx) * 4;
                let a = t.px[si + 3];
                if a == 0 {
                    continue; // прозрачный пиксель
                }
                let di = (py as usize * hal.w + pxx as usize) * 4;
                hal.fb[di] = ((t.px[si] as f32 * tint.0) as u8).clamp(0, 255);
                hal.fb[di + 1] = ((t.px[si + 1] as f32 * tint.1) as u8).clamp(0, 255);
                hal.fb[di + 2] = ((t.px[si + 2] as f32 * tint.2) as u8).clamp(0, 255);
                hal.fb[di + 3] = 255;
            }
        }
        Ok(Value::Nil)
    });

    // — загрузка Lisp-файлов поверх HAL (используется main.lisp) —
    def(env, "sys-load", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), &root);
        load_file(&p, &crate::interp::CoreCtx {
            global: Rc::new(RefCell::new(crate::interp::env_new(None))),
            registry: Rc::new(RefCell::new(vec![])),
            root: PathBuf::from("."),
        })
        .map(|_| Value::Bool(true))
        .map_err(|e| format!("sys-load: {}", e))
    });
}
