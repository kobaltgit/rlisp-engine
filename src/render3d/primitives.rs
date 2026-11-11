// src/render3d/primitives.rs — Высокоуровневая отрисовка примитивов (сетки, каркасы, батчи).

use super::raster::*;
use super::shading::*;
use crate::hal::Hal;

/// T&L: world = M_model · v, clip = M_mvp · world
/// (UV-атрибуты интерполируются отдельно — см. raster_tris/uvq).
pub(crate) fn transform_all(
    pos: &[f32],
    nrm: Option<&[f32]>,
    model: &[f32],
    mvp: &[f32],
    world: &mut Vec<f32>,
    world_nrm: &mut Vec<f32>,
    clip: &mut Vec<f32>,
) {
    let n = pos.len() / 3;
    world.clear();
    world.resize(n * 3, 0.0);
    clip.clear();
    clip.resize(n * 4, 0.0);
    world_nrm.clear();
    if let Some(normals) = nrm {
        world_nrm.resize(n * 3, 0.0);
        for i in 0..n {
            let nx = normals[i * 3];
            let ny = normals[i * 3 + 1];
            let nz = normals[i * 3 + 2];
            let wnx = model[0] * nx + model[1] * ny + model[2] * nz;
            let wny = model[4] * nx + model[5] * ny + model[6] * nz;
            let wnz = model[8] * nx + model[9] * ny + model[10] * nz;
            let l = (wnx * wnx + wny * wny + wnz * wnz).sqrt().max(1e-9);
            world_nrm[i * 3] = wnx / l;
            world_nrm[i * 3 + 1] = wny / l;
            world_nrm[i * 3 + 2] = wnz / l;
        }
    }
    for i in 0..n {
        let x = pos[i * 3];
        let y = pos[i * 3 + 1];
        let z = pos[i * 3 + 2];
        let wx = model[0] * x + model[1] * y + model[2] * z + model[3];
        let wy = model[4] * x + model[5] * y + model[6] * z + model[7];
        let wz = model[8] * x + model[9] * y + model[10] * z + model[11];
        world[i * 3] = wx;
        world[i * 3 + 1] = wy;
        world[i * 3 + 2] = wz;
        clip[i * 4] = mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3];
        clip[i * 4 + 1] = mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7];
        clip[i * 4 + 2] = mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11];
        clip[i * 4 + 3] = mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15];
    }
}

/// M = T · Ryaw · Rpitch · Rroll · S (конвенция render3d.lisp).
pub(crate) fn inst_matrix(ins: &[f32], out: &mut [f32; 16]) {
    let (tx, ty, tz) = (ins[0], ins[1], ins[2]);
    let (sx, sy, sz) = (ins[3], ins[4], ins[5]);
    // ВАЖНО: f32::sin_cos возвращает (sin, cos) — не (cos, sin)!
    // (баг мастер-ветки: перепутанная связка ломала матрицу инстанса —
    // оси подменялись, батчи рисовались «сплющенными»)
    let (syw, cy) = ins[6].sin_cos();
    let (sxi, cx) = ins[7].sin_cos();
    let (szi, cz) = ins[8].sin_cos();
    // R = Ry · Rx · Rz
    // Ry·Rx:
    //   [ cy  sy*sx  sy*cx ]
    //   [ 0   cx    -sx    ]
    //   [ -sy cy*sx  cy*cx ]
    let a00 = cy;
    let a01 = syw * sxi;
    let a02 = syw * cx;
    let a10 = 0.0;
    let a11 = cx;
    let a12 = -sxi;
    let a20 = -syw;
    let a21 = cy * sxi;
    let a22 = cy * cx;
    // · Rz:
    //   [ cz -sz 0 ]
    //   [ sz  cz 0 ]
    //   [ 0   0  1 ]
    let r00 = a00 * cz - a01 * szi;
    let r01 = a00 * szi + a01 * cz;
    let r02 = a02;
    let r10 = a10 * cz - a11 * szi;
    let r11 = a10 * szi + a11 * cz;
    let r12 = a12;
    let r20 = a20 * cz - a21 * szi;
    let r21 = a20 * szi + a21 * cz;
    let r22 = a22;
    // · S и T:
    out.fill(0.0);
    out[0] = r00 * sx;
    out[1] = r01 * sy;
    out[2] = r02 * sz;
    out[3] = tx;
    out[4] = r10 * sx;
    out[5] = r11 * sy;
    out[6] = r12 * sz;
    out[7] = ty;
    out[8] = r20 * sx;
    out[9] = r21 * sy;
    out[10] = r22 * sz;
    out[11] = tz;
    out[12] = 0.0;
    out[13] = 0.0;
    out[14] = 0.0;
    out[15] = 1.0;
}

