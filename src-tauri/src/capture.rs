//! Primary-display capture. Frames arrive as BGRA rows (possibly padded) on a channel.
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

pub struct Capture {
    pub rx: Receiver<RawFrame>,
    stop: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    ctl: Option<windows_capture::capture::CaptureControl<win::Grab, Box<dyn std::error::Error + Send + Sync>>>,
}

impl Capture {
    /// Latest-frame semantics: the channel holds one frame, newer frames replace nothing and are dropped.
    pub fn start(fps: u32) -> Result<Self, String> {
        let (tx, rx) = sync_channel::<RawFrame>(1);
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "windows")]
        let ctl = Some(win::start(fps, tx, stop.clone())?);
        #[cfg(target_os = "macos")]
        mac::start(fps, tx, stop.clone())?;
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (fps, tx);
            return Err("screen capture is not supported on this platform".into());
        }
        #[allow(unreachable_code)]
        Ok(Self {
            rx,
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
        fps: u32,
        tx: SyncSender<RawFrame>,
        stop: Arc<AtomicBool>,
    ) -> Result<CaptureControl<Grab, Box<dyn std::error::Error + Send + Sync>>, String> {
        let monitor = Monitor::primary().map_err(|e| e.to_string())?;
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / fps.max(1) as f64)),
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
    use scap::capturer::{Capturer, Options, Resolution};
    use scap::frame::{Frame, FrameType, VideoFrame};

    pub fn start(fps: u32, tx: SyncSender<RawFrame>, stop: Arc<AtomicBool>) -> Result<(), String> {
        let mut cap = Capturer::build(Options {
            fps,
            show_cursor: true,
            output_type: FrameType::BGRAFrame,
            output_resolution: Resolution::Captured,
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
