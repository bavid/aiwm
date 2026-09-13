//! Thin Tauri host. All logic lives in `aiwm-core`; this crate wires the window,
//! plugins, and IPC commands that forward to `aiwm_core::api::handlers` — the
//! same functions the loopback HTTP server uses, so both transports return
//! identical JSON.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use aiwm_core::api::dto::{
    AboutDto, AgentPermissionDto, AgentSessionDetailDto, AttachExternalDto, ColibriModelDto,
    ConfigUpdate, EnqueueDownloadDto, FeaturedModelDto, JobDetailDto, KnownModelDto,
    LaunchExternalDto, LocalApiStatusDto, ModelStackDto, NewAgentDto, NewSessionDto,
    OpenAgentSessionDto, RegisterColibriModelDto, RegistryDetailsDto, RegistrySearchDto,
    RuntimeStatusDto, SubmitJobDto,
};
use aiwm_core::api::handlers;
use aiwm_core::config::Config;
use aiwm_core::db::Document;
use aiwm_core::db::{Agent, AgentSession, Benchmark, Download, Job, JobFilter, Model, Session};
use aiwm_core::model::{ImportOutcome, ImportRequest};
use aiwm_core::orchestrator::JobState;
use aiwm_core::registry::{Fetched, RemoteModel};
use aiwm_core::runtime::DetectedEngine;
use aiwm_core::telemetry::SystemTelemetry;
use aiwm_core::{api, app, App, LaunchInfo};
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
async fn get_config(app: tauri::State<'_, Arc<App>>) -> Result<Config, String> {
    to_ipc(handlers::config(&app))
}

#[tauri::command]
async fn save_config(
    app: tauri::State<'_, Arc<App>>,
    update: ConfigUpdate,
) -> Result<Config, String> {
    to_ipc(handlers::save_config(&app, update))
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
async fn install_comfyui(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::install_comfyui(&app).map(str::to_string))
}

#[tauri::command]
async fn install_hermes(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::install_hermes(&app).map(str::to_string))
}

#[tauri::command]
async fn list_agent_runtimes(
    app: tauri::State<'_, Arc<App>>,
) -> Result<Vec<aiwm_core::api::dto::AgentRuntimeDto>, String> {
    Ok(handlers::agent_runtimes(&app))
}

#[tauri::command]
async fn registry_search(
    app: tauri::State<'_, Arc<App>>,
    params: RegistrySearchDto,
) -> Result<Fetched<Vec<RemoteModel>>, String> {
    to_ipc(handlers::registry_search(&app, params).await)
}

#[tauri::command]
async fn registry_model(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<RegistryDetailsDto, String> {
    to_ipc(handlers::registry_details(&app, &id).await)
}

#[tauri::command]
async fn list_downloads(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Download>, String> {
    to_ipc(handlers::list_downloads(&app).await)
}

#[tauri::command]
async fn enqueue_download(
    app: tauri::State<'_, Arc<App>>,
    body: EnqueueDownloadDto,
) -> Result<Download, String> {
    to_ipc(handlers::enqueue_download(&app, body).await)
}

#[tauri::command]
async fn pause_download(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::pause_download(&app, &id).await)
}

#[tauri::command]
async fn resume_download(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::resume_download(&app, &id).await)
}

#[tauri::command]
async fn cancel_download(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::cancel_download(&app, &id).await)
}

#[tauri::command]
async fn export_backup(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::export_backup_to_file(&app).await)
}

#[tauri::command]
async fn import_backup(
    app: tauri::State<'_, Arc<App>>,
    path: String,
) -> Result<aiwm_core::backup::ImportSummary, String> {
    to_ipc(handlers::import_backup_from_file(&app, &path).await)
}

#[tauri::command]
async fn cancel_job(app: tauri::State<'_, Arc<App>>, id: String) -> Result<Option<bool>, String> {
    to_ipc(handlers::cancel_job(&app, &id).await)
}

#[tauri::command]
async fn submit_job(app: tauri::State<'_, Arc<App>>, body: SubmitJobDto) -> Result<Job, String> {
    to_ipc(handlers::submit_job(&app, body).await)
}

#[tauri::command]
async fn list_sessions(
    app: tauri::State<'_, Arc<App>>,
    capability: String,
) -> Result<Vec<Session>, String> {
    to_ipc(handlers::list_sessions(&app, &capability).await)
}