/// Вычисление усредненных нормалей вершин треугольной сетки.
pub(crate) fn compute_normals(pos: &[f32], idx: &[f32]) -> Result<Vec<f32>, String> {
    if pos.len() % 3 != 0 || pos.is_empty() {
        return Err("gpu3d-compute-normals: pos должен быть массивом xyz".into());
    }
    if idx.len() % 3 != 0 {
        return Err("gpu3d-compute-normals: idx должен быть массивом треугольников".into());
    }
    let n = pos.len() / 3;
    let mut nrm = vec![0.0f32; n * 3];
    #[allow(clippy::chunks_exact_to_as_chunks)]
    for tri in idx.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 < n && i1 < n && i2 < n {
            let v0 = [pos[i0 * 3], pos[i0 * 3 + 1], pos[i0 * 3 + 2]];
            let v1 = [pos[i1 * 3], pos[i1 * 3 + 1], pos[i1 * 3 + 2]];
            let v2 = [pos[i2 * 3], pos[i2 * 3 + 1], pos[i2 * 3 + 2]];
            let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
            let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
            let fnrm = cross3(&e1, &e2);
            for &vi in &[i0, i1, i2] {
                nrm[vi * 3] += fnrm[0];
                nrm[vi * 3 + 1] += fnrm[1];
                nrm[vi * 3 + 2] += fnrm[2];
            }
        }
    }
    for i in 0..n {
        let nx = nrm[i * 3];
        let ny = nrm[i * 3 + 1];
        let nz = nrm[i * 3 + 2];
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len > 1e-9 {
            nrm[i * 3] /= len;
            nrm[i * 3 + 1] /= len;
            nrm[i * 3 + 2] /= len;
        } else {
            nrm[i * 3 + 1] = 1.0;
        }
    }
    Ok(nrm)
}

