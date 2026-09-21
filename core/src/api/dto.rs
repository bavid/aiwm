//! Response and request shapes for the API. Both transports (Tauri IPC and the
//! loopback HTTP server) serialize these identical types.

use serde::{Deserialize, Serialize};

use crate::agent::{HermesInstallStatus, PermissionDecision};
use crate::config::{CivitaiConfig, ComfyConfig, LlamaConfig, ModelsConfig, RetentionConfig};
use crate::db::{AgentSession, AgentSessionEvent, Job, JobEvent};
use crate::registry::{Freshness, RemoteModel, SearchQuery, SearchSort};
use crate::runtime::{Health, RuntimeKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AboutDto {
    pub core_version: String,
    pub data_dir: String,
    pub store_path: String,
    /// Where generated images + video land (`GET /jobs/{id}/output` serves from
    /// here).
    pub outputs_dir: String,
    /// Total bytes of the files in `outputs_dir` — the Settings retention card
    /// shows it (video clips are large).
    pub outputs_bytes: u64,
    /// Where managed runtime installs (llama.cpp, ComfyUI) land.
    pub runtimes_dir: String,
    /// Where the disposable registry cache lands.
    pub cache_dir: String,
    /// Where dataset-prep work folders land.
    pub datasets_dir: String,
    /// Where training-run work folders land.
    pub training_dir: String,
    pub core_api_port: u16,
    pub vram_budget_mb: u64,
    pub offline_mode: bool,
}

/// The user-editable slice of `config.toml` — `GET`/`PUT /config`, the Settings
/// UI. `core_api_port` and `log_filter` stay file-only (power-user territory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigUpdate {
    pub store_path: String,
    pub offline_mode: bool,
    /// `0` = auto-detect from the GPU.
    pub vram_budget_mb: u64,
    pub llama: LlamaConfig,
    /// New in 3.7; older clients that omit it keep ComfyUI on `auto`.
    #[serde(default)]
    pub comfyui: ComfyConfig,
    /// New in 6.6; older clients that omit it keep `Auto` on `balanced`.
    #[serde(default)]
    pub models: ModelsConfig,
    /// Per-folder location overrides (outputs/runtimes/cache). New in this
    /// slice; older clients that omit it leave existing overrides untouched.
    #[serde(default)]
    pub paths: PathsUpdateDto,
    /// Output-retention policy. New in this slice; older clients that omit it
    /// keep retention disabled (both rules `0`).
    #[serde(default)]
    pub retention: RetentionConfig,
    /// Which Civitai front door Discover uses. New in this slice; older
    /// clients that omit it keep the safe-for-work default.
    #[serde(default)]
    pub civitai: CivitaiConfig,
}

/// The `paths` slice of [`ConfigUpdate`] — plain strings from the Settings
/// form. An empty string means "use the portable default", not a literal path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathsUpdateDto {
    pub outputs_path: String,
    pub runtimes_path: String,
    pub cache_path: String,
    /// Required like the other paths: a `paths` object without it is rejected
    /// rather than read as "clear the override". The UI always sends all five.
    pub datasets_path: String,
    /// Required for the same reason as `datasets_path`.
    pub training_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatusDto {
    pub id: String,
    pub kind: RuntimeKind,
    pub health: Health,
    pub vram_used_mb: u64,
    /// One short status line for the UI (e.g. `"not installed"`,
    /// `"serving qwen on :48213"`). `null` when there is nothing to add.
    pub detail: Option<String>,
    /// Models currently resident on this runtime, structured (not parsed out
    /// of `detail`) -- backs the "what's resident right now" dashboard panel.
    pub loaded_models: Vec<crate::runtime::LoadedModel>,
}

/// A job plus its event trail — `GET /jobs/{id}` / `job_detail`.
#[derive(Debug, Clone, Serialize)]
pub struct JobDetailDto {
    pub job: Job,
    pub events: Vec<JobEvent>,
}

/// Body for `POST /jobs` / `submit_job`.
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitJobDto {
    pub job_type: String,
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub vram_needed_mb: u64,
    #[serde(default)]
    pub agent_session: bool,
    /// Group this job under a session (Chat/Image/Video "project"). `None` =
    /// ungrouped.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Body for `POST /sessions` / `create_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewSessionDto {
    /// `"chat"` | `"image"` | `"video"`.
    pub capability: String,
    pub name: String,
}

/// Body for `PUT /sessions/{id}` / `rename_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct RenameSessionDto {
    pub name: String,
}

/// Body for `PUT /sessions/{id}/archived` / `set_session_archived`.
#[derive(Debug, Clone, Deserialize)]
pub struct SetArchivedDto {
    pub archived: bool,
}

// --- personas (spec `2026-09-18-personas-design`) ---------------------------

