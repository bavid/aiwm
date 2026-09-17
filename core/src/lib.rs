//! AI Workstation Manager — core.
//!
//! All application logic lives here. The Tauri host (`src-tauri`) and any future
//! CLI are thin shells that call into [`api`].

// `unwrap`/`expect` are warned against in production code (workspace lints) but
// are idiomatic in tests; allow them only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//!
//! Module map (see `docs/PHASE_1_PLAN.md`):
//! - [`paths`]        — data-directory layout resolution, portable by default (WP-1)
//! - [`config`]       — load & validate `config.toml`, `AIWM_*` overrides (WP-1)
//! - [`logging`]      — process-wide tracing setup (WP-1)
//! - [`app`]          — bootstrap paths + config + logging into an [`app::App`] (WP-1)
//! - [`telemetry`]    — NVML + sysinfo sampler, 1 Hz (WP-3)
//! - [`db`]           — sqlx pool, migrations, repositories (WP-2)
//! - [`download`]     — the model download manager: queue, resume, verify, import (6.4)
//! - [`runtime`]      — `RuntimeAdapter` trait, supervisor, llama.cpp (2.2) + ComfyUI (3.1) adapters
//! - [`orchestrator`] — job state machine + engine (WP-5)
//! - [`capability`]   — capability-specific job bodies (chat 2.4, image 3.4, video 4.1)
//! - [`agent`]        — `AgentAdapter` trait + types for the OpenCode/Hermes runtimes (5.1)
//! - [`pipeline`]     — fixed image workflow-JSON templates (3.4)
//! - [`progress`]     — live per-job render progress, sourced from ComfyUI's own `/ws`
//! - [`registry`]     — online model discovery: `ModelSource` + Hugging Face (6.1)
//! - [`compat`]        — VRAM / KV-cache fit estimate before a model load (2.6)
//! - [`link`]         — canonical model file ↔ runtime layout (junction/copy, 2.3)
//! - [`scheduler`]    — `Scheduler` trait + hybrid scheduler skeleton (WP-5)
//! - [`sidecar`]      — JSON-RPC client for the Python sidecar (WP-8)
//! - [`api`]          — handlers shared by Tauri IPC and the loopback HTTP/WS API (WP-6)

pub mod agent;
pub mod api;
pub mod app;
pub mod backup;
pub mod bench;
pub mod capability;
pub mod cleanup;
pub mod compat;
pub mod config;
pub mod db;
pub mod download;
pub mod error;
pub mod launcher;
pub mod link;
pub mod logging;
pub mod model;
pub mod orchestrator;
pub mod paths;
pub mod pipeline;
pub mod progress;
pub mod rag;
pub mod recommend;
pub mod registry;
pub mod runtime;
pub mod scheduler;
pub mod select;
pub mod sidecar;
pub mod telemetry;
pub mod training;
pub mod upgrade;
pub mod voice_identity;

pub use agent::{
    AgentAdapter, AgentEvent, AgentKind, EndpointConfig, FakeAgentAdapter, HermesAgentAdapter,
    HermesInstallStatus, OpenCodeAdapter, PermissionDecision, SessionSpec,
};
pub use api::{ApiServer, Services};
pub use app::{App, AppOptions};
pub use bench::{suites as bench_suites, BenchOutcome, BenchReport, BenchRequest, PromptResult};
pub use capability::agent::{AgentSessions, CodingRuntime, LlamaCodingRuntime};
pub use cleanup::StorageReport;
pub use compat::{estimate as estimate_vram, FitVerdict, ModelDims, VramEstimate};
pub use config::Config;
pub use db::{Database, Model, ModelRepo, NewModel};
pub use download::{DownloadManager, EnqueueRequest};
pub use error::{CoreError, Result};
pub use launcher::{LaunchInfo, LaunchRequest, LaunchTool, Launcher, ToolBinaries};
pub use link::LinkStrategy;
pub use model::{
    delete_model, import_model, DeleteOutcome, GgufInfo, ImportOutcome, ImportRequest,
};
pub use orchestrator::{JobEngine, JobOutcome, JobState};
pub use paths::AppPaths;
pub use progress::{JobProgress, ProgressHub};
pub use registry::{
    CivitaiSource, Fetched, Freshness, Gated, HuggingFaceSource, ModelSource, Registry,
    RegistryStatus, RemoteFile, RemoteFormat, RemoteModel, RemoteModelDetails, SearchQuery,
    SearchSort,
};
pub use runtime::{
    ComfyUiAdapter, Health, LlamaCppAdapter, LlamaServerOptions, RuntimeAdapter, RuntimeKind,
    RuntimeRegistry, RuntimeSupervisor, SpawnSpec,
};
pub use scheduler::{Decision, HybridScheduler, PlanRequest, Scheduler};
pub use select::AutoPreference;
pub use sidecar::{Handshake, SidecarClient, SidecarSpec};
pub use upgrade::{Reasoner, UpgradeCandidate, UpgradeReport, UpgradeTarget};
pub use voice_identity::{create_voice_identity, delete_voice_identity, CreateVoiceIdentity};

/// Semantic version of the core crate, surfaced in the sidecar handshake and the
/// API `about` endpoint.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
