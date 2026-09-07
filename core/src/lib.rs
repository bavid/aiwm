//! AI Workstation Manager — core.
//!
//! All application logic lives here. The Tauri host (`src-tauri`) and any future
//! CLI are thin shells that call into [`api`].
//!
//! Module map (see `docs/PHASE_1_PLAN.md`):
//! - [`config`]       — load & validate `config.toml`, resolve data dirs (WP-1)
//! - [`telemetry`]    — NVML + sysinfo sampler, 1 Hz (WP-3)
//! - [`db`]           — sqlx pool, migrations, repositories (WP-2)
//! - [`runtime`]      — `RuntimeAdapter` trait, supervisor, fake adapter (WP-4)
//! - [`orchestrator`] — job state machine + engine (WP-5)
//! - [`scheduler`]    — `Scheduler` trait + hybrid scheduler skeleton (WP-5)
//! - [`sidecar`]      — JSON-RPC client for the Python sidecar (WP-8)
//! - [`api`]          — handlers shared by Tauri IPC and the loopback HTTP/WS API (WP-6)

pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod orchestrator;
pub mod runtime;
pub mod scheduler;
pub mod sidecar;
pub mod telemetry;

pub use error::{CoreError, Result};

/// Semantic version of the core crate, surfaced in the sidecar handshake and the
/// API `about` endpoint.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