/// Body for `POST /personas` and `PUT /personas/{id}` — the whole persona, since
/// the edit dialog always has all three fields in hand. The limits live in
/// [`crate::persona::validate`]; the prompt text itself is passed through
/// verbatim.
#[derive(Debug, Clone, Deserialize)]
pub struct PersonaBodyDto {
    pub name: String,
    /// One emoji.
    pub icon: String,
    pub system_prompt: String,
}

/// `GET`/`PUT /personas/active` — the globally active persona's id, or `null`
/// for "no global persona". The same shape in both directions so the UI has one
/// type for it.
///
/// Clearing the global persona has to be spelled `{"id": null}`: a body that
/// simply forgot the key is a malformed request (422) rather than a silent wipe
/// of the user's choice. See [`required_option_string`] for why that needs more
/// than dropping `#[serde(default)]`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ActivePersonaDto {
    #[serde(deserialize_with = "required_option_string")]
    pub id: Option<String>,
}

/// Deserialize an `Option<String>` that is still **required** to be present.
///
/// Serde treats a missing `Option<T>` field as `None` all by itself — dropping
/// `#[serde(default)]` is not enough. Routing the field through an explicit
/// `deserialize_with` removes that special case (serde then reports a missing
/// field), while `null` still deserializes to `None` as usual. That is the whole
/// difference between "clear the active persona" and "I forgot to send the key".
fn required_option_string<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

/// Body for `PUT /sessions/{id}/persona` — this chat's override. `persona_id` is
/// required for (and only read with) `mode: "persona"`.
#[derive(Debug, Clone, Deserialize)]
pub struct SetSessionPersonaDto {
    /// `"inherit"` | `"none"` | `"persona"`.
    pub mode: crate::db::PersonaMode,
    #[serde(default)]
    pub persona_id: Option<String>,
}

/// Body for `PUT /jobs/{id}/dataset-frames/{frame_id}` — a curator's edit to
/// one frame. Every field optional so the curation UI can send just the one
/// thing that changed (a caption edit vs. an exclude toggle) rather than the
/// whole row every time.
///
/// `restore: true` clears a rejection, putting an auto-dropped frame back into
/// the kept set. The clip bounds are three-valued: absent keeps the stored
/// bound, an explicit `null` clears it (back to the clip's natural start/end),
/// a number sets it — see [`deserialize_optional_nullable`].
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDatasetFrameDto {
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub excluded: Option<bool>,
    #[serde(default)]
    pub restore: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub clip_start_secs: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub clip_end_secs: Option<Option<f64>>,
}

/// Tells an absent JSON field apart from an explicit `null` one.
///
/// `#[serde(default)]` alone is not enough: serde hands a `null` to
/// `Option<Option<T>>`'s own impl, which answers the *outer* `None` — exactly
/// what an absent field produces, so "clear this bound" would be
/// indistinguishable from "leave it alone". serde only calls a
/// `deserialize_with` when the key is actually present, so wrapping the inner
/// `Option<T>` here keeps the two apart: absent -> `None` (from `default`),
/// `null` -> `Some(None)`, a value -> `Some(Some(v))`.
fn deserialize_optional_nullable<'de, D, T>(
    d: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Body for `POST /jobs/{id}/dataset-export` and `POST /datasets/{id}/export` —
/// write the curated dataset to `dest_dir` as `NNNN.<ext>` + `NNNN.txt` pairs.
///
/// `caption_order` is new in this slice; the job-keyed route predates it and
/// older clients that omit it keep the prose-first wording they had.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportDatasetDto {
    pub dest_dir: String,
    #[serde(default = "default_caption_order")]
    pub caption_order: crate::capability::dataset::CaptionOrder,
}

fn default_caption_order() -> crate::capability::dataset::CaptionOrder {
    crate::capability::dataset::CaptionOrder::ProseFirst
}

/// Body for `PUT /datasets/{id}` — today only the trigger word is editable.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDatasetDto {
    #[serde(default)]
    pub trigger_word: Option<String>,
}

/// Body for `POST /datasets/{id}/concepts` and `PUT /concepts/{id}` — one
/// concept the dataset teaches (spec 3A).
#[derive(Debug, Clone, Deserialize)]
pub struct ConceptBodyDto {
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub description: String,
}

/// Body for `POST`/`DELETE /concepts/{id}/frames` — the frames to attach to
/// or detach from one concept ("Alle im Set" sends the whole visible page).
#[derive(Debug, Clone, Deserialize)]
pub struct ConceptFramesDto {
    pub frame_ids: Vec<String>,
}

/// Response of `POST /concepts/{id}/frames`: how many of the requested frames
/// were newly attached (the rest were already assigned or belong to another
/// dataset).
#[derive(Debug, Clone, Serialize)]
pub struct AssignedDto {
    pub requested: usize,
    pub attached: u64,
}

