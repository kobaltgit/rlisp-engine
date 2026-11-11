// src/audio/device.rs — Native Audio Device (WinMM на Windows, Silent fallback на других ОС).

use super::mixer::AudioMixer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
