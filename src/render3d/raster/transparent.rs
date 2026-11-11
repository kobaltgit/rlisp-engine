// src/render3d/raster/transparent.rs — Прозрачный проход (alpha-blending, far→near сортировка, z-тест без z-записи).

use super::super::shading::*;
use super::super::types::*;
use super::tex::*;
use crate::hal::Hal;

/// Прозрачный проход (этап 9): сортируем очередь far→near по видовой глубине
/// центра и рисуем с src-alpha блендингом (z-тест есть, z-записи нет,
/// двусторонне). Порядок в очереди задаёт художник-видимость для крон, воды
/// и облаков — как у настоящего GPU с painter's sort по треугольникам.
pub(crate) fn flush_transparent(hal: &mut Hal) -> Result<u64, String> {
    if hal.r3.transq.is_empty() {
        return Ok(0);
    }
    let _ = hal.flush_gpu(true);
    let (w, h) = (hal.w, hal.h);
    if hal.r3.zbuf.len() != w * h {
        hal.r3.reset(w, h);
    }
    let mut q = std::mem::take(&mut hal.r3.transq);
    q.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // текстуры резолвим один раз на id (клон px дорог) — до разбора hal
    let mut texcache: Vec<(i64, TexRef)> = Vec::new();
    {
        let mut ids: Vec<i64> = q.iter().map(|t| t.tex).collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            if id != 0 {
                texcache.push((
                    id,
                    tex_lookup(hal, id)?
                        .ok_or_else(|| format!("gpu3d-flush-alpha: нет текстуры {}", id))?,
                ));
            }
        }
    }
    let mut drawn: u64 = 0;
    let Hal { fb, r3, .. } = &mut *hal;
    fog_lut_sync(r3);
    for t in &q {
        let tex = texcache
            .iter()
            .find(|(id, _)| *id == t.tex)
            .map(|(_, tr)| tr);
        blend_tri(fb, r3, w, h, t, tex, &mut drawn)?;
    }
    r3.drawn += drawn;
    Ok(drawn)
}

