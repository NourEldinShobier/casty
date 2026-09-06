//! Host: captures the screen and streams H.264 Annex-B over WebSocket.
//! Packet layout: [flags u8: bit0 = keyframe][pts_ms u64 LE][NAL units...]
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
    RateControlMode, UsageType,
};
use openh264::formats::YUVSlices;
use openh264::{OpenH264API, Timestamp};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use yuv::{YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

pub const PORT: u16 = 45455;

#[derive(Clone, Default)]
pub struct Shared {
    pub viewers: Arc<AtomicUsize>,
}

#[derive(Deserialize, Clone, Copy)]
pub struct Quality {
    /// Output width in pixels; 0 = native.
    #[serde(default)]
    width: u32,
    #[serde(default = "d_fps")]
    fps: u32,
    #[serde(default = "d_kbps")]
    kbps: u32,
}
fn d_fps() -> u32 {
    30
}
fn d_kbps() -> u32 {
    4000
}

pub fn name() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "Casty host".into())
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
    let app = Router::new()
        .route("/info", get(|| async { Json(serde_json::json!({ "name": name(), "port": PORT })) }))
        .route("/stream", get(stream))
        .with_state(shared);
    match tokio::net::TcpListener::bind(("0.0.0.0", PORT)).await {
        Ok(l) => {
            let _ = axum::serve(l, app).await;
        }
        Err(e) => eprintln!("casty: bind {PORT} failed: {e}"),
    }
}

async fn stream(ws: WebSocketUpgrade, Query(q): Query<Quality>, State(s): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |sock| session(sock, q, s))
}

async fn session(mut sock: WebSocket, q: Quality, s: Shared) {
    s.viewers.fetch_add(1, Relaxed);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(2);
    let stop = Arc::new(AtomicBool::new(false));
    let want_key = Arc::new(AtomicBool::new(true));
    {
        let (stop, want_key) = (stop.clone(), want_key.clone());
        std::thread::spawn(move || pipeline(q, tx, stop, want_key));
    }
    loop {
        tokio::select! {
            pkt = rx.recv() => match pkt {
                Some(p) => if sock.send(Message::Binary(p.into())).await.is_err() { break },
                None => break,
            },
            msg = sock.recv() => match msg {
                Some(Ok(Message::Text(t))) if t.as_str() == "kf" => want_key.store(true, Relaxed),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
        }
    }
    stop.store(true, Relaxed);
    s.viewers.fetch_sub(1, Relaxed);
}

fn target_dims(sw: u32, sh: u32, want_w: u32) -> (u32, u32) {
    let (w, h) = if want_w == 0 || want_w >= sw { (sw, sh) } else { (want_w, sh * want_w / sw) };
    (w & !1, h & !1)
}

fn make_encoder(w: u32, h: u32, q: Quality) -> Result<Encoder, openh264::Error> {
    let cfg = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(q.kbps * 1000))
        .max_frame_rate(FrameRate::from_hz(q.fps as f32))
        .skip_frames(false)
        .intra_frame_period(IntraFramePeriod::from_num_frames(q.fps * 10))
        .complexity(if w * h >= 1920 * 1080 || q.fps >= 60 { Complexity::Low } else { Complexity::Medium })
        // size-limited slices are the only mode OpenH264 encodes in parallel; the cap is far above any real slice
        .max_slice_len(1 << 20)
        .num_threads(if w * h >= 1920 * 1080 { 4 } else { 2 });
    Encoder::with_api_config(OpenH264API::from_source(), cfg)
}

/// Capture -> scale -> I420 -> H.264 on a dedicated thread. One per viewer.
fn pipeline(q: Quality, tx: mpsc::Sender<Vec<u8>>, stop: Arc<AtomicBool>, want_key: Arc<AtomicBool>) {
    let cap = match crate::capture::Capture::start(q.fps.clamp(1, 120)) {
        Ok(c) => c,
        Err(e) => return eprintln!("casty: capture failed: {e}"),
    };

    let mut enc: Option<(Encoder, u32, u32)> = None;
    let mut packed: Vec<u8> = Vec::new();
    let mut scaled: Vec<u8> = Vec::new();
    let mut yuv = YuvPlanarImageMut::<u8>::alloc(2, 2, YuvChromaSubsampling::Yuv420);
    let mut resizer = Resizer::new();
    let resize_opts = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
        .use_alpha(false);
    let t0 = Instant::now();
    let mut out = Vec::with_capacity(1 << 20);
    let debug = std::env::var_os("CASTY_DEBUG").is_some();
    let (mut st_t, mut st_n, mut st_conv, mut st_enc, mut st_bytes) = (Instant::now(), 0u32, 0f64, 0f64, 0usize);

    while !stop.load(Relaxed) {
        let Some(mut f) = cap.next() else { continue };
        let (sw, sh, stride) = (f.width, f.height, f.stride as usize);
        if sw < 2 || sh < 2 || f.data.len() < stride * sh as usize {
            continue;
        }
        let (tw, th) = target_dims(sw, sh, q.width);
        let t_frame = Instant::now();

        if enc.as_ref().map(|(_, w, h)| (*w, *h)) != Some((tw, th)) {
            match make_encoder(tw, th, q) {
                Ok(e) => enc = Some((e, tw, th)),
                Err(e) => return eprintln!("casty: encoder failed: {e}"),
            }
            yuv = YuvPlanarImageMut::alloc(tw, th, YuvChromaSubsampling::Yuv420);
            want_key.store(true, Relaxed);
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
            let (Ok(s), Ok(mut d)) = (
                Image::from_slice_u8(sw, sh, src, PixelType::U8x4),
                Image::from_slice_u8(tw, th, &mut scaled, PixelType::U8x4),
            ) else {
                continue;
            };
            if resizer.resize(&s, &mut d, &resize_opts).is_err() {
                continue;
            }
            (&scaled, tw * 4)
        };

        // 2. BGRA -> I420 (SIMD)
        if yuv::bgra_to_yuv420(
            &mut yuv,
            bgra,
            bgra_stride,
            YuvRange::Limited,
            YuvStandardMatrix::Bt709,
            YuvConversionMode::Balanced,
        )
        .is_err()
        {
            continue;
        }

        let t_conv = t_frame.elapsed().as_secs_f64() * 1e3;

        // 3. encode
        if want_key.swap(false, Relaxed) {
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
            st_n += 1; st_conv += t_conv; st_enc += t_frame.elapsed().as_secs_f64() * 1e3 - t_conv; st_bytes += out.len();
            if st_t.elapsed().as_secs() >= 1 {
                eprintln!("casty: {tw}x{th} {st_n} fps  convert {:.1} ms  encode {:.1} ms  {:.1} Mbit/s", st_conv / st_n as f64, st_enc / st_n as f64, st_bytes as f64 * 8.0 / 1e6 / st_t.elapsed().as_secs_f64());
                st_t = Instant::now(); st_n = 0; st_conv = 0.0; st_enc = 0.0; st_bytes = 0;
            }
        }

        // 4. ship; if the socket is behind, drop this frame and resync on the next IDR
        match tx.try_send(out.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => want_key.store(true, Relaxed),
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