/// `GET /datasets/{id}/concepts` — a concept plus its frame count and a
/// warning when its token reads as an ordinary word the base model already
/// knows.
#[derive(Debug, Clone, Serialize)]
pub struct ConceptSummaryDto {
    #[serde(flatten)]
    pub concept: crate::db::DatasetConcept,
    pub frame_count: usize,
    pub token_warning: Option<String>,
}

/// Body for `POST /datasets/{id}/frames/bulk` — move many frames to "Keep"
/// (`excluded: false`, which also clears a filter rejection) or "Discard"
/// (`excluded: true`) in one request.
#[derive(Debug, Clone, Deserialize)]
pub struct BulkFramesDto {
    pub frame_ids: Vec<String>,
    pub excluded: bool,
}

/// Response of `POST /datasets/{id}/frames/bulk`: unknown ids and ids of
/// another dataset are skipped, so `updated` can be lower than `requested`.
#[derive(Debug, Clone, Serialize)]
pub struct BulkUpdatedDto {
    pub requested: usize,
    pub updated: u64,
}

/// Body for `POST /datasets/{id}/frames/delete` — frames to delete together
/// with their files.
#[derive(Debug, Clone, Deserialize)]
pub struct DeleteFramesDto {
    pub frame_ids: Vec<String>,
}

/// Body for `POST /datasets/{id}/cleanup`. `dry_run` defaults to `true`: a
/// body that forgets the flag previews instead of deleting.
#[derive(Debug, Clone, Deserialize)]
pub struct CleanupDatasetDto {
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

/// Body for `POST /datasets/{id}/dedup`. `threshold` is a Hamming distance;
/// the core defaults it to the duplicate filter's and clamps it to `0..=16`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DedupDatasetDto {
    #[serde(default)]
    pub threshold: Option<u32>,
}

/// Body for `POST /sessions/{id}/documents` — attach a document to a chat
/// session for local RAG (7.x). `path` is resolved by the caller (a native
/// file picker in the UI); the core reads, chunks, and stores it.
#[derive(Debug, Clone, Deserialize)]
pub struct AttachDocumentDto {
    pub path: String,
}

/// Body for `POST /voice-identities` / `create_voice_identity` — save a Dia
/// voice-cloning identity (a reference clip + its transcript) under a name,
/// e.g. "Old Man Gareth". `source_audio_path` is resolved by the caller (a
/// native file picker in the UI); the core copies it into AIWM's own data
/// dir and never references the original location again.
#[derive(Debug, Clone, Deserialize)]
pub struct NewVoiceIdentityDto {
    pub name: String,
    pub source_audio_path: String,
    pub reference_transcript: String,
}

// --- training orchestrator (spec `2026-09-16-training-orchestrator-design`) -

/// `GET /training/status` — everything the Training tab's header needs to
/// decide between "set up the trainer", "wait", "repair" and "start a run".
/// `install_state` is the adapter's own serialised shape, identical to the
/// ComfyUI install card's, so both cards can share one renderer.
#[derive(Debug, Clone, Serialize)]
pub struct TrainerStatusDto {
    pub installed: bool,
    pub installing: bool,
    /// The last import probe found the venv unusable — offer a repair, not a
    /// doomed run.
    pub env_broken: bool,
    pub install_state: crate::runtime::training::InstallState,
    /// The adapter's own one-line status, ready to show verbatim.
    pub detail: String,
    /// The run currently holding the GPU, if any.
    pub alive_run_id: Option<String>,
}

/// One library model a profile can train, for the target-model dropdown.
#[derive(Debug, Clone, Serialize)]
pub struct TrainableModelDto {
    pub id: String,
    pub name: String,
    /// The library's own `family` string, not the profile's — the two differ
    /// for today's generic `flux2`/`wan` entries (see
    /// [`crate::training::profile::find_for_model`]).
    pub family: String,
}

/// A profile's three starting points, flattened out of the registry entry so
/// the UI can index them by the preset name it already has.
#[derive(Debug, Clone, Serialize)]
pub struct ProfilePresetsDto {
    pub fast: crate::training::profile::PresetValues,
    pub balanced: crate::training::profile::PresetValues,
    pub thorough: crate::training::profile::PresetValues,
}

