//! Response and request shapes for the API. Both transports (Tauri IPC and the
//! loopback HTTP server) serialize these identical types.

use serde::{Deserialize, Serialize};

use crate::config::{ComfyConfig, LlamaConfig};
use crate::db::{Job, JobEvent};
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
