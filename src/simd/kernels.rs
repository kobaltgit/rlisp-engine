// src/simd/kernels.rs — Ядра вычислений: скалярные, SSE2, AVX2, AVX-512 и диспетчеризация

use super::types::{active_level, SimdLevel};

// ——— скалярный fallback (эталон эквивалентности; LLVM может автовекторизовать) ———
// На x86_64 часть функций не вызывается (там минимум SSE2) — это fallback
// для других архитектур, оставляем полный набор.
#[allow(dead_code)]
pub mod scalar {
    pub fn add(a: &[f32], b: &[f32], out: &mut [f32]) {
        for i in 0..a.len() {
            out[i] = a[i] + b[i];
        }
    }
    pub fn sub(a: &[f32], b: &[f32], out: &mut [f32]) {
        for i in 0..a.len() {
            out[i] = a[i] - b[i];
        }
    }
    pub fn mul(a: &[f32], b: &[f32], out: &mut [f32]) {
        for i in 0..a.len() {
            out[i] = a[i] * b[i];
        }
    }
    pub fn min(a: &[f32], b: &[f32], out: &mut [f32]) {
        for i in 0..a.len() {
            out[i] = a[i].min(b[i]);
        }
    }
    pub fn max(a: &[f32], b: &[f32], out: &mut [f32]) {
        for i in 0..a.len() {
            out[i] = a[i].max(b[i]);
        }
    }
    pub fn dot(a: &[f32], b: &[f32]) -> f32 {
        let mut s = 0f32;
        for i in 0..a.len() {
            s += a[i] * b[i];
        }
        s
    }
}