/// `GET /training/profiles` — one trainable model family, with the two facts
/// the registry itself cannot know: whether its base weights are staged in
/// the library (`base_installed`) and which library models resolve to it.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileDto {
    pub family: &'static str,
    pub label: &'static str,
    pub arch: &'static str,
    pub data_kind: crate::training::profile::DataKind,
    pub fit: crate::training::profile::Fit,
    /// `fit` in plain English, so the UI never has to translate the enum.
    pub fit_label: &'static str,
    pub reserve_mb: u64,
    pub base_repo: &'static str,
    pub base_role: &'static str,
    pub base_required_files: &'static [&'static str],
    pub base_approx_gb: u32,
    /// The exact `hf` CLI line that stages this base under the configured
    /// model store, built by [`crate::training::bases::hf_download_command`]
    /// so the command the preflight shows and the manifest the runner
    /// verifies against can never disagree about the exclusions or the
    /// target directory.
    pub base_download_command: String,
    /// A library directory model with `base_role` whose folder holds every
    /// one of `base_required_files`.
    pub base_installed: bool,
    pub caption_order: crate::capability::dataset::CaptionOrder,
    pub license_note: &'static str,
    pub presets: ProfilePresetsDto,
    pub trainable_models: Vec<TrainableModelDto>,
}

/// Body for `POST /training/runs` — everything the Training tab's form
/// collects. `hyperparams` and `sample_prompts` default so a minimal client
/// can omit them and get the preset's own values (the handler still requires
/// at least one prompt).
#[derive(Debug, Clone, Deserialize)]
pub struct StartRunDto {
    pub name: String,
    pub target_model_id: String,
    pub dataset_id: String,
    pub trigger_word: String,
    pub preset: crate::db::Preset,
    #[serde(default)]
    pub hyperparams: crate::training::config::Hyperparams,
    #[serde(default)]
    pub sample_prompts: Vec<String>,
    /// "Store run in": the folder the run's own folder `<data_dir>/<run_id>`
    /// is created in. Omitted or blank means the default training folder —
    /// unlike the Settings paths, leaving it out is the normal case here.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// "Start from": the library LoRA to continue (Plan 11). Omitted or
    /// blank means a fresh LoRA — the normal case.
    #[serde(default)]
    pub init_lora_model_id: Option<String>,
}

impl StartRunDto {
    /// The chosen folder, `None` when omitted or blank.
    pub fn chosen_data_dir(&self) -> Option<std::path::PathBuf> {
        self.data_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
    }

    /// The LoRA to continue from, `None` when omitted or blank.
    pub fn chosen_init_lora(&self) -> Option<String> {
        self.init_lora_model_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    }
}

/// `GET /training/runs/{id}` — the stored row plus the two things that live
/// on disk rather than in the database.
#[derive(Debug, Clone, Serialize)]
pub struct RunDetailDto {
    pub run: crate::db::TrainingRun,
    /// The library name of `run.init_lora_model_id`, when the run continued
    /// a LoRA that is still in the library; `None` for a from-scratch run or
    /// a source LoRA that has since been deleted.
    pub init_lora_name: Option<String>,
    /// Opaque tokens for `GET /training/runs/{id}/samples/{n}`, newest
    /// checkpoint first. Deliberately *not* file paths: the work dir never
    /// leaves the core, and the route resolves the token by re-scanning.
    pub latest_samples: Vec<String>,
    /// The tail of `train.log`, already split on `\r` *and* `\n` (ai-toolkit
    /// redraws its progress bar in place) and trimmed of empty updates.
    pub log_tail: Vec<String>,
    pub work_dir: String,
}

/// `GET /training/loras` — one library LoRA with its lineage's totals. The
/// shape is [`crate::training::lineage::LoraSummary`] itself: it is already
/// what the Training tab shows, with nothing to hide or rename.
pub type LoraSummaryDto = crate::training::lineage::LoraSummary;

/// One run of a LoRA's history ([`crate::training::lineage::LineageRun`]);
/// `samples` are the same opaque tokens as [`RunDetailDto::latest_samples`].
pub type LineageRunDto = crate::training::lineage::LineageRun;

/// `GET /training/loras/{model_id}` — the LoRA and its runs, oldest first.
pub type LoraLineageDto = crate::training::lineage::LoraLineage;

// --- Story Studio (Phase 1: text + plain image, docs/TODO.md) -------------

/// Body for `POST /stories` and `PUT /stories/{id}` — every editable Story
/// field, always supplied together (no partial updates).
#[derive(Debug, Clone, Deserialize)]
pub struct StoryBodyDto {
    pub name: String,
    #[serde(default)]
    pub setting: String,
    #[serde(default)]
    pub art_style: String,
    #[serde(default)]
    pub premise: String,
}

/// Body for `POST /stories/{id}/characters` and `PUT /characters/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct CharacterBodyDto {
    pub name: String,
    #[serde(default)]
    pub traits: String,
    #[serde(default)]
    pub backstory: String,
    #[serde(default)]
    pub alignment: String,
}

/// Body for `PUT /characters/{id}/portrait` and `PUT /locations/{id}/reference`
/// — pick (`Some`) or clear (`None`) a reference image, an already-submitted
/// `job_type=image` job id.
#[derive(Debug, Clone, Deserialize)]
pub struct SetReferenceJobDto {
    pub job_id: Option<String>,
}

