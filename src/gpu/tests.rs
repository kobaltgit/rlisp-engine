// src/gpu/tests.rs — Тесты GPU-подсистемы (книга ресурсов, композит, fallback).

use super::*;

#[test]
fn ресурсы_регистр_фри_счётчики() {
    let mut r = GpuResources::default();
    r.register(1, 16 * 16 * 4);
    r.register(2, 8 * 8 * 4);
    assert_eq!(r.live(), 2);
    assert_eq!(r.bytes(), 16 * 16 * 4 + 8 * 8 * 4);
    assert_eq!(r.stats.tex_live(), 2);
    assert!(r.free(1));
    assert!(!r.free(1)); // повторное освобождение — no-op
    assert_eq!(r.live(), 1);
    assert_eq!(r.stats.tex_live(), 1);
}

#[test]
fn ресурсы_перезагрузка_того_же_id_растит_поколение() {
    let mut r = GpuResources::default();
    r.register(1, 64);
    assert_eq!(r.generation(1), 1);
    r.register(1, 64);
    assert_eq!(r.generation(1), 2);
    assert_eq!(r.live(), 1);
    assert_eq!(r.stats.tex_created, 2);
    assert_eq!(r.stats.tex_freed, 1); // перезагрузка заместила старое поколение
    assert!(r.free(1));
    assert_eq!(r.stats.tex_freed, 2);
    assert_eq!(r.stats.tex_live(), 0); // создано 2 — освобождено 2, утечки нет
}

#[test]
fn ресурсы_инвалидация_hot_reload_кроме_persistent() {
    let mut r = GpuResources::default();
    for id in 1..=5 {
        r.register(id, 64);
    }
    let freed = r.invalidate_except(&[2]);
    assert_eq!(freed, 4);
    assert_eq!(r.live(), 1);
    assert_eq!(r.stats.reload_invalidated, 4);
    assert_eq!(r.stats.reload_cycles, 1);
}

#[test]
fn стресс_100_циклов_создать_перезагрузить_без_роста() {
    let mut r = GpuResources::default();
    for cycle in 0..100 {
        // как game3d.lisp за reload: 5 процедурных текстур + 1 persistent id=1
        for id in 1..=5usize {
            r.register(id as i64, 16 * 16 * 4);
        }
        r.invalidate_except(&[1]);
        assert_eq!(r.live(), 1, "цикл {}: после reload жива одна", cycle);
        assert_eq!(r.bytes(), 16 * 16 * 4, "цикл {}: байт не растёт", cycle);
    }
    assert_eq!(r.stats.tex_created, 500);
    assert_eq!(r.stats.tex_freed, 500 - 1);
    r.free(1);
    assert_eq!(r.stats.tex_live(), 0);
}

#[test]
fn композит_readback_по_альфе_покрытия() {
    // fb: всё синее; GPU покрывает 1 пиксель красным + 1 пиксель прозрачным
    let (w, h) = (2usize, 1usize);
    let pad = 256usize;
    let mut fb = vec![0u8; w * h * 4];
    for p in 0..w * h {
        fb[p * 4] = 0;
        fb[p * 4 + 1] = 0;
        fb[p * 4 + 2] = 200;
        fb[p * 4 + 3] = 255;
    }
    let mut data = vec![0u8; pad * h];
    data[0] = 255;
    data[1] = 10;
    data[2] = 20;
    data[3] = 255; // пиксель 0: красный, покрыт
    data[4] = 99;
    data[5] = 99;
    data[6] = 99;
    data[7] = 0; // пиксель 1: прозрачный — fb не трогаем
    composite_readback(&mut fb, w, h, pad, &data);
    assert_eq!(&fb[0..4], &[255, 10, 20, 255]);
    assert_eq!(&fb[4..8], &[0, 0, 200, 255]);
}

#[test]
fn фолбэк_без_фичи_даёт_понятное_предупреждение() {
    // без фичи `wgpu` create_wgpu_backend всегда Err → откат на софт
    let probe = create_wgpu_backend(64, 64);
    match &probe {
        Ok(b) => {
            // на машине с адаптером и собранной фичей — тоже валидный исход
            assert!(!b.adapter_info().is_empty());
        }
        Err(e) => {
            assert!(fallback_warning(&probe).is_some());
            assert!(!e.is_empty());
        }
    }
}

#[test]
fn collect_int_refs_достаёт_id_из_таблиц_списков() {
    use crate::interp::Value;
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut out = vec![];
    collect_int_refs(
        &Value::List(vec![
            Value::Int(3),
            Value::Table(Rc::new(RefCell::new(
                [("tex".into(), Value::Int(7))].into_iter().collect(),
            ))),
            Value::Str("не число".into()),
        ]),
        &mut out,
        0,
    );
    out.sort();
    assert_eq!(out, vec![3, 7]);
}
