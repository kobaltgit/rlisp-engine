// src/audio.rs — Аудиосистема rlisp-engine: софтверный микшер, WAV кодек,
// процедурный чиптюн-синтезатор (SFX) и WinMM вывод для Windows.
// Ноль сторонних зависимостей.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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

/// Кодек RIFF WAVE PCM (8/16 бит, моно/стерео).
pub struct WavCodec;

impl WavCodec {
    /// Кодирует SoundBuffer в валидные байты RIFF WAV.
    pub fn encode(buf: &SoundBuffer) -> Vec<u8> {
        let channels = buf.channels;
        let sample_rate = buf.sample_rate;
        let bits_per_sample = 16u16;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * block_align as u32;
        let data_bytes = buf.samples.len() * 2;
        let file_size = 36 + data_bytes;

        let mut out = Vec::with_capacity(file_size + 8);
        // RIFF header
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");

        // "fmt " subchunk
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
        out.extend_from_slice(&1u16.to_le_bytes()); // AudioFormat 1 = PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits_per_sample.to_le_bytes());

        // "data" subchunk
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
        for &s in &buf.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }

        out
    }

    /// Декодирует байты RIFF WAVE PCM.
    pub fn decode(id: u64, name: String, bytes: &[u8]) -> Result<SoundBuffer, String> {
        if bytes.len() < 44 {
            return Err("WAV: файл слишком короткий для RIFF/WAVE".into());
        }
        if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("WAV: некорректная сигнатура RIFF/WAVE".into());
        }

        let mut pos = 12;
        let mut fmt_found = false;
        let mut channels = 1u16;
        let mut sample_rate = 44100u32;
        let mut bits_per_sample = 16u16;
        let mut audio_format = 1u16;
        let mut samples = Vec::new();

        while pos + 8 <= bytes.len() {
            let chunk_id = &bytes[pos..pos + 4];
            let chunk_size = u32::from_le_bytes([
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]) as usize;
            pos += 8;

            if chunk_id == b"fmt " {
                if chunk_size < 16 || pos + 16 > bytes.len() {
                    return Err("WAV: поврежденный fmt чанк".into());
                }
                audio_format = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
                channels = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]);
                sample_rate = u32::from_le_bytes([
                    bytes[pos + 4],
                    bytes[pos + 5],
                    bytes[pos + 6],
                    bytes[pos + 7],
                ]);
                // byte_rate = bytes[pos+8..pos+12]
                // block_align = bytes[pos+12..pos+14]
                bits_per_sample = u16::from_le_bytes([bytes[pos + 14], bytes[pos + 15]]);
                fmt_found = true;
                pos += chunk_size;
            } else if chunk_id == b"data" {
                if !fmt_found {
                    return Err("WAV: data чанк встречен раньше fmt".into());
                }
                if audio_format != 1 {
                    return Err(format!(
                        "WAV: поддерживается только PCM (формат {})",
                        audio_format
                    ));
                }
                let end = (pos + chunk_size).min(bytes.len());
                let data_slice = &bytes[pos..end];

                match bits_per_sample {
                    8 => {
                        samples.reserve(data_slice.len());
                        for &b in data_slice {
                            // 8-bit unsigned PCM: 128 is center
                            let val = ((b as i32 - 128) * 256) as i16;
                            samples.push(val);
                        }
                    }
                    16 => {
                        let count = data_slice.len() / 2;
                        samples.reserve(count);
                        for i in 0..count {
                            let s = i16::from_le_bytes([data_slice[i * 2], data_slice[i * 2 + 1]]);
                            samples.push(s);
                        }
                    }
                    other => {
                        return Err(format!("WAV: неподдерживаемая разрядность: {} бит", other));
                    }
                }
                pos += chunk_size;
            } else {
                // Пропускаем неизвестный чанк (например LIST, JUNK)
                pos += chunk_size;
            }
            // Выравнивание чанков по четной границе (RIFF spec)
            if chunk_size % 2 != 0 {
                pos += 1;
            }
        }

        if samples.is_empty() {
            return Err("WAV: не найдено сэмплов в файле".into());
        }

        Ok(SoundBuffer {
            id,
            name,
            sample_rate,
            channels,
            samples,
        })
    }
}

