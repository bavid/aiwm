//! Thin Tauri host. All logic lives in `aiwm-core`; this crate wires the window,
//! plugins, and IPC commands that forward to `aiwm_core::api::handlers` — the
//! same functions the loopback HTTP server uses, so both transports return
//! identical JSON.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use aiwm_core::api::dto::{AboutDto, RuntimeStatusDto};
use aiwm_core::api::handlers;
use aiwm_core::db::{Job, JobFilter, Model};
use aiwm_core::model::{ImportOutcome, ImportRequest};
use aiwm_core::orchestrator::JobState;
use aiwm_core::telemetry::SystemTelemetry;
use aiwm_core::{api, app, App};
use tauri::{Emitter, Manager};
use tracing_appender::non_blocking::WorkerGuard;

/// Logging guard + background services, parked in Tauri state for the process
/// lifetime.
struct HostState {
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

#[tauri::command]
async fn get_runtimes(app: tauri::State<'_, Arc<App>>) -> Result<Vec<RuntimeStatusDto>, String> {
    Ok(handlers::runtimes(&app).await)
}

#[tauri::command]
async fn install_llamacpp(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::install_llamacpp(&app).map(str::to_string))
}

#[tauri::command]
async fn cancel_job(app: tauri::State<'_, Arc<App>>, id: String) -> Result<Option<bool>, String> {
    to_ipc(handlers::cancel_job(&app, &id).await)
}

#[tauri::command]
async fn list_models(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Model>, String> {
    to_ipc(handlers::list_models(&app).await)
}

#[tauri::command]
async fn import_model(
    app: tauri::State<'_, Arc<App>>,
    request: ImportRequest,
) -> Result<ImportOutcome, String> {
    to_ipc(handlers::import_model(&app, request).await)
}

#[tauri::command]
fn get_recent_logs(
    app: tauri::State<'_, Arc<App>>,
    lines: Option<usize>,
) -> Result<Vec<String>, String> {
    to_ipc(handlers::recent_logs(&app, lines.unwrap_or(200)))
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
        .manage(HostState {
            _log_guard: Mutex::new(log_guard),
            _services: services,
        })
        .setup(|app| {
            // Push each telemetry reading to the webview as a `telemetry` event.
            let handle = app.handle().clone();
            let core = handle.state::<Arc<App>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = core.telemetry.subscribe();
                loop {
                    let snapshot = rx.borrow_and_update().clone();
                    if handle.emit("telemetry", snapshot).is_err() {
                        break;
                    }
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            about,
            get_telemetry,
            get_settings,
            list_jobs,
            get_runtimes,
            get_recent_logs,
            list_models,
            import_model,
            install_llamacpp,
            cancel_job
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
