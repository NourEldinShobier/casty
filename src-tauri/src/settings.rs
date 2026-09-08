//! User settings, persisted as JSON in the OS config dir.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Display index to cast, 0-based.
    pub display: usize,
    pub show_cursor: bool,
    pub fps_cap: u32,
    /// Bitrate ceiling per viewer in kbit/s; a viewer's preset is clamped to it.
    pub max_kbps: u32,
    pub keyframe_secs: u32,
    pub keep_on_top: bool,
    pub hide_controls_ms: u32,
    /// Send system sound to viewers.
    pub audio: bool,
    /// Accept viewers at all.
    pub allow_viewers: bool,
    /// "system" | "light" | "dark"
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: 0,
            show_cursor: true,
            fps_cap: 60,
            max_kbps: 20_000,
            keyframe_secs: 10,
            keep_on_top: true,
            hide_controls_ms: 2000,
            audio: true,
            allow_viewers: true,
            theme: "system".into(),
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("casty").join("settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> Result<(), String> {
    let p = path().ok_or("no config dir")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_vec_pretty(s).unwrap()).map_err(|e| e.to_string())
}
