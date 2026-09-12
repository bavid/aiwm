//! Response and request shapes for the API. Both transports (Tauri IPC and the
//! loopback HTTP server) serialize these identical types.

use serde::{Deserialize, Serialize};

use crate::agent::{HermesInstallStatus, PermissionDecision};
use crate::config::{ComfyConfig, LlamaConfig, ModelsConfig};
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
}

/// The `paths` slice of [`ConfigUpdate`] — plain strings from the Settings
/// form. An empty string means "use the portable default", not a literal path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathsUpdateDto {
    pub outputs_path: String,
    pub runtimes_path: String,
    pub cache_path: String,
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
    /// `https://huggingface.co/<id>/resolve/<rev>/<path>` — "Copy link".
    pub download_url: String,
    /// `null` for non-model files (README, `config.json`).
    pub vram_estimate_mb: Option<u64>,
    pub fit: crate::compat::FitVerdict,
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
}

/// Body for `PUT /models/{id}/tags` — replace a model's tag set (Phase 6.9).
#[derive(Debug, Clone, Deserialize)]
pub struct SetTagsDto {
    pub tags: Vec<String>,
}

/// Body for `PUT /models/{id}/roles` — replace a model's role set. Lets a
/// model fixed up after the fact (e.g. a chat GGUF downloaded before its
/// `coding` role was set) get the role without re-importing.
#[derive(Debug, Clone, Deserialize)]
pub struct SetRolesDto {
    pub roles: Vec<String>,
}

/// Body for `PUT /registry/token` — set (or clear, when blank) the Hugging Face
/// token. Applied on the next restart.
#[derive(Debug, Clone, Deserialize)]
pub struct SetTokenDto {
    pub token: String,
}
