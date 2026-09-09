//! `llama.cpp` / `llama-server` runtime adapter — the primary LLM runtime
//! (ADR-006).
//!
//! `llama-server` serves exactly one model per process, so this adapter runs one
//! supervised server for the resident model:
//! [`load_model`](LlamaCppAdapter::load_model) starts it and waits for
//! `GET /health`; [`unload_model`](LlamaCppAdapter::unload_model) stops it. It
//! can also *attach* to a `llama-server` the user started themselves (ADR-002
//! fallback) — an attached server's lifetime is not ours to end.
//!
//! Installing the pinned CUDA build (verified download + extraction) lives in
//! [`install`]; [`LlamaCppAdapter::install`] drives it and tracks progress.

mod client;
pub mod install;
mod launch;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;

pub use self::client::GenerationEvent;

use self::client::LlamaClient;
use self::install::InstallPhase;
use self::launch::{build_spawn_spec, resolve_server_bin};
use super::{
    free_loopback_port, Health, LoadedModel, RuntimeAdapter, RuntimeKind, RuntimeSupervisor,
    SpawnSpec, SupervisorState,
};
use crate::db::{runtime_state, Database};
use crate::{CoreError, Result};

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "llamacpp";
/// `runtimes.kind` value (matches `RuntimeKind::LlamaCpp`'s serde name).
const RUNTIME_KIND: &str = "llama_cpp";

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_GPU_LAYERS: u32 = 999;
const DEFAULT_LOAD_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) fn llama_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

/// How `llama-server` is launched. Defaults suit a single 16 GB NVIDIA card;
/// the Settings UI exposes these in a later slice.
#[derive(Debug, Clone)]
pub struct LlamaServerOptions {
    /// `-ngl` — transformer layers to offload to the GPU. `999` = "all".
    pub gpu_layers: u32,
    /// `-c` — context window. `None` lets the adapter pick a sane cap from the
    /// model's trained context ([`crate::compat::effective_ctx`]); `Some` forces
    /// an explicit value (the Settings UI, slice 2.7).
    pub ctx_size: Option<u32>,
    /// Pass `--flash-attn on`.
    pub flash_attention: bool,
    /// Pass `--jinja` — use the GGUF's embedded chat template. Needed for
    /// reliable OpenAI tool-call parsing (agents, 5.1 / ADR-021); harmless for
    /// plain chat (the embedded template is the correct one). Default on.
    pub jinja: bool,
    /// `--chat-template <name>` — override when the embedded template isn't
    /// tool-aware. `None` = use whatever the GGUF ships.
    pub chat_template: Option<String>,
    /// How long a freshly started server has to answer `/health`.
    pub load_timeout: Duration,
    /// Extra raw arguments, appended verbatim.
    pub extra_args: Vec<String>,
}

impl Default for LlamaServerOptions {
    fn default() -> Self {
        Self {
            gpu_layers: DEFAULT_GPU_LAYERS,
            ctx_size: None,
            flash_attention: true,
            jinja: true,
            chat_template: None,
            load_timeout: DEFAULT_LOAD_TIMEOUT,
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug)]
enum Origin {
    /// We started this server and own its lifecycle.
    Managed(RuntimeSupervisor),
    /// The user started it; we only talk to it.
    Attached,
}

#[derive(Debug, Default)]
enum Slot {
    #[default]
    Empty,
    Loading {
        label: String,
    },
    Loaded {
        model_id: String,
        /// Display name for the UI; falls back to `model_id` when unknown.
        label: String,
        vram_mb: u64,
        port: u16,
        origin: Origin,
    },
}

/// Whether a given model is the resident one.
enum Resident {
    None,
    Same,
    Other,
}

/// How the adapter finds `llama-server`.
#[derive(Debug)]
enum BinSource {
    /// Exactly this path, or nothing (tests / an explicit config override).
    Fixed(Option<PathBuf>),
    /// Re-scan this managed dir + `AIWM_LLAMACPP_PATH` + `PATH` on every call, so
    /// a fresh install is picked up without restarting.
    Scan(PathBuf),
}

/// Progress of [`LlamaCppAdapter::install`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallState {
    Idle,
    Running {
        phase: InstallPhase,
        done_bytes: u64,
        total_bytes: u64,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug)]
