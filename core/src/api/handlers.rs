//! Transport-agnostic API logic. Every handler takes `&App` plus typed inputs
//! and returns typed outputs — the Tauri commands and the HTTP routes are thin
//! wrappers over these.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::dto::{
    AboutDto, ActivePersonaDto, AgentPermissionDto, AgentSessionDetailDto, AssignedDto,
    AttachExternalDto, BenchmarkOptionsDto, BulkFramesDto, BulkUpdatedDto, CharacterBodyDto,
    CivitaiSearchDto, CleanupDatasetDto, ColibriModelDto, ConceptBodyDto, ConceptFramesDto,
    ConceptSummaryDto, ConfigUpdate, DedupDatasetDto, DeleteFramesDto, DetachEngineDto,
    DialogueLineDto, EnqueueDownloadDto, ExportDatasetDto, FeaturedModelDto, JobDetailDto,
    KnownModelDto, LaunchExternalDto, LocalApiStatusDto, LocationBodyDto, ModelStackDto,
    NewAgentDto, NewSessionDto, NewVoiceIdentityDto, NpcBodyDto, OpenAgentSessionDto,
    PersonaBodyDto, ProfileDto, ProfilePresetsDto, RegisterColibriModelDto, RegistryDetailsDto,
    RegistryFileDto, RegistrySearchDto, RunDetailDto, RuntimeStatusDto, SceneBodyDto,
    SceneDetailDto, SetSessionPersonaDto, StartRunDto, StoryBodyDto, SubmitJobDto,
    TrainableModelDto, TrainerStatusDto, UpdateDatasetDto, UpdateDatasetFrameDto,
};
use crate::compat::FitVerdict;
use crate::config::Config;
use crate::db::{
    Agent, AgentSession, Benchmark, Character, CharacterLogEntry, CharacterRelationship,
    CharacterUpdate, Download, EventLevel, Job, JobFilter, Location, LocationUpdate, Model,
    NewAgent, NewCharacter, NewDialogueLine, NewJob, NewLocation, NewNpc, NewScene, Npc, NpcUpdate,
    Persona, Scene, SceneImage, SceneUpdate, Session, Story, StoryUpdate, VoiceIdentity,
};
use crate::download::EnqueueRequest;
use crate::launcher::LaunchRequest;
use crate::model::{ImportOutcome, ImportRequest};
use crate::orchestrator::JobOutcome;
use crate::persona;
use crate::registry::{Fetched, RemoteFile, RemoteFormat, RemoteModel};
use crate::telemetry::SystemTelemetry;
use crate::voice_identity::{self, CreateVoiceIdentity};
use crate::{App, CoreError, LaunchInfo, Result};

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
/// offline switch. Most fields still need an app restart (`[llama]`,
/// `[paths]`, `vram_budget_mb`) — the UI says so. `[comfyui]` is the
/// exception: a change is pushed to [`ComfyUiAdapter::set_options`], which
/// live-restarts a server we manage (best-effort — a failure here doesn't
/// fail the save, since the new config is already persisted and will apply on
/// the next start regardless). Returns the full saved config.
pub async fn save_config(app: &App, update: ConfigUpdate) -> Result<Config> {
    if update.store_path.trim().is_empty() {
        return Err(CoreError::Config("store path must not be empty".into()));
    }
    let mut cfg = Config::read_from(&app.paths)?;
    let comfyui_changed = cfg.comfyui != update.comfyui;
    cfg.store_path = update.store_path.trim().into();
    cfg.offline_mode = update.offline_mode;
    cfg.vram_budget_mb = update.vram_budget_mb;
    cfg.llama = update.llama;
    cfg.comfyui = update.comfyui;
    cfg.models = update.models;
    cfg.paths.outputs_path = non_empty_path(&update.paths.outputs_path);
    cfg.paths.runtimes_path = non_empty_path(&update.paths.runtimes_path);
    cfg.paths.cache_path = non_empty_path(&update.paths.cache_path);
    cfg.retention = update.retention;
    cfg.save(&app.paths)?;
    app.set_offline(cfg.offline_mode);

    if comfyui_changed {
        if let Err(e) = app.comfyui.set_options(cfg.comfyui.to_options()).await {
            tracing::warn!(
                error = %e,
                "saved the new ComfyUI options, but the live restart failed — \
                 they'll still apply on the next manual restart"
            );
        }
    }
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
    let new = new_job_from(body);
    // A dataset prep request is fully checkable up front (e.g. a relative
    // root folder): refuse it now with a 400 instead of queueing a job that
    // can only fail.
    if new.job_type == "dataset_prep" {
        crate::capability::dataset::DatasetPrepRequest::from_params(&new.params).map_err(|e| {
            match e {
                CoreError::Config(msg) => CoreError::Config(msg),
                other => CoreError::Config(other.to_string()),
            }
        })?;
    }
    app.jobs.submit(new).await
}

fn new_job_from(body: SubmitJobDto) -> NewJob {
    let mut new = NewJob::new(body.job_type);
    new.capability = body.capability;
    new.runtime_id = body.runtime_id;
    new.model_id = body.model_id;
    new.session_id = body.session_id;
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
    new
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

/// Permanently remove a finished job — its history entry, events, and any
/// output file on disk (best-effort; a missing/already-gone file is fine). A
/// still-running job must be cancelled first, since deleting out from under
/// the engine could leave the scheduler pointed at a job that no longer
/// exists.
pub async fn delete_job(app: &App, id: &str) -> Result<()> {
    let Some(job) = app.db.jobs().get(id).await? else {
        return Err(CoreError::Config(format!("no such job {id}")));
    };
    if !job.state.is_terminal() {
        return Err(CoreError::Config(
            "this job is still running — cancel it first".into(),
        ));
    }
    if let Some(path) = &job.output_path {
        let _ = std::fs::remove_file(path);
    }
    app.db.jobs().delete(id).await
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

/// `POST /jobs/{id}/clean-audio` — run the sidecar's DSP cleanup pass
/// (DC-offset removal, a gentle high-pass filter, spectral-gate noise
/// reduction) on an already-rendered narration clip, overwriting it in
/// place. Only makes sense for finished `tts` jobs with real output.
pub async fn clean_audio(app: &App, id: &str) -> Result<f64> {
    let job = app
        .db
        .jobs()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("no such job {id}")))?;
    if job.job_type != "tts" {
        return Err(CoreError::Config(
            "only narration clips can be cleaned".into(),
        ));
    }
    let path = job_output_path(app, id)
        .await?
        .ok_or_else(|| CoreError::Config("no output for this job".into()))?;

    let duration_secs = crate::capability::audio_clean::clean_in_place(&app.tts, &path).await?;
    app.db
        .jobs()
        .append_event(id, EventLevel::Info, "cleaned")
        .await?;
    Ok(duration_secs)
}

// --- dataset prep (Lokale KI-Trainings-Engine, dataset-prep half) ----------

/// The on-disk file behind one curated frame (`GET /jobs/{id}/dataset-
/// frames/{frame_id}/image` serves it). Unlike `job_output_path`, this does
/// *not* confine the result to `outputs_dir`: a frame's path was written
/// entirely by our own ingest/extraction code (`capability::dataset`), never
/// taken from the HTTP request itself, and legitimately points anywhere on
/// disk the user's dataset root lives — a plain image file the user already
/// had is referenced in place, not copied into `outputs_dir` first.
pub async fn dataset_frame_image_path(app: &App, frame_id: &str) -> Result<Option<PathBuf>> {
    let Some(frame) = app.db.dataset_frames().get(frame_id).await? else {
        return Ok(None);
    };
    let path = PathBuf::from(&frame.frame_path);
    Ok(if path.is_file() { Some(path) } else { None })
}

/// `GET /jobs/{id}/dataset-frames` — every frame a `dataset_prep` job has
/// produced so far, for the curation grid. Safe to poll while the job is
/// still running (rows land incrementally, per-source, as the pipeline
/// works through the tree).
pub async fn list_dataset_frames(app: &App, job_id: &str) -> Result<Vec<crate::db::DatasetFrame>> {
    app.db.dataset_frames().list_for_job(job_id).await
}

/// `PUT /jobs/{id}/dataset-frames/{frame_id}` — a curator's edit: a caption
/// rewrite, an exclude toggle, a "doch behalten" on an auto-rejected frame, a
/// clip in/out trim, or any combination in one call.
pub async fn update_dataset_frame(
    app: &App,
    frame_id: &str,
    body: UpdateDatasetFrameDto,
) -> Result<crate::db::DatasetFrame> {
    if let Some(excluded) = body.excluded {
        app.db
            .dataset_frames()
            .set_excluded(frame_id, excluded)
            .await?;
    }
    if let Some(caption) = &body.caption {
        // An empty engine name records this as a hand-written edit, distinct
        // from `"florence2"`/`"qwen2.5-vl"` — see `DatasetFrameRepo::set_caption`.
        app.db
            .dataset_frames()
            .set_caption(frame_id, caption, "")
            .await?;
    }
    if body.restore == Some(true) {
        // An empty reason *is* "kept" — see `DatasetFrame::rejection_reason`.
        app.db
            .dataset_frames()
            .set_rejection_reason(frame_id, "")
            .await?;
    }
    if body.clip_start_secs.is_some() || body.clip_end_secs.is_some() {
        // `set_clip_range` writes both columns, so a one-sided edit has to
        // read the stored row and carry the untouched bound through.
        let current = app
            .db
            .dataset_frames()
            .get(frame_id)
            .await?
            .ok_or_else(|| CoreError::Config(format!("no such dataset frame {frame_id}")))?;
        let start = body.clip_start_secs.unwrap_or(current.clip_start_secs);
        let end = body.clip_end_secs.unwrap_or(current.clip_end_secs);
        app.db
            .dataset_frames()
            .set_clip_range(frame_id, start, end)
            .await?;
    }
    app.db
        .dataset_frames()
        .get(frame_id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("no such dataset frame {frame_id}")))
}

/// `POST /jobs/{id}/dataset-export` — write the curator's final, non-excluded
/// selection to `dest_dir` as `NNNN.png` + `NNNN.txt` pairs.
///
/// Always prose-first: this shim keeps the pre-dataset-object clients working
/// and `export_dataset_for_job` takes no caption order, so a `caption_order`
/// in the body is accepted and ignored here. The dataset-keyed
/// [`export_dataset_by_id`] is the route that honours it.
pub async fn export_dataset(
    app: &App,
    job_id: &str,
    dest_dir: &str,
) -> Result<crate::capability::dataset::ExportSummary> {
    crate::capability::dataset::export_dataset_for_job(
        &app.db,
        job_id,
        std::path::Path::new(dest_dir),
    )
    .await
}

