use super::*;
use crate::hal::Hal;

pub(crate) fn близко(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

pub(crate) fn ед_матрицы() -> Vec<f32> {
    let mut m = vec![0.0f32; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    let mut out = m.clone();
    out.extend_from_slice(&m);
    out
}

pub(crate) fn сброс_света(hal: &mut Hal) {
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
}

pub(crate) fn свет_плоский(hal: &mut Hal) {
    сброс_света(hal);
    hal.r3.light = [0.0, 0.0, 1.0];
    hal.r3.ambient = 1.0;
    hal.r3.diffuse = 0.0;
}

pub(crate) fn рисуем_квадрат(hal: &mut Hal, tex: i64, z: f32, uv: &[f32]) {
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

pub(crate) fn пиксель(hal: &Hal, x: usize, y: usize) -> [u8; 4] {
    let o = (y * hal.w + x) * 4;
    [hal.fb[o], hal.fb[o + 1], hal.fb[o + 2], hal.fb[o + 3]]
}

mod pipeline;
mod raster;