pub struct LlamaCppAdapter {
    bin: BinSource,
    db: Database,
    client: LlamaClient,
    opts: LlamaServerOptions,
    slot: Mutex<Slot>,
    install: Mutex<InstallState>,
    /// Serialises whole `load` / `unload` / `attach` operations.
    op_lock: AsyncMutex<()>,
    /// Held for the duration of an install.
    install_lock: AsyncMutex<()>,
}

impl LlamaCppAdapter {
    /// The adapter re-scans `AIWM_LLAMACPP_PATH`, the managed install under
    /// `runtimes_dir`, then `PATH` on demand — a fresh install needs no restart.
    /// It registers even when nothing is found ("not installed yet").
    pub fn discover(db: Database, runtimes_dir: &Path) -> Self {
        Self::new(db, BinSource::Scan(runtimes_dir.to_path_buf()))
    }

    /// Construct with an explicit (or absent) server binary — tests, or a config
    /// override that pins the path.
    pub fn with_binary(db: Database, server_bin: Option<PathBuf>) -> Self {
        Self::new(db, BinSource::Fixed(server_bin))
    }

    fn new(db: Database, bin: BinSource) -> Self {
        Self {
            bin,
            db,
            client: LlamaClient::new(),
            opts: LlamaServerOptions::default(),
            slot: Mutex::new(Slot::Empty),
            install: Mutex::new(InstallState::Idle),
            op_lock: AsyncMutex::new(()),
            install_lock: AsyncMutex::new(()),
        }
    }

