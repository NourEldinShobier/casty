//! Master output volume and mute of this machine. Used when sound is routed away from the host.

#[cfg(target_os = "windows")]
mod imp {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let d = e.GetDefaultAudioEndpoint(eRender, eConsole)?;
            d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
        }
    }
    pub fn set_mute(m: bool) -> Result<(), String> {
        unsafe { endpoint().and_then(|v| v.SetMute(m, std::ptr::null())).map_err(|e| e.to_string()) }
    }
    pub fn set_volume(pct: u8) -> Result<(), String> {
        unsafe {
            endpoint()
                .and_then(|v| v.SetMasterVolumeLevelScalar(pct.min(100) as f32 / 100.0, std::ptr::null()))
                .map_err(|e| e.to_string())
        }
    }
    pub fn volume() -> Option<u8> {
        unsafe { endpoint().and_then(|v| v.GetMasterVolumeLevelScalar()).ok().map(|f| (f * 100.0).round() as u8) }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    fn osa(script: &str) -> Result<String, String> {
        let out = std::process::Command::new("osascript").args(["-e", script]).output().map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    pub fn set_mute(m: bool) -> Result<(), String> {
        osa(&format!("set volume output muted {m}")).map(|_| ())
    }
    pub fn set_volume(pct: u8) -> Result<(), String> {
        osa(&format!("set volume output volume {}", pct.min(100))).map(|_| ())
    }
    pub fn volume() -> Option<u8> {
        osa("output volume of (get volume settings)").ok()?.parse().ok()
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    pub fn set_mute(_: bool) -> Result<(), String> { Err("unsupported".into()) }
    pub fn set_volume(_: u8) -> Result<(), String> { Err("unsupported".into()) }
    pub fn volume() -> Option<u8> { None }
}

pub use imp::{set_mute, set_volume, volume};