#[cfg(target_arch = "x86_64")]
mod sse {
    use std::arch::x86_64::*;
    pub unsafe fn add(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 4;
        let mut i = 0;
        while i < n {
            _mm_storeu_ps(
                out.as_mut_ptr().add(i),
                _mm_add_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        while i < a.len() {
            out[i] = a[i] + b[i];
            i += 1;
        }
    }
    pub unsafe fn sub(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 4;
        let mut i = 0;
        while i < n {
            _mm_storeu_ps(
                out.as_mut_ptr().add(i),
                _mm_sub_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        while i < a.len() {
            out[i] = a[i] - b[i];
            i += 1;
        }
    }
    pub unsafe fn mul(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 4;
        let mut i = 0;
        while i < n {
            _mm_storeu_ps(
                out.as_mut_ptr().add(i),
                _mm_mul_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        while i < a.len() {
            out[i] = a[i] * b[i];
            i += 1;
        }
    }
    pub unsafe fn min(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 4;
        let mut i = 0;
        while i < n {
            _mm_storeu_ps(
                out.as_mut_ptr().add(i),
                _mm_min_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        while i < a.len() {
            out[i] = a[i].min(b[i]);
            i += 1;
        }
    }
    pub unsafe fn max(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 4;
        let mut i = 0;
        while i < n {
            _mm_storeu_ps(
                out.as_mut_ptr().add(i),
                _mm_max_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        while i < a.len() {
            out[i] = a[i].max(b[i]);
            i += 1;
        }
    }
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len() - a.len() % 4;
        let mut acc = _mm_setzero_ps();
        let mut i = 0;
        while i < n {
            acc = _mm_add_ps(
                acc,
                _mm_mul_ps(
                    _mm_loadu_ps(a.as_ptr().add(i)),
                    _mm_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            i += 4;
        }
        let mut t = [0f32; 4];
        _mm_storeu_ps(t.as_mut_ptr(), acc);
        let mut s = t[0] + t[1] + t[2] + t[3];
        while i < a.len() {
            s += a[i] * b[i];
            i += 1;
        }
        s
    }
}

#[cfg(target_arch = "x86_64")]
mod avx {
    use std::arch::x86_64::*;
    // разворот ×4: 32 f32 за итерацию
    pub unsafe fn add(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..4 {
                let p = i + k * 8;
                _mm256_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm256_add_ps(
                        _mm256_loadu_ps(a.as_ptr().add(p)),
                        _mm256_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] + b[i];
            i += 1;
        }
    }
    pub unsafe fn sub(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..4 {
                let p = i + k * 8;
                _mm256_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm256_sub_ps(
                        _mm256_loadu_ps(a.as_ptr().add(p)),
                        _mm256_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] - b[i];
            i += 1;
        }
    }
    pub unsafe fn mul(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..4 {
                let p = i + k * 8;
                _mm256_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm256_mul_ps(
                        _mm256_loadu_ps(a.as_ptr().add(p)),
                        _mm256_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] * b[i];
            i += 1;
        }
    }
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len() - a.len() % 16;
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut i = 0;
        while i < n {
            acc0 = _mm256_add_ps(
                acc0,
                _mm256_mul_ps(
                    _mm256_loadu_ps(a.as_ptr().add(i)),
                    _mm256_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            acc1 = _mm256_add_ps(
                acc1,
                _mm256_mul_ps(
                    _mm256_loadu_ps(a.as_ptr().add(i + 8)),
                    _mm256_loadu_ps(b.as_ptr().add(i + 8)),
                ),
            );
            i += 16;
        }
        let mut t = [0f32; 8];
        _mm256_storeu_ps(t.as_mut_ptr(), _mm256_add_ps(acc0, acc1));
        let mut s = 0f32;
        for x in t.iter() {
            s += *x;
        }
        while i < a.len() {
            s += a[i] * b[i];
            i += 1;
        }
        s
    }
}

// AVX-512: интринсики _mm512_* стабилизированы в stable 1.89 (stdarch_x86_avx512).
// cfg has_avx512 выставляет build.rs по версии rustc — на старых stable модуль
// исключается, и уровень Avx512 никогда не выбирается (см. detect/calibrate).
#[cfg(all(target_arch = "x86_64", has_avx512))]
mod avx512 {
    use std::arch::x86_64::*;
    // разворот ×2: 32 f32 за итерацию
    pub unsafe fn add(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..2 {
                let p = i + k * 16;
                _mm512_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm512_add_ps(
                        _mm512_loadu_ps(a.as_ptr().add(p)),
                        _mm512_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] + b[i];
            i += 1;
        }
    }
    pub unsafe fn sub(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..2 {
                let p = i + k * 16;
                _mm512_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm512_sub_ps(
                        _mm512_loadu_ps(a.as_ptr().add(p)),
                        _mm512_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] - b[i];
            i += 1;
        }
    }
    pub unsafe fn mul(a: &[f32], b: &[f32], out: &mut [f32]) {
        let n = a.len() - a.len() % 32;
        let mut i = 0;
        while i < n {
            for k in 0..2 {
                let p = i + k * 16;
                _mm512_storeu_ps(
                    out.as_mut_ptr().add(p),
                    _mm512_mul_ps(
                        _mm512_loadu_ps(a.as_ptr().add(p)),
                        _mm512_loadu_ps(b.as_ptr().add(p)),
                    ),
                );
            }
            i += 32;
        }
        while i < a.len() {
            out[i] = a[i] * b[i];
            i += 1;
        }
    }
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len() - a.len() % 32;
        let mut acc0 = _mm512_setzero_ps();
        let mut acc1 = _mm512_setzero_ps();
        let mut i = 0;
        while i < n {
            acc0 = _mm512_add_ps(
                acc0,
                _mm512_mul_ps(
                    _mm512_loadu_ps(a.as_ptr().add(i)),
                    _mm512_loadu_ps(b.as_ptr().add(i)),
                ),
            );
            acc1 = _mm512_add_ps(
                acc1,
                _mm512_mul_ps(
                    _mm512_loadu_ps(a.as_ptr().add(i + 16)),
                    _mm512_loadu_ps(b.as_ptr().add(i + 16)),
                ),
            );
            i += 32;
        }
        let s = _mm512_reduce_add_ps(_mm512_add_ps(acc0, acc1));
        let mut tail = s;
        while i < a.len() {
            tail += a[i] * b[i];
            i += 1;
        }
        tail
    }
}

#[cfg(target_arch = "x86_64")]
pub fn level_add(l: SimdLevel, a: &[f32], b: &[f32], out: &mut [f32]) {
    match l {
        // без stdarch avx512 (rustc < 1.89) — тот же профиль на AVX2
        #[cfg(has_avx512)]
        SimdLevel::Avx512 => unsafe { avx512::add(a, b, out) },
        #[cfg(not(has_avx512))]
        SimdLevel::Avx512 => unsafe { avx::add(a, b, out) },
        SimdLevel::Avx2 => unsafe { avx::add(a, b, out) },
        SimdLevel::Sse2 => unsafe { sse::add(a, b, out) },
        SimdLevel::Scalar => scalar::add(a, b, out),
    }
}
#[cfg(not(target_arch = "x86_64"))]
pub fn level_add(l: SimdLevel, a: &[f32], b: &[f32], out: &mut [f32]) {
    let _ = l;
    scalar::add(a, b, out)
}

fn dispatch2(a: &[f32], b: &[f32], out: &mut [f32], op: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        match active_level() {
            SimdLevel::Avx512 => unsafe {
                let m = avx512_if(op);
                m(a, b, out);
            },
            SimdLevel::Avx2 => unsafe {
                let m = avx_if(op);
                m(a, b, out);
            },
            _ => unsafe {
                let m = sse_if(op);
                m(a, b, out);
            },
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // фолбэк для ARM/прочего: честный маппинг операции, а не add всегда
        // (раньше тут была «заглушка, не используется» — vsub/vmul на
        // aarch64 считали сложение, тест эквивалентности падал на macOS)
        match op {
            1 => scalar::sub(a, b, out),
            2 => scalar::mul(a, b, out),
            _ => scalar::add(a, b, out),
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn avx_if(op: u8) -> unsafe fn(&[f32], &[f32], &mut [f32]) {
    match op {
        1 => avx::sub,
        2 => avx::mul,
        _ => avx::add,
    }
}
#[cfg(target_arch = "x86_64")]
fn sse_if(op: u8) -> unsafe fn(&[f32], &[f32], &mut [f32]) {
    match op {
        1 => sse::sub,
        2 => sse::mul,
        3 => sse::min,
        4 => sse::max,
        _ => sse::add,
    }
}
#[cfg(all(target_arch = "x86_64", has_avx512))]
fn avx512_if(op: u8) -> unsafe fn(&[f32], &[f32], &mut [f32]) {
    match op {
        1 => avx512::sub,
        2 => avx512::mul,
        _ => avx512::add,
    }
}
#[cfg(all(target_arch = "x86_64", not(has_avx512)))]
fn avx512_if(op: u8) -> unsafe fn(&[f32], &[f32], &mut [f32]) {
    // rustc < 1.89: уровень Avx512 не выбирается (detect), а если всё же
    // пришёл — считаем на AVX2, ничего не ломается
    match op {
        1 => avx::sub,
        2 => avx::mul,
        _ => avx::add,
    }
}

pub fn vadd(a: &[f32], b: &[f32], out: &mut [f32]) {
    dispatch2(a, b, out, 0);
}
pub fn vsub(a: &[f32], b: &[f32], out: &mut [f32]) {
    dispatch2(a, b, out, 1);
}
pub fn vmul(a: &[f32], b: &[f32], out: &mut [f32]) {
    dispatch2(a, b, out, 2);
}
pub fn vmin(a: &[f32], b: &[f32], out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { sse::min(a, b, out) };
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::min(a, b, out)
    }
}
pub fn vmax(a: &[f32], b: &[f32], out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { sse::max(a, b, out) };
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::max(a, b, out)
    }
}

pub fn vdot(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        match active_level() {
            #[cfg(has_avx512)]
            SimdLevel::Avx512 => unsafe { avx512::dot(a, b) },
            #[cfg(not(has_avx512))]
            SimdLevel::Avx512 => unsafe { avx::dot(a, b) },
            SimdLevel::Avx2 => unsafe { avx::dot(a, b) },
            _ => unsafe { sse::dot(a, b) },
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::dot(a, b)
    }
}
