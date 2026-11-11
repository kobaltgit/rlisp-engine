// src/hal/types.rs — Базовые типы HAL: текстуры, мипмапы, режимы вывода и монотонные часы.

use std::path::PathBuf;

/// Уровень детализации текстуры (LOD пирамиды мипмапов).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl MipLevel {
    pub fn pixels(&self) -> &[u8] {
        &self.px
    }
}

/// Текстура «GPU» (v0.3): RGBA8, загружается из PNG или генерируется на Lisp.
#[derive(Clone, Debug)]
pub struct Tex {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,         // rgba (LOD 0)
    pub mips: Vec<MipLevel>, // mips[0] = оригинал, mips[1] = 1/2, ...
}

#[allow(dead_code)]
pub type Texture = Tex;

impl Tex {
    pub fn new(w: usize, h: usize, px: Vec<u8>) -> Self {
        let mut t = Tex {
            w,
            h,
            px,
            mips: Vec::new(),
        };
        t.generate_mips();
        t
    }

    pub fn empty() -> Self {
        Tex {
            w: 0,
            h: 0,
            px: Vec::new(),
            mips: Vec::new(),
        }
    }

    pub fn pixels(&self) -> &[u8] {
        &self.px
    }

    /// Генерация пирамиды мипмапов с усреднением блоков 2x2 пикселя (Box Filter) до размера 1x1.
    pub fn generate_mips(&mut self) {
        self.mips.clear();
        if self.w == 0 || self.h == 0 || self.px.len() < self.w * self.h * 4 {
            return;
        }
        self.mips.push(MipLevel {
            w: self.w,
            h: self.h,
            px: self.px.clone(),
        });

        let mut curr_w = self.w;
        let mut curr_h = self.h;
        let mut curr_px = self.px.clone();

        while curr_w > 1 || curr_h > 1 {
            let next_w = (curr_w / 2).max(1);
            let next_h = (curr_h / 2).max(1);
            let mut next_px = vec![0u8; next_w * next_h * 4];

            for y in 0..next_h {
                let y0 = (y * 2).min(curr_h - 1);
                let y1 = (y * 2 + 1).min(curr_h - 1);
                for x in 0..next_w {
                    let x0 = (x * 2).min(curr_w - 1);
                    let x1 = (x * 2 + 1).min(curr_w - 1);

                    let dst_idx = (y * next_w + x) * 4;

                    let idx00 = (y0 * curr_w + x0) * 4;
                    let idx10 = (y0 * curr_w + x1) * 4;
                    let idx01 = (y1 * curr_w + x0) * 4;
                    let idx11 = (y1 * curr_w + x1) * 4;

                    for c in 0..4 {
                        let sum = curr_px[idx00 + c] as u32
                            + curr_px[idx10 + c] as u32
                            + curr_px[idx01 + c] as u32
                            + curr_px[idx11 + c] as u32;
                        next_px[dst_idx + c] = ((sum + 2) / 4) as u8;
                    }
                }
            }

            self.mips.push(MipLevel {
                w: next_w,
                h: next_h,
                px: next_px.clone(),
            });

            curr_w = next_w;
            curr_h = next_h;
            curr_px = next_px;
        }
    }
}

#[derive(Clone)]
pub enum OutputMode {
    Silent,
    Ascii,
    Png { dir: PathBuf, every: usize },
}

/// Монотонные часы: native — Instant; wasm — wall-ms от JS через rl_frame
/// (std Instant на wasm32-unknown-unknown паникует в рантайме).
pub struct MonoClock {
    #[cfg(not(target_arch = "wasm32"))]
    t0: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    t0: f64,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static JS_NOW: std::cell::RefCell<f64> = std::cell::RefCell::new(0.0);
}

/// wasm-фасад: обновить опорное время (performance.now() из JS)
#[cfg(target_arch = "wasm32")]
pub fn set_js_now(ms: f64) {
    JS_NOW.with(|c| *c.borrow_mut() = ms);
}

impl MonoClock {
    pub fn now() -> MonoClock {
        MonoClock {
            #[cfg(not(target_arch = "wasm32"))]
            t0: std::time::Instant::now(),
            #[cfg(target_arch = "wasm32")]
            t0: JS_NOW.with(|c| *c.borrow()),
        }
    }
    pub fn elapsed_secs(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.t0.elapsed().as_secs_f64()
        }
        #[cfg(target_arch = "wasm32")]
        {
            (JS_NOW.with(|c| *c.borrow()) - self.t0) / 1000.0
        }
    }
}
