//! AI Workstation Manager — core.
//!
//! All application logic lives here. The Tauri host (`src-tauri`) and any future
//! CLI are thin shells that call into [`api`].

// `unwrap`/`expect` are warned against in production code (workspace lints) but
// are idiomatic in tests; allow them only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//!
//! Module map (see `docs/PHASE_1_PLAN.md`):
//! - [`paths`]        — `%APPDATA%` layout resolution (WP-1)
//! - [`config`]       — load & validate `config.toml`, `AIWM_*` overrides (WP-1)
//! - [`logging`]      — process-wide tracing setup (WP-1)
//! - [`app`]          — bootstrap paths + config + logging into an [`app::App`] (WP-1)
//! - [`telemetry`]    — NVML + sysinfo sampler, 1 Hz (WP-3)
//! - [`db`]           — sqlx pool, migrations, repositories (WP-2)
//! - [`runtime`]      — `RuntimeAdapter` trait, supervisor, llama.cpp (2.2) + ComfyUI (3.1) adapters
//! - [`orchestrator`] — job state machine + engine (WP-5)
//! - [`capability`]   — capability-specific job bodies (chat 2.4, image 3.4, video 4.1)
//! - [`agent`]        — `AgentAdapter` trait + types for the OpenCode/Hermes runtimes (5.1)
//! - [`pipeline`]     — fixed image workflow-JSON templates (3.4)
//! - [`compat`]        — VRAM / KV-cache fit estimate before a model load (2.6)
//! - [`link`]         — canonical model file ↔ runtime layout (junction/copy, 2.3)
//! - [`scheduler`]    — `Scheduler` trait + hybrid scheduler skeleton (WP-5)
//! - [`sidecar`]      — JSON-RPC client for the Python sidecar (WP-8)
//! - [`api`]          — handlers shared by Tauri IPC and the loopback HTTP/WS API (WP-6)

pub mod agent;
pub mod api;
pub mod app;
pub mod backup;
pub mod capability;
pub mod compat;
pub mod config;
pub mod db;
pub mod error;
pub mod link;
pub mod logging;
pub mod model;
pub mod orchestrator;
pub mod paths;
pub mod pipeline;
pub mod runtime;
pub mod scheduler;
pub mod sidecar;
pub mod telemetry;

pub use agent::{
    AgentAdapter, AgentEvent, AgentKind, EndpointConfig, FakeAgentAdapter, HermesAgentAdapter,
    HermesInstallStatus, OpenCodeAdapter, PermissionDecision, SessionSpec,
};
pub use api::{ApiServer, Services};
pub use app::App;
pub use capability::agent::{AgentSessions, CodingRuntime, LlamaCodingRuntime};
pub use compat::{estimate as estimate_vram, ModelDims, VramEstimate};
pub use config::Config;
pub use db::{Database, Model, ModelRepo, NewModel};
pub use error::{CoreError, Result};
pub use link::LinkStrategy;
pub use model::{import_model, GgufInfo, ImportOutcome, ImportRequest};
pub use orchestrator::{JobEngine, JobOutcome, JobState};
pub use paths::AppPaths;
pub use runtime::{
    ComfyUiAdapter, Health, LlamaCppAdapter, LlamaServerOptions, RuntimeAdapter, RuntimeKind,
    RuntimeRegistry, RuntimeSupervisor, SpawnSpec,
};
pub use scheduler::{Decision, HybridScheduler, PlanRequest, Scheduler};
pub use sidecar::{Handshake, SidecarClient, SidecarSpec};

/// Semantic version of the core crate, surfaced in the sidecar handshake and the
/// API `about` endpoint.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