    #[must_use]
    pub fn with_options(mut self, opts: LlamaServerOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Resolve the `llama-server` executable now (may hit the filesystem).
    fn server_bin(&self) -> Option<PathBuf> {
        match &self.bin {
            BinSource::Fixed(p) => p.clone(),
            BinSource::Scan(dir) => resolve_server_bin(dir, |k| std::env::var_os(k)),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.server_bin().is_some()
    }

    /// Current install progress (drives the UI status line).
    pub fn install_state(&self) -> InstallState {
        self.install
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_install_state(&self, s: InstallState) {
        *self.install.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }

    /// Download, verify and extract the pinned llama.cpp CUDA build into the
    /// managed runtimes dir, then record it in the `runtimes` table. Long-running
    /// (~645 MB) — callers spawn it and poll [`install_state`](Self::install_state).
    /// Errors immediately when the adapter was built with a fixed binary path.
    pub async fn install(&self, offline: bool) -> Result<()> {
        let BinSource::Scan(dir) = &self.bin else {
            return Err(llama_err(
                "this adapter uses a fixed binary path; nothing to install",
            ));
        };
        let dir = dir.clone();
        let Ok(_guard) = self.install_lock.try_lock() else {
            return Err(llama_err("a llama.cpp install is already running"));
        };

        self.set_install_state(InstallState::Running {
            phase: InstallPhase::Downloading,
            done_bytes: 0,
            total_bytes: install::TOTAL_DOWNLOAD_BYTES,
        });
        let _ = self
            .db
            .runtimes()
            .set_state(RUNTIME_ID, RUNTIME_KIND, runtime_state::INSTALLING, None)
            .await;

        let result = install::install(&dir, offline, |phase, done_bytes, total_bytes| {
            self.set_install_state(InstallState::Running {
                phase,
                done_bytes,
                total_bytes,
            });
        })
        .await;

        match &result {
            Ok(path) => {
                self.set_install_state(InstallState::Idle);
                let _ = self
                    .db
                    .runtimes()
                    .record_install(
                        RUNTIME_ID,
                        RUNTIME_KIND,
                        install::PINNED_BUILD,
                        &path.to_string_lossy(),
                    )
                    .await;
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_install_state(InstallState::Failed { error: msg.clone() });
                let _ = self
                    .db
                    .runtimes()
                    .set_state(RUNTIME_ID, RUNTIME_KIND, runtime_state::ERROR, Some(&msg))
                    .await;
            }
        }
        result.map(|_| ())
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn resident(&self, model_id: &str) -> Resident {
        match &*self.slot() {
            Slot::Loaded { model_id: m, .. } if m == model_id => Resident::Same,
            Slot::Loaded { .. } => Resident::Other,
            _ => Resident::None,
        }
    }

    fn loaded_port(&self) -> Option<u16> {
        match &*self.slot() {
            Slot::Loaded { port, .. } => Some(*port),
            _ => None,
        }
    }

    /// Generate a completion from the resident model (non-streaming).
    pub async fn complete(&self, prompt: &str, max_tokens: i32) -> Result<String> {
        let port = self
            .loaded_port()
            .ok_or_else(|| llama_err("no model is loaded"))?;
        self.client.complete(port, prompt, max_tokens).await
    }

    /// Stream a completion from the resident model: a [`GenerationEvent`] per
    /// token chunk, then a final `Done`. Returns early if `tx`'s receiver is
    /// dropped (that is how the chat job cancels).
    pub async fn stream_completion(
        &self,
        prompt: &str,
        max_tokens: i32,
        tx: tokio::sync::mpsc::Sender<GenerationEvent>,
    ) -> Result<()> {
        let port = self
            .loaded_port()
            .ok_or_else(|| llama_err("no model is loaded"))?;
        self.client
            .complete_stream(port, prompt, max_tokens, tx)
            .await
    }

    /// Adopt a `llama-server` the user already started on `port`. Probes
    /// `/health`; on success the model becomes resident but its process is left
    /// alone by [`unload_model`](Self::unload_model).
    pub async fn attach(&self, port: u16, model_id: &str, vram_mb: u64) -> Result<()> {
        let _op = self.op_lock.lock().await;
        if self.client.health(port).await != Health::Healthy {
            return Err(llama_err(format!(
                "no healthy llama-server found on 127.0.0.1:{port}"
            )));
        }
        // Best-effort: warn if the server is serving a different file than the
        // model the caller named.
        let model = self.db.models().get(model_id).await.ok().flatten();
        if let Some(m) = &model {
            if let Some(served) = self.client.model_path(port).await {
                if !same_file_path(&m.file_path, &served) {
                    tracing::warn!(
                        expected = %m.file_path, served = %served,
                        "attached llama-server reports a different model file"
                    );
                }
            }
        }
        let label = model.map_or_else(|| model_id.to_string(), |m| m.name);

        tracing::info!(port, model_id, "attached to a user-managed llama-server");
        *self.slot() = Slot::Loaded {
            model_id: model_id.to_string(),
            label,
            vram_mb,
            port,
            origin: Origin::Attached,
        };
        Ok(())
    }

    /// Resolve `model_id` to its on-disk file, display name and trained context.
    async fn resolve_model(&self, model_id: &str) -> Result<(PathBuf, String, Option<u32>)> {
        let model = self
            .db
            .models()
            .get(model_id)
            .await?
            .ok_or_else(|| llama_err(format!("model {model_id} is not in the library")))?;
        let path = PathBuf::from(&model.file_path);
        if !path.is_file() {
            return Err(llama_err(format!(
                "model file is missing: {}",
                path.display()
            )));
        }
        let ctx_max = model.ctx_max.and_then(|v| u32::try_from(v).ok());
        Ok((path, model.name, ctx_max))
    }

    /// Stop and clear the resident server if we manage it. Attached servers are
    /// just forgotten.
    async fn stop_current(&self) -> Result<()> {
        let taken = std::mem::take(&mut *self.slot());
        if let Slot::Loaded {
            origin: Origin::Managed(mut sup),
            model_id,
            ..
        } = taken
        {
            tracing::info!(model_id, "stopping llama-server");
            sup.stop().await?;
        }
        Ok(())
    }

    async fn await_healthy(&self, port: u16, supervisor: &RuntimeSupervisor) -> Result<()> {
        let deadline = Instant::now() + self.opts.load_timeout;
        loop {
            if supervisor.state() == SupervisorState::GaveUp {
                return Err(llama_err(
                    "llama-server keeps crashing on startup — check the model file, GPU driver and free VRAM",
                ));
            }
            if self.client.health(port).await == Health::Healthy {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(llama_err(format!(
                    "llama-server did not become ready within {}s",
                    self.opts.load_timeout.as_secs()
                )));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }
}

#[async_trait]
impl RuntimeAdapter for LlamaCppAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::LlamaCpp
    }

    fn spawn_spec(&self) -> Option<SpawnSpec> {
        // No runtime-level process: a server exists only per loaded model.
        None
    }

    async fn health(&self) -> Health {
        let probe_port = {
            match &*self.slot() {
                Slot::Empty => return Health::Unknown,
                Slot::Loading { .. } => return Health::Starting,
                Slot::Loaded { port, .. } => *port,
            }
        };
        self.client.health(probe_port).await
    }

    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()> {
        let _op = self.op_lock.lock().await;

        if let Resident::Same = self.resident(model_id) {
            return Ok(());
        }

        let bin = self.server_bin().ok_or_else(|| {
            llama_err("llama-server is not installed — run llama.cpp setup first")
        })?;
        let (model_path, label, ctx_max) = self.resolve_model(model_id).await?;
        let ctx = self
            .opts
            .ctx_size
            .unwrap_or_else(|| crate::compat::effective_ctx(ctx_max));
        let port = free_loopback_port()?;

        // One server per model: loading a different model here means swapping the
        // resident one out, even if the scheduler thought both would fit (its
        // VRAM math assumes coexistence). `loaded_models()` reports only the new
        // model afterwards, so the next `free_mb()` is accurate.
        if let Resident::Other = self.resident(model_id) {
            self.stop_current().await?;
        }

        let spec = build_spawn_spec(&bin, &model_path, port, &self.opts, ctx);
        let supervisor = RuntimeSupervisor::start(RUNTIME_ID, spec)?;
        *self.slot() = Slot::Loading {
            label: label.clone(),
        };

        if let Err(e) = self.await_healthy(port, &supervisor).await {
            *self.slot() = Slot::Empty; // `supervisor` drops → process killed
            return Err(e);
        }

        tracing::info!(model_id, label = %label, port, "llama-server is serving");
        *self.slot() = Slot::Loaded {
            model_id: model_id.to_string(),
            label,
            vram_mb,
            port,
            origin: Origin::Managed(supervisor),
        };
        Ok(())
    }

    async fn unload_model(&self, model_id: &str) -> Result<()> {
        let _op = self.op_lock.lock().await;
        let mine = matches!(&*self.slot(), Slot::Loaded { model_id: m, .. } if m == model_id);
        if mine {
            self.stop_current().await?;
        }
        Ok(())
    }

    fn loaded_models(&self) -> Vec<LoadedModel> {
        match &*self.slot() {
            Slot::Loaded {
                model_id, vram_mb, ..
            } => vec![LoadedModel {
                model_id: model_id.clone(),
                vram_mb: *vram_mb,
            }],
            _ => Vec::new(),
        }
    }

    fn detail(&self) -> Option<String> {
        match self.install_state() {
            InstallState::Running {
                phase: InstallPhase::Downloading,
                done_bytes,
                total_bytes,
            } => {
                let pct = done_bytes
                    .saturating_mul(100)
                    .checked_div(total_bytes)
                    .unwrap_or(0);
                return Some(format!("downloading llama.cpp — {pct}%"));
            }
            InstallState::Running {
                phase: InstallPhase::Extracting,
                ..
            } => return Some("extracting llama.cpp…".to_string()),
            InstallState::Failed { error } => return Some(format!("setup failed: {error}")),
            InstallState::Idle => {}
        }

        Some(match &*self.slot() {
            Slot::Empty if !self.is_installed() => "not installed".to_string(),
            Slot::Empty => "installed · idle".to_string(),
            Slot::Loading { label } => format!("loading {label}…"),
            Slot::Loaded {
                label,
                port,
                origin,
                ..
            } => {
                let verb = match origin {
                    Origin::Managed(_) => "serving",
                    Origin::Attached => "attached to",
                };
                format!("{verb} {label} on :{port}")
            }
        })
    }
}

/// Loose path equality for the attach sanity check: case- and separator-
/// insensitive, which is enough to catch "wrong server" without resolving links.
fn same_file_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('/', "\\").trim_matches('"').to_ascii_lowercase();
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;
    use crate::db::Database;

    #[test]
    fn same_file_path_ignores_case_and_separators() {
        assert!(same_file_path(
            "E:/AI/models/x.gguf",
            "E:\\ai\\MODELS\\X.GGUF"
        ));
        assert!(!same_file_path("E:\\a\\x.gguf", "E:\\a\\y.gguf"));
    }

    // --- adapter behaviour against an in-process stand-in server --------------

    async fn mock_llama(healthy: bool) -> u16 {
        let router = Router::new()
            .route(
                "/health",
                get(move || async move {
                    if healthy {
                        axum::http::StatusCode::OK
                    } else {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            )
            .route(
                "/completion",
                post(|| async { Json(serde_json::json!({ "content": "hi from mock" })) }),
            );
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    async fn adapter() -> LlamaCppAdapter {
        let db = Database::connect_in_memory().await.unwrap();
        LlamaCppAdapter::with_binary(db, None)
    }

    #[tokio::test]
    async fn complete_without_a_model_errors() {
        let err = adapter().await.complete("hello", 8).await.unwrap_err();
        assert!(err.to_string().contains("no model is loaded"));
    }

    #[tokio::test]
    async fn load_without_a_binary_reports_not_installed() {
        let a = adapter().await;
        let err = a.load_model("some-id", 1000).await.unwrap_err();
        assert!(err.to_string().contains("not installed"));
        assert_eq!(a.detail().as_deref(), Some("not installed"));
    }

    #[tokio::test]
    async fn load_rejects_an_unknown_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = LlamaCppAdapter::with_binary(db, Some(PathBuf::from("llama-server")));
        let err = a.load_model("ghost", 1000).await.unwrap_err();
        assert!(err.to_string().contains("not in the library"));
    }

    #[tokio::test]
    async fn attach_adopts_a_running_server_and_unload_leaves_it_alone() {
        let port = mock_llama(true).await;
        let a = adapter().await;

        a.attach(port, "qwen", 6000).await.unwrap();
        assert_eq!(a.loaded_models().len(), 1);
        assert_eq!(a.vram_used_mb(), 6000);
        assert_eq!(a.health().await, Health::Healthy);
        assert_eq!(a.detail(), Some(format!("attached to qwen on :{port}")));
        assert_eq!(a.complete("hey", 4).await.unwrap(), "hi from mock");

        // Unloading an attached server clears our slot without stopping anything.
        a.unload_model("qwen").await.unwrap();
        assert!(a.loaded_models().is_empty());
        assert_eq!(LlamaClient::new().health(port).await, Health::Healthy);
    }

    #[tokio::test]
    async fn attach_fails_without_a_healthy_server() {
        let port = mock_llama(false).await;
        let err = adapter().await.attach(port, "m", 1).await.unwrap_err();
        assert!(err.to_string().contains("no healthy llama-server"));
    }
}