/// Body for `PUT /characters/{id}/inventory` — full replace.
#[derive(Debug, Clone, Deserialize)]
pub struct SetInventoryDto {
    pub items: Vec<String>,
}

/// Body for `POST /characters/{id}/relationships`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewRelationshipDto {
    pub related_character_id: String,
    pub note: String,
}

/// Body for `POST /stories/{id}/npcs` and `PUT /npcs/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct NpcBodyDto {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub location_id: Option<String>,
    #[serde(default)]
    pub description: String,
}

/// Body for `POST /stories/{id}/locations` and `PUT /locations/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct LocationBodyDto {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// One dialogue line inside a [`SceneBodyDto`].
#[derive(Debug, Clone, Deserialize)]
pub struct DialogueLineDto {
    pub character_id: String,
    pub text: String,
}

/// Body for `POST /stories/{id}/scenes` and `PUT /scenes/{id}` — full replace
/// of every editable field, participants and dialogue included (`position`
/// is never client-editable).
#[derive(Debug, Clone, Deserialize)]
pub struct SceneBodyDto {
    #[serde(default)]
    pub location_id: Option<String>,
    #[serde(default)]
    pub narrative: String,
    #[serde(default)]
    pub redline: String,
    #[serde(default)]
    pub participant_ids: Vec<String>,
    #[serde(default)]
    pub dialogue: Vec<DialogueLineDto>,
}

/// A Scene plus everything the Timeline needs to render one card, composed
/// from several repos (mirrors how `JobDetailDto` composes a job with its
/// events).
#[derive(Debug, Clone, Serialize)]
pub struct SceneDetailDto {
    #[serde(flatten)]
    pub scene: crate::db::Scene,
    pub participant_ids: Vec<String>,
    pub dialogue: Vec<crate::db::DialogueLine>,
    pub images: Vec<crate::db::SceneImage>,
}

/// Body for `POST /scenes/{id}/images` — attach an already-submitted
/// `job_type=image` job as a new alternate (or, if it's the first, the
/// canonical) image for the scene.
#[derive(Debug, Clone, Deserialize)]
pub struct AddSceneImageDto {
    pub job_id: String,
}

/// Body for `POST /agents` / `create_agent` — a new agent profile.
#[derive(Debug, Clone, Deserialize)]
pub struct NewAgentDto {
    pub name: String,
    /// `"opencode"` (Hermes lands in 5.4).
    pub adapter: String,
    /// Explicit coding model, or omitted for `Auto` over the `coding` role.
    #[serde(default)]
    pub model_id: Option<String>,
    pub workspace_path: String,
    /// Extra read roots beyond the workspace.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// `null` = the adapter's default toolset.
    #[serde(default)]
    pub toolset: Option<Vec<String>>,
}

/// Body for `POST /agent-sessions` / `open_agent_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAgentSessionDto {
    pub agent_id: String,
    /// Sent as the opening turn once the session is live.
    #[serde(default)]
    pub first_message: Option<String>,
}

/// Body for `POST /agent-sessions/{id}/message`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentMessageDto {
    pub text: String,
}

/// Body for `POST /agent-sessions/{id}/permission`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentPermissionDto {
    pub request_id: String,
    /// `"allow_once"` | `"allow_always"` | `"deny"`.
    pub decision: PermissionDecision,
}

/// A session plus its transcript — `GET /agent-sessions/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSessionDetailDto {
    pub session: AgentSession,
    pub events: Vec<AgentSessionEvent>,
    /// Whether this process is currently driving the session.
    pub live: bool,
}

/// One agent runtime's availability — `GET /agent-runtimes`. Only Hermes has an
/// AIWM-driven installer; OpenCode's `install` is `null` (bring your own).
#[derive(Debug, Clone, Serialize)]
pub struct AgentRuntimeDto {
    /// `"opencode"` | `"hermes"`.
    pub id: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<HermesInstallStatus>,
}

/// Body for `POST /launcher` / `launcher_launch` — open an external terminal.
#[derive(Debug, Clone, Deserialize)]
pub struct LaunchExternalDto {
    pub tool: crate::LaunchTool,
    /// Explicit coding model, or omitted for `Auto` over the `coding` role.
    #[serde(default)]
    pub model_id: Option<String>,
    pub workspace: String,
}

// --- model discovery (Phase 6.2) -----------------------------------------

