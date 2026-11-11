// src/render3d/raster/tex.rs — Семплинг текстур, билинейная/nearest фильтрация и мипмаппинг.

use crate::hal::{Hal, MipLevel};
use super::super::types::{TexFilter, Vtx};

#[derive(Clone)]
pub(crate) struct TexRef {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) px: Vec<u8>,
    pub(crate) mips: Vec<MipLevel>,
    pub(crate) id: i64,
}

pub(crate) fn tex_lookup(hal: &Hal, id: i64) -> Result<Option<TexRef>, String> {
    if id == 0 {
        return Ok(None);
    }
    let t = hal
        .tex
        .get((id - 1) as usize)
        .ok_or_else(|| format!("gpu3d: нет текстуры {}", id))?;
    if t.w == 0 {
        return Err("gpu3d: текстура освобождена".into());
    }
    let mips = if t.mips.is_empty() && t.w > 0 && t.h > 0 {
        let mut tmp = t.clone();
        tmp.generate_mips();
        tmp.mips
    } else {
        t.mips.clone()
    };
    Ok(Some(TexRef {
        w: t.w,
        h: t.h,
        px: t.px.clone(),
        mips,
        id,
    }))
}

/// Билинейная фильтрация: взвешенное смешивание 4 соседних текселей по дробным координатам fx, fy.
#[inline]
#[allow(dead_code)]
pub(crate) fn sample_bilinear(mip: &MipLevel, u: f32, v: f32) -> [u8; 4] {
    sample_bilinear_raw(&mip.px, mip.w, mip.h, u, v)
}

#[inline]
pub(crate) fn sample_bilinear_raw(px: &[u8], w: usize, h: usize, u: f32, v: f32) -> [u8; 4] {
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return [255, 255, 255, 255];
    }
    if w == 1 && h == 1 {
        return [px[0], px[1], px[2], px[3]];
    }

    let u_coord = u.clamp(0.0, 1.0) * (w - 1) as f32;
    let v_coord = v.clamp(0.0, 1.0) * (h - 1) as f32;

    let x0 = u_coord.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let fx = u_coord - x0 as f32;

    let y0 = v_coord.floor() as usize;
    let y1 = (y0 + 1).min(h - 1);
    let fy = v_coord - y0 as f32;

    let fx_int = (fx * 256.0).round().clamp(0.0, 256.0) as u32;
    let fy_int = (fy * 256.0).round().clamp(0.0, 256.0) as u32;

    let w00 = (256 - fx_int) * (256 - fy_int);
    let w10 = fx_int * (256 - fy_int);
    let w01 = (256 - fx_int) * fy_int;
    let w11 = fx_int * fy_int;

    let idx00 = (y0 * w + x0) * 4;
    let idx10 = (y0 * w + x1) * 4;
    let idx01 = (y1 * w + x0) * 4;
    let idx11 = (y1 * w + x1) * 4;

    [
        ((px[idx00] as u32 * w00
            + px[idx10] as u32 * w10
            + px[idx01] as u32 * w01
            + px[idx11] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 1] as u32 * w00
            + px[idx10 + 1] as u32 * w10
            + px[idx01 + 1] as u32 * w01
            + px[idx11 + 1] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 2] as u32 * w00
            + px[idx10 + 2] as u32 * w10
            + px[idx01 + 2] as u32 * w01
            + px[idx11 + 2] as u32 * w11
            + 32768)
            >> 16) as u8,
        ((px[idx00 + 3] as u32 * w00
            + px[idx10 + 3] as u32 * w10
            + px[idx01 + 3] as u32 * w01
            + px[idx11 + 3] as u32 * w11
            + 32768)
            >> 16) as u8,
    ]
}

#[inline]
#[allow(dead_code)]
pub(crate) fn sample_nearest(mip: &MipLevel, u: f32, v: f32) -> [u8; 4] {
    sample_nearest_raw(&mip.px, mip.w, mip.h, u, v)
}

#[inline]
pub(crate) fn sample_nearest_raw(px: &[u8], w: usize, h: usize, u: f32, v: f32) -> [u8; 4] {
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return [255, 255, 255, 255];
    }
    let x = (u.clamp(0.0, 0.99999) * w as f32) as usize;
    let y = (v.clamp(0.0, 0.99999) * h as f32) as usize;
    let ti = (y * w + x) * 4;
    [px[ti], px[ti + 1], px[ti + 2], px[ti + 3]]
}