/// Ядро: трансформация вершин + растеризация. Один вызов = один draw call.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mesh_draw(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    vca: Option<&[f32]>, // per-vertex rgba (gpu3d-mesh-vc)
    nrm: Option<&[f32]>, // per-vertex normals (smooth shading)
    idx: &[f32],
    model: &[f32],
    mvp: &[f32],
    r: f64,
    g: f64,
    b: f64,
    texid: i64,
    prim: Prim,
) -> Result<u64, String> {
    // wgpu-режим: 3D-конвейер рисует на GPU, сброс нужен только для софтверного растра
    if !matches!(hal.backend, crate::gpu::Backend::Wgpu(_)) {
        let _ = hal.flush_gpu(true);
    }
    if pos.len() % 3 != 0 || pos.is_empty() {
        return Err("gpu3d: позиции — плоский массив xyz (длина кратна 3)".into());
    }
    let n = pos.len() / 3;
    if let Some(u) = uv {
        if u.len() != n * 2 {
            return Err(format!(
                "gpu3d: uv — пары на вершину ({}), вершин {}",
                u.len(),
                n
            ));
        }
    }
    if let Some(vc) = vca {
        if vc.len() != n * 4 {
            return Err(format!(
                "gpu3d: rgba — четвёрки на вершину ({}), вершин {}",
                vc.len(),
                n
            ));
        }
    }
    if let Some(normals) = nrm {
        if normals.len() != n * 3 {
            return Err(format!(
                "gpu3d: normal — тройки на вершину ({}), вершин {}",
                normals.len(),
                n
            ));
        }
    }
    if let Prim::Wire = prim {
        if idx.len() % 2 != 0 {
            return Err("gpu3d-wire: индексы — пары вершин (длина кратна 2)".into());
        }
    } else if idx.len() % 3 != 0 {
        return Err("gpu3d: индексы — тройки вершин (длина кратна 3)".into());
    }
    if model.len() != 16 || mvp.len() != 16 {
        return Err("gpu3d: матрицы 4x4 = 16 чисел".into());
    }
    let tex = tex_lookup(hal, texid)?;
    if tex.is_some() && uv.is_none() {
        return Err("gpu3d: текстура без uv-массива".into());
    }
    // shadow_pass: запись глубины кастеров тени в shadow_map (depth-only)
    if hal.r3.shadow_pass {
        if matches!(prim, Prim::Wire) {
            return Ok(0);
        }
        let n = pos.len() / 3;
        let mut world = Vec::with_capacity(n * 3);
        let mut world_nrm = Vec::new();
        let mut clip = Vec::with_capacity(n * 4);
        transform_all(pos, None, model, mvp, &mut world, &mut world_nrm, &mut clip);
        let drawn = raster_shadow_tris(&mut hal.r3, &clip, idx)?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    // wgpu-режим: 3D-конвейер рисует на GPU через аппаратный пайплайн
    if let crate::gpu::Backend::Wgpu(gpu) = &mut hal.backend {
        let r3_state = hal.r3.state();
        let drawn = gpu.draw_mesh_3d(
            pos,
            uv,
            vca,
            nrm,
            idx,
            model,
            mvp,
            (r as f32, g as f32, b as f32),
            texid,
            matches!(prim, Prim::Wire),
            &r3_state,
        )?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let Hal { fb, r3, .. } = &mut *hal;

    let mut world = Vec::with_capacity(n * 3);
    let mut world_nrm = Vec::with_capacity(if nrm.is_some() { n * 3 } else { 0 });
    let mut clip = Vec::with_capacity(n * 4);
    transform_all(pos, nrm, model, mvp, &mut world, &mut world_nrm, &mut clip);

    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let mut drawn: u64 = 0;
    let mut culled: u64 = 0;

    if let Prim::Wire = prim {
        // каркас: индексы — ПАРЫ вершин; clip по near-плоскости на сегмент
        for e in 0..idx.len() / 2 {
            let mut seg = [0usize; 2];
            for k in 0..2 {
                let vi = idx[e * 2 + k];
                // через переменную: без !(a >= b) — NaN-семантика прежняя
                let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
                if !idx_ok {
                    return Err(format!("gpu3d: индекс за границей: {}", vi));
                }
                seg[k] = vi as usize;
            }
            let (p, q) = (
                [
                    clip[seg[0] * 4],
                    clip[seg[0] * 4 + 1],
                    clip[seg[0] * 4 + 2],
                    clip[seg[0] * 4 + 3],
                ],
                [
                    clip[seg[1] * 4],
                    clip[seg[1] * 4 + 1],
                    clip[seg[1] * 4 + 2],
                    clip[seg[1] * 4 + 3],
                ],
            );
            let (pw, qw) = (p[3], q[3]);
            if pw <= 1e-6 && qw <= 1e-6 {
                continue;
            }
            let (pa, qa) = if pw < 1e-6 || qw < 1e-6 {
                let t = (1e-6 - pw) / (qw - pw);
                (
                    [
                        p[0] + (q[0] - p[0]) * t,
                        p[1] + (q[1] - p[1]) * t,
                        p[2] + (q[2] - p[2]) * t,
                        1e-6,
                    ],
                    [
                        q[0] + (p[0] - q[0]) * t,
                        q[1] + (p[1] - q[1]) * t,
                        q[2] + (p[2] - q[2]) * t,
                        1e-6,
                    ],
                )
            } else {
                (p, q)
            };
            if let (Some(v0), Some(v1)) = (project(&pa, w, h), project(&qa, w, h)) {
                draw_line(fb, w, h, v0, v1, col(r), col(g), col(b));
                drawn += 1;
            }
        }
    } else {
        raster_tris(
            fb,
            r3,
            w,
            h,
            &clip,
            &world,
            if nrm.is_some() {
                Some(&world_nrm)
            } else {
                None
            },
            uv,
            vca,
            idx,
            None,
            (r as f32, g as f32, b as f32),
            ambient,
            diffuse,
            &light,
            tex.as_ref(),
            false,
            &mut drawn,
            &mut culled,
        )?;
    }
    r3.drawn += drawn;
    r3.culled += culled;
    Ok(drawn)
}

/// Отрисовка каркаса (wireframe) отрезков с клиппингом и растеризацией.
#[allow(clippy::too_many_arguments)]
pub(crate) fn wire_draw(
    hal: &mut Hal,
    pos: &[f32],
    idx: &[f32],
    model: &[f32],
    mvp: &[f32],
    r: f64,
    g: f64,
    b: f64,
) -> Result<u64, String> {
    mesh_draw(
        hal,
        pos,
        None,
        None,
        None,
        idx,
        model,
        mvp,
        r,
        g,
        b,
        0,
        Prim::Wire,
    )
}

/// Батч-вершинник: инстансы разворачиваются в общие world/clip/uv/tint-массивы,
/// индексы смещаются, затем ОДНА растеризация на все инстансы.
/// pub(crate): доступен бенчмарку 3D-шейдинга (main.rs, этап 7).
#[allow(clippy::too_many_arguments)]
pub(crate) fn batch_draw(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    nrm: Option<&[f32]>,
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
    texid: i64,
) -> Result<u64, String> {
    batch_draw_impl(hal, pos, uv, nrm, idx, insts, mvp, texid, false)
}

/// Этап 16: батч с ГОТОВЫМИ матрицами инстансов (скелетная анимация).
/// Формат инстанса: 20 f32 = матрица 4×4 (строчная, аффинная) + r g b a.
pub(crate) fn batch_draw_mat(
    hal: &mut Hal,
    pos: &[f32],
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
) -> Result<u64, String> {
    batch_draw_impl(hal, pos, None, None, idx, insts, mvp, 0, true)
}

#[allow(clippy::too_many_arguments)] // формат батча — 9 параметров по дизайну HAL
fn batch_draw_impl(
    hal: &mut Hal,
    pos: &[f32],
    uv: Option<&[f32]>,
    nrm: Option<&[f32]>,
    idx: &[f32],
    insts: &[f32],
    mvp: &[f32],
    texid: i64,
    mat_mode: bool,
) -> Result<u64, String> {
    // wgpu-режим: сброс нужен только перед программным 3D-батчем
    if !matches!(hal.backend, crate::gpu::Backend::Wgpu(_)) {
        let _ = hal.flush_gpu(true);
    }
    if pos.len() % 3 != 0 || pos.is_empty() {
        return Err("gpu3d-batch: позиции — плоский xyz".into());
    }
    let n = pos.len() / 3;
    if let Some(normals) = nrm {
        if normals.len() != n * 3 {
            return Err(format!(
                "gpu3d-batch: normal — тройки на вершину ({}), вершин {}",
                normals.len(),
                n
            ));
        }
    }
    if idx.len() % 3 != 0 {
        return Err("gpu3d-batch: индексы — тройки вершин".into());
    }
    let stride = if mat_mode { 20 } else { 12 };
    if insts.len() % stride != 0 {
        return Err(if mat_mode {
            "gpu3d-batch-mat: инстансы — по 20 f32 (матрица 4×4 + r g b a)".into()
        } else {
            "gpu3d-batch: инстансы — по 12 f32 (t.xyz s.xyz yaw pitch roll r g b)".into()
        });
    }
    if insts.is_empty() {
        return Ok(0); // пустой батч — ничего не рисуем (врагов пока нет)
    }
    if mvp.len() != 16 {
        return Err("gpu3d-batch: mvp 4x4 = 16 чисел".into());
    }
    let k = insts.len() / stride;
    if k * n * 3 > 8_000_000 {
        return Err("gpu3d-batch: слишком много вершин".into());
    }
    let tex = tex_lookup(hal, texid)?;
    if tex.is_some() && uv.is_none() {
        return Err("gpu3d-batch-tex: текстура без uv".into());
    }
    // shadow_pass: запись глубины батча в shadow_map (depth-only)
    if hal.r3.shadow_pass {
        let mut clip = Vec::with_capacity(k * n * 4);
        let mut idx_all: Vec<f32> = Vec::with_capacity(k * idx.len());
        let mut m: [f32; 16] = [0.0; 16];
        for i in 0..k {
            let ins = &insts[i * stride..(i + 1) * stride];
            if mat_mode {
                m.copy_from_slice(&ins[0..16]);
            } else {
                inst_matrix(ins, &mut m);
            }
            let base = i * n;
            for vi in 0..n {
                let x = pos[vi * 3];
                let y = pos[vi * 3 + 1];
                let z = pos[vi * 3 + 2];
                let wx = m[0] * x + m[1] * y + m[2] * z + m[3];
                let wy = m[4] * x + m[5] * y + m[6] * z + m[7];
                let wz = m[8] * x + m[9] * y + m[10] * z + m[11];
                clip.push(mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3]);
                clip.push(mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7]);
                clip.push(mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11]);
                clip.push(mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15]);
            }
            for &j in idx {
                idx_all.push(j + base as f32);
            }
        }
        let drawn = raster_shadow_tris(&mut hal.r3, &clip, &idx_all)?;
        hal.r3.drawn += drawn;
        return Ok(drawn);
    }
    // wgpu-режим: батч на GPU через аппаратный instancing (только стандартные 12-f32 инстансы)
    if !mat_mode {
        if let crate::gpu::Backend::Wgpu(gpu) = &mut hal.backend {
            let r3_state = hal.r3.state();
            let drawn = gpu.draw_batch_3d(pos, uv, nrm, idx, insts, mvp, texid, &r3_state)?;
            hal.r3.drawn += drawn;
            return Ok(drawn);
        }
    }
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let Hal { fb, r3, .. } = &mut *hal;

    // единые буферы на все инстансы
    let mut world = Vec::with_capacity(k * n * 3);
    let mut world_nrm = Vec::with_capacity(if nrm.is_some() { k * n * 3 } else { 0 });
    let mut clip = Vec::with_capacity(k * n * 4);
    let mut uvq: Vec<f32> = Vec::with_capacity(k * n * 2);
    let mut tint: Vec<f32> = Vec::with_capacity(k * n * 3);
    let mut idx_all: Vec<f32> = Vec::with_capacity(k * idx.len());
    let mut m: [f32; 16] = [0.0; 16];

    for i in 0..k {
        let ins = &insts[i * stride..(i + 1) * stride];
        if mat_mode {
            m.copy_from_slice(&ins[0..16]);
        } else {
            inst_matrix(ins, &mut m);
        }
        let base = i * n;
        for vi in 0..n {
            let x = pos[vi * 3];
            let y = pos[vi * 3 + 1];
            let z = pos[vi * 3 + 2];
            let wx = m[0] * x + m[1] * y + m[2] * z + m[3];
            let wy = m[4] * x + m[5] * y + m[6] * z + m[7];
            let wz = m[8] * x + m[9] * y + m[10] * z + m[11];
            world.push(wx);
            world.push(wy);
            world.push(wz);
            if let Some(normals) = nrm {
                let nx = normals[vi * 3];
                let ny = normals[vi * 3 + 1];
                let nz = normals[vi * 3 + 2];
                let wnx = m[0] * nx + m[1] * ny + m[2] * nz;
                let wny = m[4] * nx + m[5] * ny + m[6] * nz;
                let wnz = m[8] * nx + m[9] * ny + m[10] * nz;
                let l = (wnx * wnx + wny * wny + wnz * wnz).sqrt().max(1e-9);
                world_nrm.push(wnx / l);
                world_nrm.push(wny / l);
                world_nrm.push(wnz / l);
            }
            clip.push(mvp[0] * wx + mvp[1] * wy + mvp[2] * wz + mvp[3]);
            clip.push(mvp[4] * wx + mvp[5] * wy + mvp[6] * wz + mvp[7]);
            clip.push(mvp[8] * wx + mvp[9] * wy + mvp[10] * wz + mvp[11]);
            clip.push(mvp[12] * wx + mvp[13] * wy + mvp[14] * wz + mvp[15]);
            if let Some(u) = uv {
                uvq.push(u[vi * 2]);
                uvq.push(u[vi * 2 + 1]);
            }
            if mat_mode {
                tint.push(ins[16]);
                tint.push(ins[17]);
                tint.push(ins[18]);
            } else {
                tint.push(ins[9]);
                tint.push(ins[10]);
                tint.push(ins[11]);
            }
        }
        for &j in idx {
            idx_all.push(j + base as f32);
        }
    }

    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let mut drawn: u64 = 0;
    let mut culled: u64 = 0;
    raster_tris(
        fb,
        r3,
        w,
        h,
        &clip,
        &world,
        if nrm.is_some() {
            Some(&world_nrm)
        } else {
            None
        },
        if uv.is_some() { Some(&uvq) } else { None },
        None,
        &idx_all,
        Some(&tint),
        (0.0, 0.0, 0.0),
        ambient,
        diffuse,
        &light,
        tex.as_ref(),
        false,
        &mut drawn,
        &mut culled,
    )?;
    r3.drawn += drawn;
    r3.culled += culled;
    Ok(drawn)
}