/// Query for `GET /registry/search` — the "Discover" panel on the Models tab.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistrySearchDto {
    /// Free-text search on the repo id.
    #[serde(default)]
    pub q: Option<String>,
    /// `filter=base_model:<id>` — every quant / derivative of one base.
    #[serde(default)]
    pub base_model: Option<String>,
    /// Restrict to `gguf`-tagged repos.
    #[serde(default)]
    pub gguf: bool,
    /// `downloads` | `likes` | `trending` | `updated` | `new`.
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl RegistrySearchDto {
    pub fn into_query(self) -> SearchQuery {
        let sort = match self.sort.as_deref() {
            Some("likes") => SearchSort::Likes,
            Some("trending") => SearchSort::Trending,
            Some("updated") => SearchSort::RecentlyUpdated,
            Some("new") => SearchSort::RecentlyCreated,
            _ => SearchSort::Downloads,
        };
        SearchQuery {
            text: self.q.filter(|s| !s.trim().is_empty()),
            base_model: self.base_model.filter(|s| !s.trim().is_empty()),
            gguf_only: self.gguf,
            sort,
            limit: self.limit.unwrap_or(25),
            ..SearchQuery::default()
        }
    }
}

/// `GET /civitai/search` — the "Discover" panel's Civitai source. Distinct
/// from [`RegistrySearchDto`] because Civitai's real filters genuinely
/// differ (a `types` enum, an NSFW toggle) rather than mapping onto Hugging
/// Face's `base_model` / `gguf_only`.
#[derive(Debug, Clone, Deserialize)]
pub struct CivitaiSearchDto {
    /// Free-text search on the model name.
    #[serde(default)]
    pub q: Option<String>,
    /// Comma-separated Civitai `types` values, e.g. `"Checkpoint,LORA"`. A
    /// plain string (not a `Vec`) so this DTO deserializes identically from
    /// a query string over HTTP and from a JSON body over Tauri IPC — an
    /// array field would need repeated `types=` keys the HTTP `Query`
    /// extractor doesn't reliably support. Empty/absent = every type.
    #[serde(default)]
    pub types: Option<String>,
    /// `downloads` | `likes` | `trending` | `new`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Include NSFW-flagged results. **Defaults to `false`** (excluded) —
    /// the caller must explicitly opt in; see `registry::civitai`'s module
    /// doc for why this is never silently widened.
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl CivitaiSearchDto {
    pub fn into_query(self) -> SearchQuery {
        let sort = match self.sort.as_deref() {
            Some("likes") => SearchSort::Likes,
            Some("trending") => SearchSort::Trending,
            Some("new") => SearchSort::RecentlyCreated,
            _ => SearchSort::Downloads,
        };
        let media_types = self
            .types
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        SearchQuery {
            text: self.q.filter(|s| !s.trim().is_empty()),
            sort,
            limit: self.limit.unwrap_or(25),
            nsfw: self.nsfw,
            media_types,
            ..SearchQuery::default()
        }
    }
}

/// One downloadable file, enriched with the browser link + a fit verdict
/// ([`crate::compat::verdict`] — weights + KV + overhead vs the VRAM budget and
/// free system RAM, with a plain-language reason).
#[derive(Debug, Clone, Serialize)]
pub struct RegistryFileDto {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub quant: Option<String>,
    /// `[index, total]` for a split file `…-00001-of-00003.gguf`.
    pub shard: Option<[u32; 2]>,
    /// Hugging Face: `https://huggingface.co/<id>/resolve/<rev>/<path>`.
    /// Civitai: the file's own `downloadUrl`, verbatim. Either way — "Copy
    /// link" / what `enqueue_download` fetches.
    pub download_url: String,
    /// `null` for non-model files (README, `config.json`).
    pub vram_estimate_mb: Option<u64>,
    pub fit: crate::compat::FitVerdict,
    /// Civitai's own malware-scan verdicts (`"Success"`, `"Danger"`,
    /// `"Pending"`, …), surfaced verbatim so the UI can flag anything that
    /// isn't a clean `"Success"` — never hidden. `null` for a source with no
    /// such concept (Hugging Face). This is informational only: it is never
    /// a substitute for AIWM's own import-time Pickle-format guard, which
    /// runs unconditionally regardless of what a source self-reports.
    #[serde(default)]
    pub pickle_scan_result: Option<String>,
    #[serde(default)]
    pub virus_scan_result: Option<String>,
}

/// `GET /registry/models/{id}` — the model plus every file with size, hash and
/// fit.
#[derive(Debug, Clone, Serialize)]
pub struct RegistryDetailsDto {
    #[serde(flatten)]
    pub model: RemoteModel,
    pub revision: String,
    pub files: Vec<RegistryFileDto>,
    pub freshness: Freshness,
}

/// `GET /models/known` — the curated image/video catalogue
/// ([`crate::model::KnownModel`]), each entry enriched with a fit verdict
/// against the current VRAM budget (computed from its known on-disk size —
/// no network call).
#[derive(Debug, Clone, Serialize)]
pub struct KnownModelDto {
    pub id: String,
    pub name: String,
    /// [`crate::model::ModelKind::as_str`] value.
    pub kind: String,
    pub family: Option<String>,
    pub publisher: String,
    pub repo: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub license: String,
    pub note: String,
    /// The curated "pick this one" model for its role.
    pub is_default: bool,
    /// `"image"` or `"video"` — which stack this entry belongs to.
    pub media: String,
    pub fit: crate::compat::FitVerdict,
}

/// `GET /models/stacks` — a base image/video model bundled with every
/// companion file it needs (VAE, text encoder, …), so "download the whole
/// thing" is one button instead of hunting down each piece separately.
#[derive(Debug, Clone, Serialize)]
pub struct ModelStackDto {
    pub id: String,
    pub label: String,
    /// `"image"` or `"video"`.
    pub media: String,
    pub note: String,
    /// The curated "pick this one" stack for its media type.
    pub is_default: bool,
    /// Base model first, then companions — display order.
    pub members: Vec<KnownModelDto>,
    /// Fit for the **whole stack loaded together** (every member's size
    /// summed) — a real render needs the base model *and* every companion in
    /// VRAM at once, so judging fit off the base model alone (or the best
    /// individual member) understates the requirement. Each member's own
    /// `fit` is still its size in isolation, useful for "is this one file by
    /// itself reasonable" — this field is the number that actually matters
    /// for "can I run this stack".
    pub fit: crate::compat::FitVerdict,
}

/// `GET /models/featured` — the curated chat/coding recommendations
/// ([`crate::model::FeaturedModel`]), enriched with a fit verdict computed
/// from `typical_vram_mb` (a documented estimate, not a live file lookup —
/// see `GET /registry/models/{repo}` for the real thing).
#[derive(Debug, Clone, Serialize)]
pub struct FeaturedModelDto {
    pub id: String,
    /// `"chat"` or `"coding"`.
    pub role: String,
    pub label: String,
    /// Hugging Face `owner/repo`.
    pub repo: String,
    pub quant_hint: String,
    pub typical_vram_mb: u32,
    pub license: String,
    pub note: String,
    /// Roles to check on import.
    pub import_roles: Vec<String>,
    /// The curated "pick this one" model for its role.
    pub is_default: bool,
    pub fit: crate::compat::FitVerdict,
}

/// A curated Colibri model (`GET /models/colibri`) — see
/// `crate::model::catalog::ColibriModel` for why AIWM doesn't download these
/// itself.
#[derive(Debug, Clone, Serialize)]
pub struct ColibriModelDto {
    pub id: String,
    pub label: String,
    /// Hugging Face `owner/repo` — the UI shows `hf download <repo> --local-dir …`.
    pub repo: String,
    pub ram_estimate_mb: u32,
    pub disk_estimate_bytes: u64,
    pub license: String,
    pub note: String,
}

/// Body for `POST /models/colibri` / `register_colibri_model` — the user has
/// already run `hf download` themselves; this just registers where it landed.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterColibriModelDto {
    /// A `ColibriModelDto.id` — supplies the label / RAM estimate / roles.
    pub catalog_id: String,
    /// Local path to the downloaded model directory.
    pub dir: String,
}

