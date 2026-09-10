mod discover;
#[cfg(not(target_os = "android"))]
mod audio;
#[cfg(not(target_os = "android"))]
mod capture;
#[cfg(not(target_os = "android"))]
mod host;
#[cfg(not(target_os = "android"))]
mod settings;
#[cfg(not(target_os = "android"))]
mod sysaudio;

use serde::Serialize;

#[tauri::command]
fn role() -> &'static str {
    if cfg!(target_os = "android") { "viewer" } else { "host" }
}

#[tauri::command]
fn log(msg: String) {
    eprintln!("ui: {msg}");
}

#[tauri::command]
async fn discover() -> Vec<discover::Device> {
    tokio::task::spawn_blocking(|| discover::probe(std::time::Duration::from_millis(1200))).await.unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
mod desktop {
    use super::*;
    use tauri::{Emitter, State};

    #[derive(Serialize)]
    pub struct HostInfo {
        name: String,
        port: u16,
        ips: Vec<String>,
        viewers: Vec<host::Viewer>,
        permission: bool,
        route: u8,
        volume: Option<u8>,
        os: &'static str,
        displays: Vec<capture::DisplayInfo>,
        net_error: Option<String>,
    }

    #[tauri::command]
    pub fn host_info(s: State<host::Shared>) -> HostInfo {
        HostInfo {
            name: host::name(),
            port: host::PORT,
            ips: host::ips(),
            viewers: s.viewers.lock().unwrap().clone(),
            permission: capture::has_permission(),
            route: s.route.load(std::sync::atomic::Ordering::Relaxed),
            volume: sysaudio::volume(),
            os: std::env::consts::OS,
            displays: capture::displays(),
            net_error: s.net_error.lock().unwrap().clone(),
        }
    }

    #[tauri::command]
    pub fn request_permission() -> bool {
        capture::request_permission()
    }

    /// macOS hands screen-recording access to a newly launched process, not a running one.
    #[tauri::command]
    pub fn relaunch(app: tauri::AppHandle) {
        app.restart()
    }

    #[tauri::command]
    pub fn set_route(s: State<host::Shared>, route: u8) {
        s.set_route(route)
    }

    #[tauri::command]
    pub fn set_volume(pct: u8) -> Result<(), String> {
        sysaudio::set_volume(pct)
    }

    #[tauri::command]
    pub fn get_settings(s: State<host::Shared>) -> settings::Settings {
        s.settings.read().unwrap().clone()
    }

    #[tauri::command]
    pub fn set_settings(app: tauri::AppHandle, s: State<host::Shared>, settings: settings::Settings) -> Result<(), String> {
        settings::save(&settings)?;
        *s.settings.write().unwrap() = settings.clone();
        let _ = app.emit("settings", settings);
        Ok(())
    }

    #[tauri::command]
    pub fn hostname() -> String {
        host::name()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(not(target_os = "android"))]
    let builder = {
        use desktop::*;
        let shared = host::Shared { settings: std::sync::Arc::new(std::sync::RwLock::new(settings::load())), ..Default::default() };
        tauri::async_runtime::spawn(host::serve(shared.clone()));
        {
            let s = shared.clone();
            std::thread::spawn(move || {
                if let Err(e) = discover::respond_forever(host::name(), host::PORT) {
                    eprintln!("casty: {e}");
                    *s.net_error.lock().unwrap() = Some(e);
                }
            });
        }
        builder.manage(shared).invoke_handler(tauri::generate_handler![
            role,
            log,
            discover,
            host_info,
            request_permission,
            relaunch,
            set_route,
            set_volume,
            get_settings,
            set_settings,
            hostname
        ])
    };
    #[cfg(target_os = "android")]
    let builder = builder.invoke_handler(tauri::generate_handler![role, log, discover]);
    builder
        .on_window_event(|w, e| {
            // the player is the app; closing it closes settings too
            if w.label() == "main" && matches!(e, tauri::WindowEvent::Destroyed) {
                use tauri::Manager;
                w.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("casty failed to start");
}
