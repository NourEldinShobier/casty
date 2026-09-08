# Casty

Cast a Windows or macOS screen to an Android phone over the local network. No cloud, no account, no relay.

- **Host** (Windows / macOS): captures the primary display, encodes H.264 on the CPU, serves it over HTTP on port 45455 and answers UDP discovery on port 45454.
- **Viewer**: the desktop app is a frameless, always-on-top floating player (Windows / macOS); the same UI ships as an Android APK. Finds hosts with one tap, decodes with WebCodecs on the hardware decoder.
- **Sound**: system audio (WASAPI loopback / ScreenCaptureKit) streams as PCM. One button cycles PC / Here / Both; "Here" mutes the host. The volume slider adjusts Casty's own level on the viewer, or the host's volume when sound stays there.
- **Quality**: Low 640p/30, Medium 720p/30, High 1080p/60, Native full-res/60, plus display pick. The host scales and encodes per viewer at the requested size.
- **Settings window**: theme, keep on top, control auto-hide, display, cursor, fps cap, bitrate ceiling, keyframe interval, allow viewers, sound.

Works with Mullvad's *Local network sharing* enabled. Discovery uses UDP broadcast, which that option explicitly allows.

## Build

Prereqs: Rust, Node 22, `npm i -g @tauri-apps/cli`, NASM (for OpenH264 assembly; without it encoding is ~3x slower).

```bash
# Windows installer (run from the repo root). NASM must be findable; on Windows set it explicitly:
NASM='C:\Users\<you>\AppData\Local\bin\NASM\nasm.exe' npx tauri build

# Android APK (needs Android SDK + NDK + JDK 21)
NDK_HOME=<sdk>/ndk/<version> npx tauri android build --apk --target aarch64

# macOS, Apple Silicon (run on the Mac)
brew install nasm
rustup target add aarch64-apple-darwin
npx tauri build --target aarch64-apple-darwin
```

Set `CASTY_DEBUG=1` to print per-second pipeline stats (fps, convert ms, encode ms, bitrate) on the host, plus `ui:` lines from the webview.

After editing anything in `ui/`, `touch src-tauri/src/lib.rs` before `cargo build`, or the old assets stay embedded. `npm run icons` regenerates `ui/icons.js` from lucide-static.

## How it streams

`capture.rs` (windows-capture on Windows, scap on macOS) → `fast_image_resize` (SIMD scale) → `yuv` (SIMD BGRA→I420) → OpenH264 (size-limited slices, multi-threaded) → chunked HTTP response of `[len u32][packet]` → `VideoDecoder` / Web Audio in the viewer. Control messages go back as `POST /ctl?sid=`.

Why HTTP and not WebSocket: WebView2 refuses `ws://` from the app origin (`http://tauri.localhost` counts as a secure context). Plain HTTP streaming works in every webview and needs no extra plugin.

Backpressure: if the socket falls behind, frames are dropped and the encoder is asked for an IDR. The viewer does the same when its decode queue grows.

## Known limits

- Software encode. Hardware encode (NVENC / Media Foundation / VideoToolbox) is the next step if CPU use matters.
- Windows Graphics Capture only delivers frames when the screen changes, so a static desktop streams at ~0 fps by design. WASAPI loopback likewise sends nothing while the PC is silent.
- No input. View only.
