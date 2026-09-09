//! Response and request shapes for the API. Both transports (Tauri IPC and the
//! loopback HTTP server) serialize these identical types.

use serde::{Deserialize, Serialize};

use crate::agent::PermissionDecision;
use crate::config::{ComfyConfig, LlamaConfig};
use crate::db::{AgentSession, AgentSessionEvent, Job, JobEvent};
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
    #[serde(default)]
    pub params: serde_json::Value,
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