pub(crate) struct TexGradients {
    pub d_uq_dx: f32,
    pub d_uq_dy: f32,
    pub d_vq_dx: f32,
    pub d_vq_dy: f32,
    pub d_iw_dx: f32,
    pub d_iw_dy: f32,
    pub tw: f32,
    pub th: f32,
    pub tu: f32,
    pub tv: f32,
}

impl TexGradients {
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        p0: &Vtx,
        p1: &Vtx,
        p2: &Vtx,
        sum: f32,
        tw: usize,
        th: usize,
        tu: f32,
        tv: f32,
    ) -> Option<Self> {
        if sum.abs() < 1e-9 || tw == 0 || th == 0 {
            return None;
        }
        let d_w0_dx = p2.sy - p1.sy;
        let d_w0_dy = p1.sx - p2.sx;
        let d_w1_dx = p0.sy - p2.sy;
        let d_w1_dy = p2.sx - p0.sx;
        let d_w2_dx = p1.sy - p0.sy;
        let d_w2_dy = p0.sx - p1.sx;

        let inv_sum = 1.0 / sum;
        let d_uq_dx = (d_w0_dx * p0.uq + d_w1_dx * p1.uq + d_w2_dx * p2.uq) * inv_sum;
        let d_uq_dy = (d_w0_dy * p0.uq + d_w1_dy * p1.uq + d_w2_dy * p2.uq) * inv_sum;
        let d_vq_dx = (d_w0_dx * p0.vq + d_w1_dx * p1.vq + d_w2_dx * p2.vq) * inv_sum;
        let d_vq_dy = (d_w0_dy * p0.vq + d_w1_dy * p1.vq + d_w2_dy * p2.vq) * inv_sum;
        let d_iw_dx = (d_w0_dx * p0.iw + d_w1_dx * p1.iw + d_w2_dx * p2.iw) * inv_sum;
        let d_iw_dy = (d_w0_dy * p0.iw + d_w1_dy * p1.iw + d_w2_dy * p2.iw) * inv_sum;

        Some(TexGradients {
            d_uq_dx,
            d_uq_dy,
            d_vq_dx,
            d_vq_dy,
            d_iw_dx,
            d_iw_dy,
            tw: tw as f32,
            th: th as f32,
            tu,
            tv,
        })
    }

    #[inline]
    pub fn select_lod(&self, uq: f32, vq: f32, iwp: f32, max_lod: usize) -> usize {
        let iwp_sq = (iwp * iwp).max(1e-12);
        let du_dx = (self.d_uq_dx * iwp - uq * self.d_iw_dx) / iwp_sq * self.tu;
        let dv_dx = (self.d_vq_dx * iwp - vq * self.d_iw_dx) / iwp_sq * self.tv;
        let du_dy = (self.d_uq_dy * iwp - uq * self.d_iw_dy) / iwp_sq * self.tu;
        let dv_dy = (self.d_vq_dy * iwp - vq * self.d_iw_dy) / iwp_sq * self.tv;

        let dudx_tex = du_dx * self.tw;
        let dvdx_tex = dv_dx * self.th;
        let dudy_tex = du_dy * self.tw;
        let dvdy_tex = dv_dy * self.th;

        let rho_x_sq = dudx_tex * dudx_tex + dvdx_tex * dvdx_tex;
        let rho_y_sq = dudy_tex * dudy_tex + dvdy_tex * dvdy_tex;
        let rho_sq = rho_x_sq.max(rho_y_sq);

        if rho_sq <= 1.0 {
            0
        } else {
            let lod = (0.5 * rho_sq.log2()).round().max(0.0) as usize;
            lod.min(max_lod)
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_tex(
    tex: &TexRef,
    filter: TexFilter,
    use_mips: bool,
    grad: Option<&TexGradients>,
    uq: f32,
    vq: f32,
    iwp: f32,
    uf: f32,
    vf: f32,
) -> [u8; 4] {
    let (sample_px, sample_w, sample_h) = if use_mips && !tex.mips.is_empty() {
        let max_lod = tex.mips.len() - 1;
        let lod = match grad {
            Some(g) => g.select_lod(uq, vq, iwp, max_lod),
            None => 0,
        };
        let mip = &tex.mips[lod];
        (&mip.px[..], mip.w, mip.h)
    } else {
        (&tex.px[..], tex.w, tex.h)
    };

    match filter {
        TexFilter::Nearest => sample_nearest_raw(sample_px, sample_w, sample_h, uf, vf),
        TexFilter::Bilinear => sample_bilinear_raw(sample_px, sample_w, sample_h, uf, vf),
    }
}
