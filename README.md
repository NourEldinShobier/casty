# Casty

Cast a Windows or macOS screen to an Android phone over the local network. No cloud, no account, no relay.

- **Host** (Windows / macOS): captures the primary display, encodes H.264 on the CPU, serves it over WebSocket on port 45455 and answers UDP discovery on port 45454.
- **Viewer** (Android, or the desktop app itself): finds hosts with one tap, decodes with WebCodecs on the phone's hardware decoder, draws to a canvas.
- **Quality**: Low 640p/30, Medium 720p/30, High 1080p/60, Native full-res/60. The host scales and encodes per viewer at the requested size.

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

Set `CASTY_DEBUG=1` to print per-second pipeline stats (fps, convert ms, encode ms, bitrate) on the host.

## How it streams

`capture.rs` (windows-capture on Windows, scap on macOS) → `fast_image_resize` (SIMD scale) → `yuv` (SIMD BGRA→I420) → OpenH264 (size-limited slices, multi-threaded) → WebSocket packets `[keyframe u8][pts_ms u64][Annex-B NALs]` → `VideoDecoder` in the viewer.

Backpressure: if the socket falls behind, frames are dropped and the encoder is asked for an IDR. The viewer does the same when its decode queue grows.

## Known limits

- Software encode. Hardware encode (NVENC / Media Foundation / VideoToolbox) is the next step if CPU use matters.
- Primary display only. Windows Graphics Capture only delivers frames when the screen changes, so a static desktop streams at ~0 fps by design.
- No audio, no input. View only.
