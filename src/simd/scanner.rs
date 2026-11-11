// src/simd/scanner.rs — SIMD-токенизатор и byte-class скан для горячего пути reader'а

/// Класс байта, на котором останавливается лексический скан.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    /// ASCII-пробельный байт (09..0D, 20)
    Ws(u8),
    /// разделитель/синтаксис лексем: ( ) [ ] ' ` , ; "
    Delim(u8),
    /// байт >= 0x80 — нужен скалярный разбор (UTF-8)
    NonAscii(u8),
}

pub(crate) fn hit_of(b: u8) -> Hit {
    match b {
        0x09..=0x0D | 0x20 => Hit::Ws(b),
        b'(' | b')' | b'[' | b']' | b'\'' | b'`' | b',' | b';' | b'"' => Hit::Delim(b),
        _ => Hit::NonAscii(b),
    }
}

/// Скалярный fallback (и хвост < 16 байт): ровно та же семантика классов.
pub fn next_hit_scalar(bytes: &[u8], mut pos: usize) -> Option<(usize, Hit)> {
    while pos < bytes.len() {
        let b = bytes[pos];
        if b < 0x80 {
            if matches!(b, 0x09..=0x0D | 0x20)
                || matches!(
                    b,
                    b'(' | b')' | b'[' | b']' | b'\'' | b'`' | b',' | b';' | b'"'
                )
            {
                return Some((pos, hit_of(b)));
            }
            pos += 1;
        } else {
            return Some((pos, Hit::NonAscii(b)));
        }
    }
    None
}

/// Первый «особый» байт с позиции pos: Ws | Delim | NonAscii; None — конец.
/// Портативно: на x86_64 — SSE2, иначе скаляр.
pub fn next_hit(bytes: &[u8], pos: usize) -> Option<(usize, Hit)> {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            return next_hit_sse2(bytes, pos);
        }
    }
    next_hit_scalar(bytes, pos)
}

/// memchr-стиль: первый байт `target` с позиции pos (для пропуска ;-комментариев
/// и сканов внутри строк). Портативно, на x86_64 — SSE2.
pub fn find_byte(bytes: &[u8], pos: usize, target: u8) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            return find_byte_sse2(bytes, pos, target);
        }
    }
    (pos..bytes.len()).find(|&i| bytes[i] == target)
}

#[cfg(target_arch = "x86_64")]
static WS_SET: [u8; 16] = [
    0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
#[cfg(target_arch = "x86_64")]
static DL_SET: [u8; 16] = [
    b'(', b')', b'[', b']', b'\'', b'`', b',', b';', b'"', 0, 0, 0, 0, 0, 0, 0,
];

#[cfg(target_arch = "x86_64")]
#[inline]
pub(crate) fn next_hit_sse2(bytes: &[u8], mut pos: usize) -> Option<(usize, Hit)> {
    use std::arch::x86_64::*;
    unsafe {
        // векторы-наборы читаются из static (без постройки на стеке каждый вызов)
        let ws_v = _mm_loadu_si128(WS_SET.as_ptr() as *const __m128i);
        let dl_v = _mm_loadu_si128(DL_SET.as_ptr() as *const __m128i);
        let ws_vecs: [__m128i; 6] = [
            _mm_set1_epi8(0x09),
            _mm_set1_epi8(0x0A),
            _mm_set1_epi8(0x0B),
            _mm_set1_epi8(0x0C),
            _mm_set1_epi8(0x0D),
            _mm_set1_epi8(0x20),
        ];
        let dl_vecs: [__m128i; 9] = [
            _mm_set1_epi8(b'(' as i8),
            _mm_set1_epi8(b')' as i8),
            _mm_set1_epi8(b'[' as i8),
            _mm_set1_epi8(b']' as i8),
            _mm_set1_epi8(b'\'' as i8),
            _mm_set1_epi8(b'`' as i8),
            _mm_set1_epi8(b',' as i8),
            _mm_set1_epi8(b';' as i8),
            _mm_set1_epi8(b'"' as i8),
        ];
        let zero = _mm_setzero_si128();
        let sign_mask_v = _mm_cmplt_epi8(zero, zero); // все единицы
        while pos + 16 <= bytes.len() {
            let v = _mm_loadu_si128(bytes.as_ptr().add(pos) as *const __m128i);
            // WS: точные 6 сравнений (байт 0 в наборе не даёт ложных срабатываний:
            // в полном блоке текста нулевых байтов нет, а байт 0 — атом-символ)
            let mut m_ws = _mm_movemask_epi8(_mm_cmpeq_epi8(v, ws_vecs[0]));
            for w in ws_vecs[1..].iter() {
                m_ws |= _mm_movemask_epi8(_mm_cmpeq_epi8(v, *w));
            }
            let mut m_dl = _mm_movemask_epi8(_mm_cmpeq_epi8(v, dl_vecs[0]));
            for d in dl_vecs[1..].iter() {
                m_dl |= _mm_movemask_epi8(_mm_cmpeq_epi8(v, *d));
            }
            // не-ASCII (байты >= 0x80 знаковые отрицательные)
            let m_na = _mm_movemask_epi8(_mm_cmplt_epi8(v, zero));
            let m_all = m_ws | m_dl | m_na;
            if m_all == 0 {
                pos += 16;
                continue;
            }
            let _ = (&ws_v, &dl_v, &sign_mask_v);
            let bit = m_all.trailing_zeros() as usize;
            let b = bytes[pos + bit];
            let hit = hit_of(b);
            if b == 0 {
                // нулевой байт — атом-символ (семантика старого пути), не остановка
                return next_hit_scalar(bytes, pos + bit + 1).map(|(i, h)| (i, h));
            }
            return Some((pos + bit, hit));
        }
        next_hit_scalar(bytes, pos)
    }
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn find_byte_sse2(bytes: &[u8], mut pos: usize, target: u8) -> Option<usize> {
    use std::arch::x86_64::*;
    unsafe {
        let t = _mm_set1_epi8(target as i8);
        while pos + 16 <= bytes.len() {
            let v = _mm_loadu_si128(bytes.as_ptr().add(pos) as *const __m128i);
            let m = _mm_movemask_epi8(_mm_cmpeq_epi8(v, t));
            if m != 0 {
                return Some(pos + m.trailing_zeros() as usize);
            }
            pos += 16;
        }
        (pos..bytes.len()).find(|&i| bytes[i] == target)
    }
}