// --- datasets as objects, concepts, captioners (spec 3A/3D) ----------------

/// `GET /captioners` — the captioner registry plus whether each one's files
/// are in the model library; the "Beschreiben mit" dropdown filters on it.
pub async fn list_captioners(
    app: &App,
) -> Result<Vec<crate::capability::dataset::CaptionerStatus>> {
    crate::capability::dataset::captioner_statuses(&app.db).await
}

/// `GET /datasets` — every dataset, newest prep run included; a dataset
/// outlives the `dataset_prep` job that produced it.
pub async fn list_datasets(app: &App) -> Result<Vec<crate::db::Dataset>> {
    app.db.datasets().list().await
}

/// `GET /datasets/{id}` — `None` when there is no such dataset (the route
/// answers 404).
pub async fn get_dataset(app: &App, id: &str) -> Result<Option<crate::db::Dataset>> {
    app.db.datasets().get(id).await
}

/// `PUT /datasets/{id}` — today only the trigger word; returns the stored row
/// so the caller never has to guess how it was normalised.
pub async fn update_dataset(
    app: &App,
    id: &str,
    body: UpdateDatasetDto,
) -> Result<crate::db::Dataset> {
    if let Some(trigger_word) = &body.trigger_word {
        app.db.datasets().set_trigger_word(id, trigger_word).await?;
    }
    app.db
        .datasets()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("no such dataset {id}")))
}

/// `DELETE /datasets/{id}` — drops the dataset, its frames and its concepts
/// (SQLite cascade) *and* its files: the app-owned work folder and an export
/// that lies inside the outputs folder. Source files and a user-chosen
/// export folder stay. Refused while a training run of the dataset is not
/// finished. `None` for an unknown dataset (the route answers 404).
pub async fn delete_dataset(
    app: &App,
    id: &str,
) -> Result<Option<crate::capability::dataset::DatasetDeleteSummary>> {
    crate::capability::dataset::housekeeping::delete_dataset_with_files(
        &app.db,
        &app.paths.outputs_dir(),
        id,
    )
    .await
}

/// `GET /datasets/{id}/usage` — the dataset's disk use, measured by walking
/// its folders; what the delete and cleanup confirmations show.
pub async fn dataset_usage(
    app: &App,
    id: &str,
) -> Result<Option<crate::capability::dataset::DatasetUsage>> {
    crate::capability::dataset::housekeeping::usage(&app.db, &app.paths.outputs_dir(), id).await
}

/// `POST /datasets/{id}/frames/bulk` — move a whole selection to Keep or
/// Discard in one request; keeping also clears a filter rejection.
pub async fn bulk_update_dataset_frames(
    app: &App,
    id: &str,
    body: BulkFramesDto,
) -> Result<Option<BulkUpdatedDto>> {
    crate::capability::dataset::housekeeping::check_frame_ids(&body.frame_ids)?;
    if app.db.datasets().get(id).await?.is_none() {
        return Ok(None);
    }
    let updated = app
        .db
        .dataset_frames()
        .set_excluded_many(id, &body.frame_ids, body.excluded)
        .await?;
    Ok(Some(BulkUpdatedDto {
        requested: body.frame_ids.len(),
        updated,
    }))
}

/// `POST /datasets/{id}/frames/delete` — delete frames with their files.
pub async fn delete_dataset_frames(
    app: &App,
    id: &str,
    body: DeleteFramesDto,
) -> Result<Option<crate::capability::dataset::FramesDeleteSummary>> {
    crate::capability::dataset::housekeeping::delete_frames(
        &app.db,
        &app.paths.outputs_dir(),
        id,
        &body.frame_ids,
    )
    .await
}

/// `POST /datasets/{id}/cleanup` — preview (`dry_run`) or delete every
/// discarded frame with its file.
pub async fn cleanup_dataset(
    app: &App,
    id: &str,
    body: CleanupDatasetDto,
) -> Result<Option<crate::capability::dataset::CleanupSummary>> {
    crate::capability::dataset::housekeeping::cleanup(
        &app.db,
        &app.paths.outputs_dir(),
        id,
        body.dry_run,
    )
    .await
}

/// `POST /datasets/{id}/dedup` — mark near-duplicates across the whole
/// dataset `duplicate_global`, keeping the sharpest of each group.
pub async fn dedup_dataset(
    app: &App,
    id: &str,
    body: DedupDatasetDto,
) -> Result<Option<crate::capability::dataset::DedupSummary>> {
    crate::capability::dataset::housekeeping::dedup(&app.db, id, body.threshold).await
}

/// `GET /datasets/{id}/frames` — the dataset-keyed curation set. Unlike the
/// job-keyed list this still works once the prep job has been deleted.
pub async fn list_dataset_frames_for_dataset(
    app: &App,
    dataset_id: &str,
) -> Result<Vec<crate::db::DatasetFrame>> {
    app.db.dataset_frames().list_for_dataset(dataset_id).await
}

/// `GET /datasets/{id}/concepts` — each concept with the number of frames
/// carrying it and a token warning when the token is an ordinary word.
pub async fn list_concepts(app: &App, dataset_id: &str) -> Result<Vec<ConceptSummaryDto>> {
    let concepts = app.db.concepts().list_for_dataset(dataset_id).await?;
    let counts = app.db.concepts().counts_for_dataset(dataset_id).await?;
    Ok(concepts
        .into_iter()
        .map(|c| ConceptSummaryDto {
            frame_count: counts.get(&c.id).copied().unwrap_or(0),
            token_warning: crate::capability::dataset::token_warning(&c.token),
            concept: c,
        })
        .collect())
}

/// The empty-field and duplicate-token checks both `create_concept` and
/// `update_concept` run. `exclude_id` is the row being edited, which must not
/// clash with itself.
async fn check_concept_body(
    app: &App,
    dataset_id: &str,
    body: &ConceptBodyDto,
    exclude_id: Option<&str>,
) -> Result<()> {
    if body.name.trim().is_empty() || body.token.trim().is_empty() {
        return Err(CoreError::Config(
            "concept name and token must not be empty".into(),
        ));
    }
    // A duplicate token is a user mistake, not a server fault: the UNIQUE
    // (dataset_id, token) violation would otherwise surface as a DB error
    // -> HTTP 500. Pre-check and answer 400. Compared trimmed, because that
    // is how `ConceptRepo` stores it.
    let token = body.token.trim();
    if app
        .db
        .concepts()
        .list_for_dataset(dataset_id)
        .await?
        .iter()
        .any(|c| c.token == token && Some(c.id.as_str()) != exclude_id)
    {
        return Err(CoreError::Config(format!(
            "token {token:?} is already used by another concept in this dataset"
        )));
    }
    Ok(())
}

/// `POST /datasets/{id}/concepts` — add one concept the dataset teaches.
pub async fn create_concept(
    app: &App,
    dataset_id: &str,
    body: ConceptBodyDto,
) -> Result<crate::db::DatasetConcept> {
    check_concept_body(app, dataset_id, &body, None).await?;
    app.db
        .concepts()
        .create(crate::db::NewConcept {
            dataset_id: dataset_id.to_string(),
            name: body.name,
            token: body.token,
            description: body.description,
        })
        .await
}

/// `PUT /concepts/{id}` — rewrite name, token and description in one go.
pub async fn update_concept(app: &App, id: &str, body: ConceptBodyDto) -> Result<()> {
    let current = app
        .db
        .concepts()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("no such concept {id}")))?;
    check_concept_body(app, &current.dataset_id, &body, Some(id)).await?;
    app.db
        .concepts()
        .update(id, &body.name, &body.token, &body.description)
        .await
}

/// `DELETE /concepts/{id}` — the assignments go with it (cascade).
pub async fn delete_concept(app: &App, id: &str) -> Result<()> {
    app.db.concepts().delete(id).await
}

/// `POST /concepts/{id}/frames` — attach frames. Already-assigned ids and ids
/// from another dataset are skipped, so `attached` can be lower than
/// `requested`; the UI reports the difference rather than claiming success
/// for frames that never moved.
pub async fn assign_concept(
    app: &App,
    concept_id: &str,
    body: ConceptFramesDto,
) -> Result<AssignedDto> {
    let attached = app
        .db
        .concepts()
        .assign(concept_id, &body.frame_ids)
        .await?;
    Ok(AssignedDto {
        requested: body.frame_ids.len(),
        attached,
    })
}

/// `DELETE /concepts/{id}/frames` — detach frames; unknown pairs are no-ops.
pub async fn unassign_concept(app: &App, concept_id: &str, body: ConceptFramesDto) -> Result<()> {
    app.db
        .concepts()
        .unassign(concept_id, &body.frame_ids)
        .await
}

/// `GET /datasets/{id}/frame-concepts` — frame id -> concept ids, for the
/// curation grid's per-frame concept chips. Frames without a concept are
/// simply absent.
pub async fn frame_concept_map(
    app: &App,
    dataset_id: &str,
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    app.db.concepts().map_for_dataset(dataset_id).await
}

/// `POST /datasets/{id}/export` — the full export: composed captions, the
/// caller's caption order, and in clips mode the trimmed source clips.
pub async fn export_dataset_by_id(
    app: &App,
    dataset_id: &str,
    body: ExportDatasetDto,
) -> Result<crate::capability::dataset::ExportSummary> {
    crate::capability::dataset::export_dataset(
        &app.db,
        &crate::capability::dataset::ExportRequest {
            dataset_id: dataset_id.to_string(),
            dest_dir: PathBuf::from(body.dest_dir),
            caption_order: body.caption_order,
        },
    )
    .await
}

// --- training orchestrator (spec `2026-09-16-training-orchestrator-design`) -

/// Longest trigger word the form accepts. A trigger is a made-up token the
/// LoRA binds to, prepended to every caption — long ones cost caption budget
/// and start colliding with real vocabulary.
const MAX_TRIGGER_WORD_CHARS: usize = 30;

/// How many sample prompts a run may carry: one so there is something to look
/// at, at most three so the sampling pass stays a rounding error next to the
/// training step it interrupts.
const MAX_SAMPLE_PROMPTS: usize = 3;

/// Lines of `train.log` [`get_training_run`] returns.
const LOG_TAIL_LINES: usize = 40;

/// How far back into `train.log` the tail reads. A long run's log is
/// megabytes of redrawn progress bars; the last chunk is all the UI shows.
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// `GET /training/status` — the Training tab's header.
pub fn trainer_status(app: &App) -> TrainerStatusDto {
    TrainerStatusDto {
        installed: app.training.is_installed(),
        installing: app.training.is_installing(),
        env_broken: app.training.env_broken(),
        install_state: app.training.install_state(),
        detail: crate::runtime::RuntimeAdapter::detail(app.training.as_ref()).unwrap_or_default(),
        alive_run_id: app.training.alive_run(),
    }
}

