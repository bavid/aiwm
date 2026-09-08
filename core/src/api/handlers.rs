//! Transport-agnostic API logic. Every handler takes `&App` plus typed inputs
//! and returns typed outputs — the Tauri commands and the HTTP routes are thin
//! wrappers over these.

use std::collections::BTreeMap;

use super::dto::{AboutDto, RuntimeStatusDto, SubmitJobDto};
use crate::db::{Job, JobEvent, JobFilter, Model, NewJob};
use crate::model::{ImportOutcome, ImportRequest};
use crate::orchestrator::JobOutcome;
use crate::telemetry::SystemTelemetry;
use crate::{App, CoreError, Result};

pub fn about(app: &App) -> AboutDto {
    AboutDto {
        core_version: crate::CORE_VERSION.to_string(),
        data_dir: app.paths.root().display().to_string(),
        store_path: app.config.store_path.display().to_string(),
        core_api_port: app.config.core_api_port,
        vram_budget_mb: app.scheduler.budget_mb(),
        offline_mode: app.config.offline_mode,
    }
}

pub fn telemetry(app: &App) -> SystemTelemetry {
    app.telemetry.latest()
}

pub async fn settings(app: &App) -> Result<BTreeMap<String, String>> {
    app.db.settings().all().await
}

pub async fn set_setting(app: &App, key: &str, value: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(CoreError::Config("setting key must not be empty".into()));
    }
    app.db.settings().set(key, value).await
}

pub async fn list_jobs(app: &App, filter: JobFilter) -> Result<Vec<Job>> {
    app.db.jobs().list(&filter).await
}

pub async fn job_events(app: &App, id: &str) -> Result<Option<(Job, Vec<JobEvent>)>> {
    match app.db.jobs().get(id).await? {
        Some(job) => {
            let events = app.db.jobs().events(id).await?;
            Ok(Some((job, events)))
        }
        None => Ok(None),
    }
}

pub async fn submit_job(app: &App, body: SubmitJobDto) -> Result<Job> {
    let mut new = NewJob::new(body.job_type);
    new.capability = body.capability;
    new.runtime_id = body.runtime_id;
    new.model_id = body.model_id;
    new.params = if body.params.is_null() {
        serde_json::json!({})
    } else {
        body.params
    };
    if body.vram_needed_mb > 0 {
        new.params["vram_needed_mb"] = body.vram_needed_mb.into();
    }
    if body.agent_session {
        new.params["agent_session"] = true.into();
    }
    app.jobs.submit(new).await
}

/// Ask a job to stop. `Ok(Some(true))` = a cancel was applied or signalled,
/// `Ok(Some(false))` = the job is already finished or in a step with no cancel
/// hook, `Ok(None)` = no such job.
pub async fn cancel_job(app: &App, id: &str) -> Result<Option<bool>> {
    match app.db.jobs().get(id).await? {
        Some(_) => Ok(Some(app.jobs.cancel(id).await?)),
        None => Ok(None),
    }
}

/// Run the next queued job now (used by tests and the daemon loop).
pub async fn run_next_job(app: &App) -> Result<Option<JobOutcome>> {
    app.jobs.run_next().await
}

pub async fn list_models(app: &App) -> Result<Vec<Model>> {
    app.db.models().list().await
}

pub async fn import_model(app: &App, req: ImportRequest) -> Result<ImportOutcome> {
    crate::model::import_model(&app.db, &app.config.store_path, req).await
}

pub async fn runtimes(app: &App) -> Vec<RuntimeStatusDto> {
    let mut out = Vec::new();
    for adapter in app.runtimes.all() {
        out.push(RuntimeStatusDto {
            id: adapter.id().to_string(),
            kind: adapter.kind(),
            health: adapter.health().await,
            vram_used_mb: adapter.vram_used_mb(),
            detail: adapter.detail(),
        });
    }
    out
}

/// Kick off the pinned llama.cpp download+install in the background. Returns
/// immediately (`"started"`); progress shows up in [`runtimes`]'s `detail`.
/// Returns `"already_installed"` when it is already there, and errors up front
/// on offline mode or an in-flight install.
pub fn install_llamacpp(app: &App) -> Result<&'static str> {
    if app.config.offline_mode {
        return Err(CoreError::Config(
            "offline mode is on — cannot download llama.cpp".into(),
        ));
    }
    if app.llama.is_installed() {
        return Ok("already_installed");
    }
    if matches!(
        app.llama.install_state(),
        crate::runtime::InstallState::Running { .. }
    ) {
        return Err(CoreError::Config(
            "a llama.cpp install is already running".into(),
        ));
    }

    let llama = app.llama.clone();
    let offline = app.config.offline_mode;
    tokio::spawn(async move {
        if let Err(e) = llama.install(offline).await {
            tracing::error!(error = %e, "llama.cpp install failed");
        }
    });
    Ok("started")
}

/// Tail of the current day's log file.
pub fn recent_logs(app: &App, lines: usize) -> Result<Vec<String>> {
    let dir = app.paths.logs_dir();
    let newest = std::fs::read_dir(&dir)
        .map_err(|e| CoreError::Config(format!("read {}: {e}", dir.display())))?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("aiwm.log"))
        .max_by_key(std::fs::DirEntry::file_name);

    let Some(entry) = newest else {
        return Ok(Vec::new());
    };
    let content = std::fs::read_to_string(entry.path())
        .map_err(|e| CoreError::Config(format!("read log: {e}")))?;
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].iter().map(|s| s.to_string()).collect())
}
