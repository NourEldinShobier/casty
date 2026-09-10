//! Host: captures the screen (and sound) and streams over a chunked HTTP response
//! (WebView2 refuses ws:// from the app origin; plain HTTP works in every webview).
//! Stream body: repeated [len u32 LE][packet]
//! Video packet: [flags u8: bit0 keyframe][pts_ms u64 LE][Annex-B NALs]
//! Audio packet: [2u8][pts_ms u64 LE][rate u32 LE][channels u8][i16 LE samples, interleaved]
//! Viewer -> host: POST /ctl?sid=N with text "kf" | "audio:0|1" | "pause:0|1" | "route:N" | "vol:NN"
use crate::capture::{Capture, Options};
use crate::settings::Settings;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashMap;
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType,
};
use openh264::formats::YUVSlices;
use openh264::{OpenH264API, Timestamp};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::mpsc;
use yuv::{YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

pub const PORT: u16 = 45455;

#[derive(Serialize, Clone)]
pub struct Viewer {
    pub id: u64,
    pub name: String,
    pub label: String,
    pub audio: bool,
}

/// Where the host's sound plays. 0 = here (host speakers), 1 = viewers, 2 = both.
#[derive(Clone, Default)]
pub struct Shared {
    pub viewers: Arc<Mutex<Vec<Viewer>>>,
    /// Set when a listening port could not be opened, so the UI can say so instead of looking healthy.
    pub net_error: Arc<Mutex<Option<String>>>,
    pub sessions: Arc<Mutex<HashMap<u64, Arc<Ctl>>>>,
    pub settings: Arc<RwLock<Settings>>,
    pub route: Arc<AtomicU8>,
    pub muted_by_us: Arc<AtomicBool>,
}

impl Shared {
    pub fn set_route(&self, r: u8) {
        self.route.store(r, Relaxed);
        // sound leaves the host only when routed to viewers alone
        let mute = r == 1;
        if mute != self.muted_by_us.load(Relaxed) {
            if crate::sysaudio::set_mute(mute).is_ok() {
                self.muted_by_us.store(mute, Relaxed);
            }
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct Quality {
    #[serde(default)]
    width: u32,
    #[serde(default = "d_fps")]
    fps: u32,
    #[serde(default = "d_kbps")]
    kbps: u32,
    #[serde(default)]
    display: Option<usize>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    audio: u8,
}
fn d_fps() -> u32 {
    30
}
fn d_kbps() -> u32 {
    4000
}

pub fn name() -> String {
    hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|_| "Casty host".into())
}

pub fn ips() -> Vec<String> {
    local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, ip)| match ip {
            std::net::IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4.to_string()),
            _ => None,
        })
        .collect()
}

pub async fn serve(shared: Shared) {
    let info = shared.clone();
    let app = Router::new()
        .route(
            "/info",
            get(move || {
                let s = info.clone();
                async move {
                    let d = crate::capture::displays();
                    let cur = s.settings.read().unwrap().display;
                    // the viewer webview has its own origin, so allow cross-origin reads
                    (
                        [("access-control-allow-origin", "*")],
                        Json(serde_json::json!({
                            "name": name(), "port": PORT, "os": std::env::consts::OS,
                            "displays": d, "display": cur,
                            "viewers": s.viewers.lock().unwrap().len(),
                            "audio": s.settings.read().unwrap().audio,
                        })),
                    )
                }
            }),
        )
        .route("/stream", get(stream))
        .route("/ctl", post(ctl))
        .with_state(shared.clone());
    match tokio::net::TcpListener::bind(("0.0.0.0", PORT)).await {
        Ok(l) => {
            let _ = axum::serve(l, app).await;
        }
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                "Casty is already running on this machine, so this window cannot be watched.".to_string()
            } else {
                format!("Port {PORT} could not be opened: {e}")
            };
            eprintln!("casty: {msg}");
            *shared.net_error.lock().unwrap() = Some(msg);
        }
    }
}

