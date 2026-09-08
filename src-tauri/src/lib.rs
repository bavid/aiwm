//! Thin Tauri host. All logic lives in `aiwm-core`; this crate wires the window,
//! plugins, and IPC commands that forward to `aiwm_core::api::handlers` — the
//! same functions the loopback HTTP server uses, so both transports return
//! identical JSON.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use aiwm_core::api::dto::AboutDto;
use aiwm_core::api::handlers;
use aiwm_core::db::{Job, JobFilter};
use aiwm_core::orchestrator::JobState;
use aiwm_core::telemetry::SystemTelemetry;
use aiwm_core::{api, app, App};
use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

/// Logging guard + background services, parked in Tauri state for the process
/// lifetime.
struct Runtime {
    _log_guard: Mutex<WorkerGuard>,
    _services: api::Services,
}

fn to_ipc<T>(r: aiwm_core::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

#[tauri::command]
fn about(app: tauri::State<'_, Arc<App>>) -> AboutDto {
    handlers::about(&app)
}

#[tauri::command]
fn get_telemetry(app: tauri::State<'_, Arc<App>>) -> SystemTelemetry {
    handlers::telemetry(&app)
}

#[tauri::command]
async fn get_settings(app: tauri::State<'_, Arc<App>>) -> Result<BTreeMap<String, String>, String> {
    to_ipc(handlers::settings(&app).await)
}

#[tauri::command]
async fn list_jobs(
    app: tauri::State<'_, Arc<App>>,
    states: Option<Vec<String>>,
    limit: Option<u32>,
) -> Result<Vec<Job>, String> {
    let states = states
        .unwrap_or_default()
        .iter()
        .map(|s| s.parse::<JobState>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    to_ipc(handlers::list_jobs(&app, JobFilter { states, limit }).await)
}

pub fn run() {
    if let Err(err) = try_run() {
        eprintln!("aiwm-tauri: fatal: {err}");
        std::process::exit(1);
    }
}

fn try_run() -> anyhow::Result<()> {
    let (core, log_guard) = tauri::async_runtime::block_on(app::bootstrap_process())?;
    let services = tauri::async_runtime::block_on(api::spawn(core.clone()))?;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(core)
        .manage(Runtime {
            _log_guard: Mutex::new(log_guard),
            _services: services,
        })
        .invoke_handler(tauri::generate_handler![
            about,
            get_telemetry,
            get_settings,
            list_jobs
        ])
        .build(tauri::generate_context!())?
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let core = handle.state::<Arc<App>>();
                tauri::async_runtime::block_on(core.db.close());
            }
        });

    Ok(())
}
