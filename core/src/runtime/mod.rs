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

mod fake;
mod job;
mod llamacpp;
mod registry;
mod supervisor;

pub use fake::{FakeConfig, FakeRuntimeAdapter};
pub use job::JobObject;
pub use llamacpp::{InstallState, LlamaCppAdapter, LlamaServerOptions};
pub use registry::RuntimeRegistry;
pub use supervisor::{RuntimeSupervisor, SupervisorState};

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    LlamaCpp,
    ComfyUi,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl SpawnSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
}

/// A model currently resident in a runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
