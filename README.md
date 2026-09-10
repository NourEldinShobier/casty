# Casty

Cast a Windows or macOS screen to an Android phone over the local network. No cloud, no account, no relay.

- **Host** (Windows / macOS): captures the primary display, encodes H.264 on the CPU, serves it over HTTP on port 45455 and answers UDP discovery on port 45454.
- **Viewer**: the desktop app is a frameless, always-on-top floating player (Windows / macOS); the same UI ships as an Android APK. Finds hosts with one tap, decodes with WebCodecs on the hardware decoder.
- **Sound**: system audio (WASAPI loopback / ScreenCaptureKit) streams as PCM. One button cycles PC / Here / Both; "Here" mutes the host. The volume slider adjusts Casty's own level on the viewer, or the host's volume when sound stays there.
- **Quality**: Low 640p/30, Medium 720p/30, High 1080p/60, Native full-res/60, plus display pick. The host scales and encodes per viewer at the requested size.
- **Remote control**: the viewer can drive the host's mouse and keyboard. Off on the host until switched on, since watching is passive and control is not. Input is queued in the viewer and flushed once per frame in a single request; consecutive pointer moves collapse into the newest one, so a fast gesture costs one line while clicks and keys keep their order.
- **Settings window**: theme, keep on top, control auto-hide, display, cursor, fps cap, bitrate ceiling, keyframe interval, allow viewers, allow remote control, sound.

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

Input travels back on that same control channel, one command per line: `m <x> <y>` with coordinates
normalised 0..1 across the display, `d`/`u` for buttons, `s` for scroll, `kd`/`ku` carrying a DOM
`KeyboardEvent.code`, and `rel` to release everything. Physical `code` values are used rather than
`key`, so a different keyboard layout on the viewer still lands on the right physical key, and
normalised coordinates mean the viewer never needs to know the host's resolution or DPI.

Why HTTP and not WebSocket: WebView2 refuses `ws://` from the app origin (`http://tauri.localhost` counts as a secure context). Plain HTTP streaming works in every webview and needs no extra plugin.

Backpressure: if the socket falls behind, frames are dropped and the encoder is asked for an IDR. The viewer does the same when its decode queue grows.

## Known limits

- Software encode. Hardware encode (NVENC / Media Foundation / VideoToolbox) is the next step if CPU use matters.
- Windows Graphics Capture only delivers frames when the screen changes, so a static desktop streams at ~0 fps by design. WASAPI loopback likewise sends nothing while the PC is silent.
- Remote control drives the primary display only. A session watching any other display cannot take control.
- Ctrl+Alt+Del cannot be injected: it is a Secure Attention Sequence that no ordinary process can synthesise. Ctrl+Shift+Esc reaches Task Manager instead.
- macOS needs Accessibility access before it can be controlled, separately from Screen Recording. Settings shows a button that opens the right pane.
- Ctrl+Alt+Shift+C is kept local while controlling, so there is always a way out of a captured keyboard.
- No clipboard sync yet.