/// Body for `POST /downloads` — queue a model download (a Discover card's
/// "Download & import").
#[derive(Debug, Clone, Deserialize)]
pub struct EnqueueDownloadDto {
    pub url: String,
    /// A path or basename; only the basename is kept.
    pub filename: String,
    /// `chat` | `checkpoint` | … — passed to `import_model` on completion.
    #[serde(default)]
    pub model_type: Option<String>,
    /// Expected SHA-256 (from the registry) — verified before the import.
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    /// Roles to stamp on the model once imported (e.g. `["chat", "coding"]`
    /// for an agent pick). Older clients that omit it get a plain download.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Where the file comes from (Civitai model/version or Hugging Face
    /// repo/revision) and the base label the source gives it — recorded on
    /// the model at import (Plan 14). Omitted → nothing is recorded.
    #[serde(default)]
    pub origin: Option<crate::download::DownloadOriginDto>,
}

/// Body for `PUT /models/{id}/tags` — replace a model's tag set (Phase 6.9).
#[derive(Debug, Clone, Deserialize)]
pub struct SetTagsDto {
    pub tags: Vec<String>,
}

/// Optional body for `POST /models/{id}/benchmark`. Every field is optional and
/// the whole body may be omitted — that is the Model Library's plain "Test
/// model" button, which still means the single-prompt quick test.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BenchmarkOptionsDto {
    /// Id of a [`crate::bench::suites`] suite to run instead of the default
    /// prompt. Refused with a 400 when it names no known suite.
    #[serde(default)]
    pub suite: Option<String>,
    /// Generation passes per prompt. Passed through as given — the job clamps
    /// it to `1..=10` itself, so a silly number is not worth a refusal.
    #[serde(default)]
    pub runs: Option<u32>,
}

