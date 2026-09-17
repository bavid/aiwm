//! Thin Tauri host. All logic lives in `aiwm-core`; this crate wires the window,
//! plugins, and IPC commands that forward to `aiwm_core::api::handlers` — the
//! same functions the loopback HTTP server uses, so both transports return
//! identical JSON.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use aiwm_core::api::dto::{
    AboutDto, ActivePersonaDto, AgentPermissionDto, AgentSessionDetailDto, AssignedDto,
    AttachExternalDto, BenchmarkOptionsDto, CharacterBodyDto, CivitaiSearchDto, ColibriModelDto,
    ConceptBodyDto, ConceptFramesDto, ConceptSummaryDto, ConfigUpdate, EnqueueDownloadDto,
    ExportDatasetDto, FeaturedModelDto, JobDetailDto, KnownModelDto, LaunchExternalDto,
    LocalApiStatusDto, LocationBodyDto, ModelStackDto, NewAgentDto, NewSessionDto,
    NewVoiceIdentityDto, NpcBodyDto, OpenAgentSessionDto, PersonaBodyDto, ProfileDto,
    RegisterColibriModelDto, RegistryDetailsDto, RegistrySearchDto, RunDetailDto, RuntimeStatusDto,
    SceneBodyDto, SceneDetailDto, SetSessionPersonaDto, StartRunDto, StoryBodyDto, SubmitJobDto,
    TrainerStatusDto, UpdateDatasetDto, UpdateDatasetFrameDto,
};
use aiwm_core::api::handlers;
use aiwm_core::capability::dataset::{CaptionerStatus, ExportSummary};
use aiwm_core::config::Config;
use aiwm_core::db::Document;
use aiwm_core::db::{
    Agent, AgentSession, Benchmark, Character, CharacterLogEntry, CharacterRelationship, Dataset,
    DatasetConcept, DatasetFrame, Download, Job, JobFilter, Location, Model, Npc, Persona,
    SceneImage, Session, Story, TrainingRun, VoiceIdentity,
};
use aiwm_core::model::{ImportOutcome, ImportRequest};
use aiwm_core::orchestrator::JobState;
use aiwm_core::persona::{EffectivePersona, SetSessionPersona};
use aiwm_core::registry::{Fetched, RemoteModel};
use aiwm_core::runtime::training::Probe;
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
    to_ipc(handlers::save_config(&app, update).await)
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
async fn check_tool_versions(
    app: tauri::State<'_, Arc<App>>,
) -> Result<Vec<aiwm_core::runtime::ToolVersionCheck>, String> {
    to_ipc(handlers::check_tool_versions(&app).await)
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
async fn delete_download(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_download(&app, &id).await)
}

#[tauri::command]
async fn clear_finished_downloads(app: tauri::State<'_, Arc<App>>) -> Result<u64, String> {
    to_ipc(handlers::clear_finished_downloads(&app).await)
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
async fn delete_job(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_job(&app, &id).await)
}

