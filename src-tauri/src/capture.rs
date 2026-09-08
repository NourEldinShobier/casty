//! Display capture (and system audio on macOS, which ScreenCaptureKit delivers alongside video).
//! Video frames arrive as BGRA rows (possibly padded); audio as interleaved i16 chunks.
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Duration;

pub struct RawFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
}

pub struct AudioChunk {
    pub rate: u32,
    pub channels: u8,
    pub pcm: Vec<i16>,
}

#[derive(Serialize, Clone)]
pub struct DisplayInfo {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

pub struct Options {
    pub display: usize,
    pub fps: u32,
    pub cursor: bool,
    pub audio: bool,
}

pub struct Capture {
    pub rx: Receiver<RawFrame>,
    pub audio_rx: Receiver<AudioChunk>,
    stop: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    ctl: Option<windows_capture::capture::CaptureControl<win::Grab, Box<dyn std::error::Error + Send + Sync>>>,
}

impl Capture {
    /// Latest-frame semantics: one video frame buffered, newer ones dropped while the encoder is busy.
    pub fn start(o: Options) -> Result<Self, String> {
        let (tx, rx) = sync_channel::<RawFrame>(1);
        let (atx, audio_rx) = sync_channel::<AudioChunk>(16);
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "windows")]
        let ctl = {
            if o.audio {
                crate::audio::start_loopback(atx, stop.clone());
            }
            Some(win::start(&o, tx, stop.clone())?)
        };
        #[cfg(target_os = "macos")]
        mac::start(&o, tx, atx, stop.clone())?;
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (o, tx, atx);
            return Err("screen capture is not supported on this platform".into());
        }
        #[allow(unreachable_code)]
        Ok(Self {
            rx,
            audio_rx,
            stop,
            #[cfg(target_os = "windows")]
            ctl,
        })
    }

    pub fn next(&self) -> Option<RawFrame> {
        self.rx.recv_timeout(Duration::from_millis(100)).ok()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        #[cfg(target_os = "windows")]
        if let Some(c) = self.ctl.take() {
            let _ = c.stop();
        }
    }
}

pub fn displays() -> Vec<DisplayInfo> {
    #[cfg(target_os = "windows")]
    {
        return windows_capture::monitor::Monitor::enumerate()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, m)| DisplayInfo {
                index: i,
                name: m.name().unwrap_or_else(|_| format!("Display {}", i + 1)),
                width: m.width().unwrap_or(0),
                height: m.height().unwrap_or(0),
            })
            .collect();
    }
    #[cfg(target_os = "macos")]
    {
        return scap::get_all_targets()
            .into_iter()
            .filter_map(|t| match t {
                scap::Target::Display(d) => Some(d),
                _ => None,
            })
            .enumerate()
            .map(|(i, d)| DisplayInfo { index: i, name: d.title, width: 0, height: 0 })
            .collect();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };

    pub struct Grab {
        tx: SyncSender<RawFrame>,
        stop: Arc<AtomicBool>,
    }

    impl GraphicsCaptureApiHandler for Grab {
        type Flags = (SyncSender<RawFrame>, Arc<AtomicBool>);
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { tx: ctx.flags.0, stop: ctx.flags.1 })
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, ctl: InternalCaptureControl) -> Result<(), Self::Error> {
            if self.stop.load(Relaxed) {
                ctl.stop();
                return Ok(());
            }
            let mut fb = frame.buffer()?;
            let (width, height, stride) = (fb.width(), fb.height(), fb.row_pitch());
            // ponytail: one GPU->CPU copy per frame; zero-copy would mean encoding inside this callback
            let _ = self.tx.try_send(RawFrame { data: fb.as_raw_buffer().to_vec(), width, height, stride });
            Ok(())
        }
    }

    pub fn start(
        o: &Options,
        tx: SyncSender<RawFrame>,
        stop: Arc<AtomicBool>,
    ) -> Result<CaptureControl<Grab, Box<dyn std::error::Error + Send + Sync>>, String> {
        let monitor = Monitor::from_index(o.display + 1).or_else(|_| Monitor::primary()).map_err(|e| e.to_string())?;
        let settings = Settings::new(
            monitor,
            if o.cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor },
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / o.fps.max(1) as f64)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            (tx, stop),
        );
        Grab::start_free_threaded(settings).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use scap::capturer::{Capturer, Options as ScapOptions, Resolution};
    use scap::frame::{AudioFormat, Frame, FrameType, VideoFrame};

    pub fn start(o: &Options, tx: SyncSender<RawFrame>, atx: SyncSender<AudioChunk>, stop: Arc<AtomicBool>) -> Result<(), String> {
        let target = scap::get_all_targets()
            .into_iter()
            .filter(|t| matches!(t, scap::Target::Display(_)))
            .nth(o.display);
        let mut cap = Capturer::build(ScapOptions {
            fps: o.fps,
            show_cursor: o.cursor,
            target,
            output_type: FrameType::BGRAFrame,
            output_resolution: Resolution::Captured,
            captures_audio: o.audio,
            exclude_current_process_audio: true,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            cap.start_capture();
            while !stop.load(Relaxed) {
                match cap.get_next_frame() {
                    Ok(Frame::Video(VideoFrame::BGRA(f))) if f.height > 0 => {
                        let stride = (f.data.len() / f.height as usize) as u32;
                        let _ = tx.try_send(RawFrame { data: f.data, width: f.width as u32, height: f.height as u32, stride });
                    }
                    Ok(Frame::Audio(a)) => {
                        let ch = a.channels() as usize;
                        let n = a.sample_count();
                        let mut pcm = vec![0i16; n * ch];
                        let sample = |plane: &[u8], i: usize| -> i16 {
                            match a.format() {
                                AudioFormat::F32 => (f32::from_ne_bytes(plane[i * 4..i * 4 + 4].try_into().unwrap()).clamp(-1.0, 1.0) * 32767.0) as i16,
                                AudioFormat::I16 => i16::from_ne_bytes(plane[i * 2..i * 2 + 2].try_into().unwrap()),
                                _ => 0,
                            }
                        };
                        if a.is_planar() {
                            for c in 0..ch {
                                let plane = a.plane_data(c);
                                for i in 0..n {
                                    pcm[i * ch + c] = sample(plane, i);
                                }
                            }
                        } else {
                            let plane = a.raw_data();
                            for i in 0..n * ch {
                                pcm[i] = sample(plane, i);
                            }
                        }
                        let _ = atx.try_send(AudioChunk { rate: a.rate(), channels: ch as u8, pcm });
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            cap.stop_capture();
        });
        Ok(())
    }
}

pub fn has_permission() -> bool {
    #[cfg(target_os = "macos")]
    return scap::has_permission();
    #[cfg(not(target_os = "macos"))]
    true
}

pub fn request_permission() -> bool {
    #[cfg(target_os = "macos")]
    return scap::request_permission();
    #[cfg(not(target_os = "macos"))]
    true
}