/// Голос воспроизведения звука в микшере.
#[derive(Clone, Debug)]
pub struct Voice {
    pub id: u64,
    pub sound_id: u64,
    pub pos: f64,
    pub volume_l: f32,
    pub volume_r: f32,
    pub pitch: f32,
    pub is_looping: bool,
    pub is_active: bool,
}

/// Событие аудио для моста веб-редактора (`/api/audio-events`).
#[derive(Clone, Debug)]
pub struct AudioEvent {
    pub id: u64,
    pub sound_id: u64,
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    pub pan: f32,
    pub action: String, // "play", "stop", "master-volume"
}

/// Софтверный аудиомикшер: держит буферы звуков и активные голоса.
pub struct AudioMixer {
    pub sounds: HashMap<u64, SoundBuffer>,
    pub voices: Vec<Voice>,
    pub next_sound_id: u64,
    pub next_voice_id: u64,
    pub master_volume: f32,
    pub events: Vec<AudioEvent>,
    pub next_event_id: u64,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            sounds: HashMap::new(),
            voices: Vec::with_capacity(32),
            next_sound_id: 1,
            next_voice_id: 1,
            master_volume: 1.0,
            events: Vec::new(),
            next_event_id: 1,
        }
    }

    /// Регистрация существующего SoundBuffer.
    pub fn add_sound(&mut self, mut buf: SoundBuffer) -> u64 {
        let id = self.next_sound_id;
        self.next_sound_id += 1;
        buf.id = id;
        self.sounds.insert(id, buf);
        id
    }

    /// Воспроизведение звука с параметрами (громкость, питч, панорама, зацикливание).
    pub fn play(
        &mut self,
        sound_id: u64,
        volume: f32,
        pitch: f32,
        pan: f32,
        looping: bool,
    ) -> Result<u64, String> {
        let sound = self
            .sounds
            .get(&sound_id)
            .ok_or_else(|| format!("audio: неизвестный звук id={}", sound_id))?;

        let vol = volume.clamp(0.0, 2.0);
        let p = pitch.clamp(0.1, 4.0);
        let pan_clamped = pan.clamp(-1.0, 1.0);
        let vol_l = vol * (1.0 - pan_clamped.max(0.0));
        let vol_r = vol * (1.0 + pan_clamped.min(0.0));

        let voice_id = self.next_voice_id;
        self.next_voice_id += 1;

        // Ограничиваем число одновременных голосов (32)
        if self.voices.len() >= 32 {
            // Удаляем неактивный голос или самый старый
            if let Some(pos) = self.voices.iter().position(|v| !v.is_active) {
                self.voices.swap_remove(pos);
            } else {
                self.voices.remove(0);
            }
        }

        self.voices.push(Voice {
            id: voice_id,
            sound_id,
            pos: 0.0,
            volume_l: vol_l,
            volume_r: vol_r,
            pitch: p,
            is_looping: looping,
            is_active: true,
        });

        // Записываем событие для веб-редактора
        let ev_id = self.next_event_id;
        self.next_event_id += 1;
        self.events.push(AudioEvent {
            id: ev_id,
            sound_id,
            name: sound.name.clone(),
            volume: vol,
            pitch: p,
            pan: pan_clamped,
            action: "play".into(),
        });
        if self.events.len() > 100 {
            self.events.drain(0..50);
        }

        Ok(voice_id)
    }

    pub fn stop(&mut self, voice_id: u64) {
        self.voices.retain(|v| v.id != voice_id);
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
        let ev_id = self.next_event_id;
        self.next_event_id += 1;
        self.events.push(AudioEvent {
            id: ev_id,
            sound_id: 0,
            name: String::new(),
            volume: 0.0,
            pitch: 1.0,
            pan: 0.0,
            action: "stop_all".into(),
        });
    }

    pub fn set_voice_volume(&mut self, voice_id: u64, vol: f32) {
        let v_clamp = vol.clamp(0.0, 2.0);
        for v in &mut self.voices {
            if v.id == voice_id {
                v.volume_l = v_clamp;
                v.volume_r = v_clamp;
            }
        }
    }

    pub fn set_master_volume(&mut self, vol: f32) {
        self.master_volume = vol.clamp(0.0, 2.0);
    }

    /// Смешивает активные голоса в выходной стерео-буфер (PCM 16-bit, 44100 Hz, interleaved L/R).
    pub fn mix(&mut self, output: &mut [i16]) {
        let target_rate = 44100.0f64;
        let master = self.master_volume;
        let num_frames = output.len() / 2;

        let mut mix_buf_l = vec![0.0f32; num_frames];
        let mut mix_buf_r = vec![0.0f32; num_frames];

        for voice in &mut self.voices {
            if !voice.is_active {
                continue;
            }
            let sound = match self.sounds.get(&voice.sound_id) {
                Some(s) => s,
                None => {
                    voice.is_active = false;
                    continue;
                }
            };

            let frames_in_buf = sound.frames_count();
            if frames_in_buf == 0 {
                voice.is_active = false;
                continue;
            }

            let step = (sound.sample_rate as f64 / target_rate) * voice.pitch as f64;
            let ch = sound.channels as usize;

            for i in 0..num_frames {
                let p = voice.pos;
                let idx0 = p.floor() as usize;
                let frac = (p - idx0 as f64) as f32;

                if idx0 >= frames_in_buf {
                    if voice.is_looping {
                        voice.pos %= frames_in_buf as f64;
                        let new_idx0 = voice.pos.floor() as usize;
                        let new_frac = (voice.pos - new_idx0 as f64) as f32;
                        let (s_l, s_r) = sample_at(sound, new_idx0, new_frac, ch);
                        mix_buf_l[i] += s_l * voice.volume_l;
                        mix_buf_r[i] += s_r * voice.volume_r;
                        voice.pos += step;
                    } else {
                        voice.is_active = false;
                        break;
                    }
                } else {
                    let (s_l, s_r) = sample_at(sound, idx0, frac, ch);
                    mix_buf_l[i] += s_l * voice.volume_l;
                    mix_buf_r[i] += s_r * voice.volume_r;
                    voice.pos += step;
                }
            }
        }

        // Очищаем завершившиеся голоса
        self.voices.retain(|v| v.is_active);

        // Финальный микс с мастер-громкостью и клиппингом в i16
        for i in 0..num_frames {
            let val_l = (mix_buf_l[i] * master).clamp(-32767.0, 32767.0);
            let val_r = (mix_buf_r[i] * master).clamp(-32767.0, 32767.0);
            output[i * 2] = val_l as i16;
            output[i * 2 + 1] = val_r as i16;
        }
    }
}

