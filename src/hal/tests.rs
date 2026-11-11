// src/hal/tests.rs — Тесты диспетчеризации бэкендов, менеджера ресурсов, base64 и аудио.

use super::*;
use crate::gpu::GpuStats;
use crate::interp::{fmt_value, Value};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[test]
fn диспетчер_дефолт_software_пишет_в_fb() {
    let mut hal = Hal::new(OutputMode::Silent, 32, 32);
    hal.clear_all(1, 2, 3);
    assert_eq!(&hal.fb[0..4], &[1, 2, 3, 255]);
    hal.fill_rect(0, 0, 4, 4, 9, 9, 9);
    assert_eq!(&hal.fb[0..4], &[9, 9, 9, 255]);
    let sprite = vec![5, 6, 7, 255];
    hal.blit(2, 2, 1, 1, &sprite).unwrap();
    assert_eq!(
        &hal.fb[(2 * 32 + 2) * 4..(2 * 32 + 2) * 4 + 4],
        &[5, 6, 7, 255]
    );
    // без flush — no-op
    let fb_before = hal.fb.clone();
    let (sub, rb) = hal.flush_gpu(true);
    assert_eq!((sub, rb), (0, 0));
    assert_eq!(hal.fb, fb_before);
}

#[test]
fn диспетчер_blit_tex_software_тинт_и_ошибка_освобождённой() {
    let mut hal = Hal::new(OutputMode::Silent, 16, 16);
    let px = vec![255u8; 4 * 4 * 4];
    hal.tex.push(Tex::new(4, 4, px));
    hal.tex_register(1, 4, 4, &[255u8; 64]);
    hal.blit_tex(1, 0, 0, 1.0, [0.5, 0.5, 0.5]).unwrap();
    assert_eq!(hal.fb[0], 127); // 255 * 0.5
    hal.tex_free(1);
    assert!(hal.blit_tex(1, 0, 0, 1.0, [1.0, 1.0, 1.0]).is_err());
    // вызов неизвестного id
    assert!(hal.blit_tex(99, 0, 0, 1.0, [1.0, 1.0, 1.0]).is_err());
}

#[test]
fn ресуры_цикл_текстур_и_счётчики() {
    let mut hal = Hal::new(OutputMode::Silent, 16, 16);
    let px = vec![0u8; 64];
    hal.tex.push(Tex::new(4, 4, px.clone()));
    hal.tex_register(1, 4, 4, &px);
    assert_eq!(hal.res.stats.tex_created, 1);
    assert_eq!(hal.res.live(), 1);
    hal.tex_free(1);
    assert_eq!(hal.res.stats.tex_freed, 1);
    assert_eq!(hal.res.live(), 0);
    assert_eq!(hal.res.stats.tex_live(), 0);
}

#[test]
fn стресс_hot_reload_100_циклов_без_роста_памяти() {
    let mut hal = Hal::new(OutputMode::Silent, 16, 16);
    let px = vec![7u8; 16 * 16 * 4]; // «процедурная текстура» 1 КБ
    for cycle in 0..100 {
        // как на каждом reload 3D-игры: пересоздаются 5 текстур
        for _ in 0..5 {
            let id = (hal.tex.len() + 1) as i64;
            hal.tex.push(Tex::new(16, 16, px.clone()));
            hal.tex_register(id, 16, 16, &px);
        }
        // persistent-мир держит только id=1
        let keep = [1i64];
        hal.invalidate_textures_except(&keep);
        assert_eq!(hal.res.live(), 1, "цикл {}: жив один хэндл", cycle);
        assert_eq!(
            hal.res.bytes(),
            16 * 16 * 4,
            "цикл {}: память не растёт",
            cycle
        );
        // реестр сжат: хвостовые пустые слоты срезаны
        assert!(
            hal.tex.iter().all(|t| t.w > 0),
            "цикл {}: нет пустых слотов",
            cycle
        );
    }
    let GpuStats {
        tex_created,
        tex_freed,
        reload_invalidated,
        ..
    } = hal.res.stats.clone();
    assert_eq!(tex_created, 500);
    assert_eq!(tex_freed, 499);
    assert_eq!(reload_invalidated, 499);
    assert_eq!(hal.res.stats.tex_live(), 1);
    // полная зачистка — ноль живых
    hal.invalidate_textures_except(&[]);
    assert_eq!(hal.res.stats.tex_live(), 0);
    assert_eq!(hal.res.bytes(), 0);
    assert!(hal.tex.is_empty());
}

#[test]
fn resize_window_меняет_размер_и_фб() {
    let mut hal = Hal::new(OutputMode::Silent, 32, 32);
    hal.resize_window(64, 48);
    assert_eq!((hal.w, hal.h), (64, 48));
    assert_eq!(hal.fb.len(), 64 * 48 * 4);
    // отказ слишком большой поверхности — размер не меняется
    hal.resize_window(4096, 4096);
    assert_eq!((hal.w, hal.h), (64, 48));
}

// ——— этап 8: base64-декодер (RFC 4648 тест-векторы) и JSON → Lisp ———