/// Один треугольник прозрачного прохода: блендинг, z-тест без z-записи.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_tri(
    fb: &mut [u8],
    r3: &mut R3,
    w: usize,
    h: usize,
    t: &TransTri,
    tex: Option<&TexRef>,
    drawn: &mut u64,
) -> Result<(), String> {
    let light = {
        let l = r3.light;
        let len = dot3(&l, &l).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let (ambient, diffuse) = (r3.ambient, r3.diffuse);
    let spec_on = t.spec > 0.0;
    let fog_on = t.fog[3] > 0.0;
    let (fog_density, fog_r, fog_g, fog_b) = (t.fog[3], t.fog[0], t.fog[1], t.fog[2]);
    let (tu, tv) = (r3.tiling[0], r3.tiling[1]);
    let nnrm = t.nrm;
    let nl_ok = dot3(&nnrm, &nnrm) > 0.5;
    let _shade = if !nl_ok {
        ambient
    } else {
        let ndl = dot3(&nnrm, &light).abs(); // двустороннее освещение
        (ambient + diffuse * ndl.max(0.0)).clamp(0.0, 1.0)
    };
    let (p0, p1, p2) = (&t.pts[0], &t.pts[1], &t.pts[2]);
    let has_smooth = r3.smooth && (p0.nxq.abs() + p0.nyq.abs() + p0.nzq.abs() > 1e-6);
    let (tw, th) = tex.map(|x| (x.w, x.h)).unwrap_or((0, 0));

    let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
    let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(w as i64 - 1);
    let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
    let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(h as i64 - 1);
    if minx > maxx || miny > maxy {
        return Ok(());
    }
    let tri_area_2 = edge(p1.sx, p1.sy, p2.sx, p2.sy, p0.sx, p0.sy);
    let tex_grad = if tex.is_some() && t.mipmaps {
        TexGradients::compute(p0, p1, p2, tri_area_2, tw, th, tu, tv)
    } else {
        None
    };
    for py in miny..=maxy {
        let row = py as usize * w;
        let fy = py as f32 + 0.5;
        for px in minx..=maxx {
            let fx = px as f32 + 0.5;
            let w0 = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy);
            let w1 = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy);
            let w2 = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
            // двусторонне: внутри полигона веса одного знака
            if (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) || (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) {
                let sum = w0 + w1 + w2;
                if sum.abs() < 1e-9 {
                    continue;
                }
                let z = (w0 * p0.sz + w1 * p1.sz + w2 * p2.sz) / sum;
                let pi = row + px as usize;
                if z < r3.zbuf[pi] {
                    let iwp = (w0 * p0.iw + w1 * p1.iw + w2 * p2.iw) / sum;
                    if iwp.abs() <= 1e-12 {
                        continue;
                    }
                    let (pix_nnrm, pix_ndl) = if has_smooth {
                        let nx = ((w0 * p0.nxq + w1 * p1.nxq + w2 * p2.nxq) / sum) / iwp;
                        let ny = ((w0 * p0.nyq + w1 * p1.nyq + w2 * p2.nyq) / sum) / iwp;
                        let nz = ((w0 * p0.nzq + w1 * p1.nzq + w2 * p2.nzq) / sum) / iwp;
                        let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
                        if nlen < 1e-9 {
                            (
                                nnrm,
                                if nl_ok {
                                    dot3(&nnrm, &light).abs()
                                } else {
                                    0.0
                                },
                            )
                        } else {
                            let inv_nlen = 1.0 / nlen;
                            let norm = [nx * inv_nlen, ny * inv_nlen, nz * inv_nlen];
                            let ndl = dot3(&norm, &light).abs();
                            (norm, ndl)
                        }
                    } else {
                        (
                            nnrm,
                            if nl_ok {
                                dot3(&nnrm, &light).abs()
                            } else {
                                0.0
                            },
                        )
                    };
                    let need_world_pos =
                        spec_on || r3.shadow_enabled || !r3.point_lights.is_empty();
                    let (wx, wy, wz) = if need_world_pos {
                        (
                            ((w0 * p0.wxq + w1 * p1.wxq + w2 * p2.wxq) / sum) / iwp,
                            ((w0 * p0.wyq + w1 * p1.wyq + w2 * p2.wyq) / sum) / iwp,
                            ((w0 * p0.wzq + w1 * p1.wzq + w2 * p2.wzq) / sum) / iwp,
                        )
                    } else {
                        (0.0, 0.0, 0.0)
                    };
                    let shadow_factor = if r3.shadow_enabled {
                        shadow_sample(r3, wx, wy, wz, r3.shadow_bias)
                    } else {
                        1.0
                    };
                    let dir_diffuse = diffuse * pix_ndl.max(0.0) * shadow_factor;
                    let dir_shade = (ambient + dir_diffuse).clamp(0.0, 1.0);
                    let s = if spec_on {
                        let v = [r3.view[0] - wx, r3.view[1] - wy, r3.view[2] - wz];
                        let vl = dot3(&v, &v).sqrt().max(1e-9);
                        let hv = [
                            light[0] + v[0] / vl,
                            light[1] + v[1] / vl,
                            light[2] + v[2] / vl,
                        ];
                        let hl = dot3(&hv, &hv).sqrt().max(1e-9);
                        spec_highlight(
                            dot3(&pix_nnrm, &[hv[0] / hl, hv[1] / hl, hv[2] / hl]),
                            t.shininess,
                            t.spec,
                        ) * shadow_factor
                    } else {
                        0.0
                    };
                    let (pt_diff, pt_spec) = compute_point_lights(
                        &r3.point_lights,
                        &r3.view,
                        wx,
                        wy,
                        wz,
                        &pix_nnrm,
                        true,
                        spec_on,
                        t.shininess,
                        t.spec,
                    );
                    let fogf = if fog_on {
                        fog_factor(fog_density, 1.0 / iwp)
                    } else {
                        0.0
                    };
                    // per-vertex rgb/alpha (градиент воды/листвы) — всегда
                    // интерполяция: у mesh-vc цвет живёт в вершинах, тинт
                    // для прозрачного прохода не используется
                    let f = 1.0 / iwp;
                    let rq = ((w0 * p0.rq + w1 * p1.rq + w2 * p2.rq) / sum) * f;
                    let gq = ((w0 * p0.gq + w1 * p1.gq + w2 * p2.gq) / sum) * f;
                    let bq = ((w0 * p0.bq + w1 * p1.bq + w2 * p2.bq) / sum) * f;
                    let (mut c0, mut c1, mut c2) = (
                        rq * dir_shade + rq * pt_diff[0],
                        gq * dir_shade + gq * pt_diff[1],
                        bq * dir_shade + bq * pt_diff[2],
                    );
                    let mut al =
                        (((w0 * p0.aq + w1 * p1.aq + w2 * p2.aq) / sum) * f).clamp(0.0, 1.0);
                    if let Some(tx) = tex {
                        let uq = (w0 * p0.uq + w1 * p1.uq + w2 * p2.uq) / sum;
                        let vq = (w0 * p0.vq + w1 * p1.vq + w2 * p2.vq) / sum;
                        let uf = tiled_coord(uq / iwp, tu);
                        let vf = tiled_coord(vq / iwp, tv);
                        let texel = sample_tex(
                            tx,
                            t.filter,
                            t.mipmaps,
                            tex_grad.as_ref(),
                            uq,
                            vq,
                            iwp,
                            uf,
                            vf,
                        );
                        al *= texel[3] as f32 / 255.0;
                        if al <= 0.004 {
                            continue;
                        }
                        c0 *= texel[0] as f32 / 255.0;
                        c1 *= texel[1] as f32 / 255.0;
                        c2 *= texel[2] as f32 / 255.0;
                    } else if al <= 0.004 {
                        continue;
                    }
                    if spec_on || pt_spec[0] > 0.0 || pt_spec[1] > 0.0 || pt_spec[2] > 0.0 {
                        c0 = (c0 + s + pt_spec[0]).min(1.0);
                        c1 = (c1 + s + pt_spec[1]).min(1.0);
                        c2 = (c2 + s + pt_spec[2]).min(1.0);
                    }
                    if fog_on {
                        let g = 1.0 - fogf;
                        c0 = c0 * g + fog_r * fogf;
                        c1 = c1 * g + fog_g * fogf;
                        c2 = c2 * g + fog_b * fogf;
                    }
                    let o = pi * 4;
                    let dr = fb[o] as f32 / 255.0;
                    let dg = fb[o + 1] as f32 / 255.0;
                    let db = fb[o + 2] as f32 / 255.0;
                    let iv = 1.0 - al;
                    fb[o] = col((c0 * al + dr * iv) as f64);
                    fb[o + 1] = col((c1 * al + dg * iv) as f64);
                    fb[o + 2] = col((c2 * al + db * iv) as f64);
                    fb[o + 3] = 255;
                }
            }
        }
    }
    *drawn += 1;
    Ok(())
}