/// `POST /training/install` — kick off the pinned `ai-toolkit` install in the
/// background. Same contract as [`install_comfyui`]: `"started"` /
/// `"already_installed"`, errors up front on offline mode or an in-flight run.
pub fn install_trainer(app: &App) -> Result<&'static str> {
    if app.offline() {
        return Err(CoreError::Config(
            "offline mode is on — cannot download the trainer".into(),
        ));
    }
    if app.training.is_installed() {
        return Ok("already_installed");
    }
    if app.training.is_installing() {
        return Err(CoreError::Config(
            "a trainer install is already running".into(),
        ));
    }

    let training = app.training.clone();
    let offline = app.offline();
    tokio::spawn(async move {
        if let Err(e) = training.install(offline).await {
            tracing::error!(error = %e, "trainer install failed");
        }
    });
    Ok("started")
}

/// `POST /training/probe` — ask the trainer venv what PyTorch it has. The one
/// check that distinguishes "the files are there" from "a run would start";
/// it sets or clears `env_broken` as a side effect.
pub async fn probe_trainer(app: &App) -> Result<crate::runtime::training::Probe> {
    app.training.probe().await
}

/// `GET /training/profiles` — the static registry, joined with the two facts
/// only the library knows: whether each profile's base weights are staged,
/// and which library models resolve to it.
pub async fn list_training_profiles(app: &App) -> Result<Vec<ProfileDto>> {
    use crate::training::profile::{find_for_model, find_staged_base, PROFILES};

    let models = app.db.models().list().await?;
    let mut out = Vec::with_capacity(PROFILES.len());
    for profile in PROFILES {
        let candidates = app.db.models().for_role(profile.base.role).await?;
        // The exact rule the runner's preflight resolves a real run's base
        // directory with, so this badge cannot promise what Start refuses.
        let base_installed = find_staged_base(profile, &candidates).is_some();

        let trainable_models = models
            .iter()
            .filter(|m| {
                find_for_model(m.family.as_deref(), &m.name, m.param_count)
                    .is_some_and(|p| p.family == profile.family)
            })
            .map(|m| TrainableModelDto {
                id: m.id.clone(),
                name: m.name.clone(),
                family: m.family.clone().unwrap_or_default(),
            })
            .collect();

        out.push(ProfileDto {
            family: profile.family,
            label: profile.label,
            arch: profile.arch,
            data_kind: profile.data_kind,
            fit: profile.vram.fit,
            fit_label: profile.vram.fit.label(),
            reserve_mb: profile.vram.reserve_mb,
            base_repo: profile.base.repo,
            base_role: profile.base.role,
            base_required_files: profile.base.required_files,
            base_approx_gb: profile.base.approx_gb,
            base_download_command: crate::training::bases::find_base(profile.family)
                .map(|base| {
                    crate::training::bases::hf_download_command(base, &app.config.store_path)
                })
                .unwrap_or_default(),
            base_installed,
            caption_order: profile.caption_order,
            license_note: profile.license_note,
            presets: ProfilePresetsDto {
                fast: profile.fast,
                balanced: profile.balanced,
                thorough: profile.thorough,
            },
            trainable_models,
        });
    }
    Ok(out)
}

/// `GET /training/runs` — every run, newest first.
pub async fn list_training_runs(app: &App) -> Result<Vec<crate::db::TrainingRun>> {
    app.db.training_runs().list().await
}

/// The trigger word as it will be stored, or a sentence saying why it cannot
/// be. Checked here rather than in the runner so the form can reject it
/// before anything touches the GPU or the disk.
fn check_trigger_word(raw: &str) -> Result<String> {
    let trigger = raw.trim();
    if trigger.is_empty() {
        return Err(CoreError::Config(
            "the trigger word is what you will type to call this LoRA — it cannot be empty".into(),
        ));
    }
    if trigger.chars().any(char::is_whitespace) {
        return Err(CoreError::Config(format!(
            "the trigger word must be a single word without spaces — got {trigger:?}"
        )));
    }
    if trigger.chars().count() > MAX_TRIGGER_WORD_CHARS {
        return Err(CoreError::Config(format!(
            "the trigger word is too long: keep it under {MAX_TRIGGER_WORD_CHARS} characters"
        )));
    }
    Ok(trigger.to_string())
}

/// The sample prompts as they will be stored. The runner re-checks that at
/// least one survives; this is the friendlier, earlier half of the same rule.
fn check_sample_prompts(raw: &[String]) -> Result<Vec<String>> {
    let prompts: Vec<String> = raw
        .iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect();
    if prompts.is_empty() {
        return Err(CoreError::Config(
            "add at least one sample prompt so you can see what the run learns".into(),
        ));
    }
    if prompts.len() > MAX_SAMPLE_PROMPTS {
        return Err(CoreError::Config(format!(
            "at most {MAX_SAMPLE_PROMPTS} sample prompts — every extra one pauses the training \
             to render"
        )));
    }
    Ok(prompts)
}

/// `POST /training/runs` — validate the form, then create the row and launch
/// the detached trainer. Everything the preflight refuses comes back as a
/// plain sentence with a 400.
/// ADR-009: refuse to drive the trainer while offline mode is on.
///
/// Staged base weights are not enough. On a family's first run `ai-toolkit`
/// fetches two more pieces from the Hub itself — the Qwen3 text encoder and
/// the FLUX.2 VAE live in *different* repos from the base checkpoint (see
/// [`crate::training::bases`]) — which the real 4B run demonstrated by
/// downloading 8 GB of `Qwen/Qwen3-4B` after the local blob had already
/// loaded. Without this a run started offline would spend minutes loading a
/// model and then die on a download, so it is refused in the first second
/// instead, as a sentence the Training tab can show as-is.
fn check_trainer_online(app: &App) -> Result<()> {
    if app.offline() {
        return Err(crate::training::training_refusal(
            "offline mode is on — the trainer needs the Hugging Face Hub on a family's \
             first run (Qwen3 text encoder, FLUX.2 VAE); turn offline mode off for this run",
        ));
    }
    Ok(())
}

pub async fn start_training_run(app: &App, body: StartRunDto) -> Result<crate::db::TrainingRun> {
    check_trainer_online(app)?;
    let trigger_word = check_trigger_word(&body.trigger_word)?;
    let sample_prompts = check_sample_prompts(&body.sample_prompts)?;

    app.training_runner
        .create_and_start(crate::training::runner::StartRequest {
            name: body.name,
            target_model_id: body.target_model_id,
            dataset_id: body.dataset_id,
            trigger_word,
            preset: body.preset,
            hyperparams: body.hyperparams,
            sample_prompts,
        })
        .await
}

/// The directory a run's output actually lives in: the path recorded on the
/// row (a run started before the data folder moved keeps its own), falling
/// back to the runner's derived path for a row that never got one.
fn run_work_dir(app: &App, run: &crate::db::TrainingRun) -> PathBuf {
    let recorded = run.work_dir.trim();
    if recorded.is_empty() {
        app.training_runner.work_dir(&run.id)
    } else {
        PathBuf::from(recorded)
    }
}

/// The sample images ai-toolkit wrote alongside the latest checkpoint,
/// newest-checkpoint first. Never returned to a caller as paths — see
/// [`RunDetailDto::latest_samples`].
fn latest_sample_paths(app: &App, run: &crate::db::TrainingRun) -> Vec<PathBuf> {
    let dir = crate::training::config::training_folder(&run_work_dir(app, run)).join(&run.name);
    crate::training::progress::scan_work_dir(&dir, &run.name)
        .map(|state| state.latest_samples)
        .unwrap_or_else(|e| {
            tracing::debug!(run = %run.id, error = %e, "could not scan a run's work directory");
            Vec::new()
        })
}

/// The last [`LOG_TAIL_LINES`] updates of a run's `train.log`. A missing log
/// is not an error — a run that has not written one yet simply has no tail.
async fn log_tail(app: &App, run: &crate::db::TrainingRun) -> Vec<String> {
    let path = run_work_dir(app, run).join("train.log");
    let Ok(meta) = tokio::fs::metadata(&path).await else {
        return Vec::new();
    };
    let from = meta.len().saturating_sub(LOG_TAIL_BYTES);
    let Ok((chunk, _)) = crate::training::progress::tail_log(&path, from).await else {
        return Vec::new();
    };
    let lines: Vec<String> = crate::training::progress::split_updates(&chunk)
        .map(str::to_string)
        .collect();
    let start = lines.len().saturating_sub(LOG_TAIL_LINES);
    lines[start..].to_vec()
}

/// `GET /training/runs/{id}` — the row plus the two things that live on disk.
/// `None` when there is no such run (the route answers 404).
pub async fn get_training_run(app: &App, id: &str) -> Result<Option<RunDetailDto>> {
    let Some(run) = app.db.training_runs().get(id).await? else {
        return Ok(None);
    };
    let latest_samples = (0..latest_sample_paths(app, &run).len())
        .map(|i| i.to_string())
        .collect();
    let log_tail = log_tail(app, &run).await;
    Ok(Some(RunDetailDto {
        work_dir: run_work_dir(app, &run).to_string_lossy().into_owned(),
        latest_samples,
        log_tail,
        run,
    }))
}

/// The stored row after a lifecycle call, so the caller never has to refetch
/// to learn what the transition actually settled on (a `pause` on a process
/// that had already vanished lands in `interrupted`, not `paused`).
async fn reload_run(app: &App, id: &str) -> Result<crate::db::TrainingRun> {
    app.db
        .training_runs()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Config(format!("no such training run {id}")))
}

/// `POST /training/runs/{id}/pause` — stop the process, keep the checkpoints.
pub async fn pause_training_run(app: &App, id: &str) -> Result<crate::db::TrainingRun> {
    app.training_runner.pause(id).await?;
    reload_run(app, id).await
}

/// `POST /training/runs/{id}/resume` — relaunch from the latest checkpoint.
pub async fn resume_training_run(app: &App, id: &str) -> Result<crate::db::TrainingRun> {
    check_trainer_online(app)?;
    app.training_runner.resume(id).await?;
    reload_run(app, id).await
}

/// `POST /training/runs/{id}/cancel` — stop for good. The work directory and
/// its checkpoints survive; only [`delete_training_run`] with `purge` removes
/// them.
pub async fn cancel_training_run(app: &App, id: &str) -> Result<crate::db::TrainingRun> {
    app.training_runner.cancel(id).await?;
    reload_run(app, id).await
}