#[test]
fn base64_rfc4648_векторы() {
    let d = |s: &str| base64_decode(s).unwrap();
    assert!(d("").is_empty());
    assert_eq!(d("Zg=="), b"f");
    assert_eq!(d("Zm8="), b"fo");
    assert_eq!(d("Zm9v"), b"foo");
    assert_eq!(d("Zm9vYg=="), b"foob");
    assert_eq!(d("Zm9vYmU="), b"foobe");
    assert_eq!(d("Zm9vYmVy"), b"foober");
    // переносы строк внутри payload (письма и длинные data-URI)
    assert_eq!(d("Zm9v\nYmVy"), b"foober");
    // data-URI: всё до первой запятой отбрасывается
    assert_eq!(d("data:application/octet-stream;base64,Zm9vYmU="), b"foobe");
}

#[test]
fn base64_ошибки_и_каноничность() {
    // недопустимый символ
    assert!(base64_decode("Zm*v").is_err());
    // данные после паддинга
    assert!(base64_decode("Zg==A").is_err());
    // «висячий» символ — длина значимых символов не кратна группе
    assert!(base64_decode("Z").is_err());
    assert!(base64_decode("Zm8").is_ok()); // 3 символа — каноничный хвост
                                           // ненулевые биты «после паддинга»
    assert!(base64_decode("Zh==").is_err());
    // data-URI без запятой
    assert!(base64_decode("data:image/png;base64").is_err());
}

#[test]
fn json_парсер_даёт_таблицы_и_списки() {
    use crate::json::{json_to_value, parse};
    let v = json_to_value(&parse(r#"{"a":1,"b":[1,2.5,"x"],"c":{"d":true}}"#).unwrap());
    match &v {
        Value::Table(t) => {
            assert_eq!(t.borrow().len(), 3);
            // числа без дробной части → Int, с дробной → Float
            assert!(matches!(t.borrow().get("a"), Some(Value::Int(1))));
            match t.borrow().get("b").unwrap() {
                Value::List(l) => {
                    assert_eq!(l.len(), 3);
                    assert!(matches!(l[0], Value::Int(1)));
                    assert!(matches!(l[1], Value::Float(x) if x == 2.5));
                    assert!(matches!(l[2], Value::Str(ref s) if s == "x"));
                }
                other => panic!("b: не список, а {}", fmt_value(other)),
            }
            match t.borrow().get("c").unwrap() {
                Value::Table(inner) => {
                    assert!(matches!(inner.borrow().get("d"), Some(Value::Bool(true))))
                }
                other => panic!("c: не таблица, а {}", fmt_value(other)),
            }
        }
        other => panic!("корень: не таблица, а {}", fmt_value(other)),
    }
    // битый JSON — ошибка, не паника
    assert!(crate::json::parse("{oops").is_err());
}

#[test]
fn фолбэк_флаг_wgpu_без_устройства_даёт_software() {
    // логика выбора: probe Err => Software, и предупреждение печатается
    let probe = crate::gpu::create_wgpu_backend(16, 16);
    if let Err(e) = &probe {
        let warn = crate::gpu::fallback_warning(&probe).unwrap();
        assert!(warn.contains("--wgpu") && warn.contains(e));
    }
    // в любом исходе Hal::new с Software работает
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.clear_all(4, 5, 6);
    assert_eq!(&hal.fb[0..4], &[4, 5, 6, 255]);
}

#[test]
fn audio_lisp_primitives_flow() {
    let env = crate::interp::env_new(None);
    let hal = Rc::new(RefCell::new(Hal::new(OutputMode::Silent, 8, 8)));
    register_hal(env, &hal, PathBuf::from("."));

    // Создание звука через sound-synth
    let synth_fn = crate::interp::env_get(env, "sound-synth").unwrap();
    let sid_val = crate::interp::apply(
        &synth_fn,
        &[
            Value::Str("sawtooth".into()),
            Value::Float(440.0),
            Value::Float(0.1),
        ],
        0,
    )
    .expect("sound-synth");
    let sid = match sid_val {
        Value::Int(n) => n,
        other => panic!("ожидался Int id, получено: {}", fmt_value(&other)),
    };
    assert!(sid > 0);

    // Проверка воспроизведения
    let play_fn = crate::interp::env_get(env, "sound-play").unwrap();
    let vid_val = crate::interp::apply(&play_fn, &[Value::Int(sid), Value::Float(0.9)], 0)
        .expect("sound-play");
    let vid = match vid_val {
        Value::Int(n) => n,
        other => panic!("ожидался voice_id Int, получено: {}", fmt_value(&other)),
    };
    assert!(vid > 0);

    // Статистика
    let stats_fn = crate::interp::env_get(env, "sound-stats").unwrap();
    let stats_val = crate::interp::apply(&stats_fn, &[], 0).expect("sound-stats");
    match stats_val {
        Value::Table(t) => {
            let tb = t.borrow();
            assert!(matches!(tb.get("sounds"), Some(Value::Int(1))));
            assert!(matches!(tb.get("voices"), Some(Value::Int(1))));
        }
        other => panic!("sound-stats должен вернуть таблицу: {}", fmt_value(&other)),
    }

    // Остановка звука
    let stop_fn = crate::interp::env_get(env, "sound-stop").unwrap();
    crate::interp::apply(&stop_fn, &[Value::Int(vid)], 0).expect("sound-stop");

    let stats_val2 = crate::interp::apply(&stats_fn, &[], 0).expect("sound-stats");
    if let Value::Table(t) = stats_val2 {
        let tb = t.borrow();
        assert!(matches!(tb.get("voices"), Some(Value::Int(0))));
    }
}