/// Query for `GET /benchmarks/history` — the Benchmark tab's cross-model list.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BenchmarkHistoryDto {
    /// Keep only rows measured with this suite; absent = every row, including
    /// the suite-less quick tests.
    #[serde(default)]
    pub suite: Option<String>,
    /// Rows to return, newest first. Default 50, clamped to `1..=200`.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Body for `PUT /models/{id}/roles` — replace a model's role set. Lets a
/// model fixed up after the fact (e.g. a chat GGUF downloaded before its
/// `coding` role was set) get the role without re-importing.
#[derive(Debug, Clone, Deserialize)]
pub struct SetRolesDto {
    pub roles: Vec<String>,
}

/// Body for `PUT /models/{id}/name` — rename a model's display name (never
/// touches the file on disk).
#[derive(Debug, Clone, Deserialize)]
pub struct RenameModelDto {
    pub name: String,
}

/// Body for `PUT /registry/token` — set (or clear, when blank) the Hugging Face
/// token. Applied on the next restart.
#[derive(Debug, Clone, Deserialize)]
pub struct SetTokenDto {
    pub token: String,
}

/// Body for `POST /external-engines/attach` — bring-your-own-engine (7.x):
/// point the llama.cpp runtime slot at an already-running external server
/// (Ollama, LM Studio, …) instead of a self-managed one.
#[derive(Debug, Clone, Deserialize)]
pub struct AttachExternalDto {
    pub port: u16,
    pub model_id: String,
    /// The caller's own estimate — AIWM has no way to introspect VRAM usage
    /// of a process it doesn't manage.
    pub vram_mb: u64,
}

/// Body for `POST /external-engines/detach`.
#[derive(Debug, Clone, Deserialize)]
pub struct DetachEngineDto {
    pub model_id: String,
}

/// `GET /local-api/status` — the unified local API endpoint (7.x): one
/// OpenAI-compatible address that always forwards to whichever model is
/// currently resident on llama.cpp. The token itself is never echoed back,
/// only whether one is configured (mirrors [`crate::registry::RegistryStatus`]).
#[derive(Debug, Clone, Serialize)]
pub struct LocalApiStatusDto {
    /// `http://127.0.0.1:<core_api_port>/v1` — what an external tool points at.
    pub endpoint: String,
    pub token_set: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clip-bound fields of [`UpdateDatasetFrameDto`] are three-valued, and
    /// the difference between "keep" and "clear" is the whole point of the
    /// clip in/out editor: a curator who clears the out-point must get the
    /// clip's natural end back, not silently keep the old trim.
    #[test]
    fn clip_bounds_distinguish_absent_null_and_a_number() {
        let absent: UpdateDatasetFrameDto = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.clip_start_secs, None);
        assert_eq!(absent.clip_end_secs, None);

        let cleared: UpdateDatasetFrameDto =
            serde_json::from_str(r#"{"clip_end_secs": null}"#).unwrap();
        assert_eq!(cleared.clip_start_secs, None, "absent stays absent");
        assert_eq!(cleared.clip_end_secs, Some(None), "explicit null = clear");

        let set: UpdateDatasetFrameDto =
            serde_json::from_str(r#"{"clip_start_secs": 1.5}"#).unwrap();
        assert_eq!(set.clip_start_secs, Some(Some(1.5)));
    }

    /// A cleanup body that forgets `dry_run` previews; it never deletes.
    #[test]
    fn cleanup_body_defaults_to_a_dry_run() {
        let body: CleanupDatasetDto = serde_json::from_str("{}").unwrap();
        assert!(body.dry_run);
        let real: CleanupDatasetDto = serde_json::from_str(r#"{"dry_run": false}"#).unwrap();
        assert!(!real.dry_run);
        let dedup: DedupDatasetDto = serde_json::from_str("{}").unwrap();
        assert_eq!(dedup.threshold, None);
    }

    /// The job-keyed export route predates `caption_order`; a body without it
    /// must still deserialize, prose-first.
    #[test]
    fn export_body_defaults_to_prose_first() {
        let body: ExportDatasetDto = serde_json::from_str(r#"{"dest_dir": "E:\\out"}"#).unwrap();
        assert_eq!(
            body.caption_order,
            crate::capability::dataset::CaptionOrder::ProseFirst
        );
        let tags: ExportDatasetDto =
            serde_json::from_str(r#"{"dest_dir": "E:\\out", "caption_order": "tags_first"}"#)
                .unwrap();
        assert_eq!(
            tags.caption_order,
            crate::capability::dataset::CaptionOrder::TagsFirst
        );
    }
}
