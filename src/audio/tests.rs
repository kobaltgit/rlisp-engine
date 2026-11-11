// src/audio/tests.rs — Модульные тесты для кодека, синтезатора и микшера.

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