#[tauri::command]
async fn create_session(
    app: tauri::State<'_, Arc<App>>,
    body: NewSessionDto,
) -> Result<Session, String> {
    to_ipc(handlers::create_session(&app, body).await)
}

#[tauri::command]
async fn rename_session(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    name: String,
) -> Result<(), String> {
    to_ipc(handlers::rename_session(&app, &id, &name).await)
}

#[tauri::command]
async fn set_session_archived(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    archived: bool,
) -> Result<(), String> {
    to_ipc(handlers::set_session_archived(&app, &id, archived).await)
}

#[tauri::command]
async fn delete_session(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_session(&app, &id).await)
}

#[tauri::command]
async fn list_documents(
    app: tauri::State<'_, Arc<App>>,
    session_id: String,
) -> Result<Vec<Document>, String> {
    to_ipc(handlers::list_documents(&app, &session_id).await)
}

#[tauri::command]
async fn attach_document(
    app: tauri::State<'_, Arc<App>>,
    session_id: String,
    path: String,
) -> Result<Document, String> {
    to_ipc(handlers::attach_document(&app, &session_id, &path).await)
}

#[tauri::command]
async fn delete_document(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_document(&app, &id).await)
}

#[tauri::command]
async fn job_detail(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Option<JobDetailDto>, String> {
    to_ipc(handlers::job_detail(&app, &id).await)
}

#[tauri::command]
async fn list_models(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Model>, String> {
    to_ipc(handlers::list_models(&app).await)
}

#[tauri::command]
fn list_known_models(app: tauri::State<'_, Arc<App>>) -> Vec<KnownModelDto> {
    handlers::known_models(&app)
}

#[tauri::command]
fn list_model_stacks(app: tauri::State<'_, Arc<App>>) -> Vec<ModelStackDto> {
    handlers::model_stacks(&app)
}

#[tauri::command]
fn list_featured_models(app: tauri::State<'_, Arc<App>>) -> Vec<FeaturedModelDto> {
    handlers::featured_models(&app)
}

#[tauri::command]
fn list_colibri_models(app: tauri::State<'_, Arc<App>>) -> Vec<ColibriModelDto> {
    handlers::colibri_models(&app)
}

#[tauri::command]
async fn register_colibri_model(
    app: tauri::State<'_, Arc<App>>,
    body: RegisterColibriModelDto,
) -> Result<Model, String> {
    to_ipc(handlers::register_colibri_model(&app, body).await)
}

#[tauri::command]
async fn install_colibri(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::install_colibri(&app).map(str::to_string))
}

#[tauri::command]
async fn list_benchmarks(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Benchmark>, String> {
    to_ipc(handlers::latest_benchmarks(&app).await)
}

#[tauri::command]
async fn model_benchmarks(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Vec<Benchmark>, String> {
    to_ipc(handlers::model_benchmarks(&app, &id).await)
}

#[tauri::command]
async fn benchmark_model(app: tauri::State<'_, Arc<App>>, id: String) -> Result<Job, String> {
    to_ipc(handlers::benchmark_model(&app, &id).await)
}

#[tauri::command]
async fn upgrade_check(app: tauri::State<'_, Arc<App>>, id: String) -> Result<Job, String> {
    to_ipc(handlers::upgrade_check(&app, &id).await)
}

#[tauri::command]
async fn storage_report(
    app: tauri::State<'_, Arc<App>>,
) -> Result<aiwm_core::StorageReport, String> {
    to_ipc(handlers::storage_report(&app).await)
}

#[tauri::command]
async fn model_tags(
    app: tauri::State<'_, Arc<App>>,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, String> {
    to_ipc(handlers::model_tags(&app).await)
}

#[tauri::command]
async fn set_model_tags(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    to_ipc(handlers::set_model_tags(&app, &id, &tags).await)
}

#[tauri::command]
async fn set_model_roles(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    roles: Vec<String>,
) -> Result<Vec<String>, String> {
    to_ipc(handlers::set_model_roles(&app, &id, &roles).await)
}

#[tauri::command]
fn registry_status(app: tauri::State<'_, Arc<App>>) -> aiwm_core::RegistryStatus {
    handlers::registry_status(&app)
}

#[tauri::command]
fn set_hf_token(app: tauri::State<'_, Arc<App>>, token: String) -> Result<(), String> {
    to_ipc(handlers::set_hf_token(&app, &token))
}

#[tauri::command]
fn local_api_status(app: tauri::State<'_, Arc<App>>) -> LocalApiStatusDto {
    handlers::local_api_status(&app)
}

#[tauri::command]
fn set_local_api_token(app: tauri::State<'_, Arc<App>>, token: String) -> Result<(), String> {
    to_ipc(handlers::set_local_api_token(&app, &token))
}

#[tauri::command]
async fn external_engines(app: tauri::State<'_, Arc<App>>) -> Result<Vec<DetectedEngine>, String> {
    Ok(handlers::external_engines(&app).await)
}

#[tauri::command]
async fn attach_external_engine(
    app: tauri::State<'_, Arc<App>>,
    body: AttachExternalDto,
) -> Result<(), String> {
    to_ipc(handlers::attach_external_engine(&app, body).await)
}

#[tauri::command]
async fn detach_engine(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(
        handlers::detach_engine(&app, aiwm_core::api::dto::DetachEngineDto { model_id: id }).await,
    )
}

#[tauri::command]
async fn delete_model(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<aiwm_core::DeleteOutcome, String> {
    to_ipc(handlers::delete_model(&app, &id).await)
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

// --- agents (Phase 5.1c) ---

#[tauri::command]
async fn list_agents(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Agent>, String> {
    to_ipc(handlers::list_agents(&app).await)
}

#[tauri::command]
async fn create_agent(app: tauri::State<'_, Arc<App>>, body: NewAgentDto) -> Result<Agent, String> {
    to_ipc(handlers::create_agent(&app, body).await)
}

#[tauri::command]
async fn delete_agent(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_agent(&app, &id).await)
}

#[tauri::command]
async fn open_agent_session(
    app: tauri::State<'_, Arc<App>>,
    body: OpenAgentSessionDto,
) -> Result<AgentSession, String> {
    to_ipc(handlers::open_agent_session(&app, body).await)
}

#[tauri::command]
async fn agent_session_detail(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Option<AgentSessionDetailDto>, String> {
    to_ipc(handlers::agent_session_detail(&app, &id).await)
}

#[tauri::command]
async fn agent_session_message(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    text: String,
) -> Result<(), String> {
    to_ipc(handlers::agent_session_message(&app, &id, &text).await)
}

#[tauri::command]
async fn agent_session_permission(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: AgentPermissionDto,
) -> Result<(), String> {
    to_ipc(handlers::agent_session_permission(&app, &id, body).await)
}

#[tauri::command]
async fn stop_agent_session(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::stop_agent_session(&app, &id).await)
}

// --- external launcher ---

#[tauri::command]
fn launcher_status(app: tauri::State<'_, Arc<App>>) -> Option<LaunchInfo> {
    handlers::launcher_status(&app)
}

#[tauri::command]
async fn launch_external(
    app: tauri::State<'_, Arc<App>>,
    body: LaunchExternalDto,
) -> Result<LaunchInfo, String> {
    to_ipc(handlers::launch_external(&app, body).await)
}

#[tauri::command]
async fn stop_external_launch(app: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    to_ipc(handlers::stop_external_launch(&app).await)
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
        .plugin(tauri_plugin_dialog::init())
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
            get_config,
            save_config,
            list_jobs,
            get_runtimes,
            get_recent_logs,
            list_models,
            list_known_models,
            list_model_stacks,
            list_featured_models,
            list_colibri_models,
            register_colibri_model,
            import_model,
            list_benchmarks,
            model_benchmarks,
            benchmark_model,
            upgrade_check,
            storage_report,
            delete_model,
            model_tags,
            set_model_tags,
            set_model_roles,
            registry_status,
            set_hf_token,
            local_api_status,
            set_local_api_token,
            external_engines,
            attach_external_engine,
            detach_engine,
            install_llamacpp,
            install_comfyui,
            install_hermes,
            install_colibri,
            list_agent_runtimes,
            registry_search,
            registry_model,
            list_downloads,
            enqueue_download,
            pause_download,
            resume_download,
            cancel_download,
            export_backup,
            import_backup,
            cancel_job,
            submit_job,
            job_detail,
            list_sessions,
            create_session,
            rename_session,
            set_session_archived,
            delete_session,
            list_documents,
            attach_document,
            delete_document,
            list_agents,
            create_agent,
            delete_agent,
            open_agent_session,
            agent_session_detail,
            agent_session_message,
            agent_session_permission,
            stop_agent_session,
            launcher_status,
            launch_external,
            stop_external_launch
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
