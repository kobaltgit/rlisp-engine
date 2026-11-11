// src/audio/synth.rs — Процедурный чиптюн-синтезатор (SFX) и буфер аудиоданных.

/// Форма волны для процедурного синтезатора чиптюнов (sfxr / PICO-8 стиль).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Square,
    Sawtooth,
    Triangle,
    Sine,
    Noise,
}

impl Waveform {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "square" | "sqr" => Some(Waveform::Square),
            "saw" | "sawtooth" => Some(Waveform::Sawtooth),
            "triangle" | "tri" => Some(Waveform::Triangle),
            "sine" | "sin" => Some(Waveform::Sine),
            "noise" | "nz" => Some(Waveform::Noise),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Waveform::Square => "square",
            Waveform::Sawtooth => "sawtooth",
            Waveform::Triangle => "triangle",
            Waveform::Sine => "sine",
            Waveform::Noise => "noise",
        }
    }
}

/// Буфер аудиоданных (PCM 16-bit).
#[derive(Clone)]
pub struct SoundBuffer {
    pub id: u64,
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl SoundBuffer {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            0.0
        } else {
            self.samples.len() as f32 / (self.sample_rate as f32 * self.channels as f32)
        }
    }

    pub fn frames_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}

/// Параметры процедурного синтеза звука (ADSR + pitch slide + waveform).
#[derive(Clone, Debug)]
pub struct SynthParams {
    pub wave: Waveform,
    pub freq: f32,
    pub duration: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub pitch_slide: f32,
    pub duty_cycle: f32,
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            wave: Waveform::Square,
            freq: 440.0,
            duration: 0.25,
            attack: 0.01,
            decay: 0.05,
            sustain: 0.5,
            release: 0.1,
            pitch_slide: 0.0,
            duty_cycle: 0.5,
        }
    }
}

/// Процедурный генератор чиптюн-звуков (ноль зависимостей).
pub fn synth_sound(id: u64, name: String, params: &SynthParams) -> SoundBuffer {
    let sample_rate = 44100u32;
    let total_frames = ((params.duration.max(0.01)) * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(total_frames);

    let mut phase = 0.0f32;
    let mut current_freq = params.freq.max(10.0);
    let mut rng_state: u32 = 0x12345678 ^ (params.freq as u32);

    let dur = params.duration.max(0.01);
    let att = params.attack.max(0.001);
    let dec = params.decay.max(0.001);
    let rel = params.release.max(0.001);
    let sus = params.sustain.clamp(0.0, 1.0);

    for i in 0..total_frames {
        let t = i as f32 / sample_rate as f32;

        // ADSR Envelope
        let env = if t < att {
            t / att
        } else if t < att + dec {
            1.0 - (1.0 - sus) * ((t - att) / dec)
        } else if t < dur - rel {
            sus
        } else if t < dur {
            let rel_progress = (t - (dur - rel)) / rel;
            sus * (1.0 - rel_progress.clamp(0.0, 1.0))
        } else {
            0.0
        };

        // Pitch slide (экспоненциальный или линейный)
        current_freq += params.pitch_slide / sample_rate as f32;
        current_freq = current_freq.clamp(10.0, 20000.0);

        // Генерация формы волны
        let raw_sample = match params.wave {
            Waveform::Sine => (phase * 2.0 * std::f32::consts::PI).sin(),
            Waveform::Square => {
                if phase < params.duty_cycle.clamp(0.05, 0.95) {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Sawtooth => 2.0 * phase - 1.0,
            Waveform::Triangle => {
                if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                }
            }
            Waveform::Noise => {
                // Xorshift32 PRNG
                rng_state ^= rng_state << 13;
                rng_state ^= rng_state >> 17;
                rng_state ^= rng_state << 5;
                (rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0
            }
        };

        phase += current_freq / sample_rate as f32;
        while phase >= 1.0 {
            phase -= 1.0;
        }

        let mixed = (raw_sample * env * 0.8).clamp(-1.0, 1.0);
        let sample_i16 = (mixed * 32767.0) as i16;
        samples.push(sample_i16);
    }

    SoundBuffer {
        id,
        name,
        sample_rate,
        channels: 1, // Mono synth
        samples,
    }
}