#[inline(always)]
fn sample_at(sound: &SoundBuffer, idx: usize, frac: f32, ch: usize) -> (f32, f32) {
    let frames = sound.frames_count();
    let idx1 = if idx + 1 < frames { idx + 1 } else { idx };

    if ch == 1 {
        let s0 = sound.samples[idx] as f32;
        let s1 = sound.samples[idx1] as f32;
        let s = s0 + frac * (s1 - s0);
        (s, s)
    } else {
        let s0_l = sound.samples[idx * 2] as f32;
        let s1_l = sound.samples[idx1 * 2] as f32;
        let s_l = s0_l + frac * (s1_l - s0_l);

        let s0_r = sound.samples[idx * 2 + 1] as f32;
        let s1_r = sound.samples[idx1 * 2 + 1] as f32;
        let s_r = s0_r + frac * (s1_r - s0_r);
        (s_l, s_r)
    }
}

// -----------------------------------------------------------------------------
// Native Audio Device (WinMM на Windows, Silent fallback на других ОС)
// -----------------------------------------------------------------------------

pub struct NativeAudioDevice {
    _running: Arc<AtomicBool>,
}

#[cfg(target_os = "windows")]
mod winmm {
    use super::*;
    use std::ffi::c_void;
    use std::thread;
    use std::time::Duration;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WaveFormatEx {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    #[repr(C)]
    struct WaveHdr {
        lp_data: *mut u8,
        dw_buffer_length: u32,
        dw_bytes_recorded: u32,
        dw_user: usize,
        dw_flags: u32,
        dw_loops: u32,
        lp_next: *mut WaveHdr,
        reserved: usize,
    }

    type HWaveOut = *mut c_void;

    const WAVE_FORMAT_PCM: u16 = 1;
    const WHDR_DONE: u32 = 0x00000001;

    #[link(name = "winmm")]
    extern "system" {
        fn waveOutOpen(
            phwo: *mut HWaveOut,
            u_device_id: u32,
            pwfx: *const WaveFormatEx,
            dw_callback: usize,
            dw_instance: usize,
            fdw_open: u32,
        ) -> u32;

        fn waveOutPrepareHeader(hwo: HWaveOut, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutUnprepareHeader(hwo: HWaveOut, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutWrite(hwo: HWaveOut, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutReset(hwo: HWaveOut) -> u32;
        fn waveOutClose(hwo: HWaveOut) -> u32;
    }

    pub fn start_winmm_playback(
        mixer: Arc<Mutex<AudioMixer>>,
        running: Arc<AtomicBool>,
    ) -> Option<NativeAudioDevice> {
        let fmt = WaveFormatEx {
            w_format_tag: WAVE_FORMAT_PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 44100 * 4,
            n_block_align: 4,
            w_bits_per_sample: 16,
            cb_size: 0,
        };

        let mut hwo: HWaveOut = std::ptr::null_mut();
        unsafe {
            // WAVE_MAPPER = 0xFFFFFFFFu32
            let res = waveOutOpen(&mut hwo, 0xFFFFFFFF, &fmt, 0, 0, 0);
            if res != 0 || hwo.is_null() {
                return None;
            }
        }

        let run_clone = running.clone();
        let hwo_raw = hwo as usize;
        thread::Builder::new()
            .name("rlisp-audio".into())
            .spawn(move || {
                let hwo = hwo_raw as HWaveOut;
                // Двойная буферизация: 2 буфера по 2048 сэмплов (46 мс на буфер)
                const BUFFER_SAMPLES: usize = 2048 * 2; // stereo = 4096 i16
                let mut buf0 = vec![0i16; BUFFER_SAMPLES];
                let mut buf1 = vec![0i16; BUFFER_SAMPLES];

                let mut hdr0 = WaveHdr {
                    lp_data: buf0.as_mut_ptr() as *mut u8,
                    dw_buffer_length: (BUFFER_SAMPLES * 2) as u32,
                    dw_bytes_recorded: 0,
                    dw_user: 0,
                    dw_flags: WHDR_DONE,
                    dw_loops: 0,
                    lp_next: std::ptr::null_mut(),
                    reserved: 0,
                };
                let mut hdr1 = WaveHdr {
                    lp_data: buf1.as_mut_ptr() as *mut u8,
                    dw_buffer_length: (BUFFER_SAMPLES * 2) as u32,
                    dw_bytes_recorded: 0,
                    dw_user: 0,
                    dw_flags: WHDR_DONE,
                    dw_loops: 0,
                    lp_next: std::ptr::null_mut(),
                    reserved: 0,
                };

                unsafe {
                    waveOutPrepareHeader(hwo, &mut hdr0, std::mem::size_of::<WaveHdr>() as u32);
                    waveOutPrepareHeader(hwo, &mut hdr1, std::mem::size_of::<WaveHdr>() as u32);
                }

                while run_clone.load(Ordering::Relaxed) {
                    // Буфер 0
                    if (hdr0.dw_flags & WHDR_DONE) != 0 {
                        {
                            let mut m = mixer.lock().unwrap();
                            m.mix(&mut buf0);
                        }
                        unsafe {
                            hdr0.dw_flags &= !WHDR_DONE;
                            waveOutWrite(hwo, &mut hdr0, std::mem::size_of::<WaveHdr>() as u32);
                        }
                    }

                    // Буфер 1
                    if (hdr1.dw_flags & WHDR_DONE) != 0 {
                        {
                            let mut m = mixer.lock().unwrap();
                            m.mix(&mut buf1);
                        }
                        unsafe {
                            hdr1.dw_flags &= !WHDR_DONE;
                            waveOutWrite(hwo, &mut hdr1, std::mem::size_of::<WaveHdr>() as u32);
                        }
                    }

                    thread::sleep(Duration::from_millis(10));
                }

                unsafe {
                    waveOutReset(hwo);
                    waveOutUnprepareHeader(hwo, &mut hdr0, std::mem::size_of::<WaveHdr>() as u32);
                    waveOutUnprepareHeader(hwo, &mut hdr1, std::mem::size_of::<WaveHdr>() as u32);
                    waveOutClose(hwo);
                }
            })
            .ok();

        Some(NativeAudioDevice { _running: running })
    }
}

impl NativeAudioDevice {
    pub fn start(mixer: Arc<Mutex<AudioMixer>>) -> Option<Self> {
        let running = Arc::new(AtomicBool::new(true));

        #[cfg(target_os = "windows")]
        {
            winmm::start_winmm_playback(mixer, running)
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = mixer;
            Some(NativeAudioDevice { _running: running })
        }
    }
}

impl Drop for NativeAudioDevice {
    fn drop(&mut self) {
        self._running.store(false, Ordering::Relaxed);
    }
}

// -----------------------------------------------------------------------------
// Тесты для проверки кодека, синтезатора и микшера
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_all_waveforms() {
        for wave in [
            Waveform::Square,
            Waveform::Sawtooth,
            Waveform::Triangle,
            Waveform::Sine,
            Waveform::Noise,
        ] {
            let params = SynthParams {
                wave,
                freq: 220.0,
                duration: 0.05,
                attack: 0.01,
                decay: 0.01,
                sustain: 0.5,
                release: 0.01,
                pitch_slide: 50.0,
                duty_cycle: 0.5,
            };
            let buf = synth_sound(1, format!("test_{}", wave.as_str()), &params);
            assert!(!buf.samples.is_empty());
            assert_eq!(buf.channels, 1);
            assert_eq!(buf.sample_rate, 44100);
        }
    }

    #[test]
    fn wav_encode_decode_roundtrip() {
        let params = SynthParams {
            wave: Waveform::Sine,
            freq: 440.0,
            duration: 0.1,
            ..Default::default()
        };
        let original = synth_sound(42, "sine440".into(), &params);
        let wav_bytes = WavCodec::encode(&original);
        assert!(wav_bytes.len() > 44);

        let decoded =
            WavCodec::decode(42, "sine440".into(), &wav_bytes).expect("WAV decode should succeed");
        assert_eq!(decoded.sample_rate, original.sample_rate);
        assert_eq!(decoded.channels, original.channels);
        assert_eq!(decoded.samples.len(), original.samples.len());
        // Сэмплы должны совпадать побайтово
        assert_eq!(decoded.samples, original.samples);
    }

    #[test]
    fn mixer_playback_and_volume() {
        let mut mixer = AudioMixer::new();
        let sound = synth_sound(
            1,
            "beep".into(),
            &SynthParams {
                wave: Waveform::Square,
                freq: 440.0,
                duration: 0.05,
                ..Default::default()
            },
        );
        let sid = mixer.add_sound(sound);

        let vid = mixer
            .play(sid, 0.8, 1.0, 0.0, false)
            .expect("play should succeed");
        assert_eq!(mixer.voices.len(), 1);

        let mut out = vec![0i16; 512];
        mixer.mix(&mut out);

        // В выходном буфере должен появиться звук
        let non_zero = out.iter().any(|&s| s != 0);
        assert!(non_zero, "Микшер должен был записать ненулевые сэмплы");

        mixer.stop(vid);
        mixer.mix(&mut out);
        assert_eq!(
            mixer.voices.len(),
            0,
            "Остановленный голос должен быть удален"
        );
    }
}
