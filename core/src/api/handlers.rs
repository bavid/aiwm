//! Transport-agnostic API logic. Every handler takes `&App` plus typed inputs
//! and returns typed outputs — the Tauri commands and the HTTP routes are thin
//! wrappers over these.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::dto::{
    AboutDto, AgentPermissionDto, AgentSessionDetailDto, ConfigUpdate, EnqueueDownloadDto,
    JobDetailDto, NewAgentDto, OpenAgentSessionDto, RegistryDetailsDto, RegistryFileDto,
    RegistrySearchDto, RuntimeStatusDto, SubmitJobDto,
};
use crate::compat::FitVerdict;
use crate::config::Config;
use crate::db::{
    Agent, AgentSession, Benchmark, Download, Job, JobFilter, Model, NewAgent, NewJob,
};
use crate::download::EnqueueRequest;
use crate::model::{ImportOutcome, ImportRequest};
use crate::orchestrator::JobOutcome;
use crate::registry::{Fetched, RemoteFile, RemoteFormat, RemoteModel};
use crate::telemetry::SystemTelemetry;
use crate::{App, CoreError, Result};

pub fn about(app: &App) -> AboutDto {
    let outputs_dir = app.paths.outputs_dir();
    AboutDto {
        core_version: crate::CORE_VERSION.to_string(),
        data_dir: app.paths.root().display().to_string(),
        store_path: app.config.store_path.display().to_string(),
        outputs_dir: outputs_dir.display().to_string(),
        outputs_bytes: dir_file_bytes(&outputs_dir),
        runtimes_dir: app.paths.runtimes_dir().display().to_string(),
        cache_dir: app.paths.cache_dir().display().to_string(),
        core_api_port: app.config.core_api_port,
        vram_budget_mb: app.scheduler.budget_mb(),
        offline_mode: app.offline(),
    }
}

/// Sum of the regular files directly in `dir` (the outputs folder is flat —
/// `<job_id>.png` / `.mp4`). Missing dir → 0.
fn dir_file_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// The current `config.toml` as it sits on disk, with the *live* offline flag
/// overlaid (the Settings UI shows and rewrites this).
pub fn config(app: &App) -> Result<Config> {
    let mut cfg = Config::read_from(&app.paths)?;
    cfg.offline_mode = app.offline();
    Ok(cfg)
}

/// Apply the user-editable fields, persist `config.toml`, and flip the live
/// offline switch. Everything else needs an app restart to take effect — the UI
/// says so. Returns the full saved config.
pub fn save_config(app: &App, update: ConfigUpdate) -> Result<Config> {
    if update.store_path.trim().is_empty() {
        return Err(CoreError::Config("store path must not be empty".into()));
    }
    let mut cfg = Config::read_from(&app.paths)?;
    cfg.store_path = update.store_path.trim().into();
    cfg.offline_mode = update.offline_mode;
    cfg.vram_budget_mb = update.vram_budget_mb;
    cfg.llama = update.llama;
    cfg.comfyui = update.comfyui;
    cfg.models = update.models;
    cfg.paths.outputs_path = non_empty_path(&update.paths.outputs_path);
    cfg.paths.runtimes_path = non_empty_path(&update.paths.runtimes_path);
    cfg.paths.cache_path = non_empty_path(&update.paths.cache_path);
    cfg.save(&app.paths)?;
    app.set_offline(cfg.offline_mode);
    Ok(cfg)
}

/// A blank field means "use the portable default", not a literal path.
fn non_empty_path(s: &str) -> Option<PathBuf> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
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