#[derive(Deserialize)]
pub struct Sid {
    sid: u64,
}

const CORS: [(&str, &str); 3] = [
    ("access-control-allow-origin", "*"),
    ("access-control-expose-headers", "x-casty-session"),
    ("cache-control", "no-store"),
];

async fn ctl(Query(Sid { sid }): Query<Sid>, State(s): State<Shared>, body: String) -> impl IntoResponse {
    let c = s.sessions.lock().unwrap().get(&sid).cloned();
    if let Some(c) = c {
        control(body.trim(), &c, &s);
    }
    (CORS, "ok")
}

pub struct Ctl {
    stop: AtomicBool,
    want_key: AtomicBool,
    paused: AtomicBool,
    audio: AtomicBool,
}

async fn stream(Query(q): Query<Quality>, State(s): State<Shared>) -> impl IntoResponse {
    if !s.settings.read().unwrap().allow_viewers {
        return (axum::http::StatusCode::FORBIDDEN, CORS, [("x-casty-session", "0".to_string())], Body::empty());
    }
    let id = rand_id();
    let settings = s.settings.read().unwrap().clone();
    let label = if q.width == 0 { "native".into() } else { format!("{}p", q.width * 9 / 16) };
    s.viewers.lock().unwrap().push(Viewer {
        id,
        name: if q.name.is_empty() { "viewer".into() } else { q.name.clone() },
        label: format!("{label} - {} fps", q.fps.min(settings.fps_cap)),
        audio: q.audio != 0,
    });
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(2);
    let (atx, mut arx) = mpsc::channel::<Vec<u8>>(8);
    let ctl = Arc::new(Ctl {
        stop: AtomicBool::new(false),
        want_key: AtomicBool::new(true),
        paused: AtomicBool::new(false),
        audio: AtomicBool::new(q.audio != 0 && settings.audio),
    });
    s.sessions.lock().unwrap().insert(id, ctl.clone());
    {
        let (ctl, q, settings) = (ctl.clone(), q.clone(), settings.clone());
        std::thread::spawn(move || pipeline(q, settings, tx, atx, ctl));
    }
    let (btx, brx) = mpsc::channel::<Result<Bytes, std::convert::Infallible>>(4);
    let shared = s.clone();
    tokio::spawn(async move {
        let frame = |p: Vec<u8>| {
            let mut b = Vec::with_capacity(p.len() + 4);
            b.extend_from_slice(&(p.len() as u32).to_le_bytes());
            b.extend_from_slice(&p);
            Bytes::from(b)
        };
        loop {
            let pkt = tokio::select! {
                v = rx.recv() => v,
                a = arx.recv() => a,
                _ = btx.closed() => None, // viewer went away; do not wait for the next frame to notice
            };
            let Some(p) = pkt else { break };
            // send().await blocks while the viewer is slow, which backs up into the encoder's try_send drop
            if btx.send(Ok(frame(p))).await.is_err() {
                break;
            }
        }
        ctl.stop.store(true, Relaxed);
        shared.sessions.lock().unwrap().remove(&id);
        shared.viewers.lock().unwrap().retain(|v| v.id != id);
        if shared.viewers.lock().unwrap().is_empty() {
            shared.set_route(0);
        }
    });
    (
        axum::http::StatusCode::OK,
        CORS,
        [("x-casty-session", id.to_string())],
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(brx)),
    )
}

fn control(t: &str, ctl: &Ctl, s: &Shared) {
    let (k, v) = t.split_once(':').unwrap_or((t, ""));
    match k {
        "kf" => ctl.want_key.store(true, Relaxed),
        "audio" => ctl.audio.store(v == "1" && s.settings.read().unwrap().audio, Relaxed),
        "pause" => {
            ctl.paused.store(v == "1", Relaxed);
            ctl.want_key.store(true, Relaxed);
        }
        "route" => s.set_route(v.parse().unwrap_or(0)),
        "vol" => {
            if let Ok(p) = v.parse::<u8>() {
                let _ = crate::sysaudio::set_volume(p);
            }
        }
        _ => {}
    }
}

