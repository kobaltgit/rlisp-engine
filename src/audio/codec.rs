// src/audio/codec.rs — Кодек RIFF WAVE PCM (8/16 бит, моно/стерео).

use super::synth::SoundBuffer;

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
