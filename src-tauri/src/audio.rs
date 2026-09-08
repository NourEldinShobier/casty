//! Windows system-audio loopback via WASAPI (cpal treats an output device opened for input as loopback).
#![cfg(target_os = "windows")]
use crate::capture::AudioChunk;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

pub fn start_loopback(tx: SyncSender<AudioChunk>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(dev) = host.default_output_device() else { return eprintln!("casty: no output device for loopback") };
        let Ok(cfg) = dev.default_output_config() else { return eprintln!("casty: no output config") };
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as u8;
        let fmt = cfg.sample_format();
        let stream_cfg: cpal::StreamConfig = cfg.into();
        // ~20 ms per packet
        let chunk = (rate as usize / 50) * channels as usize;
        let mut acc: Vec<i16> = Vec::with_capacity(chunk * 2);
        let mut push = move |samples: Vec<i16>| {
            acc.extend(samples);
            while acc.len() >= chunk {
                let pcm: Vec<i16> = acc.drain(..chunk).collect();
                let _ = tx.try_send(AudioChunk { rate, channels, pcm });
            }
        };
        let err = |e| eprintln!("casty: audio stream error: {e}");
        let stream = match fmt {
            cpal::SampleFormat::F32 => dev.build_input_stream(
                &stream_cfg,
                move |d: &[f32], _| push(d.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_input_stream(&stream_cfg, move |d: &[i16], _| push(d.to_vec()), err, None),
            other => return eprintln!("casty: unsupported audio format {other:?}"),
        };
        let Ok(stream) = stream else { return eprintln!("casty: loopback stream failed") };
        if stream.play().is_err() {
            return;
        }
        while !stop.load(Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
}