pub async fn job_detail(app: &App, id: &str) -> Result<Option<JobDetailDto>> {
    match app.db.jobs().get(id).await? {
        Some(job) => {
            let events = app.db.jobs().events(id).await?;
            Ok(Some(JobDetailDto { job, events }))
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

/// The on-disk file a finished image job produced (`GET /jobs/{id}/output`
/// serves it). `None` when the job has no output, or its recorded path is
/// missing or — defensively — escaped the outputs directory.
pub async fn job_output_path(app: &App, id: &str) -> Result<Option<PathBuf>> {
    let Some(job) = app.db.jobs().get(id).await? else {
        return Ok(None);
    };
    let Some(raw) = job.output_path else {
        return Ok(None);
    };

    let path = PathBuf::from(&raw);
    let (Ok(canonical), Ok(root)) = (path.canonicalize(), app.paths.outputs_dir().canonicalize())
    else {
        return Ok(None); // file gone, or the outputs dir was never created
    };
    if canonical.starts_with(&root) && canonical.is_file() {
        Ok(Some(canonical))
    } else {
        Ok(None)
    }
}

pub async fn list_models(app: &App) -> Result<Vec<Model>> {
    app.db.models().list().await
}

// --- tags & registry status (Phase 6.9) --------------------------------------

/// `model_id -> [tags]` for every tagged model.
pub async fn model_tags(app: &App) -> Result<BTreeMap<String, Vec<String>>> {
    app.db.models().all_tags().await
}

/// Replace one model's tags; returns the cleaned set.
pub async fn set_model_tags(app: &App, id: &str, tags: &[String]) -> Result<Vec<String>> {
    if app.db.models().get(id).await?.is_none() {
        return Err(CoreError::Config(format!(
            "model {id} is not in the library"
        )));
    }
    app.db.models().set_tags(id, tags).await
}

/// The registry health line (last fetch, cache size, rate-limit, token).
pub fn registry_status(app: &App) -> crate::registry::RegistryStatus {
    app.registry.status()
}

/// Write (or clear, when blank) the Hugging Face token. Takes effect on the
/// next restart. The token lives in a machine-local file, never a backup.
pub fn set_hf_token(app: &App, token: &str) -> Result<()> {
    let path = app.paths.hf_token_file();
    let token = token.trim();
    if token.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, token).map_err(|e| CoreError::Config(format!("write hf token: {e}")))
}

/// Storage overview + the "safe to delete" reports (Phase 6.8).
pub async fn storage_report(app: &App) -> Result<crate::cleanup::StorageReport> {
    let models = app.db.models().list().await?;
    Ok(crate::cleanup::report(
        &models,
        &app.config.store_path,
        crate::cleanup::DEFAULT_STALE_DAYS,
    ))
}

/// Delete one model — its file, its runtime links, and its DB rows. Refused
/// while the model is loaded. **Permanent** (the caller confirms).
pub async fn delete_model(app: &App, id: &str) -> Result<crate::model::DeleteOutcome> {
    let model = app
        .db
        .models()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("model {id} is not in the library")))?;
    if app.runtimes.runtime_with_model(id).is_some() {
        return Err(CoreError::Config(format!(
            "\u{201c}{}\u{201d} is loaded right now — unload it first (close the Chat tab or stop the agent using it)",
            model.name
        )));
    }
    crate::model::delete_model(&app.db, &model).await
}

/// The curated "known models" list the Models tab shows for assisted import
/// (`GET /models/known`). Static — no `App` needed, but kept here for symmetry.
pub fn known_models(_app: &App) -> &'static [crate::model::KnownModel] {
    crate::model::KNOWN_MODELS
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
    if app.offline() {
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
    let offline = app.offline();
    tokio::spawn(async move {
        if let Err(e) = llama.install(offline).await {
            tracing::error!(error = %e, "llama.cpp install failed");
        }
    });
    Ok("started")
}

/// Kick off the pinned ComfyUI install (uv + source + venv + torch + deps) in
/// the background. Same contract as [`install_llamacpp`]: `"started"` /
/// `"already_installed"`, errors up front on offline mode or an in-flight run.
pub fn install_comfyui(app: &App) -> Result<&'static str> {
    if app.offline() {
        return Err(CoreError::Config(
            "offline mode is on — cannot download ComfyUI".into(),
        ));
    }
    if app.comfyui.is_installed() {
        return Ok("already_installed");
    }
    if app.comfyui.is_installing() {
        return Err(CoreError::Config(
            "a ComfyUI install is already running".into(),
        ));
    }

    let comfyui = app.comfyui.clone();
    let offline = app.offline();
    tokio::spawn(async move {
        if let Err(e) = comfyui.install(offline).await {
            tracing::error!(error = %e, "ComfyUI install failed");
        }
    });
    Ok("started")
}

/// Kick off the pinned Hermes install (`uv` venv + `hermes-agent` + a
/// best-effort `hermes postinstall`) in the background. Same contract as
/// [`install_comfyui`].
pub fn install_hermes(app: &App) -> Result<&'static str> {
    if app.offline() {
        return Err(CoreError::Config(
            "offline mode is on — cannot download Hermes".into(),
        ));
    }
    if app.hermes.is_installed() {
        return Ok("already_installed");
    }
    if app.hermes.is_installing() {
        return Err(CoreError::Config(
            "a Hermes install is already running".into(),
        ));
    }

    let hermes = app.hermes.clone();
    let offline = app.offline();
    tokio::spawn(async move {
        if let Err(e) = hermes.install(offline).await {
            tracing::error!(error = %e, "Hermes install failed");
        }
    });
    Ok("started")
}

