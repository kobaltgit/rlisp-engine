// src/hal/lisp.rs — Lisp-биндинги HAL: окно, растр, ввод, время, файлы, память, текстуры, звук.

use super::core::{ascii_frame, base64_decode, f2b, mem_block, str_val, to_rgb, Hal};
use super::types::{OutputMode, Tex};
use crate::interp::{
    env_define, int_val, load_file, need_args, num_val, resolve_path, Gc, Value,
};
use crate::png;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub fn register_hal(env: Gc, hal: &Rc<RefCell<Hal>>, root: PathBuf) {
    fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: Gc, name: &str, f: F) {
        env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
    }
    use need_args as need;

    // — окно (по ТЗ §3.1; «поверхность» — фреймбуфер или offscreen-wgpu) —
    let h = hal.clone();
    def(env, "window-create", move |args| {
        need(args, 2)?;
        let w = num_val(&args[0])? as usize;
        let hh = num_val(&args[1])? as usize;
        let title = args.get(2).map(str_val).unwrap_or_else(|| "rlisp".into());
        let mut hal = h.borrow_mut();
        hal.resize_window(w, hh);
        hal.title = title;
        hal.fill_all(8, 8, 18);
        Ok(Value::Int(1))
    });
    let h = hal.clone();
    def(env, "window-size", move |_: &[Value]| {
        let hal = h.borrow();
        Ok(Value::List(vec![
            Value::Int(hal.w as i64),
            Value::Int(hal.h as i64),
        ]))
    });

    // — GPU: сырые растровые команды (по ТЗ §3.2); бэкенд выбирает Hal —
    let h = hal.clone();
    def(env, "gpu-clear", move |args| {
        need(args, 3)?;
        let r = f2b(num_val(&args[0])?);
        let g = f2b(num_val(&args[1])?);
        let b = f2b(num_val(&args[2])?);
        h.borrow_mut().clear_all(r, g, b);
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
        h.borrow_mut().blit(x, y, w, hh, &bytes)?;
        Ok(Value::Nil)
    });
    let h = hal.clone();
    def(env, "gpu-present", move |_: &[Value]| {
        use std::io::Write;
        let frame = {
            let mut hal = h.borrow_mut();
            let need_rb = hal.readback_requested || !matches!(&hal.out, OutputMode::Silent);
            hal.readback_requested = false;
            // wgpu: досылаем отложенные 2D-команды в fb ДО вывода кадра (при Silent — без блокировки readback)
            let _ = hal.flush_gpu(need_rb);
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
        let mut hal = h.borrow_mut();
        // MCP/скрипты: перед снимком досылаем GPU-слой в fb
        let _ = hal.flush_gpu(true);
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
        Ok(Value::Float(h.borrow().start.elapsed_secs()))
    });
    def(env, "time-sleep", move |args| {
        need(args, 1)?;
        std::thread::sleep(std::time::Duration::from_secs_f64(
            num_val(&args[0])?.clamp(0.0, 60.0),
        ));
        Ok(Value::Nil)
    });

    // — файлы (по ТЗ §3.5) —
    let r = root.clone();
    def(env, "file-read", move |args| {
        need(args, 1)?;
        let name = str_val(&args[0]);
        if let Some(b) = crate::interp::embedded_lookup(&name) {
            return Ok(Value::Str(
                String::from_utf8(b.to_vec())
                    .map_err(|_| "file-read: embed не UTF-8".to_string())?,
            ));
        }
        let p = resolve_path(&name, &r);
        let s =
            std::fs::read_to_string(&p).map_err(|e| format!("file-read {}: {}", p.display(), e))?;
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
        let name = str_val(&args[0]);
        Ok(Value::Bool(
            crate::interp::embedded_lookup(&name).is_some() || resolve_path(&name, &r).exists(),
        ))
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

    // — этап 8: сырые данные для glTF-пайплайна (модели из интернета) —
    // json-parse: строка → Lisp-таблицы/списки (через существующий json.rs)
    def(env, "json-parse", move |args| {
        need(args, 1)?;
        let j = crate::json::parse(&str_val(&args[0]))?;
        Ok(crate::json::json_to_value(&j))
    });
    // base64-decode: строка (или data-URI) → f32-вектор байтов 0..255
    def(env, "base64-decode", move |args| {
        need(args, 1)?;
        let bytes = base64_decode(&str_val(&args[0]))?;
        Ok(Value::F32Vec(Rc::new(RefCell::new(
            bytes.into_iter().map(|b| b as f32).collect(),
        ))))
    });
    // file-read-bytes: бинарный файл (например .bin glTF) → f32-вектор байтов.
    // file-read (строка) не годится: std::fs::read_to_string требует валидный UTF-8.
    let r = root.clone();
    def(env, "file-read-bytes", move |args| {
        need(args, 1)?;
        let name = str_val(&args[0]);
        let bytes = match crate::interp::embedded_lookup(&name) {
            Some(b) => b.to_vec(),
            None => {
                let p = resolve_path(&name, &r);
                std::fs::read(&p).map_err(|e| format!("file-read-bytes {}: {}", p.display(), e))?
            }
        };
        Ok(Value::F32Vec(Rc::new(RefCell::new(
            bytes.into_iter().map(|b| b as f32).collect(),
        ))))
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
        Ok(Value::Int(
            *bb.get(i).ok_or("mem-read-u8: за границей")? as i64
        ))
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
        let name = str_val(&args[0]);
        let bytes = match crate::interp::embedded_lookup(&name) {
            Some(b) => b.to_vec(),
            None => {
                let p = resolve_path(&name, &r);
                std::fs::read(&p).map_err(|e| format!("texture-load {}: {}", p.display(), e))?
            }
        };
        let (px, w, hh) = png::decode_png(&bytes)?;
        let mut hal = h.borrow_mut();
        hal.tex.push(Tex::new(w, hh, px.clone()));
        let id = hal.tex.len() as i64;
        // менеджер ресурсов: хэндл в книгу + загрузка на активный бэкенд
        hal.tex_register(id, w, hh, &px);
        Ok(Value::Int(id))
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
            return Err(format!(
                "texture-upload: байт {} < {}x{}x4",
                px.len(),
                w,
                hh
            ));
        }
        let mut hal = h.borrow_mut();
        hal.tex.push(Tex::new(w, hh, px.clone()));
        let id = hal.tex.len() as i64;
        hal.tex_register(id, w, hh, &px);
        Ok(Value::Int(id))
    });
    // texture-load-png: PNG-байты (f32-вектор 0..255, например из
    // base64-decode) → декодер движка (png.rs) → текстура в реестре.
    // Путь для встроенных в glTF текстур без временных файлов.
    let h = hal.clone();
    def(env, "texture-load-png", move |args| {
        need(args, 1)?;
        let bytes: Vec<u8> = match &args[0] {
            Value::F32Vec(r) => {
                let b = r.borrow();
                let mut out = Vec::with_capacity(b.len());
                for x in b.iter() {
                    out.push(*x as u8);
                }
                out
            }
            Value::Vector(r) => {
                let b = r.borrow();
                let mut out = Vec::with_capacity(b.len());
                for x in b.iter() {
                    out.push(int_val(x)? as u8);
                }
                out
            }
            _ => return Err("texture-load-png: нужны PNG-байты (f32-вектор 0-255)".into()),
        };
        let (px, w, hh) = png::decode_png(&bytes)?;
        let mut hal = h.borrow_mut();
        hal.tex.push(Tex::new(w, hh, px.clone()));
        let id = hal.tex.len() as i64;
        hal.tex_register(id, w, hh, &px);
        Ok(Value::Int(id))
    });
    let h = hal.clone();
    def(env, "texture-size", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let id = int_val(&args[0])? as usize;
        let t = hal
            .tex
            .get(id.wrapping_sub(1))
            .ok_or("texture-size: нет такой текстуры")?;
        Ok(Value::List(vec![
            Value::Int(t.w as i64),
            Value::Int(t.h as i64),
        ]))
    });
    let h = hal.clone();
    def(env, "texture-mips", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let id = int_val(&args[0])? as usize;
        let t = hal
            .tex
            .get(id.wrapping_sub(1))
            .ok_or("texture-mips: нет такой текстуры")?;
        Ok(Value::Int(t.mips.len() as i64))
    });
    let h = hal.clone();
    def(env, "texture-pixels", move |args| {
        need(args, 1)?;
        let hal = h.borrow();
        let id = int_val(&args[0])? as usize;
        let t = hal
            .tex
            .get(id.wrapping_sub(1))
            .ok_or("texture-pixels: нет такой текстуры")?;
        Ok(Value::Vector(Rc::new(RefCell::new(
            t.px.iter().map(|&b| Value::Int(b as i64)).collect(),
        ))))
    });
    let h = hal.clone();
    def(env, "texture-free", move |args| {
        need(args, 1)?;
        let mut hal = h.borrow_mut();
        let id = int_val(&args[0])?;
        // менеджер ресурсов: хэндл освобождается и в книге, и на бэкенде
        hal.tex_free(id);
        Ok(Value::Nil)
    });
    // 2D-спрайт из текстуры: blit с опциональным тинтом (r g b 0..1) и масштабом
    let h = hal.clone();
    def(env, "gpu-blit-tex", move |args| {
        need(args, 4)?;
        let id = int_val(&args[0])?;
        let x = num_val(&args[1])? as i64;
        let y = num_val(&args[2])? as i64;
        let scale = args
            .get(3)
            .map(num_val)
            .transpose()?
            .unwrap_or(1.0)
            .max(0.0) as f32;
        let tint = match args.get(4) {
            Some(v) => to_rgb(v)?,
            None => (1.0f32, 1.0, 1.0),
        };
        h.borrow_mut()
            .blit_tex(id, x, y, scale, [tint.0, tint.1, tint.2])?;
        Ok(Value::Nil)
    });

    // — загрузка Lisp-файлов поверх HAL (используется main.lisp) —
    let root_sys = root.clone();
    def(env, "sys-load", move |args| {
        need(args, 1)?;
        let p = resolve_path(&str_val(&args[0]), &root_sys);
        load_file(
            &p,
            &crate::interp::CoreCtx {
                global: Rc::new(RefCell::new(crate::interp::env_new(None))),
                registry: Rc::new(RefCell::new(vec![])),
                root: PathBuf::from("."),
            },
        )
        .map(|_| Value::Bool(true))
        .map_err(|e| format!("sys-load: {}", e))
    });

    // ——— Аудиосистема: загрузка WAV, чиптюн-синтезатор, управление голосами ———
    let h = hal.clone();
    let root_clone = root.clone();
    def(env, "sound-load", move |args| {
        need(args, 1)?;
        let path_str = str_val(&args[0]);
        let resolved = resolve_path(&path_str, &root_clone);
        let bytes = std::fs::read(&resolved)
            .map_err(|e| format!("sound-load {}: {}", resolved.display(), e))?;
        let name = resolved
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound")
            .to_string();
        let sound = crate::audio::WavCodec::decode(0, name, &bytes)?;
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        let id = audio.add_sound(sound);
        Ok(Value::Int(id as i64))
    });

    let h = hal.clone();
    def(env, "sound-synth", move |args| {
        need(args, 3)?;
        let wave_str = str_val(&args[0]);
        let wave = crate::audio::Waveform::from_str(&wave_str).ok_or_else(|| {
            format!(
                "sound-synth: неизвестная форма волны '{}' (square, saw, triangle, sine, noise)",
                wave_str
            )
        })?;
        let freq = num_val(&args[1])? as f32;
        let duration = num_val(&args[2])? as f32;
        let attack = args.get(3).map(num_val).transpose()?.unwrap_or(0.01) as f32;
        let decay = args.get(4).map(num_val).transpose()?.unwrap_or(0.05) as f32;
        let sustain = args.get(5).map(num_val).transpose()?.unwrap_or(0.5) as f32;
        let release = args.get(6).map(num_val).transpose()?.unwrap_or(0.1) as f32;
        let pitch_slide = args.get(7).map(num_val).transpose()?.unwrap_or(0.0) as f32;
        let duty_cycle = args.get(8).map(num_val).transpose()?.unwrap_or(0.5) as f32;

        let params = crate::audio::SynthParams {
            wave,
            freq,
            duration,
            attack,
            decay,
            sustain,
            release,
            pitch_slide,
            duty_cycle,
        };
        let sound =
            crate::audio::synth_sound(0, format!("synth_{}_{:.0}hz", wave.as_str(), freq), &params);
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        let id = audio.add_sound(sound);
        Ok(Value::Int(id as i64))
    });

    let h = hal.clone();
    def(env, "sound-play", move |args| {
        need(args, 1)?;
        let sound_id = int_val(&args[0])? as u64;
        let vol = args.get(1).map(num_val).transpose()?.unwrap_or(1.0) as f32;
        let pitch = args.get(2).map(num_val).transpose()?.unwrap_or(1.0) as f32;
        let pan = args.get(3).map(num_val).transpose()?.unwrap_or(0.0) as f32;
        let looping = args
            .get(4)
            .map(|v| match v {
                Value::Bool(b) => *b,
                _ => false,
            })
            .unwrap_or(false);

        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        let voice_id = audio.play(sound_id, vol, pitch, pan, looping)?;
        Ok(Value::Int(voice_id as i64))
    });

    let h = hal.clone();
    def(env, "sound-stop", move |args| {
        need(args, 1)?;
        let voice_id = int_val(&args[0])? as u64;
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        audio.stop(voice_id);
        Ok(Value::Nil)
    });

    let h = hal.clone();
    def(env, "sound-stop-all", move |_args| {
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        audio.stop_all();
        Ok(Value::Nil)
    });

    let h = hal.clone();
    def(env, "sound-volume", move |args| {
        need(args, 2)?;
        let voice_id = int_val(&args[0])? as u64;
        let vol = num_val(&args[1])? as f32;
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        audio.set_voice_volume(voice_id, vol);
        Ok(Value::Nil)
    });

    let h = hal.clone();
    def(env, "sound-master-volume", move |args| {
        let hal = h.borrow();
        let mut audio = hal.audio.lock().unwrap();
        if let Some(arg) = args.first() {
            let vol = num_val(arg)? as f32;
            audio.set_master_volume(vol);
        }
        Ok(Value::Float(audio.master_volume as f64))
    });

    let h = hal.clone();
    def(env, "sound-stats", move |_args| {
        let hal = h.borrow();
        let audio = hal.audio.lock().unwrap();
        let mut tbl = HashMap::new();
        tbl.insert("voices".into(), Value::Int(audio.voices.len() as i64));
        tbl.insert("sounds".into(), Value::Int(audio.sounds.len() as i64));
        tbl.insert(
            "master-volume".into(),
            Value::Float(audio.master_volume as f64),
        );
        Ok(Value::Table(Rc::new(RefCell::new(tbl))))
    });
}
