//! Response and request shapes for the API. Both transports (Tauri IPC and the
//! loopback HTTP server) serialize these identical types.

use serde::{Deserialize, Serialize};

use crate::runtime::{Health, RuntimeKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AboutDto {
    pub core_version: String,
    pub data_dir: String,
    pub store_path: String,
    pub core_api_port: u16,
    pub vram_budget_mb: u64,
    pub offline_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatusDto {
    pub id: String,
    pub kind: RuntimeKind,
    pub health: Health,
    pub vram_used_mb: u64,
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