/// `DELETE /training/runs/{id}` — drop a finished run from the history.
/// Refused while the run is still going: the row is what the poller and the
/// recovery sweep track a live process by, so removing it would orphan a
/// trainer nobody could then stop. `purge` also removes the work directory
/// (checkpoints and preview images included) — off by default, because those
/// files are the user's.
pub async fn delete_training_run(app: &App, id: &str, purge: bool) -> Result<()> {
    let run = reload_run(app, id).await?;
    if !run.state.is_terminal() {
        return Err(CoreError::Config(format!(
            "\"{}\" is still {} — cancel it before deleting it",
            run.name,
            run.state.as_str()
        )));
    }
    if purge {
        let dir = run_work_dir(app, &run);
        if dir.is_dir() {
            if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
                tracing::warn!(run = %run.id, error = %e, "could not remove a run's work directory");
            }
        }
    }
    app.db.training_runs().delete(id).await
}

/// The file behind `GET /training/runs/{id}/samples/{n}` — the `n`-th of the
/// latest sample images. The request contributes only the index: the path
/// itself comes from re-scanning the run's own work directory, exactly like
/// `dataset_frame_image_path` resolves a frame id.
pub async fn training_sample_path(app: &App, id: &str, n: usize) -> Result<Option<PathBuf>> {
    let Some(run) = app.db.training_runs().get(id).await? else {
        return Ok(None);
    };
    Ok(latest_sample_paths(app, &run)
        .into_iter()
        .nth(n)
        .filter(|p| p.is_file()))
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

/// Rename a model's display name. The file on disk is never touched.
pub async fn rename_model(app: &App, id: &str, name: &str) -> Result<Model> {
    app.db.models().rename(id, name).await
}

/// Replace one model's roles (e.g. add `coding` after the fact); returns the
/// cleaned set.
pub async fn set_model_roles(app: &App, id: &str, roles: &[String]) -> Result<Vec<String>> {
    if app.db.models().get(id).await?.is_none() {
        return Err(CoreError::Config(format!(
            "model {id} is not in the library"
        )));
    }
    app.db.models().set_roles(id, roles).await
}

/// The registry health line (last fetch, cache size, rate-limit, token).
pub fn registry_status(app: &App) -> crate::registry::RegistryStatus {
    app.registry.status()
}

/// Write (or clear, when blank) the Hugging Face token. Takes effect on the
/// next restart. The token lives in a machine-local file, never a backup.
pub fn set_hf_token(app: &App, token: &str) -> Result<()> {
    write_token_file(&app.paths.hf_token_file(), token)
}

/// Shared by every machine-local-file token setting (`set_hf_token`,
/// `set_civitai_token`, …): write (or delete, when blank) `path`. Never
/// touches `config.toml`, so the value never lands in a backup export.
fn write_token_file(path: &std::path::Path, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(path, token).map_err(|e| CoreError::Config(format!("write token: {e}")))
}

/// The unified local API endpoint's bearer token, if one has been configured.
/// Same machine-local-file treatment as [`set_hf_token`] — never in
/// `config.toml`, never in a backup.
pub fn local_api_token(app: &App) -> Option<String> {
    std::fs::read_to_string(app.paths.local_api_token_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write (or clear, when blank) the local API bearer token. Takes effect
/// immediately — the proxy reads the file on every request, no restart needed.
pub fn set_local_api_token(app: &App, token: &str) -> Result<()> {
    let path = app.paths.local_api_token_file();
    let token = token.trim();
    if token.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, token)
        .map_err(|e| CoreError::Config(format!("write local API token: {e}")))
}

/// `GET /local-api/status` — the endpoint address plus whether a token is
/// configured (the Settings UI shows this; the token value is never returned).
pub fn local_api_status(app: &App) -> LocalApiStatusDto {
    LocalApiStatusDto {
        endpoint: format!("http://127.0.0.1:{}/v1", app.config.core_api_port),
        token_set: local_api_token(app).is_some(),
    }
}

/// `GET /external-engines` — bring-your-own-engine (7.x): already-running
/// local LLM servers found on well-known ports (Ollama, LM Studio), so the
/// user can attach to one instead of installing AIWM's own llama-server.
pub async fn external_engines(_app: &App) -> Vec<crate::runtime::DetectedEngine> {
    crate::runtime::detect_external_engines().await
}

/// `POST /external-engines/attach` — point the llama.cpp runtime slot at an
/// already-running external server instead of a self-managed one.
pub async fn attach_external_engine(app: &App, body: AttachExternalDto) -> Result<()> {
    app.llama
        .attach_external(body.port, &body.model_id, body.vram_mb)
        .await
}

/// `POST /external-engines/detach` — release the runtime slot without
/// touching a process AIWM doesn't own (a no-op if `model_id` isn't the
/// currently resident one).
pub async fn detach_engine(app: &App, body: DetachEngineDto) -> Result<()> {
    use crate::runtime::RuntimeAdapter;
    app.llama.unload_model(&body.model_id).await
}

/// `POST /models/{id}/unload` — manually free a resident model's VRAM/RAM
/// without waiting for the scheduler to evict it for something else. Works
/// for any runtime (llama.cpp, ComfyUI, Colibri) since it looks the model up
/// by which one currently has it loaded, rather than assuming llama.cpp like
/// [`detach_engine`]. A no-op error rather than silent success when the model
/// isn't actually resident, so a stale "Unload" click says why it did nothing.
pub async fn unload_model(app: &App, model_id: &str) -> Result<()> {
    match app.runtimes.runtime_with_model(model_id) {
        Some(runtime) => runtime.unload_model(model_id).await,
        None => Err(CoreError::Config(format!(
            "model {model_id} is not currently loaded"
        ))),
    }
}

/// `POST /outputs/cleanup` — apply the configured output-retention policy to
/// `<outputs_dir>` right now (the Settings "Clean up now" button). Reads
/// `config.toml` fresh rather than the startup snapshot, so a policy change
/// just saved via `save_config` applies immediately, no restart needed. See
/// `cleanup::outputs`'s module doc for why this only ever deletes files, never
/// a job's DB row.
pub async fn cleanup_outputs(app: &App) -> Result<crate::cleanup::SweepResult> {
    let cfg = Config::read_from(&app.paths)?;
    let policy = cfg.retention.to_policy();
    let dir = app.paths.outputs_dir();
    tokio::task::spawn_blocking(move || crate::cleanup::outputs::sweep(&dir, policy))
        .await
        .map_err(|e| CoreError::Config(format!("cleanup task did not finish: {e}")))
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
/// (`GET /models/known`), each enriched with a fit verdict against the
/// current VRAM budget (from its known on-disk size — no network call).
pub fn known_models(app: &App) -> Vec<KnownModelDto> {
    let (budget_mb, free_ram_mb) = fit_inputs(app);
    crate::model::KNOWN_MODELS
        .iter()
        .map(|m| enrich_known(m, budget_mb, free_ram_mb))
        .collect()
}

/// The curated "stacks" — a base image/video model plus every companion file
/// (VAE, text encoder, …) it needs to actually run — for the Models tab's
/// "download the whole thing" button (`GET /models/stacks`).
pub fn model_stacks(app: &App) -> Vec<ModelStackDto> {
    let (budget_mb, free_ram_mb) = fit_inputs(app);
    crate::model::MODEL_STACKS
        .iter()
        .map(|s| {
            let resolved: Vec<&crate::model::KnownModel> = s
                .member_ids
                .iter()
                .filter_map(|id| crate::model::KNOWN_MODELS.iter().find(|m| &m.id == id))
                .collect();
            ModelStackDto {
                id: s.id.to_string(),
                label: s.label.to_string(),
                media: s.media.to_string(),
                note: s.note.to_string(),
                is_default: s.is_default,
                fit: combined_fit(&resolved, budget_mb, free_ram_mb),
                members: resolved
                    .iter()
                    .map(|m| enrich_known(m, budget_mb, free_ram_mb))
                    .collect(),
            }
        })
        .collect()
}

/// Fit for every member's size **summed** — a real render needs the base
/// model and every companion resident in VRAM at once, so judging a stack's
/// fit off any single member (even the largest) understates what it actually
/// takes. Found live: Flux's four members each showed green/yellow
/// individually while the combined ~19 GB plainly doesn't fit a 16 GB card.
fn combined_fit(
    members: &[&crate::model::KnownModel],
    budget_mb: u64,
    free_ram_mb: u64,
) -> crate::compat::FitVerdict {
    let total_mb: u64 = members
        .iter()
        .map(|m| m.size_bytes / crate::model::MIB)
        .sum();
    crate::compat::verdict_from_total_mb(total_mb, budget_mb, free_ram_mb)
}

fn enrich_known(m: &crate::model::KnownModel, budget_mb: u64, free_ram_mb: u64) -> KnownModelDto {
    let dims = crate::compat::ModelDims {
        size_bytes: m.size_bytes,
        ..Default::default()
    };
    let ctx = crate::compat::effective_ctx(None);
    KnownModelDto {
        id: m.id.to_string(),
        name: m.name.to_string(),
        kind: m.kind.to_string(),
        family: m.family.map(str::to_string),
        publisher: m.publisher.to_string(),
        repo: m.repo.to_string(),
        file: m.file.to_string(),
        url: m.url.to_string(),
        sha256: m.sha256.to_string(),
        size_bytes: m.size_bytes,
        license: m.license.to_string(),
        note: m.note.to_string(),
        is_default: m.is_default,
        media: m.media.to_string(),
        fit: crate::compat::verdict(&dims, ctx, budget_mb, free_ram_mb),
    }
}

/// The curated chat/coding recommendations (`GET /models/featured`), each
/// enriched with a fit verdict from `typical_vram_mb` — see
/// [`FeaturedModelDto`].
pub fn featured_models(app: &App) -> Vec<FeaturedModelDto> {
    let (budget_mb, free_ram_mb) = fit_inputs(app);
    crate::model::FEATURED_MODELS
        .iter()
        .map(|m| FeaturedModelDto {
            id: m.id.to_string(),
            role: m.role.to_string(),
            label: m.label.to_string(),
            repo: m.repo.to_string(),
            quant_hint: m.quant_hint.to_string(),
            typical_vram_mb: m.typical_vram_mb,
            license: m.license.to_string(),
            note: m.note.to_string(),
            import_roles: m.import_roles.iter().map(|s| (*s).to_string()).collect(),
            is_default: m.is_default,
            fit: crate::compat::verdict_from_total_mb(
                u64::from(m.typical_vram_mb),
                budget_mb,
                free_ram_mb,
            ),
        })
        .collect()
}

/// `(vram_budget_mb, free_ram_mb)` for a local (no-network) fit judgement —
/// shared by [`known_models`] and [`featured_models`].
fn fit_inputs(app: &App) -> (u64, u64) {
    let budget_mb = app.scheduler.budget_mb();
    let host = app.telemetry.latest().host;
    (
        budget_mb,
        host.ram_total_mb.saturating_sub(host.ram_used_mb),
    )
}

pub async fn import_model(app: &App, req: ImportRequest) -> Result<ImportOutcome> {
    crate::model::import_model(&app.db, &app.config.store_path, req).await
}

/// The curated Colibri model(s) (`GET /models/colibri`) — no network call and
/// no fit verdict computed server-side: the constraint is system RAM, and the
/// UI already has `SystemTelemetry.host` to compare against directly.
pub fn colibri_models(_app: &App) -> Vec<ColibriModelDto> {
    crate::model::COLIBRI_MODELS
        .iter()
        .map(|m| ColibriModelDto {
            id: m.id.to_string(),
            label: m.label.to_string(),
            repo: m.repo.to_string(),
            ram_estimate_mb: m.ram_estimate_mb,
            disk_estimate_bytes: m.disk_estimate_bytes,
            license: m.license.to_string(),
            note: m.note.to_string(),
        })
        .collect()
}

/// Register a Colibri model directory the user already downloaded themselves
/// (`hf download <repo> --local-dir <dir>`) as a Model Library entry.
pub async fn register_colibri_model(app: &App, req: RegisterColibriModelDto) -> Result<Model> {
    let catalog = crate::model::COLIBRI_MODELS
        .iter()
        .find(|m| m.id == req.catalog_id)
        .ok_or_else(|| CoreError::Config(format!("unknown colibri model {:?}", req.catalog_id)))?;
    crate::model::register_directory_model(
        &app.db,
        std::path::Path::new(&req.dir),
        catalog.label,
        "colibri",
        vec!["chat".into()],
        Some(i64::from(catalog.ram_estimate_mb)),
    )
    .await
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
            loaded_models: adapter.loaded_models(),
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

/// Kick off the pinned Colibri release download+extract (CPU-only — see
/// `runtime::colibri`'s module doc for why the GPU tier isn't automated) in
/// the background. Same contract as [`install_comfyui`].
pub fn install_colibri(app: &App) -> Result<&'static str> {
    if app.offline() {
        return Err(CoreError::Config(
            "offline mode is on — cannot download Colibri".into(),
        ));
    }
    if app.colibri.is_installed() {
        return Ok("already_installed");
    }
    if app.colibri.is_installing() {
        return Err(CoreError::Config(
            "a Colibri install is already running".into(),
        ));
    }

    let colibri = app.colibri.clone();
    let offline = app.offline();
    tokio::spawn(async move {
        if let Err(e) = colibri.install(offline).await {
            tracing::error!(error = %e, "Colibri install failed");
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

/// Deterministic "is a newer version available upstream" check for the five
/// externally-sourced tools AIWM installs or resolves (`GET
/// /runtimes/versions`) — see [`crate::runtime`]'s `version_check` module doc
/// for exactly which upstream each tool is checked against. **Not** the
/// LLM-driven per-model [`upgrade_check`] above (job type `upgrade_check`,
/// Phase 6.7) — that answers "is there a better MODEL on Hugging Face"; this
/// answers "did AIWM's own curated pin for ComfyUI/llama.cpp/Colibri/Hermes/
/// OpenCode fall behind upstream". Refuses up front in offline mode, exactly
/// like `install_llamacpp` et al.
pub async fn check_tool_versions(app: &App) -> Result<Vec<crate::runtime::ToolVersionCheck>> {
    crate::runtime::check_versions(app.offline()).await
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

// --- sessions ---------------------------------------------------------------

const SESSION_CAPABILITIES: [&str; 3] = ["chat", "image", "video"];

pub async fn create_session(app: &App, body: NewSessionDto) -> Result<Session> {
    if body.name.trim().is_empty() {
        return Err(CoreError::Config("session name must not be empty".into()));
    }
    if !SESSION_CAPABILITIES.contains(&body.capability.as_str()) {
        return Err(CoreError::Config(format!(
            "unknown session capability {:?}",
            body.capability
        )));
    }
    app.db
        .sessions()
        .create(&body.capability, body.name.trim())
        .await
}

pub async fn list_sessions(app: &App, capability: &str) -> Result<Vec<Session>> {
    app.db.sessions().list_for(capability).await
}

pub async fn rename_session(app: &App, id: &str, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(CoreError::Config("session name must not be empty".into()));
    }
    app.db.sessions().rename(id, name.trim()).await
}

pub async fn set_session_archived(app: &App, id: &str, archived: bool) -> Result<()> {
    app.db.sessions().set_archived(id, archived).await
}

pub async fn delete_session(app: &App, id: &str) -> Result<()> {
    app.db.sessions().delete(id).await
}

// --- personas (spec `2026-09-18-personas-design`) ---------------------------
//
// Every "unknown id" here comes back as an `Option`/enum rather than an error,
// so the transports can turn it into a 404; the *limits* come back as
// `CoreError::Config`, which is already a 400.

pub async fn list_personas(app: &App) -> Result<Vec<Persona>> {
    app.db.personas().list().await
}

pub async fn create_persona(app: &App, body: PersonaBodyDto) -> Result<Persona> {
    persona::create(&app.db, &body.name, &body.icon, &body.system_prompt).await
}

/// `None` = no such persona (404).
pub async fn update_persona(app: &App, id: &str, body: PersonaBodyDto) -> Result<Option<Persona>> {
    persona::update(&app.db, id, &body.name, &body.icon, &body.system_prompt).await
}

/// Deleting also clears every reference to the persona (sessions pointing at it,
/// the global active key). Idempotent, like sessions and documents: an unknown id
/// is a success, and the returned flag only says whether a row was really there.
pub async fn delete_persona(app: &App, id: &str) -> Result<bool> {
    app.db.personas().delete(id).await
}

pub async fn active_persona(app: &App) -> Result<ActivePersonaDto> {
    Ok(ActivePersonaDto {
        id: persona::active(&app.db).await?.map(|p| p.id),
    })
}

/// `false` = the id names no persona (404). `{ id: null }` clears the global
/// choice.
///
/// Takes the same [`ActivePersonaDto`] [`active_persona`] hands back, so HTTP and
/// Tauri cross into the core through one shape rather than two.
pub async fn set_active_persona(app: &App, body: ActivePersonaDto) -> Result<bool> {
    persona::set_active(&app.db, body.id.as_deref()).await
}

pub async fn set_session_persona(
    app: &App,
    session_id: &str,
    body: SetSessionPersonaDto,
) -> Result<persona::SetSessionPersona> {
    persona::set_session_persona(&app.db, session_id, body.mode, body.persona_id.as_deref()).await
}

/// The persona a chat would actually use plus where it came from, so the Chat
/// tab's chip never re-implements the rule. `session_id` is optional: `None` is
/// the "Ungrouped" case, where only the global persona applies.
pub async fn effective_persona(
    app: &App,
    session_id: Option<&str>,
) -> Result<persona::EffectivePersona> {
    persona::resolve_effective(&app.db, session_id).await
}

// --- documents / local RAG (7.x) -------------------------------------------

/// `POST /sessions/{id}/documents` — read, chunk, and store a document for
/// local RAG grounding in this chat session. `path` is a path on this
/// machine; the file picker resolves it before calling here.
pub async fn attach_document(
    app: &App,
    session_id: &str,
    path: &str,
) -> Result<crate::db::Document> {
    if app.db.sessions().get(session_id).await?.is_none() {
        return Err(CoreError::Config(format!(
            "session {session_id} does not exist"
        )));
    }
    let file_path = std::path::Path::new(path);
    let name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();
    let format = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = crate::rag::read_document(file_path)?;
    let chunks = crate::rag::chunk_text(
        &text,
        crate::rag::DEFAULT_CHUNK_WORDS,
        crate::rag::DEFAULT_OVERLAP_WORDS,
    );
    app.db
        .documents()
        .insert(
            crate::db::NewDocument {
                session_id: session_id.to_string(),
                name,
                source_path: path.to_string(),
                format,
            },
            &chunks,
        )
        .await
}

pub async fn list_documents(app: &App, session_id: &str) -> Result<Vec<crate::db::Document>> {
    app.db.documents().list_for_session(session_id).await
}

pub async fn delete_document(app: &App, id: &str) -> Result<()> {
    app.db.documents().delete(id).await
}

// --- voice identities (Dia voice cloning) -----------------------------------

/// `POST /voice-identities` — save a reference clip + transcript under a
/// name, once, for reuse across many Dia narration calls. Copies the picked
/// file into AIWM's own data dir (`voice_identity::create_voice_identity`)
/// rather than referencing it in place.
pub async fn create_voice_identity(app: &App, body: NewVoiceIdentityDto) -> Result<VoiceIdentity> {
    voice_identity::create_voice_identity(
        &app.db,
        &app.paths.voice_identities_dir(),
        CreateVoiceIdentity {
            name: body.name,
            source_audio_path: PathBuf::from(body.source_audio_path),
            reference_transcript: body.reference_transcript,
        },
    )
    .await
}

pub async fn list_voice_identities(app: &App) -> Result<Vec<VoiceIdentity>> {
    app.db.voice_identities().list().await
}

pub async fn delete_voice_identity(app: &App, id: &str) -> Result<()> {
    let Some(identity) = app.db.voice_identities().get(id).await? else {
        return Ok(()); // already gone — deleting is idempotent, same as sessions/documents
    };
    voice_identity::delete_voice_identity(&app.db, &identity).await
}

// --- Story Studio (Phase 1: text + plain image, docs/TODO.md) -------------
//
// Portrait/reference images and Scene images are never generated here --
// they are plain `job_id`s pointing at a `job_type=image` job the caller
// already submitted through the ordinary Image capability (`submit_job`
// above). Story Studio only remembers which job was picked.

pub async fn list_stories(app: &App) -> Result<Vec<Story>> {
    app.db.stories().list().await
}

pub async fn create_story(app: &App, body: StoryBodyDto) -> Result<Story> {
    app.db.stories().create(story_update_from(body)).await
}

pub async fn update_story(app: &App, id: &str, body: StoryBodyDto) -> Result<()> {
    app.db.stories().update(id, story_update_from(body)).await
}

fn story_update_from(body: StoryBodyDto) -> StoryUpdate {
    StoryUpdate {
        name: body.name,
        setting: body.setting,
        art_style: body.art_style,
        premise: body.premise,
    }
}

pub async fn delete_story(app: &App, id: &str) -> Result<()> {
    app.db.stories().delete(id).await
}

pub async fn list_characters(app: &App, story_id: &str) -> Result<Vec<Character>> {
    app.db.characters().list_for_story(story_id).await
}

pub async fn create_character(
    app: &App,
    story_id: &str,
    body: CharacterBodyDto,
) -> Result<Character> {
    app.db
        .characters()
        .create(NewCharacter {
            story_id: story_id.to_string(),
            name: body.name,
            traits: body.traits,
            backstory: body.backstory,
            alignment: body.alignment,
        })
        .await
}

pub async fn update_character(app: &App, id: &str, body: CharacterBodyDto) -> Result<()> {
    app.db
        .characters()
        .update(
            id,
            CharacterUpdate {
                name: body.name,
                traits: body.traits,
                backstory: body.backstory,
                alignment: body.alignment,
            },
        )
        .await
}

pub async fn delete_character(app: &App, id: &str) -> Result<()> {
    app.db.characters().delete(id).await
}

/// Pick (`Some`) or clear (`None`) a character's reference portrait.
pub async fn set_character_portrait(app: &App, id: &str, job_id: Option<&str>) -> Result<()> {
    app.db.characters().set_portrait(id, job_id).await
}

pub async fn set_character_inventory(app: &App, id: &str, items: &[String]) -> Result<()> {
    app.db.characters().set_inventory(id, items).await
}

pub async fn list_character_relationships(
    app: &App,
    id: &str,
) -> Result<Vec<CharacterRelationship>> {
    app.db.characters().list_relationships(id).await
}

pub async fn add_character_relationship(
    app: &App,
    character_id: &str,
    related_character_id: &str,
    note: &str,
) -> Result<CharacterRelationship> {
    app.db
        .characters()
        .add_relationship(character_id, related_character_id, note)
        .await
}

pub async fn remove_character_relationship(app: &App, id: &str) -> Result<()> {
    app.db.characters().remove_relationship(id).await
}

pub async fn character_log(app: &App, id: &str) -> Result<Vec<CharacterLogEntry>> {
    app.db.characters().logs_for(id).await
}

pub async fn list_npcs(app: &App, story_id: &str) -> Result<Vec<Npc>> {
    app.db.npcs().list_for_story(story_id).await
}

pub async fn create_npc(app: &App, story_id: &str, body: NpcBodyDto) -> Result<Npc> {
    app.db
        .npcs()
        .create(NewNpc {
            story_id: story_id.to_string(),
            name: body.name,
            role: body.role,
            location_id: body.location_id,
            description: body.description,
        })
        .await
}

pub async fn update_npc(app: &App, id: &str, body: NpcBodyDto) -> Result<()> {
    app.db
        .npcs()
        .update(
            id,
            NpcUpdate {
                name: body.name,
                role: body.role,
                location_id: body.location_id,
                description: body.description,
            },
        )
        .await
}

pub async fn delete_npc(app: &App, id: &str) -> Result<()> {
    app.db.npcs().delete(id).await
}

pub async fn list_locations(app: &App, story_id: &str) -> Result<Vec<Location>> {
    app.db.locations().list_for_story(story_id).await
}

pub async fn create_location(app: &App, story_id: &str, body: LocationBodyDto) -> Result<Location> {
    app.db
        .locations()
        .create(NewLocation {
            story_id: story_id.to_string(),
            name: body.name,
            description: body.description,
        })
        .await
}

pub async fn update_location(app: &App, id: &str, body: LocationBodyDto) -> Result<()> {
    app.db
        .locations()
        .update(
            id,
            LocationUpdate {
                name: body.name,
                description: body.description,
            },
        )
        .await
}

pub async fn delete_location(app: &App, id: &str) -> Result<()> {
    app.db.locations().delete(id).await
}

/// Pick (`Some`) or clear (`None`) a location's reference image.
pub async fn set_location_reference(app: &App, id: &str, job_id: Option<&str>) -> Result<()> {
    app.db.locations().set_reference(id, job_id).await
}

fn dialogue_from(lines: Vec<DialogueLineDto>) -> Vec<NewDialogueLine> {
    lines
        .into_iter()
        .map(|l| NewDialogueLine {
            character_id: l.character_id,
            text: l.text,
        })
        .collect()
}

/// Composes a [`SceneDetailDto`] -- everything the Timeline needs for one
/// scene card -- from the scene row plus its participants, dialogue, and
/// images (mirrors how `job_detail` composes a job with its events).
async fn scene_detail(app: &App, scene: Scene) -> Result<SceneDetailDto> {
    let participant_ids = app.db.scenes().participants(&scene.id).await?;
    let dialogue = app.db.scenes().dialogue(&scene.id).await?;
    let images = app.db.scene_images().list_for_scene(&scene.id).await?;
    Ok(SceneDetailDto {
        scene,
        participant_ids,
        dialogue,
        images,
    })
}

/// Every scene in a story's timeline, in order, each with its full detail.
pub async fn list_scenes(app: &App, story_id: &str) -> Result<Vec<SceneDetailDto>> {
    let scenes = app.db.scenes().list_for_story(story_id).await?;
    let mut out = Vec::with_capacity(scenes.len());
    for scene in scenes {
        out.push(scene_detail(app, scene).await?);
    }
    Ok(out)
}

pub async fn create_scene(app: &App, story_id: &str, body: SceneBodyDto) -> Result<SceneDetailDto> {
    let scene = app
        .db
        .scenes()
        .create(NewScene {
            story_id: story_id.to_string(),
            location_id: body.location_id,
            narrative: body.narrative,
            redline: body.redline,
            participant_ids: body.participant_ids,
            dialogue: dialogue_from(body.dialogue),
        })
        .await?;
    scene_detail(app, scene).await
}

pub async fn update_scene(app: &App, id: &str, body: SceneBodyDto) -> Result<SceneDetailDto> {
    app.db
        .scenes()
        .update(
            id,
            SceneUpdate {
                location_id: body.location_id,
                narrative: body.narrative,
                redline: body.redline,
                participant_ids: body.participant_ids,
                dialogue: dialogue_from(body.dialogue),
            },
        )
        .await?;
    let scene = app
        .db
        .scenes()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::Db(format!("no scene {id}")))?;
    scene_detail(app, scene).await
}

pub async fn delete_scene(app: &App, id: &str) -> Result<()> {
    app.db.scenes().delete(id).await
}

/// Attaches an already-submitted `job_type=image` job as a new image for a
/// scene (the first one becomes canonical automatically).
pub async fn add_scene_image(app: &App, scene_id: &str, job_id: &str) -> Result<SceneImage> {
    app.db.scene_images().add(scene_id, job_id).await
}

pub async fn set_canonical_scene_image(app: &App, id: &str) -> Result<()> {
    app.db.scene_images().set_canonical(id).await
}

pub async fn delete_scene_image(app: &App, id: &str) -> Result<()> {
    app.db.scene_images().delete(id).await
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

// --- external launcher ------------------------------------------------------

/// Open a real, independent terminal running the requested tool against a
/// pinned local model. See the `launcher` module docs for why this
/// deliberately does not go through the same Job-Object supervision as
/// everything else `App` spawns.
pub async fn launch_external(app: &App, body: LaunchExternalDto) -> Result<LaunchInfo> {
    if body.workspace.trim().is_empty() {
        return Err(CoreError::Config("workspace path must not be empty".into()));
    }
    app.launcher
        .launch(LaunchRequest {
            tool: body.tool,
            model_id: body.model_id.filter(|s| !s.trim().is_empty()),
            workspace: PathBuf::from(body.workspace.trim()),
        })
        .await
}

/// What's pinned for an external launch right now, if anything.
pub fn launcher_status(app: &App) -> Option<LaunchInfo> {
    app.launcher.status()
}

/// Release the pinned model. Cannot close the terminal window itself — see
/// the `launcher` module docs.
pub async fn stop_external_launch(app: &App) -> Result<()> {
    app.launcher.stop().await
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

// --- model discovery (Phase 6.2, + Civitai) ----------------------------------

/// `GET /registry/search` — the "Discover" panel's Hugging Face source.
/// `Fetched.freshness` tells the UI whether this is `live`, a `stale` cache
/// (the Hub was unreachable) or an `offline` cache.
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
    let (budget_mb, free_ram_mb) = fit_inputs(app);
    let d = fetched.data;
    let files = d
        .files
        .iter()
        .map(|f| enrich_hf_file(id, &d.revision, &d.model, f, budget_mb, free_ram_mb))
        .collect();
    Ok(RegistryDetailsDto {
        model: d.model,
        revision: d.revision,
        files,
        freshness: fetched.freshness,
    })
}

/// The registry health line for the Civitai source (Diagnostics).
pub fn civitai_status(app: &App) -> crate::registry::RegistryStatus {
    app.civitai_registry.status()
}

/// Write (or clear, when blank) the Civitai API key. Takes effect on the next
/// restart. Same machine-local-file treatment as [`set_hf_token`] — never in
/// a backup. Only needed for gated/early-access Civitai content; anonymous
/// browsing works without one.
pub fn set_civitai_token(app: &App, token: &str) -> Result<()> {
    write_token_file(&app.paths.civitai_token_file(), token)
}

/// `GET /civitai/search` — the "Discover" panel's Civitai source, for
/// image/video checkpoints and LoRAs. Defaults to excluding NSFW results
/// (`CivitaiSearchDto::nsfw` defaults `false`) — the caller must opt in.
pub async fn civitai_search(
    app: &App,
    params: CivitaiSearchDto,
) -> Result<Fetched<Vec<RemoteModel>>> {
    app.civitai_registry.search(&params.into_query()).await
}

/// `GET /civitai/models/{id}` — one Civitai model's primary version, with
/// every file enriched with a VRAM fit verdict *and* Civitai's own
/// pickle/virus-scan verdicts (surfaced verbatim, never hidden — see
/// `registry::civitai`'s module doc for why that is informational only and
/// never a substitute for AIWM's own import-time Pickle guard).
pub async fn civitai_details(app: &App, id: &str) -> Result<RegistryDetailsDto> {
    let fetched = app.civitai_registry.details(id).await?;
    let (budget_mb, free_ram_mb) = fit_inputs(app);
    let d = fetched.data;
    let files = d
        .files
        .iter()
        .map(|f| enrich_civitai_file(&d.model, f, budget_mb, free_ram_mb))
        .collect();
    Ok(RegistryDetailsDto {
        model: d.model,
        revision: d.revision,
        files,
        freshness: fetched.freshness,
    })
}

/// The GGUF / safetensors weight files, not `README.md` / `config.json`. A
/// Civitai Pickle/unknown-format file never reaches here as "safetensors":
/// `registry::civitai::format_of_version` only ever reports
/// [`RemoteFormat::Safetensors`] when every real weight file in the version
/// is actually safetensors-format.
fn is_weight_file(model: &RemoteModel, path: &str) -> bool {
    matches!(model.format, RemoteFormat::Gguf | RemoteFormat::Safetensors)
        && (path.ends_with(".gguf") || path.ends_with(".safetensors"))
}

/// VRAM estimate + fit verdict for one file — shared by every source, since
/// it depends only on the source-agnostic [`RemoteModel`] / [`RemoteFile`]
/// fields, never on how a URL is built.
fn weight_fit(
    model: &RemoteModel,
    f: &RemoteFile,
    budget_mb: u64,
    free_ram_mb: u64,
) -> (Option<u64>, FitVerdict) {
    if !is_weight_file(model, &f.path) || f.size == 0 {
        return (None, FitVerdict::Unknown);
    }
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
}

/// Hugging Face has no per-file download URL of its own — build the
/// `/resolve/<rev>/<path>` link from the repo id + revision.
fn enrich_hf_file(
    id: &str,
    revision: &str,
    model: &RemoteModel,
    f: &RemoteFile,
    budget_mb: u64,
    free_ram_mb: u64,
) -> RegistryFileDto {
    let (vram_estimate_mb, fit) = weight_fit(model, f, budget_mb, free_ram_mb);
    RegistryFileDto {
        download_url: f.download_url.clone().unwrap_or_else(|| {
            format!("https://huggingface.co/{id}/resolve/{revision}/{}", f.path)
        }),
        path: f.path.clone(),
        size_bytes: f.size,
        sha256: f.sha256.clone(),
        quant: f.quant.clone(),
        shard: f.shard.map(|(a, b)| [a, b]),
        vram_estimate_mb,
        fit,
        pickle_scan_result: f.pickle_scan_result.clone(),
        virus_scan_result: f.virus_scan_result.clone(),
    }
}

/// Civitai always hands back its own absolute `downloadUrl` per file — never
/// fall back to Hugging Face's `/resolve/` URL shape for a Civitai file (that
/// would build a broken cross-source URL out of a numeric Civitai id).
fn enrich_civitai_file(
    model: &RemoteModel,
    f: &RemoteFile,
    budget_mb: u64,
    free_ram_mb: u64,
) -> RegistryFileDto {
    let (vram_estimate_mb, fit) = weight_fit(model, f, budget_mb, free_ram_mb);
    RegistryFileDto {
        download_url: f.download_url.clone().unwrap_or_default(),
        path: f.path.clone(),
        size_bytes: f.size,
        sha256: f.sha256.clone(),
        quant: f.quant.clone(),
        shard: f.shard.map(|(a, b)| [a, b]),
        vram_estimate_mb,
        fit,
        pickle_scan_result: f.pickle_scan_result.clone(),
        virus_scan_result: f.virus_scan_result.clone(),
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
            roles: dto.roles,
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

/// Remove one finished (`done` / `failed`) download from the history —
/// housekeeping only, never touches an already-imported model.
pub async fn delete_download(app: &App, id: &str) -> Result<()> {
    app.downloads.delete(id).await
}

/// Clear every finished download at once — the "history is full of stuff I
/// already deleted" cleanup. Returns how many rows were removed.
pub async fn clear_finished_downloads(app: &App) -> Result<u64> {
    app.downloads.clear_finished().await
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

/// The built-in, versioned benchmark suites a run may name — the Benchmark
/// tab's picker. Static data; no store access.
pub fn bench_suites() -> &'static [crate::bench::suites::Suite] {
    crate::bench::suites::all()
}

/// Default rows for [`benchmark_history`] when the caller names no `limit`.
const DEFAULT_HISTORY_LIMIT: i64 = 50;
/// Upper bound for [`benchmark_history`], below the store's own `500` — the
/// Benchmark tab compares a screenful, it never needs the whole table.
const MAX_HISTORY_LIMIT: i64 = 200;

/// A suite id the caller supplied, checked against the built-in catalogue.
/// `None`/blank means "no suite", which is a valid ask everywhere here.
fn known_suite(id: Option<&str>) -> Result<Option<&'static str>> {
    let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    crate::bench::suites::find(id)
        .map(|s| Some(s.id))
        .ok_or_else(|| CoreError::Config(format!("unknown benchmark suite \"{id}\"")))
}

/// Benchmarks across all models, newest first — the Benchmark tab's history.
/// An unknown `suite` is a refusal rather than an empty list: silently showing
/// nothing would read as "this model was never tested".
pub async fn benchmark_history(
    app: &App,
    suite: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<Benchmark>> {
    let suite = known_suite(suite)?;
    let limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    app.db.benchmarks().list_all(suite, limit).await
}

/// Queue a "Test model" job for a local GGUF model. It goes through the
/// scheduler like a chat job (load / evict / run), then [`crate::bench`]
/// records a `benchmarks` row.
///
/// `opts` is what the Benchmark tab adds on top of the Model Library's plain
/// button: a suite to run instead of the single default prompt, and how many
/// passes per prompt. Omitting both is the original quick test, byte for byte.
pub async fn benchmark_model(app: &App, model_id: &str, opts: BenchmarkOptionsDto) -> Result<Job> {
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
    let suite = known_suite(opts.suite.as_deref())?;
    let ctx = crate::compat::effective_ctx(model.ctx_max.and_then(|v| u32::try_from(v).ok()));
    let vram = crate::compat::estimate(&model.vram_dims(), ctx).total_mb;
    let mut new = NewJob::new("bench").on("llamacpp", &model.id, vram);
    // Only written when asked for — a plain quick test keeps the exact params
    // it had before suites existed. `runs` goes through unvalidated on purpose:
    // `BenchRequest::from_params` clamps it to `1..=10`.
    if let Some(suite) = suite {
        new.params["suite"] = suite.into();
    }
    if let Some(runs) = opts.runs {
        new.params["runs"] = runs.into();
    }
    app.jobs.submit(new).await
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
    use std::net::Ipv4Addr;

    use axum::routing::get;
    use axum::{Json, Router};

    use super::*;
    use crate::registry::SearchSort;
    use crate::runtime::RuntimeAdapter;

    /// A live `App` on a throwaway data dir, plus one dataset and two frames
    /// in it — the shape every dataset/concept handler test needs. The
    /// returned `TempDir` must outlive the `App` (it backs its data dir).
    async fn dataset_fixture() -> (std::sync::Arc<App>, tempfile::TempDir, String, Vec<String>) {
        let tmp = tempfile::tempdir().unwrap();
        let app = std::sync::Arc::new(
            App::load(crate::AppPaths::rooted(tmp.path()))
                .await
                .unwrap(),
        );
        let job = app
            .db
            .jobs()
            .insert(NewJob::new("dataset_prep"))
            .await
            .unwrap();
        let ds = app
            .db
            .datasets()
            .create(crate::db::NewDataset {
                name: "Demo".into(),
                mode: crate::db::DatasetMode::Clips,
                source_root: "E:\\Data\\Demo".into(),
                prep_job_id: Some(job.id.clone()),
            })
            .await
            .unwrap();
        let mut frames = Vec::new();
        for i in 0..2 {
            let f = app
                .db
                .dataset_frames()
                .insert(crate::db::NewDatasetFrame {
                    job_id: job.id.clone(),
                    dataset_id: Some(ds.id.clone()),
                    tag: "Ghibli".into(),
                    source_path: "clip.mp4".into(),
                    frame_path: format!("clip_{i}.png"),
                    timestamp_secs: Some(f64::from(i)),
                    rejection_reason: String::new(),
                    duration_secs: Some(6.0),
                })
                .await
                .unwrap();
            frames.push(f.id);
        }
        (app, tmp, ds.id, frames)
    }

    fn frame_edit() -> UpdateDatasetFrameDto {
        UpdateDatasetFrameDto {
            caption: None,
            excluded: None,
            restore: None,
            clip_start_secs: None,
            clip_end_secs: None,
        }
    }

    /// `set_clip_range` writes *both* bounds, so a one-sided edit has to read
    /// the row first and carry the other bound through — otherwise clearing
    /// the out-point would silently throw the in-point away too.
    #[tokio::test]
    async fn clearing_one_clip_bound_leaves_the_other_one_alone() {
        let (app, _tmp, _ds, frames) = dataset_fixture().await;

        let both = update_dataset_frame(
            &app,
            &frames[0],
            UpdateDatasetFrameDto {
                clip_start_secs: Some(Some(1.5)),
                clip_end_secs: Some(Some(4.0)),
                ..frame_edit()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (both.clip_start_secs, both.clip_end_secs),
            (Some(1.5), Some(4.0))
        );

        let cleared = update_dataset_frame(
            &app,
            &frames[0],
            UpdateDatasetFrameDto {
                clip_end_secs: Some(None),
                ..frame_edit()
            },
        )
        .await
        .unwrap();
        assert_eq!(cleared.clip_start_secs, Some(1.5), "in-point survives");
        assert_eq!(cleared.clip_end_secs, None, "out-point cleared");
    }

    /// "Doch behalten" on an auto-rejected frame: the rejection goes away and
    /// the frame rejoins the kept set.
    #[tokio::test]
    async fn restore_clears_the_rejection_reason() {
        let (app, _tmp, _ds, frames) = dataset_fixture().await;
        app.db
            .dataset_frames()
            .set_rejection_reason(&frames[0], "blur")
            .await
            .unwrap();

        let restored = update_dataset_frame(
            &app,
            &frames[0],
            UpdateDatasetFrameDto {
                restore: Some(true),
                ..frame_edit()
            },
        )
        .await
        .unwrap();
        assert_eq!(restored.rejection_reason, "");
    }

    fn concept_body(name: &str, token: &str) -> ConceptBodyDto {
        ConceptBodyDto {
            name: name.into(),
            token: token.into(),
            description: String::new(),
        }
    }

    /// Two concepts in one dataset sharing a token is a user mistake, not a
    /// server fault: without the pre-check the UNIQUE (dataset_id, token)
    /// violation would surface as `CoreError::Db` -> HTTP 500.
    #[tokio::test]
    async fn creating_a_concept_with_a_taken_token_is_a_config_error() {
        let (app, _tmp, ds, _frames) = dataset_fixture().await;
        create_concept(&app, &ds, concept_body("Kenji", "kenji_xy"))
            .await
            .unwrap();

        let dup = create_concept(&app, &ds, concept_body("Other", " kenji_xy ")).await;
        assert!(matches!(dup, Err(CoreError::Config(_))), "got: {dup:?}");

        let blank = create_concept(&app, &ds, concept_body("Kenji", "  ")).await;
        assert!(matches!(blank, Err(CoreError::Config(_))), "got: {blank:?}");
    }

    /// Renaming a concept to its *own* token must stay legal — the duplicate
    /// pre-check has to exclude the row being edited.
    #[tokio::test]
    async fn updating_a_concept_allows_its_own_token_but_not_a_sibling_s() {
        let (app, _tmp, ds, _frames) = dataset_fixture().await;
        let kenji = create_concept(&app, &ds, concept_body("Kenji", "kenji_xy"))
            .await
            .unwrap();
        create_concept(&app, &ds, concept_body("Mira", "mira_xy"))
            .await
            .unwrap();

        update_concept(&app, &kenji.id, concept_body("Kenji R.", "kenji_xy"))
            .await
            .unwrap();
        assert_eq!(
            app.db
                .concepts()
                .get(&kenji.id)
                .await
                .unwrap()
                .unwrap()
                .name,
            "Kenji R."
        );

        let clash = update_concept(&app, &kenji.id, concept_body("Kenji", "mira_xy")).await;
        assert!(matches!(clash, Err(CoreError::Config(_))), "got: {clash:?}");
    }

    /// "Alle im Set" sends the whole visible page; the response tells the user
    /// how much of it actually landed.
    #[tokio::test]
    async fn assigning_reports_requested_and_newly_attached_separately() {
        let (app, _tmp, ds, frames) = dataset_fixture().await;
        let c = create_concept(&app, &ds, concept_body("Kenji", "kenji_xy"))
            .await
            .unwrap();
        assign_concept(
            &app,
            &c.id,
            ConceptFramesDto {
                frame_ids: vec![frames[0].clone()],
            },
        )
        .await
        .unwrap();

        // frames[0] again, frames[1] new, and an id that is not a frame at all.
        let got = assign_concept(
            &app,
            &c.id,
            ConceptFramesDto {
                frame_ids: vec![frames[0].clone(), frames[1].clone(), "nope".into()],
            },
        )
        .await
        .unwrap();
        assert_eq!((got.requested, got.attached), (3, 1));

        let summaries = list_concepts(&app, &ds).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].frame_count, 2);
        assert_eq!(summaries[0].token_warning, None, "kenji_xy is a good token");
    }

    /// An ordinary word as a token trains nothing new — the summary carries
    /// the warning the Concepts panel shows inline.
    #[tokio::test]
    async fn a_common_word_token_comes_back_with_a_warning() {
        let (app, _tmp, ds, _frames) = dataset_fixture().await;
        create_concept(&app, &ds, concept_body("Anime look", "anime"))
            .await
            .unwrap();
        let summaries = list_concepts(&app, &ds).await.unwrap();
        assert!(
            summaries[0]
                .token_warning
                .as_deref()
                .is_some_and(|w| w.contains("anime")),
            "got: {:?}",
            summaries[0].token_warning
        );
    }

    /// Bare-minimum ComfyUI stand-in — just enough for `attach` + `health` to
    /// succeed, mirroring `runtime::comfyui`'s own test fixture.
    async fn mock_comfy() -> u16 {
        let router = Router::new().route(
            "/system_stats",
            get(|| async {
                Json(serde_json::json!({
                    "system": { "comfyui_version": "0.34.0-mock" },
                    "devices": [{ "name": "cuda:0 test", "vram_total": 17_000_000_000u64, "vram_free": 15_000_000_000u64 }]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    /// End-to-end wiring check for the 3.7 live-apply gap: saving a changed
    /// `[comfyui]` block through the same handler the HTTP/Tauri surfaces call
    /// must reach `ComfyUiAdapter::set_options` with the freshly derived
    /// options — not just rewrite `config.toml` and leave the running server
    /// on its old flags until a full app restart.
    #[tokio::test]
    async fn save_config_live_applies_a_changed_comfyui_vram_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();

        let port = mock_comfy().await;
        app.comfyui.attach(port).await.unwrap();
        assert!(!app.comfyui.detail().unwrap().contains("lowvram"));

        let mut update = ConfigUpdate {
            store_path: app.config.store_path.display().to_string(),
            offline_mode: false,
            vram_budget_mb: app.config.vram_budget_mb,
            llama: app.config.llama.clone(),
            comfyui: app.config.comfyui.clone(),
            models: app.config.models,
            paths: Default::default(),
            retention: Default::default(),
        };
        update.comfyui.vram_mode = "lowvram".to_string();

        let saved = save_config(&app, update).await.unwrap();
        assert_eq!(saved.comfyui.vram_mode, "lowvram");

        // Reached the real adapter: an attached server isn't restarted (not
        // ours to kill), but the new options are recorded live, immediately
        // visible in `detail()` -- no app restart needed.
        let detail = app.comfyui.detail().unwrap();
        assert!(detail.contains("lowvram"), "{detail}");
        assert!(
            detail.starts_with(&format!("attached to :{port}")),
            "{detail}"
        );
        assert_eq!(app.comfyui.health().await, crate::runtime::Health::Healthy);
    }

    /// The common case (an unrelated field changes, `[comfyui]` doesn't)
    /// must not touch the running server at all.
    #[tokio::test]
    async fn save_config_leaves_comfyui_alone_when_its_block_is_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();

        let port = mock_comfy().await;
        app.comfyui.attach(port).await.unwrap();

        let update = ConfigUpdate {
            store_path: app.config.store_path.display().to_string(),
            offline_mode: false,
            vram_budget_mb: 12_000, // an unrelated field changes
            llama: app.config.llama.clone(),
            comfyui: app.config.comfyui.clone(), // unchanged
            models: app.config.models,
            paths: Default::default(),
            retention: Default::default(),
        };

        let saved = save_config(&app, update).await.unwrap();
        assert_eq!(saved.vram_budget_mb, 12_000);
        // Still attached on the same port, health untouched -- no restart
        // attempt was made for an unrelated config change.
        assert_eq!(app.comfyui.health().await, crate::runtime::Health::Healthy);
    }

    #[test]
    fn combined_fit_sums_every_members_size_not_just_the_base() {
        let flux: Vec<&crate::model::KnownModel> = crate::model::MODEL_STACKS
            .iter()
            .find(|s| s.id == "flux")
            .unwrap()
            .member_ids
            .iter()
            .filter_map(|id| crate::model::KNOWN_MODELS.iter().find(|m| &m.id == id))
            .collect();
        assert_eq!(flux.len(), 4, "flux stack: model + T5 + CLIP-L + VAE");

        // A budget sized for the base model alone plus real headroom --
        // comfortable for member [0] by itself, but not for all four summed.
        let base_alone_mb = flux[0].size_bytes / crate::model::MIB;
        let budget_mb = base_alone_mb + 4_000;

        let base_only = crate::compat::verdict_from_total_mb(base_alone_mb, budget_mb, 32_000);
        assert_eq!(base_only, crate::compat::FitVerdict::Green);

        let combined = combined_fit(&flux, budget_mb, 32_000);
        assert!(
            matches!(
                combined,
                crate::compat::FitVerdict::Red { .. } | crate::compat::FitVerdict::Yellow { .. }
            ),
            "expected the whole stack to be tight/red at a budget sized for the base model alone, got {combined:?}"
        );
    }

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

    #[tokio::test]
    async fn clean_audio_reports_a_clear_error_for_a_nonexistent_job() {
        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();

        let err = clean_audio(&app, "no-such-job").await.unwrap_err();
        assert!(err.to_string().contains("no such job"), "{err}");
    }

    #[tokio::test]
    async fn clean_audio_refuses_a_job_that_is_not_narration() {
        use crate::db::NewJob;

        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();
        let job = app.db.jobs().insert(NewJob::new("image")).await.unwrap();

        let err = clean_audio(&app, &job.id).await.unwrap_err();
        assert!(err.to_string().contains("only narration clips"), "{err}");
    }

    #[tokio::test]
    async fn cleanup_outputs_is_a_noop_until_a_policy_is_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();
        let outputs = app.paths.outputs_dir();
        std::fs::create_dir_all(&outputs).unwrap();
        let old_file = outputs.join("job-old.png");
        std::fs::write(&old_file, b"stale bytes").unwrap();
        let ancient = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 86_400);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&old_file)
            .unwrap()
            .set_modified(ancient)
            .unwrap();

        // No retention configured yet -- the file must survive.
        let result = cleanup_outputs(&app).await.unwrap();
        assert_eq!(result.deleted_files, 0);
        assert!(old_file.exists());

        // Save a policy through the same path the Settings UI uses, then the
        // very next cleanup call (no restart) picks it up and removes it.
        let mut cfg = config(&app).unwrap();
        cfg.retention = crate::config::RetentionConfig {
            max_age_days: 30,
            max_total_mb: 0,
        };
        save_config(
            &app,
            ConfigUpdate {
                store_path: cfg.store_path.display().to_string(),
                offline_mode: cfg.offline_mode,
                vram_budget_mb: cfg.vram_budget_mb,
                llama: cfg.llama,
                comfyui: cfg.comfyui,
                models: cfg.models,
                paths: crate::api::dto::PathsUpdateDto::default(),
                retention: cfg.retention,
            },
        )
        .await
        .unwrap();

        let result = cleanup_outputs(&app).await.unwrap();
        assert_eq!(result.deleted_files, 1);
        assert_eq!(result.freed_bytes, "stale bytes".len() as u64);
        assert!(!old_file.exists(), "the stale output should be gone");
    }

    #[tokio::test]
    async fn runtimes_reports_which_models_are_loaded_where() {
        let tmp = tempfile::tempdir().unwrap();
        let app = crate::App::load(crate::AppPaths::rooted(tmp.path()))
            .await
            .unwrap();

        let statuses = runtimes(&app).await;
        assert!(!statuses.is_empty(), "expected at least one runtime");
        for s in &statuses {
            // Nothing is loaded in a fresh app -- the field must exist and
            // default to empty, not be silently missing from the DTO.
            assert!(
                s.loaded_models.is_empty(),
                "{} should report no loaded models yet",
                s.id
            );
        }

        // Round-trips through JSON with the field present (not skipped),
        // which is what the UI's "resident models" panel depends on.
        let json = serde_json::to_value(&statuses[0]).unwrap();
        assert!(json.get("loaded_models").is_some());
    }

    #[test]
    fn submit_job_dto_carries_the_session_id_through() {
        let body = SubmitJobDto {
            job_type: "chat".into(),
            capability: None,
            runtime_id: None,
            model_id: None,
            vram_needed_mb: 0,
            agent_session: false,
            session_id: Some("sess-123".into()),
            params: serde_json::Value::Null,
        };
        let new = new_job_from(body);
        assert_eq!(new.session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn submit_job_dto_defaults_to_no_session() {
        let body = SubmitJobDto {
            job_type: "chat".into(),
            capability: None,
            runtime_id: None,
            model_id: None,
            vram_needed_mb: 0,
            agent_session: false,
            session_id: None,
            params: serde_json::Value::Null,
        };
        let new = new_job_from(body);
        assert_eq!(new.session_id, None);
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
            name: None,
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
            nsfw: false,
            preview_image_url: None,
            allow_commercial_use: vec![],
            model_kind_hint: None,
            base_model_family: None,
        }
    }
}