/// Availability of the agent runtimes (`GET /agent-runtimes`). The UI's profile
/// form uses `installed` to enable each runtime option.
pub fn agent_runtimes(app: &App) -> Vec<crate::api::dto::AgentRuntimeDto> {
    use crate::api::dto::AgentRuntimeDto;
    vec![
        AgentRuntimeDto {
            id: "opencode".into(),
            installed: app.opencode.is_installed(),
            install: None,
        },
        AgentRuntimeDto {
            id: "hermes".into(),
            installed: app.hermes.is_installed(),
            install: Some(app.hermes.install_state()),
        },
    ]
}

// --- agents (Phase 5.1c) ---------------------------------------------------

pub async fn create_agent(app: &App, body: NewAgentDto) -> Result<Agent> {
    if body.name.trim().is_empty() {
        return Err(CoreError::Config("agent name must not be empty".into()));
    }
    if body.workspace_path.trim().is_empty() {
        return Err(CoreError::Config("workspace path must not be empty".into()));
    }
    if crate::AgentKind::from_adapter(body.adapter.trim()).is_none() {
        return Err(CoreError::Config(format!(
            "unknown agent adapter {:?}",
            body.adapter
        )));
    }
    app.db
        .agents()
        .create(NewAgent {
            name: body.name.trim().to_string(),
            adapter: body.adapter.trim().to_string(),
            model_id: body.model_id.filter(|s| !s.trim().is_empty()),
            workspace_path: body.workspace_path.trim().to_string(),
            allowed_paths: body.allowed_paths,
            toolset: body.toolset,
        })
        .await
}

pub async fn list_agents(app: &App) -> Result<Vec<Agent>> {
    app.db.agents().list().await
}

pub async fn delete_agent(app: &App, id: &str) -> Result<()> {
    app.db.agents().delete(id).await
}

/// Open a session and (optionally) send the first turn. The coding model is
/// placed + pinned before the runtime session opens.
pub async fn open_agent_session(app: &App, body: OpenAgentSessionDto) -> Result<AgentSession> {
    app.agents
        .open(&body.agent_id, body.first_message.as_deref())
        .await
}

/// A session plus its transcript. `None` = no such session.
pub async fn agent_session_detail(app: &App, id: &str) -> Result<Option<AgentSessionDetailDto>> {
    let Some(session) = app.db.agents().session(id).await? else {
        return Ok(None);
    };
    let events = app.db.agents().session_events(id).await?;
    Ok(Some(AgentSessionDetailDto {
        live: app.agents.is_live(id),
        session,
        events,
    }))
}

pub async fn agent_session_message(app: &App, id: &str, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(CoreError::Config("message must not be empty".into()));
    }
    app.agents.message(id, text).await
}

pub async fn agent_session_permission(app: &App, id: &str, body: AgentPermissionDto) -> Result<()> {
    app.agents.reply(id, &body.request_id, body.decision).await
}

pub async fn stop_agent_session(app: &App, id: &str) -> Result<()> {
    app.agents.stop(id).await
}

// --- backup / restore (Phase 5.5) ----------------------------------------

/// Build the export archive (zip bytes) — `GET /export` streams this.
pub async fn export_backup(app: &App) -> Result<Vec<u8>> {
    crate::backup::export(&app.paths, &app.db).await
}

/// Write the export to `<data>/exports/` and return the path (the Tauri command
/// uses this; the browser downloads the bytes instead).
pub async fn export_backup_to_file(app: &App) -> Result<String> {
    let path = crate::backup::export_to_file(&app.paths, &app.db).await?;
    Ok(path.to_string_lossy().into_owned())
}

/// Validate an export archive and stage it for the next startup.
pub async fn import_backup(app: &App, zip_bytes: &[u8]) -> Result<crate::backup::ImportSummary> {
    crate::backup::stage_import(&app.paths, zip_bytes, &app.db).await
}

