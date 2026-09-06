mod discover;
#[cfg(not(target_os = "android"))]
mod capture;
#[cfg(not(target_os = "android"))]
mod host;

use serde::Serialize;

#[tauri::command]
fn role() -> &'static str {
    if cfg!(target_os = "android") { "viewer" } else { "host" }
}

#[tauri::command]
async fn discover() -> Vec<discover::Device> {
    tokio::task::spawn_blocking(|| discover::probe(std::time::Duration::from_millis(1200)))
        .await
        .unwrap_or_default()
}

#[derive(Serialize)]
struct HostInfo {
    name: String,
    port: u16,
    ips: Vec<String>,
    viewers: usize,
    permission: bool,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn host_info(s: tauri::State<host::Shared>) -> HostInfo {
    HostInfo {
        name: host::name(),
        port: host::PORT,
        ips: host::ips(),
        viewers: s.viewers.load(std::sync::atomic::Ordering::Relaxed),
        permission: capture::has_permission(),
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn request_permission() -> bool {
    capture::request_permission()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(not(target_os = "android"))]
    let builder = {
        let shared = host::Shared::default();
        tauri::async_runtime::spawn(host::serve(shared.clone()));
        std::thread::spawn(|| discover::respond_forever(host::name(), host::PORT));
        builder
            .manage(shared)
            .invoke_handler(tauri::generate_handler![role, discover, host_info, request_permission])
    };
    #[cfg(target_os = "android")]
    let builder = builder.invoke_handler(tauri::generate_handler![role, discover]);
    builder.run(tauri::generate_context!()).expect("casty failed to start");
}
