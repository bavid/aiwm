//! Runtime adapters and process supervision.
//!
//! An adapter knows *how* to run one runtime (llama.cpp, ComfyUI, …): where its
//! binary is, how to health-check it, how to load and unload models. The
//! [`RuntimeSupervisor`] owns the *lifecycle* mechanics — spawning the child
//! into a Windows Job Object so it can never outlive the app, and restarting it
//! with backoff if it crashes.
//!
//! WP-4 ships the trait, the supervisor and a [`FakeRuntimeAdapter`]. Real
//! adapters (llama.cpp in Phase 2) implement the same trait.

mod colibri;
mod comfyui;
pub(crate) mod download;
mod external;
mod fake;
mod job;
mod llamacpp;
mod registry;
mod supervisor;
pub mod tts;
pub mod vision;

pub use colibri::install as colibri_install;
pub use colibri::{
    ColibriAdapter, GenerationEvent as ColibriGenerationEvent, InstallState as ColibriInstallState,
};
pub use comfyui::{
    ComfyDirs, ComfyLaunch, ComfyOptions, ComfyUiAdapter, GeneratedMedia, SystemStats, VramMode,
};
pub use external::{detect as detect_external_engines, DetectedEngine};
pub use fake::{FakeConfig, FakeRuntimeAdapter};
pub use job::JobObject;
pub use llamacpp::{GenerationEvent, InstallState, LlamaCppAdapter, LlamaServerOptions};
pub use registry::RuntimeRegistry;
pub use supervisor::{RuntimeSupervisor, SupervisorState};
pub use tts::TtsAdapter;
pub use vision::VisionAdapter;

use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

/// Reserve a free loopback port by binding `:0` and releasing it. A tiny race
/// remains until the runtime process binds it; negligible on a single-user
/// desktop. Shared by the llama.cpp and ComfyUI adapters.
pub(crate) fn free_loopback_port() -> Result<u16> {
    let port_err = |e: std::io::Error, what: &str| CoreError::Runtime {
        runtime: "supervisor".into(),
        message: format!("could not {what} a local port: {e}"),
    };
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| port_err(e, "reserve"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| port_err(e, "read"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    LlamaCpp,
    ComfyUi,
    Colibri,
    Tts,
    /// The Florence-2 / Qwen2.5-VL captioning pipeline for `job_type=
    /// dataset_prep` — see [`vision::VisionAdapter`].
    Vision,
    Fake,
}

/// Result of probing a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Unknown,
    Starting,
    Healthy,
    Unhealthy,
}

/// How to launch a runtime process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Vars to set on the child (added to the inherited environment).
    pub env: Vec<(String, String)>,
    /// Vars to strip from the inherited environment before spawning — the agent
    /// sandbox uses this to keep cloud-provider API keys out of the child
    /// (ADR-010).
    pub env_remove: Vec<String>,
}

impl SpawnSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
}

/// A model currently resident in a runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedModel {
    pub model_id: String,
    pub vram_mb: u64,
}

/// A managed AI runtime. Implementations talk to one runtime's HTTP API; the
/// core drives them through this trait only.
#[async_trait]
pub trait RuntimeAdapter: Send + Sync + std::fmt::Debug {
    /// Stable identifier, e.g. `"llamacpp"`.
    fn id(&self) -> &str;

    fn kind(&self) -> RuntimeKind;

    /// How to launch the runtime, or `None` when there is no process to manage.
    fn spawn_spec(&self) -> Option<SpawnSpec>;

    /// Probe the running runtime.
    async fn health(&self) -> Health;

    /// Load `model_id`, charging `vram_mb` against this runtime's budget.
    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()>;

    /// Unload `model_id`. A no-op if it is not loaded.
    async fn unload_model(&self, model_id: &str) -> Result<()>;

    /// Models currently loaded in this runtime, with their VRAM cost.
    fn loaded_models(&self) -> Vec<LoadedModel>;

    /// Total VRAM (MB) attributed to this runtime's loaded models.
    fn vram_used_mb(&self) -> u64 {
        self.loaded_models().iter().map(|m| m.vram_mb).sum()
    }

    /// One short, human-readable status line for the UI, e.g.
    /// `"not installed"` or `"serving qwen2.5-coder on :48213"`. `None` when the
    /// adapter has nothing useful to add beyond [`health`](Self::health).
    fn detail(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_loopback_port_is_a_usable_high_port() {
        let port = free_loopback_port().unwrap();
        assert!(port >= 1024);
        // The port is actually free right after: we can bind it ourselves.
        assert!(std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }
}