/// Same, from a path on disk (the Tauri command; the user picks a file).
pub async fn import_backup_from_file(
    app: &App,
    path: &str,
) -> Result<crate::backup::ImportSummary> {
    let bytes =
        std::fs::read(path).map_err(|e| CoreError::Config(format!("cannot read {path}: {e}")))?;
    import_backup(app, &bytes).await
}

// --- model discovery (Phase 6.2) ---------------------------------------------

/// `GET /registry/search` — the "Discover" panel. `Fetched.freshness` tells the
/// UI whether this is `live`, a `stale` cache (the Hub was unreachable) or an
/// `offline` cache.
pub async fn registry_search(
    app: &App,
    params: RegistrySearchDto,
) -> Result<Fetched<Vec<RemoteModel>>> {
    app.registry.search(&params.into_query()).await
}

/// `GET /registry/models/{id}` — one repo, with every file enriched with the
/// browser link and a VRAM fit verdict against the current budget + free RAM.
pub async fn registry_details(app: &App, id: &str) -> Result<RegistryDetailsDto> {
    let fetched = app.registry.details(id).await?;
    let budget_mb = app.scheduler.budget_mb();
    let host = app.telemetry.latest().host;
    let free_ram_mb = host.ram_total_mb.saturating_sub(host.ram_used_mb);
    let d = fetched.data;
    let files = d
        .files
        .iter()
        .map(|f| enrich_file(id, &d.revision, &d.model, f, budget_mb, free_ram_mb))
        .collect();
    Ok(RegistryDetailsDto {
        model: d.model,
        revision: d.revision,
        files,
        freshness: fetched.freshness,
    })
}

/// The GGUF / safetensors weight files, not `README.md` / `config.json`.
fn is_weight_file(model: &RemoteModel, path: &str) -> bool {
    matches!(model.format, RemoteFormat::Gguf | RemoteFormat::Safetensors)
        && (path.ends_with(".gguf") || path.ends_with(".safetensors"))
}

fn enrich_file(
    id: &str,
    revision: &str,
    model: &RemoteModel,
    f: &RemoteFile,
    budget_mb: u64,
    free_ram_mb: u64,
) -> RegistryFileDto {
    let (vram_estimate_mb, fit) = if is_weight_file(model, &f.path) && f.size > 0 {
        let dims = crate::compat::ModelDims {
            size_bytes: f.size,
            ctx_max: model.ctx_max,
            param_count: model.param_count,
            ..Default::default()
        };
        let ctx = crate::compat::effective_ctx(model.ctx_max);
        (
            Some(crate::compat::estimate(&dims, ctx).total_mb),
            crate::compat::verdict(&dims, ctx, budget_mb, free_ram_mb),
        )
    } else {
        (None, FitVerdict::Unknown)
    };
    RegistryFileDto {
        download_url: format!("https://huggingface.co/{id}/resolve/{revision}/{}", f.path),
        path: f.path.clone(),
        size_bytes: f.size,
        sha256: f.sha256.clone(),
        quant: f.quant.clone(),
        shard: f.shard.map(|(a, b)| [a, b]),
        vram_estimate_mb,
        fit,
    }
}

// --- downloads (Phase 6.4) --------------------------------------------------

pub async fn list_downloads(app: &App) -> Result<Vec<Download>> {
    app.downloads.list().await
}

/// Queue a model download (from a Discover card). Refused in offline mode.
pub async fn enqueue_download(app: &App, dto: EnqueueDownloadDto) -> Result<Download> {
    let filename = std::path::Path::new(dto.filename.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| CoreError::Config("download needs a filename".into()))?
        .to_string();
    app.downloads
        .enqueue(EnqueueRequest {
            url: dto.url,
            filename,
            model_type: dto.model_type.filter(|s| !s.trim().is_empty()),
            sha256: dto.sha256.filter(|s| !s.trim().is_empty()),
            size_bytes: dto.size_bytes,
        })
        .await
}

pub async fn pause_download(app: &App, id: &str) -> Result<()> {
    app.downloads.pause(id).await
}

pub async fn resume_download(app: &App, id: &str) -> Result<()> {
    app.downloads.resume(id).await
}

pub async fn cancel_download(app: &App, id: &str) -> Result<()> {
    app.downloads.cancel(id).await
}

// --- benchmarks (Phase 6.5) -----------------------------------------------

/// The most recent benchmark for every model that has one — the Model Library
/// score column polls this and joins by `model_id`.
pub async fn latest_benchmarks(app: &App) -> Result<Vec<Benchmark>> {
    app.db.benchmarks().latest_all().await
}