/// Copy a finished job's output file to `dest_path` — the write-side
/// counterpart of the existing "Browse…" `@tauri-apps/plugin-dialog` load
/// flow. `<a download>` is inert inside Tauri's webview sandbox, so the
/// gallery's Download button opens a native save dialog on the JS side, then
/// calls this to actually place the bytes there.
#[tauri::command]
async fn save_job_output(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    dest_path: String,
) -> Result<(), String> {
    let source = handlers::job_output_path(&app, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no output for this job".to_string())?;
    std::fs::copy(&source, &dest_path).map_err(|e| format!("saving to {dest_path}: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn clean_audio(app: tauri::State<'_, Arc<App>>, id: String) -> Result<f64, String> {
    to_ipc(handlers::clean_audio(&app, &id).await)
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
async fn list_dataset_frames(
    app: tauri::State<'_, Arc<App>>,
    job_id: String,
) -> Result<Vec<DatasetFrame>, String> {
    to_ipc(handlers::list_dataset_frames(&app, &job_id).await)
}

#[tauri::command]
async fn update_dataset_frame(
    app: tauri::State<'_, Arc<App>>,
    frame_id: String,
    body: UpdateDatasetFrameDto,
) -> Result<DatasetFrame, String> {
    to_ipc(handlers::update_dataset_frame(&app, &frame_id, body).await)
}

#[tauri::command]
async fn export_dataset(
    app: tauri::State<'_, Arc<App>>,
    job_id: String,
    dest_dir: String,
) -> Result<ExportSummary, String> {
    to_ipc(handlers::export_dataset(&app, &job_id, &dest_dir).await)
}

#[tauri::command]
async fn list_captioners(app: tauri::State<'_, Arc<App>>) -> Result<Vec<CaptionerStatus>, String> {
    to_ipc(handlers::list_captioners(&app).await)
}

#[tauri::command]
async fn list_datasets(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Dataset>, String> {
    to_ipc(handlers::list_datasets(&app).await)
}

#[tauri::command]
async fn get_dataset(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Option<Dataset>, String> {
    to_ipc(handlers::get_dataset(&app, &id).await)
}

#[tauri::command]
async fn update_dataset(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: UpdateDatasetDto,
) -> Result<Dataset, String> {
    to_ipc(handlers::update_dataset(&app, &id, body).await)
}

#[tauri::command]
async fn delete_dataset(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_dataset(&app, &id).await)
}

#[tauri::command]
async fn list_dataset_frames_for_dataset(
    app: tauri::State<'_, Arc<App>>,
    dataset_id: String,
) -> Result<Vec<DatasetFrame>, String> {
    to_ipc(handlers::list_dataset_frames_for_dataset(&app, &dataset_id).await)
}

#[tauri::command]
async fn frame_concept_map(
    app: tauri::State<'_, Arc<App>>,
    dataset_id: String,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    // A `HashMap` would serialize identically; `BTreeMap` keeps the dev-tools
    // view of the payload stable, same as `storage_report`'s breakdown.
    to_ipc(handlers::frame_concept_map(&app, &dataset_id).await).map(|m| m.into_iter().collect())
}

#[tauri::command]
async fn list_concepts(
    app: tauri::State<'_, Arc<App>>,
    dataset_id: String,
) -> Result<Vec<ConceptSummaryDto>, String> {
    to_ipc(handlers::list_concepts(&app, &dataset_id).await)
}

#[tauri::command]
async fn create_concept(
    app: tauri::State<'_, Arc<App>>,
    dataset_id: String,
    body: ConceptBodyDto,
) -> Result<DatasetConcept, String> {
    to_ipc(handlers::create_concept(&app, &dataset_id, body).await)
}

#[tauri::command]
async fn update_concept(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: ConceptBodyDto,
) -> Result<(), String> {
    to_ipc(handlers::update_concept(&app, &id, body).await)
}

#[tauri::command]
async fn delete_concept(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_concept(&app, &id).await)
}

#[tauri::command]
async fn assign_concept(
    app: tauri::State<'_, Arc<App>>,
    concept_id: String,
    body: ConceptFramesDto,
) -> Result<AssignedDto, String> {
    to_ipc(handlers::assign_concept(&app, &concept_id, body).await)
}

#[tauri::command]
async fn unassign_concept(
    app: tauri::State<'_, Arc<App>>,
    concept_id: String,
    body: ConceptFramesDto,
) -> Result<(), String> {
    to_ipc(handlers::unassign_concept(&app, &concept_id, body).await)
}

#[tauri::command]
async fn export_dataset_by_id(
    app: tauri::State<'_, Arc<App>>,
    dataset_id: String,
    body: ExportDatasetDto,
) -> Result<ExportSummary, String> {
    to_ipc(handlers::export_dataset_by_id(&app, &dataset_id, body).await)
}

#[tauri::command]
async fn delete_document(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_document(&app, &id).await)
}

// --- training orchestrator ---

#[tauri::command]
fn training_status(app: tauri::State<'_, Arc<App>>) -> TrainerStatusDto {
    handlers::trainer_status(&app)
}

#[tauri::command]
async fn install_trainer(app: tauri::State<'_, Arc<App>>) -> Result<String, String> {
    to_ipc(handlers::install_trainer(&app).map(str::to_string))
}

#[tauri::command]
async fn probe_trainer(app: tauri::State<'_, Arc<App>>) -> Result<Probe, String> {
    to_ipc(handlers::probe_trainer(&app).await)
}

#[tauri::command]
async fn list_training_profiles(
    app: tauri::State<'_, Arc<App>>,
) -> Result<Vec<ProfileDto>, String> {
    to_ipc(handlers::list_training_profiles(&app).await)
}

#[tauri::command]
async fn list_training_runs(app: tauri::State<'_, Arc<App>>) -> Result<Vec<TrainingRun>, String> {
    to_ipc(handlers::list_training_runs(&app).await)
}

#[tauri::command]
async fn start_training_run(
    app: tauri::State<'_, Arc<App>>,
    body: StartRunDto,
) -> Result<TrainingRun, String> {
    to_ipc(handlers::start_training_run(&app, body).await)
}

#[tauri::command]
async fn get_training_run(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Option<RunDetailDto>, String> {
    to_ipc(handlers::get_training_run(&app, &id).await)
}

#[tauri::command]
async fn pause_training_run(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<TrainingRun, String> {
    to_ipc(handlers::pause_training_run(&app, &id).await)
}

#[tauri::command]
async fn resume_training_run(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<TrainingRun, String> {
    to_ipc(handlers::resume_training_run(&app, &id).await)
}

#[tauri::command]
async fn cancel_training_run(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<TrainingRun, String> {
    to_ipc(handlers::cancel_training_run(&app, &id).await)
}

#[tauri::command]
async fn delete_training_run(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    purge: bool,
) -> Result<(), String> {
    to_ipc(handlers::delete_training_run(&app, &id, purge).await)
}

// --- Story Studio (Phase 1: text + plain image, docs/TODO.md) ---

#[tauri::command]
async fn list_stories(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Story>, String> {
    to_ipc(handlers::list_stories(&app).await)
}

#[tauri::command]
async fn create_story(
    app: tauri::State<'_, Arc<App>>,
    body: StoryBodyDto,
) -> Result<Story, String> {
    to_ipc(handlers::create_story(&app, body).await)
}

#[tauri::command]
async fn update_story(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: StoryBodyDto,
) -> Result<(), String> {
    to_ipc(handlers::update_story(&app, &id, body).await)
}

#[tauri::command]
async fn delete_story(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_story(&app, &id).await)
}

#[tauri::command]
async fn list_characters(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
) -> Result<Vec<Character>, String> {
    to_ipc(handlers::list_characters(&app, &story_id).await)
}

#[tauri::command]
async fn create_character(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
    body: CharacterBodyDto,
) -> Result<Character, String> {
    to_ipc(handlers::create_character(&app, &story_id, body).await)
}

#[tauri::command]
async fn update_character(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: CharacterBodyDto,
) -> Result<(), String> {
    to_ipc(handlers::update_character(&app, &id, body).await)
}

#[tauri::command]
async fn delete_character(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_character(&app, &id).await)
}

#[tauri::command]
async fn set_character_portrait(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    job_id: Option<String>,
) -> Result<(), String> {
    to_ipc(handlers::set_character_portrait(&app, &id, job_id.as_deref()).await)
}

#[tauri::command]
async fn set_character_inventory(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    items: Vec<String>,
) -> Result<(), String> {
    to_ipc(handlers::set_character_inventory(&app, &id, &items).await)
}

#[tauri::command]
async fn list_character_relationships(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Vec<CharacterRelationship>, String> {
    to_ipc(handlers::list_character_relationships(&app, &id).await)
}

#[tauri::command]
async fn add_character_relationship(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    related_character_id: String,
    note: String,
) -> Result<CharacterRelationship, String> {
    to_ipc(handlers::add_character_relationship(&app, &id, &related_character_id, &note).await)
}

#[tauri::command]
async fn remove_character_relationship(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<(), String> {
    to_ipc(handlers::remove_character_relationship(&app, &id).await)
}

#[tauri::command]
async fn character_log(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<Vec<CharacterLogEntry>, String> {
    to_ipc(handlers::character_log(&app, &id).await)
}

#[tauri::command]
async fn list_npcs(app: tauri::State<'_, Arc<App>>, story_id: String) -> Result<Vec<Npc>, String> {
    to_ipc(handlers::list_npcs(&app, &story_id).await)
}

#[tauri::command]
async fn create_npc(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
    body: NpcBodyDto,
) -> Result<Npc, String> {
    to_ipc(handlers::create_npc(&app, &story_id, body).await)
}

#[tauri::command]
async fn update_npc(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: NpcBodyDto,
) -> Result<(), String> {
    to_ipc(handlers::update_npc(&app, &id, body).await)
}

#[tauri::command]
async fn delete_npc(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_npc(&app, &id).await)
}

#[tauri::command]
async fn list_locations(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
) -> Result<Vec<Location>, String> {
    to_ipc(handlers::list_locations(&app, &story_id).await)
}

#[tauri::command]
async fn create_location(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
    body: LocationBodyDto,
) -> Result<Location, String> {
    to_ipc(handlers::create_location(&app, &story_id, body).await)
}

#[tauri::command]
async fn update_location(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: LocationBodyDto,
) -> Result<(), String> {
    to_ipc(handlers::update_location(&app, &id, body).await)
}

#[tauri::command]
async fn delete_location(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_location(&app, &id).await)
}

#[tauri::command]
async fn set_location_reference(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    job_id: Option<String>,
) -> Result<(), String> {
    to_ipc(handlers::set_location_reference(&app, &id, job_id.as_deref()).await)
}

#[tauri::command]
async fn list_scenes(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
) -> Result<Vec<SceneDetailDto>, String> {
    to_ipc(handlers::list_scenes(&app, &story_id).await)
}

#[tauri::command]
async fn create_scene(
    app: tauri::State<'_, Arc<App>>,
    story_id: String,
    body: SceneBodyDto,
) -> Result<SceneDetailDto, String> {
    to_ipc(handlers::create_scene(&app, &story_id, body).await)
}

#[tauri::command]
async fn update_scene(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: SceneBodyDto,
) -> Result<SceneDetailDto, String> {
    to_ipc(handlers::update_scene(&app, &id, body).await)
}

#[tauri::command]
async fn delete_scene(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_scene(&app, &id).await)
}

#[tauri::command]
async fn add_scene_image(
    app: tauri::State<'_, Arc<App>>,
    scene_id: String,
    job_id: String,
) -> Result<SceneImage, String> {
    to_ipc(handlers::add_scene_image(&app, &scene_id, &job_id).await)
}

#[tauri::command]
async fn set_canonical_scene_image(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<(), String> {
    to_ipc(handlers::set_canonical_scene_image(&app, &id).await)
}

#[tauri::command]
async fn delete_scene_image(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_scene_image(&app, &id).await)
}

// --- personas (spec `2026-09-18-personas-design`) ---------------------------
//
// `Option`/`bool` returns mirror the HTTP 404s: `None`/`false` means "no such
// id", and the caller decides what to say about it.

#[tauri::command]
async fn list_personas(app: tauri::State<'_, Arc<App>>) -> Result<Vec<Persona>, String> {
    to_ipc(handlers::list_personas(&app).await)
}

#[tauri::command]
async fn create_persona(
    app: tauri::State<'_, Arc<App>>,
    body: PersonaBodyDto,
) -> Result<Persona, String> {
    to_ipc(handlers::create_persona(&app, body).await)
}

#[tauri::command]
async fn update_persona(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: PersonaBodyDto,
) -> Result<Option<Persona>, String> {
    to_ipc(handlers::update_persona(&app, &id, body).await)
}

#[tauri::command]
async fn delete_persona(app: tauri::State<'_, Arc<App>>, id: String) -> Result<bool, String> {
    to_ipc(handlers::delete_persona(&app, &id).await)
}

#[tauri::command]
async fn active_persona(app: tauri::State<'_, Arc<App>>) -> Result<ActivePersonaDto, String> {
    to_ipc(handlers::active_persona(&app).await)
}

#[tauri::command]
async fn set_active_persona(
    app: tauri::State<'_, Arc<App>>,
    id: Option<String>,
) -> Result<bool, String> {
    to_ipc(handlers::set_active_persona(&app, id.as_deref()).await)
}

#[tauri::command]
async fn set_session_persona(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: SetSessionPersonaDto,
) -> Result<SetSessionPersona, String> {
    to_ipc(handlers::set_session_persona(&app, &id, body).await)
}

#[tauri::command]
async fn effective_persona(
    app: tauri::State<'_, Arc<App>>,
    session_id: Option<String>,
) -> Result<EffectivePersona, String> {
    to_ipc(handlers::effective_persona(&app, session_id.as_deref()).await)
}

#[tauri::command]
async fn list_voice_identities(
    app: tauri::State<'_, Arc<App>>,
) -> Result<Vec<VoiceIdentity>, String> {
    to_ipc(handlers::list_voice_identities(&app).await)
}

#[tauri::command]
async fn create_voice_identity(
    app: tauri::State<'_, Arc<App>>,
    body: NewVoiceIdentityDto,
) -> Result<VoiceIdentity, String> {
    to_ipc(handlers::create_voice_identity(&app, body).await)
}

#[tauri::command]
async fn delete_voice_identity(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::delete_voice_identity(&app, &id).await)
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

/// The built-in benchmark suites — static data, so no `App` state is needed.
/// Cloned into an owned `Vec` because a Tauri command's return type has to be
/// `'static`-owned for the IPC serializer.
#[tauri::command]
fn bench_suites() -> Vec<aiwm_core::bench::suites::Suite> {
    handlers::bench_suites().to_vec()
}

#[tauri::command]
async fn benchmark_history(
    app: tauri::State<'_, Arc<App>>,
    suite: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Benchmark>, String> {
    to_ipc(handlers::benchmark_history(&app, suite.as_deref(), limit).await)
}

/// `body` is optional — the Model Library's plain "Test model" button omits it
/// and still gets the single-prompt quick test.
#[tauri::command]
async fn benchmark_model(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    body: Option<BenchmarkOptionsDto>,
) -> Result<Job, String> {
    to_ipc(handlers::benchmark_model(&app, &id, body.unwrap_or_default()).await)
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

/// `POST /outputs/cleanup`'s Tauri counterpart — apply the configured output
/// retention policy right now (the Settings "Clean up now" button).
#[tauri::command]
async fn cleanup_outputs(
    app: tauri::State<'_, Arc<App>>,
) -> Result<aiwm_core::cleanup::SweepResult, String> {
    to_ipc(handlers::cleanup_outputs(&app).await)
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
async fn rename_model(
    app: tauri::State<'_, Arc<App>>,
    id: String,
    name: String,
) -> Result<Model, String> {
    to_ipc(handlers::rename_model(&app, &id, &name).await)
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
fn civitai_status(app: tauri::State<'_, Arc<App>>) -> aiwm_core::RegistryStatus {
    handlers::civitai_status(&app)
}

#[tauri::command]
fn set_civitai_token(app: tauri::State<'_, Arc<App>>, token: String) -> Result<(), String> {
    to_ipc(handlers::set_civitai_token(&app, &token))
}

#[tauri::command]
async fn civitai_search(
    app: tauri::State<'_, Arc<App>>,
    params: CivitaiSearchDto,
) -> Result<Fetched<Vec<RemoteModel>>, String> {
    to_ipc(handlers::civitai_search(&app, params).await)
}

#[tauri::command]
async fn civitai_model(
    app: tauri::State<'_, Arc<App>>,
    id: String,
) -> Result<RegistryDetailsDto, String> {
    to_ipc(handlers::civitai_details(&app, &id).await)
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
async fn unload_model(app: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    to_ipc(handlers::unload_model(&app, &id).await)
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
            check_tool_versions,
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
            bench_suites,
            benchmark_history,
            upgrade_check,
            storage_report,
            cleanup_outputs,
            delete_model,
            unload_model,
            model_tags,
            set_model_tags,
            set_model_roles,
            rename_model,
            registry_status,
            set_hf_token,
            civitai_status,
            set_civitai_token,
            civitai_search,
            civitai_model,
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
            delete_download,
            clear_finished_downloads,
            export_backup,
            import_backup,
            cancel_job,
            delete_job,
            save_job_output,
            clean_audio,
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
            list_personas,
            create_persona,
            update_persona,
            delete_persona,
            active_persona,
            set_active_persona,
            set_session_persona,
            effective_persona,
            list_voice_identities,
            create_voice_identity,
            delete_voice_identity,
            list_stories,
            create_story,
            update_story,
            delete_story,
            list_characters,
            create_character,
            update_character,
            delete_character,
            set_character_portrait,
            set_character_inventory,
            list_character_relationships,
            add_character_relationship,
            remove_character_relationship,
            character_log,
            list_npcs,
            create_npc,
            update_npc,
            delete_npc,
            list_locations,
            create_location,
            update_location,
            delete_location,
            set_location_reference,
            list_scenes,
            create_scene,
            update_scene,
            delete_scene,
            add_scene_image,
            set_canonical_scene_image,
            delete_scene_image,
            list_dataset_frames,
            update_dataset_frame,
            export_dataset,
            list_captioners,
            list_datasets,
            get_dataset,
            update_dataset,
            delete_dataset,
            list_dataset_frames_for_dataset,
            frame_concept_map,
            list_concepts,
            create_concept,
            update_concept,
            delete_concept,
            assign_concept,
            unassign_concept,
            export_dataset_by_id,
            training_status,
            install_trainer,
            probe_trainer,
            list_training_profiles,
            list_training_runs,
            start_training_run,
            get_training_run,
            pause_training_run,
            resume_training_run,
            cancel_training_run,
            delete_training_run,
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
