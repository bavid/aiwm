//! Thin Tauri host. All logic lives in `aiwm-core`; this crate only wires the
//! window, plugins and the IPC command surface. Real commands (telemetry, jobs,
//! models, settings) land in WP-6.

use serde::Serialize;

#[derive(Debug, Serialize)]
struct AboutInfo {
    core_version: String,
    tauri_host_version: String,
}

#[tauri::command]
fn about() -> AboutInfo {
    AboutInfo {
        core_version: aiwm_core::CORE_VERSION.to_string(),
        tauri_host_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Entry point called by `main`. Errors are printed and turned into a non-zero
/// exit rather than a panic.
pub fn run() {
    if let Err(err) = try_run() {
        eprintln!("aiwm-tauri: fatal: {err}");
        std::process::exit(1);
    }
}

fn try_run() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![about])
        .run(tauri::generate_context!())
}