/// Every benchmark run for one model, newest first.
pub async fn model_benchmarks(app: &App, model_id: &str) -> Result<Vec<Benchmark>> {
    app.db.benchmarks().list_for(model_id).await
}

/// Queue a "Test model" job for a local GGUF model. It goes through the
/// scheduler like a chat job (load / evict / run), then [`crate::bench`]
/// records a `benchmarks` row.
pub async fn benchmark_model(app: &App, model_id: &str) -> Result<Job> {
    let model = app
        .db
        .models()
        .get(model_id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("model {model_id} is not in the library")))?;
    if model.format != "gguf" {
        return Err(CoreError::Config(
            "benchmarking is llama.cpp / GGUF models only for now".into(),
        ));
    }
    let ctx = crate::compat::effective_ctx(model.ctx_max.and_then(|v| u32::try_from(v).ok()));
    let vram = crate::compat::estimate(&model.vram_dims(), ctx).total_mb;
    app.jobs
        .submit(NewJob::new("bench").on("llamacpp", &model.id, vram))
        .await
}

// --- upgrade check (Phase 6.7) -------------------------------------------

/// Queue an "is there something better?" job for one installed model. It reasons
/// with an `Auto`-picked chat/coding model and queries Hugging Face — refused in
/// offline mode. The result (an [`crate::UpgradeReport`]) lands in `jobs.result`.
pub async fn upgrade_check(app: &App, model_id: &str) -> Result<Job> {
    if app.offline() {
        return Err(CoreError::Config(
            "offline mode is on — the upgrade check needs Hugging Face".into(),
        ));
    }
    if app.db.models().get(model_id).await?.is_none() {
        return Err(CoreError::Config(format!(
            "model {model_id} is not in the library"
        )));
    }
    let mut new = NewJob::new("upgrade_check");
    new.params = serde_json::json!({ "target_model_id": model_id });
    app.jobs.submit(new).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::SearchSort;

    #[test]
    fn is_weight_file_needs_the_format_and_the_extension() {
        let gguf = RemoteModel {
            format: RemoteFormat::Gguf,
            ..sample_model()
        };
        assert!(is_weight_file(&gguf, "model-q4_k_m.gguf"));
        assert!(!is_weight_file(&gguf, "README.md"));
        assert!(!is_weight_file(&gguf, "config.json"));

        let other = RemoteModel {
            format: RemoteFormat::Other,
            ..sample_model()
        };
        assert!(!is_weight_file(&other, "weights.gguf"));
    }

    #[test]
    fn search_dto_maps_the_sort_strings() {
        assert_eq!(dto_sort(None), SearchSort::Downloads);
        assert_eq!(dto_sort(Some("downloads")), SearchSort::Downloads);
        assert_eq!(dto_sort(Some("likes")), SearchSort::Likes);
        assert_eq!(dto_sort(Some("trending")), SearchSort::Trending);
        assert_eq!(dto_sort(Some("updated")), SearchSort::RecentlyUpdated);
        assert_eq!(dto_sort(Some("new")), SearchSort::RecentlyCreated);
        assert_eq!(dto_sort(Some("garbage")), SearchSort::Downloads);
    }

    fn dto_sort(s: Option<&str>) -> SearchSort {
        RegistrySearchDto {
            sort: s.map(str::to_string),
            ..RegistrySearchDto::default()
        }
        .into_query()
        .sort
    }

    #[test]
    fn search_dto_drops_blank_text_and_defaults_the_limit() {
        let q = RegistrySearchDto {
            q: Some("   ".into()),
            base_model: Some(String::new()),
            ..RegistrySearchDto::default()
        }
        .into_query();
        assert_eq!(q.text, None);
        assert_eq!(q.base_model, None);
        assert_eq!(q.limit, 25);
    }

    fn sample_model() -> RemoteModel {
        RemoteModel {
            id: "x/y".into(),
            author: None,
            downloads: 0,
            likes: 0,
            trending_score: None,
            created_at: None,
            last_modified: None,
            pipeline_tag: None,
            library_name: None,
            gated: crate::registry::Gated::No,
            license: None,
            base_model: None,
            tags: vec![],
            param_count: None,
            arch: None,
            ctx_max: None,
            precision: None,
            format: RemoteFormat::Gguf,
        }
    }
}
