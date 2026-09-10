//! Remote input injection.
//!
//! One thread owns the injector; request handlers only queue text lines, so a slow
//! synthetic event can never block the HTTP server. Commands, one per line:
//!
//!   m <x> <y>    move the pointer; x and y are 0..1 across the captured display
//!   d <button>   button down   (0 left, 1 middle, 2 right, 3 back, 4 forward)
//!   u <button>   button up
//!   s <dx> <dy>  scroll in wheel lines, positive is down and right
//!   kd <code>    key down, a DOM KeyboardEvent.code such as KeyA or ShiftLeft
//!   ku <code>    key up
//!   t <text>     type text directly, for characters no physical key maps to
//!   rel          release every modifier and button this session may still hold
//!
//! Coordinates are normalised because the viewer knows the video's aspect, not the
//! host's resolution or DPI. Physical `code` values are used rather than `key` so a
//! different keyboard layout on the viewer still lands on the right physical key.
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

pub struct Injector {
    tx: Sender<String>,
}

impl Injector {
    pub fn new() -> Self {
        let (tx, rx) = channel::<String>();
        std::thread::spawn(move || {
            let mut enigo: Option<Enigo> = None;
            let mut screen = (0i32, 0i32);
            let mut measured = Instant::now() - Duration::from_secs(60);
            for line in rx {
                // Built lazily and retried: on macOS this fails until Accessibility is granted,
                // and we want it to start working the moment the user grants it.
                if enigo.is_none() {
                    enigo = Enigo::new(&Settings::default()).ok();
                }
                let Some(e) = enigo.as_mut() else { continue };
                if measured.elapsed() > Duration::from_secs(2) {
                    if let Ok(s) = e.main_display() {
                        screen = s;
                    }
                    measured = Instant::now();
                }
                if apply(e, &line, screen).is_none() {
                    // a malformed or unmappable line is dropped; never kill the thread
                }
            }
        });
        Self { tx }
    }

    pub fn send(&self, line: &str) {
        let _ = self.tx.send(line.to_string());
    }
}

impl Default for Injector {
    fn default() -> Self {
        Self::new()
    }
}

fn apply(e: &mut Enigo, line: &str, screen: (i32, i32)) -> Option<()> {
    let mut it = line.split(' ');
    let verb = it.next()?;
    match verb {
        "m" => {
            let x: f32 = it.next()?.parse().ok()?;
            let y: f32 = it.next()?.parse().ok()?;
            if screen.0 <= 0 || screen.1 <= 0 {
                return None;
            }
            let px = (x.clamp(0.0, 1.0) * screen.0 as f32) as i32;
            let py = (y.clamp(0.0, 1.0) * screen.1 as f32) as i32;
            let _ = e.move_mouse(px, py, Coordinate::Abs);
        }
        "d" | "u" => {
            let dir = if verb == "d" { Direction::Press } else { Direction::Release };
            let _ = e.button(button(it.next()?)?, dir);
        }
        "s" => {
            let dx: i32 = it.next()?.parse().ok()?;
            let dy: i32 = it.next()?.parse().ok()?;
            if dx != 0 {
                let _ = e.scroll(dx, Axis::Horizontal);
            }
            if dy != 0 {
                let _ = e.scroll(dy, Axis::Vertical);
            }
        }
        "kd" | "ku" => {
            let dir = if verb == "kd" { Direction::Press } else { Direction::Release };
            let _ = e.key(key(it.next()?)?, dir);
        }
        "t" => {
            let _ = e.text(line.splitn(2, ' ').nth(1)?);
        }
        "rel" => release_all(e),
        _ => {}
    }
    Some(())
}

/// A dropped connection must not leave a modifier or button stuck down on the host.
fn release_all(e: &mut Enigo) {
    for k in [Key::Shift, Key::Control, Key::Alt, Key::Meta] {
        let _ = e.key(k, Direction::Release);
    }
    for b in [Button::Left, Button::Middle, Button::Right] {
        let _ = e.button(b, Direction::Release);
    }
}

fn button(n: &str) -> Option<Button> {
    Some(match n {
        "0" => Button::Left,
        "1" => Button::Middle,
        "2" => Button::Right,
        "3" => Button::Back,
        "4" => Button::Forward,
        _ => return None,
    })
}

fn key(code: &str) -> Option<Key> {
    Some(match code {
        "Escape" => Key::Escape,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Enter" | "NumpadEnter" => Key::Return,
        "Space" => Key::Space,
        "ArrowUp" => Key::UpArrow,
        "ArrowDown" => Key::DownArrow,
        "ArrowLeft" => Key::LeftArrow,
        "ArrowRight" => Key::RightArrow,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "CapsLock" => Key::CapsLock,
        "ShiftLeft" | "ShiftRight" => Key::Shift,
        "ControlLeft" | "ControlRight" => Key::Control,
        "AltLeft" | "AltRight" => Key::Alt,
        "MetaLeft" | "MetaRight" => Key::Meta,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "Minus" | "NumpadSubtract" => Key::Unicode('-'),
        "Equal" => Key::Unicode('='),
        "NumpadAdd" => Key::Unicode('+'),
        "NumpadMultiply" => Key::Unicode('*'),
        "NumpadDivide" => Key::Unicode('/'),
        "NumpadDecimal" => Key::Unicode('.'),
        "BracketLeft" => Key::Unicode('['),
        "BracketRight" => Key::Unicode(']'),
        "Backslash" => Key::Unicode('\\'),
        "Semicolon" => Key::Unicode(';'),
        "Quote" => Key::Unicode('\''),
        "Backquote" => Key::Unicode('`'),
        "Comma" => Key::Unicode(','),
        "Period" => Key::Unicode('.'),
        "Slash" => Key::Unicode('/'),
        _ => {
            let rest = code
                .strip_prefix("Key")
                .or_else(|| code.strip_prefix("Digit"))
                .or_else(|| code.strip_prefix("Numpad"))?;
            let c = rest.chars().next()?;
            if !c.is_ascii_alphanumeric() || rest.chars().count() != 1 {
                return None;
            }
            Key::Unicode(c.to_ascii_lowercase())
        }
    })
}

/// macOS refuses synthetic events until the app is trusted for Accessibility.
#[cfg(target_os = "macos")]
pub fn ready() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

#[cfg(not(target_os = "macos"))]
pub fn ready() -> bool {
    true
}

/// Opens the pane the user has to flip the switch in; nothing else can grant this.
pub fn request() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_physical_codes() {
        assert!(matches!(key("KeyA"), Some(Key::Unicode('a'))));
        assert!(matches!(key("Digit7"), Some(Key::Unicode('7'))));
        assert!(matches!(key("ShiftRight"), Some(Key::Shift)));
        assert!(matches!(key("ArrowLeft"), Some(Key::LeftArrow)));
        assert!(matches!(key("F11"), Some(Key::F11)));
        assert!(key("NoSuchKey").is_none());
        assert!(key("Key").is_none());
    }

    #[test]
    fn maps_buttons() {
        assert!(matches!(button("2"), Some(Button::Right)));
        assert!(button("9").is_none());
    }
}