fn rand_id() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn target_dims(sw: u32, sh: u32, want_w: u32) -> (u32, u32) {
    let (w, h) = if want_w == 0 || want_w >= sw { (sw, sh) } else { (want_w, sh * want_w / sw) };
    (w & !1, h & !1)
}

fn make_encoder(w: u32, h: u32, fps: u32, kbps: u32, keyframe_secs: u32) -> Result<Encoder, openh264::Error> {
    let cfg = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(kbps * 1000))
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .skip_frames(false)
        .intra_frame_period(IntraFramePeriod::from_num_frames(fps * keyframe_secs.max(1)))
        .complexity(if w * h >= 1920 * 1080 || fps >= 60 { Complexity::Low } else { Complexity::Medium })
        // size-limited slices are the only mode OpenH264 encodes in parallel; the cap is far above any real slice
        .max_slice_len(1 << 20)
        .num_threads(if w * h >= 1920 * 1080 { 4 } else { 2 });
    Encoder::with_api_config(OpenH264API::from_source(), cfg)
}

/// Capture -> scale -> I420 -> H.264 on a dedicated thread; audio chunks forwarded from the capture. One per viewer.
fn pipeline(q: Quality, st: Settings, tx: mpsc::Sender<Vec<u8>>, atx: mpsc::Sender<Vec<u8>>, ctl: Arc<Ctl>) {
    let fps = q.fps.clamp(1, st.fps_cap.max(1));
    let kbps = q.kbps.min(st.max_kbps).max(200);
    let cap = match Capture::start(Options {
        display: q.display.unwrap_or(st.display),
        fps,
        cursor: st.show_cursor,
        audio: st.audio,
    }) {
        Ok(c) => c,
        Err(e) => return eprintln!("casty: capture failed: {e}"),
    };

    let mut enc: Option<(Encoder, u32, u32)> = None;
    let mut packed: Vec<u8> = Vec::new();
    let mut scaled: Vec<u8> = Vec::new();
    let mut yuv = YuvPlanarImageMut::<u8>::alloc(2, 2, YuvChromaSubsampling::Yuv420);
    let mut resizer = Resizer::new();
    let resize_opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)).use_alpha(false);
    let t0 = Instant::now();
    let mut out = Vec::with_capacity(1 << 20);
    let debug = std::env::var_os("CASTY_DEBUG").is_some();
    let (mut st_t, mut st_n, mut st_conv, mut st_enc, mut st_bytes) = (Instant::now(), 0u32, 0f64, 0f64, 0usize);

    while !ctl.stop.load(Relaxed) {
        // audio first: cheap, and it must not wait behind a slow encode
        while let Ok(a) = cap.audio_rx.try_recv() {
            if !ctl.audio.load(Relaxed) || ctl.paused.load(Relaxed) {
                continue;
            }
            let mut p = Vec::with_capacity(14 + a.pcm.len() * 2);
            p.push(2);
            p.extend_from_slice(&(t0.elapsed().as_millis() as u64).to_le_bytes());
            p.extend_from_slice(&a.rate.to_le_bytes());
            p.push(a.channels);
            for s in &a.pcm {
                p.extend_from_slice(&s.to_le_bytes());
            }
            let _ = atx.try_send(p);
        }
        let Some(mut f) = cap.next() else { continue };
        if ctl.paused.load(Relaxed) {
            continue;
        }
        let (sw, sh, stride) = (f.width, f.height, f.stride as usize);
        if sw < 2 || sh < 2 || f.data.len() < stride * sh as usize {
            continue;
        }
        let (tw, th) = target_dims(sw, sh, q.width);
        let t_frame = Instant::now();

        if enc.as_ref().map(|(_, w, h)| (*w, *h)) != Some((tw, th)) {
            match make_encoder(tw, th, fps, kbps, st.keyframe_secs) {
                Ok(e) => enc = Some((e, tw, th)),
                Err(e) => return eprintln!("casty: encoder failed: {e}"),
            }
            yuv = YuvPlanarImageMut::alloc(tw, th, YuvChromaSubsampling::Yuv420);
            ctl.want_key.store(true, Relaxed);
        }
        let (encoder, _, _) = enc.as_mut().unwrap();

        // 1. scale, only when the viewer asked for less than native
        let (bgra, bgra_stride): (&[u8], u32) = if (tw, th) == (sw, sh) {
            (&f.data, stride as u32)
        } else {
            let src: &mut [u8] = if stride == sw as usize * 4 {
                &mut f.data
            } else {
                packed.clear();
                for row in f.data.chunks(stride).take(sh as usize) {
                    packed.extend_from_slice(&row[..sw as usize * 4]);
                }
                &mut packed
            };
            scaled.resize(tw as usize * th as usize * 4, 0);
            let (Ok(s), Ok(mut d)) =
                (Image::from_slice_u8(sw, sh, src, PixelType::U8x4), Image::from_slice_u8(tw, th, &mut scaled, PixelType::U8x4))
            else {
                continue;
            };
            if resizer.resize(&s, &mut d, &resize_opts).is_err() {
                continue;
            }
            (&scaled, tw * 4)
        };

        // 2. BGRA -> I420 (SIMD)
        if yuv::bgra_to_yuv420(&mut yuv, bgra, bgra_stride, YuvRange::Limited, YuvStandardMatrix::Bt709, YuvConversionMode::Balanced)
            .is_err()
        {
            continue;
        }
        let t_conv = t_frame.elapsed().as_secs_f64() * 1e3;

        // 3. encode
        if ctl.want_key.swap(false, Relaxed) {
            encoder.force_intra_frame();
        }
        let pts_ms = t0.elapsed().as_millis() as u64;
        let slices = YUVSlices::new(
            (yuv.y_plane.borrow(), yuv.u_plane.borrow(), yuv.v_plane.borrow()),
            (tw as usize, th as usize),
            (yuv.y_stride as usize, yuv.u_stride as usize, yuv.v_stride as usize),
        );
        let Ok(bs) = encoder.encode_at(&slices, Timestamp::from_millis(pts_ms)) else { continue };
        let ft = bs.frame_type();
        if matches!(ft, FrameType::Skip | FrameType::Invalid) {
            continue;
        }
        out.clear();
        out.push(matches!(ft, FrameType::IDR) as u8);
        out.extend_from_slice(&pts_ms.to_le_bytes());
        bs.write_vec(&mut out);
        if debug {
            st_n += 1;
            st_conv += t_conv;
            st_enc += t_frame.elapsed().as_secs_f64() * 1e3 - t_conv;
            st_bytes += out.len();
            if st_t.elapsed().as_secs() >= 1 {
                eprintln!(
                    "casty: {tw}x{th} {st_n} fps  convert {:.1} ms  encode {:.1} ms  {:.1} Mbit/s",
                    st_conv / st_n as f64,
                    st_enc / st_n as f64,
                    st_bytes as f64 * 8.0 / 1e6 / st_t.elapsed().as_secs_f64()
                );
                st_t = Instant::now();
                st_n = 0;
                st_conv = 0.0;
                st_enc = 0.0;
                st_bytes = 0;
            }
        }

        // 4. ship; if the socket is behind, drop this frame and resync on the next IDR
        match tx.try_send(out.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => ctl.want_key.store(true, Relaxed),
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dims_are_even_and_scaled() {
        assert_eq!(super::target_dims(2560, 1440, 1280), (1280, 720));
        assert_eq!(super::target_dims(1921, 1081, 0), (1920, 1080));
        assert_eq!(super::target_dims(1280, 720, 1920), (1280, 720));
    }
}
