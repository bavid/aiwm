//! Thin Tauri host. All logic lives in `aiwm-core`; this crate only wires the
//! window, plugins and the IPC command surface. Real commands (telemetry, jobs,
//! models, settings) land in WP-6.

use std::sync::Mutex;

use aiwm_core::app::{self, App};
use serde::Serialize;
use tracing_appender::non_blocking::WorkerGuard;

/// Logging guard parked in Tauri state so it lives for the whole process and
/// flushes buffered file output on shutdown.
struct LogGuard(#[allow(dead_code)] Mutex<WorkerGuard>);

#[derive(Debug, Serialize)]
struct AboutInfo {
    core_version: String,
    tauri_host_version: String,
    data_dir: String,
    store_path: String,
    core_api_port: u16,
    offline_mode: bool,
}

#[tauri::command]
fn about(core: tauri::State<'_, App>) -> AboutInfo {
    AboutInfo {
        core_version: aiwm_core::CORE_VERSION.to_string(),
        tauri_host_version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: core.paths.root().display().to_string(),
        store_path: core.config.store_path.display().to_string(),
        core_api_port: core.config.core_api_port,
        offline_mode: core.config.offline_mode,
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

fn try_run() -> anyhow::Result<()> {
    let (core, log_guard) = app::bootstrap_process()?;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(core)
        .manage(LogGuard(Mutex::new(log_guard)))
        .invoke_handler(tauri::generate_handler![about])
        .run(tauri::generate_context!())?;

    Ok(())
}
