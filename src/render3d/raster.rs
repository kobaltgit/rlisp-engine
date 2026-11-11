// src/render3d/raster.rs — Растеризация треугольников и линий, семплинг текстур и z-буфер.

pub mod tex;
pub mod transparent;
pub(crate) use tex::*;
pub(crate) use transparent::*;
use super::shading::*;
use super::types::*;

#[allow(dead_code)]
pub(crate) type RasterPoint = Vtx;
#[allow(dead_code)]
pub(crate) type AlphaTri = TransTri;
#[allow(unused_imports)]
pub(crate) use super::shading::clip_near6 as clip_poly;
#[allow(unused_imports)]
pub(crate) use transparent::flush_transparent as flush_alpha;
#[allow(unused_imports)]
pub(crate) use raster_tris as raster_poly;

pub(crate) fn raster_shadow_tris(r3: &mut R3, clip: &[f32], idx: &[f32]) -> Result<u64, String> {
    let sw = r3.shadow_w;
    let sh = r3.shadow_h;
    if sw == 0 || sh == 0 || r3.shadow_map.len() != sw * sh {
        return Ok(0);
    }
    let n = clip.len() / 4;
    let mut poly_buf: Vec<[f32; 4]> = Vec::with_capacity(8);
    let mut clip_buf: Vec<[f32; 4]> = Vec::with_capacity(8);
    let mut drawn = 0u64;

    for f in 0..idx.len() / 3 {
        let mut tri = [0usize; 3];
        for k in 0..3 {
            let vi = idx[f * 3 + k];
            let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
            if !idx_ok {
                return Err(format!("gpu3d-shadow: индекс за границей: {}", vi));
            }
            tri[k] = vi as usize;
        }

        poly_buf.clear();
        for &t in &tri {
            poly_buf.push([
                clip[t * 4],
                clip[t * 4 + 1],
                clip[t * 4 + 2],
                clip[t * 4 + 3],
            ]);
        }

        let poly: &[[f32; 4]] = if poly_buf.iter().any(|v| v[3] < 1e-6) {
            clip_near_pos(&poly_buf, &mut clip_buf);
            if clip_buf.len() < 3 {
                continue;
            }
            &clip_buf
        } else {
            &poly_buf
        };

        let mut pts = Vec::with_capacity(poly.len());
        for v in poly {
            if v[3] <= 1e-6 {
                continue;
            }
            let inv_w = 1.0 / v[3];
            let nx = v[0] * inv_w;
            let ny = v[1] * inv_w;
            let nz = v[2] * inv_w;
            pts.push((
                (nx * 0.5 + 0.5) * sw as f32,
                (1.0 - (ny * 0.5 + 0.5)) * sh as f32,
                nz,
            ));
        }
        if pts.len() < 3 {
            continue;
        }

        for kk in 1..pts.len() - 1 {
            let (p0, p1, p2) = (&pts[0], &pts[kk], &pts[kk + 1]);
            let area = edge(p0.0, p0.1, p1.0, p1.1, p2.0, p2.1);
            if area.abs() < 1e-6 {
                continue;
            }

            let minx = (p0.0.min(p1.0).min(p2.0).floor() as i64).max(0);
            let maxx = (p0.0.max(p1.0).max(p2.0).ceil() as i64).min(sw as i64 - 1);
            let miny = (p0.1.min(p1.1).min(p2.1).floor() as i64).max(0);
            let maxy = (p0.1.max(p1.1).max(p2.1).ceil() as i64).min(sh as i64 - 1);
            if minx > maxx || miny > maxy {
                continue;
            }

            for py in miny..=maxy {
                let row = py as usize * sw;
                let fy = py as f32 + 0.5;
                for px in minx..=maxx {
                    let fx = px as f32 + 0.5;
                    let w0 = edge(p1.0, p1.1, p2.0, p2.1, fx, fy);
                    let w1 = edge(p2.0, p2.1, p0.0, p0.1, fx, fy);
                    let w2 = edge(p0.0, p0.1, p1.0, p1.1, fx, fy);
                    if (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
                        || (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0)
                    {
                        let sum = w0 + w1 + w2;
                        let z = if sum.abs() < 1e-9 {
                            p0.2
                        } else {
                            (w0 * p0.2 + w1 * p1.2 + w2 * p2.2) / sum
                        };
                        let pi = row + px as usize;
                        if z < r3.shadow_map[pi] {
                            r3.shadow_map[pi] = z;
                        }
                    }
                }
            }
            drawn += 1;
        }
    }
    Ok(drawn)
}

pub(crate) fn project6(c: &[f32; 17], w: usize, h: usize) -> Option<Vtx> {
    if c[3] <= 1e-6 {
        return None;
    }
    let nx = c[0] / c[3];
    let ny = c[1] / c[3];
    let nz = c[2] / c[3];
    Some(Vtx {
        sx: (nx * 0.5 + 0.5) * w as f32,
        sy: (1.0 - (ny * 0.5 + 0.5)) * h as f32, // y вниз, как в фреймбуфере
        sz: nz,
        uq: c[4],  // u/w
        vq: c[5],  // v/w
        iw: c[6],  // 1/w
        wxq: c[7], // мир/w
        wyq: c[8],
        wzq: c[9],
        aq: c[10],
        rq: c[11],
        gq: c[12],
        bq: c[13],
        nxq: c[14],
        nyq: c[15],
        nzq: c[16],
    })
}

pub(crate) fn project(c: &[f32; 4], w: usize, h: usize) -> Option<Vtx> {
    if c[3] <= 1e-6 {
        return None;
    }
    let inv = 1.0 / c[3];
    let nx = c[0] * inv;
    let ny = c[1] * inv;
    let nz = c[2] * inv;
    Some(Vtx {
        sx: (nx * 0.5 + 0.5) * w as f32,
        sy: (1.0 - (ny * 0.5 + 0.5)) * h as f32,
        sz: nz,
        uq: 0.0,
        vq: 0.0,
        iw: inv,
        wxq: 0.0,
        wyq: 0.0,
        wzq: 0.0,
        aq: inv,
        rq: 0.0,
        gq: 0.0,
        bq: 0.0,
        nxq: 0.0,
        nyq: 0.0,
        nzq: 0.0,
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Prim {
    Tri,
    Wire,
}



#[allow(clippy::too_many_arguments)]
pub(crate) fn raster_tris(
    fb: &mut [u8],
    r3: &mut R3,
    w: usize,
    h: usize,
    clip: &[f32],
    world: &[f32],
    world_nrm: Option<&[f32]>,
    uvq: Option<&[f32]>,
    vca: Option<&[f32]>, // per-vertex rgba (gpu3d-mesh-vc): n*4 f32
    idx: &[f32],
    tint: Option<&[f32]>,   // rgb на вершину (батч: цвет инстанса)
    solid: (f32, f32, f32), // один цвет (обычный mesh)
    ambient: f32,
    diffuse: f32,
    light: &[f32; 3],
    tex: Option<&TexRef>,
    blend_pass: bool, // true = прозрачный проход (блендинг, без z-записи)
    drawn: &mut u64,
    culled: &mut u64,
) -> Result<(), String> {
    let n = world.len() / 3;
    // LUT тумана актуализируется раз на draw-вызов (fog меняется и напрямую
    // из тестов, и через gpu3d-fog — снимок-ключ ловит любое изменение)
    fog_lut_sync(r3);
    // скретч-буферы переиспользуются всеми треугольниками (без аллокаций
    // на треугольник в горячем цикле — цена тесселяции ревизии 4)
    let mut poly_buf: Vec<[f32; 17]> = Vec::with_capacity(8);
    let mut clip_buf: Vec<[f32; 17]> = Vec::with_capacity(8);
    let mut pts_buf: Vec<Vtx> = Vec::with_capacity(8);
    // кэш проекций: вершина регулярной сетки переиспользуется до 4 квад,
    // project6 с перспективным делением — самая дорогая часть цикла
    let mut vcache: Vec<Option<Vtx>> = vec![None; n];
    for f in 0..idx.len() / 3 {
        let mut tri = [0usize; 3];
        for k in 0..3 {
            let vi = idx[f * 3 + k];
            // через переменную: без !(a >= b) — NaN-семантика прежняя
            let idx_ok = vi >= 0.0 && vi.fract() == 0.0 && (vi as usize) < n;
            if !idx_ok {
                return Err(format!("gpu3d: индекс за границей: {}", vi));
            }
            tri[k] = vi as usize;
        }
        // полигон в клип-пространстве + атрибуты u/w, v/w, 1/w, мир/w, a/w, rgb/w, nx/w, ny/w, nz/w
        let blend_base = if r3.blend > 0.0 {
            r3.blend.min(1.0)
        } else {
            1.0
        };
        let base_rgb = match tint {
            Some(t) => [t[tri[0] * 3], t[tri[0] * 3 + 1], t[tri[0] * 3 + 2]],
            None => [solid.0, solid.1, solid.2],
        };
        poly_buf.clear();
        for &t in &tri {
            let cw = clip[t * 4 + 3];
            let iw = if cw > 1e-6 { 1.0 / cw } else { 0.0 };
            // перспективно-корректные атрибуты: в полигон идут u/w, v/w,
            // мировая позиция /w, альфа/w, rgb/w и нормали/w (аффинны на ребре)
            let (uq, vq) = if let Some(u) = uvq {
                (u[t * 2] * iw, u[t * 2 + 1] * iw)
            } else {
                (0.0, 0.0)
            };
            let (a, rr, gg, bb) = if let Some(vc) = vca {
                (vc[t * 4 + 3], vc[t * 4], vc[t * 4 + 1], vc[t * 4 + 2])
            } else {
                (blend_base, base_rgb[0], base_rgb[1], base_rgb[2])
            };
            let (nxq, nyq, nzq) = if let Some(wn) = world_nrm {
                (wn[t * 3] * iw, wn[t * 3 + 1] * iw, wn[t * 3 + 2] * iw)
            } else {
                (0.0, 0.0, 0.0)
            };
            poly_buf.push([
                clip[t * 4],
                clip[t * 4 + 1],
                clip[t * 4 + 2],
                cw,
                uq,
                vq,
                iw,
                world[t * 3] * iw,
                world[t * 3 + 1] * iw,
                world[t * 3 + 2] * iw,
                a * iw,
                rr * iw,
                gg * iw,
                bb * iw,
                nxq,
                nyq,
                nzq,
            ]);
        }

        let poly: &[[f32; 17]] = if poly_buf.iter().any(|v| v[3] < 1e-6) {
            clip_near6(&poly_buf, &mut clip_buf);
            if clip_buf.len() < 3 {
                *culled += 1;
                continue;
            }
            &clip_buf
        } else {
            &poly_buf
        };

        let pts = &mut pts_buf;
        pts.clear();
        let mut ok = true;
        if poly.len() == 3 {
            // без клипа: проекции берём из кэша вершины (порядок tri[])
            for (k, &vi) in tri.iter().enumerate() {
                let c = &mut vcache[vi];
                if c.is_none() {
                    *c = project6(&poly[k], w, h);
                }
                match c {
                    Some(p) => pts.push(*p),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        } else {
            for v in poly {
                match project6(v, w, h) {
                    Some(p) => pts.push(p),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            *culled += 1;
            continue;
        }

        // — состояние материалов (этап 7): выключенные фичи — ноль расходов —
        let spec_on = r3.spec > 0.0;
        let (spec, shininess) = (r3.spec, r3.shininess);
        let fog_on = r3.fog[3] > 0.0;
        let (fog_density, fog_r, fog_g, fog_b) = (r3.fog[3], r3.fog[0], r3.fog[1], r3.fog[2]);
        let fog_snap = r3.fog;
        let alpha_on = r3.alpha_test > 0.0;
        let alpha_thr = r3.alpha_test;
        let smooth_shading = r3.smooth && world_nrm.is_some();
        let (tu, tv) = (r3.tiling[0], r3.tiling[1]);

        // LUT тумана — локальная копия (без займа r3 в горячем цикле)
        let lut = r3.fog_lut;
        // вентилятор: (0, k, k+1)
        for kk in 1..pts.len() - 1 {
            let (p0, p1, p2) = (&pts[0], &pts[kk], &pts[kk + 1]);
            // backface cull: фронт = CCW в NDC → после y-flip площадь < 0.
            // Куллинг ПОЗЖЕ — прозрачные грани двусторонни, задние тоже
            // должны попасть в прозрачную очередь (решение ниже).
            let area = edge(p0.sx, p0.sy, p1.sx, p1.sy, p2.sx, p2.sy);
            // нормаль грани в мире (flat shading)
            let (wa, wb, wc) = (
                [
                    world[tri[0] * 3],
                    world[tri[0] * 3 + 1],
                    world[tri[0] * 3 + 2],
                ],
                [
                    world[tri[1] * 3],
                    world[tri[1] * 3 + 1],
                    world[tri[1] * 3 + 2],
                ],
                [
                    world[tri[2] * 3],
                    world[tri[2] * 3 + 1],
                    world[tri[2] * 3 + 2],
                ],
            );
            let e1 = [wb[0] - wa[0], wb[1] - wa[1], wb[2] - wa[2]];
            let e2 = [wc[0] - wa[0], wc[1] - wa[1], wc[2] - wa[2]];
            let nrm = cross3(&e1, &e2);
            let nl = dot3(&nrm, &nrm).sqrt();
            let nnrm = if nl < 1e-9 {
                [0.0f32; 3]
            } else {
                [nrm[0] / nl, nrm[1] / nl, nrm[2] / nl]
            };
            // прозрачный проход двусторонний: нормаль разворачиваем к свету
            let _shade = if nl < 1e-9 {
                ambient
            } else {
                let mut ndl = dot3(&nnrm, light);
                if blend_pass && ndl < 0.0 {
                    ndl = -ndl;
                }
                (ambient + diffuse * ndl.max(0.0)).clamp(0.0, 1.0)
            };

            // — прозрачная очередь (этап 9): альфа треугольника = средняя
            // перспектива-корректная альфа вершин; < 1 → ставим в очередь
            // (сорт far→near и блендинг — в gpu3d-flush-alpha) —
            let va = [
                if p0.iw > 1e-12 { p0.aq / p0.iw } else { 1.0 },
                if p1.iw > 1e-12 { p1.aq / p1.iw } else { 1.0 },
                if p2.iw > 1e-12 { p2.aq / p2.iw } else { 1.0 },
            ];
            let al_avg = (va[0] + va[1] + va[2]) / 3.0;
            if !blend_pass && al_avg < 0.999 {
                let depth = 1.0 / 3.0
                    * (if p0.iw > 1e-12 { 1.0 / p0.iw } else { 0.0 }
                        + if p1.iw > 1e-12 { 1.0 / p1.iw } else { 0.0 }
                        + if p2.iw > 1e-12 { 1.0 / p2.iw } else { 0.0 });
                r3.transq.push(TransTri {
                    depth,
                    pts: [*p0, *p1, *p2],
                    nrm: nnrm,
                    tex: tex.map(|t| t.id).unwrap_or(0),
                    spec,
                    shininess,
                    fog: fog_snap,
                    filter: r3.tex_filter,
                    mipmaps: r3.tex_mipmaps,
                });
                continue;
            }
            // непрозрачный проход: задняя грань — кулим (прозрачные прошли выше)
            if !blend_pass && area >= -1e-6 {
                *culled += 1;
                continue;
            }
            // цвет грани: per-vertex RGB (вода/небо) или тинт первой вершины
            let vc_rgb = !blend_pass
                && p0.rq == p1.rq
                && p0.rq == p2.rq
                && p0.gq == p1.gq
                && p0.gq == p2.gq
                && p0.bq == p1.bq
                && p0.bq == p2.bq;
            let (tr, tg, tb) = if vc_rgb {
                (p0.rq / p0.iw, p0.gq / p0.iw, p0.bq / p0.iw)
            } else {
                match tint {
                    Some(t) => (t[tri[0] * 3], t[tri[0] * 3 + 1], t[tri[0] * 3 + 2]),
                    None => solid,
                }
            };

            let (tw, th) = tex.map(|t| (t.w, t.h)).unwrap_or((0, 0));

            // bbox
            let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
            let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(w as i64 - 1);
            let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
            let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(h as i64 - 1);
            if minx > maxx || miny > maxy {
                continue;
            }
            let tri_area_2 = edge(p1.sx, p1.sy, p2.sx, p2.sy, p0.sx, p0.sy);
            let tex_grad = if tex.is_some() && r3.tex_mipmaps {
                TexGradients::compute(p0, p1, p2, tri_area_2, tw, th, tu, tv)
            } else {
                None
            };

            // — инкрементальный сканлайн (этап 9.2): вместо перебора bbox с
            // тремя edge-функциями на каждый пиксель — спан по строке из
            // полулиний wi ≤ 0 и пошаговые атрибуты: числители N_q = Σ wi·qi
            // аффинны вдоль строки вместе с wi, знаменатель Σ wi инвариантен
            // (площадь) — пиксель = одно деление вместо семи —
            let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
            const A_SZ: usize = 0;
            const A_IW: usize = 1;
            const A_U: usize = 2;
            const A_V: usize = 3;
            const A_WX: usize = 4;
            const A_WY: usize = 5;
            const A_WZ: usize = 6;
            const A_R: usize = 7;
            const A_G: usize = 8;
            const A_B: usize = 9;
            const A_A: usize = 10;
            const A_NX: usize = 11;
            const A_NY: usize = 12;
            const A_NZ: usize = 13;
            const NA: usize = 14;
            let q: [[f32; 3]; NA] = [
                [p0.sz, p1.sz, p2.sz],
                [p0.iw, p1.iw, p2.iw],
                [p0.uq, p1.uq, p2.uq],
                [p0.vq, p1.vq, p2.vq],
                [p0.wxq, p1.wxq, p2.wxq],
                [p0.wyq, p1.wyq, p2.wyq],
                [p0.wzq, p1.wzq, p2.wzq],
                [p0.rq, p1.rq, p2.rq],
                [p0.gq, p1.gq, p2.gq],
                [p0.bq, p1.bq, p2.bq],
                [p0.aq, p1.aq, p2.aq],
                [p0.nxq, p1.nxq, p2.nxq],
                [p0.nyq, p1.nyq, p2.nyq],
                [p0.nzq, p1.nzq, p2.nzq],
            ];
            for py in miny..=maxy {
                let fy = py as f32 + 0.5;
                let fx0 = minx as f32 + 0.5;
                // edge-функции строки в опорной точке (левый край bbox)
                let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
                let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
                let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
                let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
                if from > to {
                    continue;
                }
                let inv_d = 1.0 / (w0r + w1r + w2r);
                let wr = [w0r, w1r, w2r];
                let mut n = [0.0f32; NA];
                let mut dn = [0.0f32; NA];
                for a in 0..NA {
                    n[a] = wr[0] * q[a][0] + wr[1] * q[a][1] + wr[2] * q[a][2];
                    dn[a] = ax[0] * q[a][0] + ax[1] * q[a][1] + ax[2] * q[a][2];
                }
                let skip = (from - minx) as f32;
                if skip != 0.0 {
                    for a in 0..NA {
                        n[a] += dn[a] * skip;
                    }
                }
                let row = py as usize * w;
                for px in from..=to {
                    // z-тест ДО атрибутов: окклюдированный пиксель платит
                    // только за z, шаг и continue (шаг обязателен — следующий
                    // пиксель строки требует шагнувших числителей)
                    let z = n[A_SZ] * inv_d;
                    let pi = row + px as usize;
                    if z >= r3.zbuf[pi] {
                        for a in 0..NA {
                            n[a] += dn[a];
                        }
                        continue;
                    }
                    // видимый: значения ЭТОГО пикселя — чтение ДО шага
                    // (без «отката» n−dn: на клипнутых вершинах атрибуты
                    // ~1e4-1e5, вычитание близких чисел топит точность f32)
                    let iwp = n[A_IW] * inv_d;
                    let uq = n[A_U] * inv_d;
                    let vq = n[A_V] * inv_d;
                    let wxq = n[A_WX] * inv_d;
                    let wyq = n[A_WY] * inv_d;
                    let wzq = n[A_WZ] * inv_d;
                    let rq = n[A_R] * inv_d;
                    let gq = n[A_G] * inv_d;
                    let bq = n[A_B] * inv_d;
                    let alq = n[A_A] * inv_d;
                    let nxq = n[A_NX] * inv_d;
                    let nyq = n[A_NY] * inv_d;
                    let nzq = n[A_NZ] * inv_d;
                    for a in 0..NA {
                        n[a] += dn[a];
                    }
                    if iwp.abs() <= 1e-12 {
                        // деградированный iw: z пишется (opaque), цвет нет
                        if !blend_pass {
                            r3.zbuf[pi] = z;
                        }
                        continue;
                    }
                    // истинная глубина = 1/(1/w) = w_clip — она же глубина тумана
                    let f = 1.0 / iwp;

                    // сглаженные нормали и шейдинг Фонга (этап: per-pixel normal)
                    let (pix_nnrm, pix_ndl) = if smooth_shading {
                        let nx = nxq * f;
                        let ny = nyq * f;
                        let nz = nzq * f;
                        let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
                        if nlen < 1e-9 {
                            (nnrm, if nl < 1e-9 { 0.0 } else { dot3(&nnrm, light) })
                        } else {
                            let inv_nlen = 1.0 / nlen;
                            let norm = [nx * inv_nlen, ny * inv_nlen, nz * inv_nlen];
                            let mut ndl = dot3(&norm, light);
                            if blend_pass && ndl < 0.0 {
                                ndl = -ndl;
                            }
                            (norm, ndl)
                        }
                    } else {
                        let mut ndl = if nl < 1e-9 { 0.0 } else { dot3(&nnrm, light) };
                        if blend_pass && ndl < 0.0 {
                            ndl = -ndl;
                        }
                        (nnrm, ndl)
                    };

                    let need_world_pos =
                        spec_on || r3.shadow_enabled || !r3.point_lights.is_empty();
                    let (wx, wy, wz) = if need_world_pos {
                        (wxq * f, wyq * f, wzq * f)
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

                    // блик Blinn-Phong (этап 7): half-vector на
                    // перспектива-корректной мировой позиции пикселя
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
                            shininess,
                            spec,
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
                        blend_pass,
                        spec_on,
                        shininess,
                        spec,
                    );

                    // туман (этап 7): фактор по LUT (интерполяция между
                    // узлами), смешение ПОСЛЕ шейдинга и блика
                    let fogf = if fog_on {
                        fog_lut_lookup(&lut, fog_density, f)
                    } else {
                        0.0
                    };

                    // базовый цвет пикселя: per-vertex градиент (этап 9)
                    // или равномерный цвет грани × Lambert / Phong + point lights
                    let (mut c0, mut c1, mut c2) = if !vc_rgb {
                        (
                            rq * f * dir_shade + rq * f * pt_diff[0],
                            gq * f * dir_shade + gq * f * pt_diff[1],
                            bq * f * dir_shade + bq * f * pt_diff[2],
                        )
                    } else {
                        (
                            tr * dir_shade + tr * pt_diff[0],
                            tg * dir_shade + tg * pt_diff[1],
                            tb * dir_shade + tb * pt_diff[2],
                        )
                    };
                    let mut al = if blend_pass { alq.clamp(0.0, 1.0) } else { 1.0 };
                    if let Some(t) = tex {
                        // перспектива-корректный UV: (u/w)/(1/w)
                        // тайлинг (этап 7): (1,1) — clamp к краю,
                        // иначе repeat через fract
                        let uf = tiled_coord(uq * f, tu);
                        let vf = tiled_coord(vq * f, tv);
                        let texel = sample_tex(
                            t,
                            r3.tex_filter,
                            r3.tex_mipmaps,
                            tex_grad.as_ref(),
                            uq,
                            vq,
                            iwp,
                            uf,
                            vf,
                        );
                        // alpha-test (этап 7, только opaque): discard
                        // ДО записи z — за дыркой видно то, что позади
                        if alpha_on && !blend_pass && !alpha_keep(texel[3], alpha_thr) {
                            continue;
                        }
                        if blend_pass {
                            al *= texel[3] as f32 / 255.0;
                            if al <= 0.004 {
                                continue; // полностью прозрачно
                            }
                        }
                        c0 *= texel[0] as f32 / 255.0;
                        c1 *= texel[1] as f32 / 255.0;
                        c2 *= texel[2] as f32 / 255.0;
                    } else if blend_pass && al <= 0.004 {
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
                    if blend_pass {
                        // src-alpha блендинг поверх кадра; z-буфер
                        // НЕ пишется (порядок даёт сорт far→near)
                        let dr = fb[o] as f32 / 255.0;
                        let dg = fb[o + 1] as f32 / 255.0;
                        let db = fb[o + 2] as f32 / 255.0;
                        let iv = 1.0 - al;
                        fb[o] = col((c0 * al + dr * iv) as f64);
                        fb[o + 1] = col((c1 * al + dg * iv) as f64);
                        fb[o + 2] = col((c2 * al + db * iv) as f64);
                    } else {
                        r3.zbuf[pi] = z;
                        fb[o] = col(c0 as f64);
                        fb[o + 1] = col(c1 as f64);
                        fb[o + 2] = col(c2 as f64);
                    }
                    fb[o + 3] = 255;
                }
            }
            *drawn += 1;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_line(fb: &mut [u8], w: usize, h: usize, a: Vtx, b: Vtx, r: u8, g: u8, bl: u8) {
    let steps = ((b.sx - a.sx).abs().max((b.sy - a.sy).abs()).ceil() as usize).clamp(1, 4096);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (a.sx + (b.sx - a.sx) * t).round() as i64;
        let y = (a.sy + (b.sy - a.sy) * t).round() as i64;
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            continue;
        }
        let o = (y as usize * w + x as usize) * 4;
        fb[o] = r;
        fb[o + 1] = g;
        fb[o + 2] = bl;
        fb[o + 3] = 255;
    }
}
