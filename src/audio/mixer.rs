// src/audio/mixer.rs — Софтверный аудиомикшер и управление голосами.

use std::collections::HashMap;
use super::synth::SoundBuffer;

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

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
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
    pub fn play(&mut self, sound_id: u64, volume: f32, pitch: f32, pan: f32, looping: bool) -> Result<u64, String> {
        let sound = self.sounds.get(&sound_id)
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

        self.voices.retain(|v| v.is_active);

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
