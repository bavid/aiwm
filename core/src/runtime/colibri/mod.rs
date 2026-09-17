//! Colibri runtime adapter — a CPU-only local inference engine for MoE models
//! too large to fit in VRAM (github.com/JustVugg/colibri), used here as an
//! additional OpenAI-compatible chat backend for models like Qwen3.6-35B-A3B
//! and OLMoE that don't fit AIWM's usual GGUF/llama.cpp path.
//!
//! Only the CPU path is automated: Colibri's GPU tier needs a from-source
//! Windows build (CUDA Toolkit + MSVC Build Tools + MSYS2) that AIWM's
//! pinned-download installer pattern can't reproduce safely, and it hits a
//! Smart App Control gotcha only the user can clear.
//!
//! Unlike llama.cpp/ComfyUI, Colibri's real constraint is system RAM, not
//! VRAM — the [`RuntimeAdapter`] trait is VRAM-shaped (`load_model(id,
//! vram_mb)`), so this adapter always reports `0` and never competes for the
//! scheduler's VRAM budget; the RAM check belongs to the capability layer
//! that calls it (a preflight, not scheduling), same idea as the existing
//! video-job RAM preflight.

mod client;
pub mod install;
mod launch;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub use self::client::GenerationEvent;

use self::client::ColibriClient;
use self::install::InstallPhase;
use self::launch::{build_spawn_spec, resolve_launcher};
use super::{
    free_loopback_port, Health, LoadedModel, RuntimeAdapter, RuntimeKind, RuntimeSupervisor,
    SpawnSpec, SupervisorState,
};
use crate::db::{runtime_state, Database};
use crate::{CoreError, Result};

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "colibri";
/// `runtimes.kind` value (matches `RuntimeKind::Colibri`'s serde name).
const RUNTIME_KIND: &str = "colibri";

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_LOAD_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) fn colibri_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

/// We started this server and own its lifecycle. Unlike llama.cpp/ComfyUI,
/// there is no "attach to a user-started one" mode yet — one real Colibri
/// user story (Qwen3.6/OLMoE as an extra chat backend) doesn't need it.
#[derive(Debug, Default)]
enum Slot {
    #[default]
    Empty,
    Loading {
        label: String,
    },
    Loaded {
        model_id: String,
        label: String,
        port: u16,
        api_key: String,
        supervisor: RuntimeSupervisor,
    },
}

enum Resident {
    None,
    Same,
    Other,
}

/// How the adapter finds the Colibri launcher.
#[derive(Debug)]
enum BinSource {
    /// Exactly this path, or nothing (tests / an explicit config override).
    Fixed(Option<PathBuf>),
    /// Re-scan the managed dir + `AIWM_COLIBRI_PATH` + `PATH` on every call, so
    /// a fresh install is picked up without restarting.
    Scan(PathBuf),
}

/// Progress of [`ColibriAdapter::install`].
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
pub struct ColibriAdapter {
    bin: BinSource,
    db: Database,
    client: ColibriClient,
    slot: Mutex<Slot>,
    install: Mutex<InstallState>,
    /// Serialises whole `load` / `unload` operations.
    op_lock: AsyncMutex<()>,
    /// Held for the duration of an install.
    install_lock: AsyncMutex<()>,
}

impl ColibriAdapter {
    /// The adapter re-scans `AIWM_COLIBRI_PATH`, the managed install under
    /// `runtimes_dir`, then `PATH` on demand — a fresh install needs no restart.
    /// It registers even when nothing is found ("not installed yet").
    pub fn discover(db: Database, runtimes_dir: &Path) -> Self {
        Self::new(db, BinSource::Scan(runtimes_dir.to_path_buf()))
    }

    /// Construct with an explicit (or absent) launcher path — tests, or a
    /// config override that pins it.
    pub fn with_binary(db: Database, launcher: Option<PathBuf>) -> Self {
        Self::new(db, BinSource::Fixed(launcher))
    }

    fn new(db: Database, bin: BinSource) -> Self {
        Self {
            bin,
            db,
            client: ColibriClient::new(),
            slot: Mutex::new(Slot::Empty),
            install: Mutex::new(InstallState::Idle),
            op_lock: AsyncMutex::new(()),
            install_lock: AsyncMutex::new(()),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resolve the Colibri launcher now (may hit the filesystem).
    fn launcher_bin(&self) -> Option<PathBuf> {
        match &self.bin {
            BinSource::Fixed(p) => p.clone(),
            BinSource::Scan(dir) => resolve_launcher(dir, |k| std::env::var_os(k)),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.launcher_bin().is_some()
    }

    /// Current install progress (drives the UI status line).
    pub fn install_state(&self) -> InstallState {
        self.install
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn is_installing(&self) -> bool {
        matches!(self.install_state(), InstallState::Running { .. })
    }

    fn set_install_state(&self, s: InstallState) {
        *self.install.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }

    /// Download, verify and extract the pinned Colibri release into the
    /// managed runtimes dir, then record it in the `runtimes` table.
    /// Errors immediately when the adapter was built with a fixed binary path.
    pub async fn install(&self, offline: bool) -> Result<()> {
        let BinSource::Scan(dir) = &self.bin else {
            return Err(colibri_err(
                "this adapter uses a fixed binary path; nothing to install",
            ));
        };
        let dir = dir.clone();
        let Ok(_guard) = self.install_lock.try_lock() else {
            return Err(colibri_err("a Colibri install is already running"));
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

    fn resident(&self, model_id: &str) -> Resident {
        match &*self.slot() {
            Slot::Loaded { model_id: m, .. } if m == model_id => Resident::Same,
            Slot::Loaded { .. } => Resident::Other,
            _ => Resident::None,
        }
    }

    /// Resolve `model_id` to its on-disk model directory + display name.
    /// Colibri models are a whole downloaded directory (`config.json` plus
    /// shards), never a single file.
    async fn resolve_model(&self, model_id: &str) -> Result<(PathBuf, String)> {
        let model = self
            .db
            .models()
            .get(model_id)
            .await?
            .ok_or_else(|| colibri_err(format!("model {model_id} is not in the library")))?;
        let path = PathBuf::from(&model.file_path);
        if !path.is_dir() {
            return Err(colibri_err(format!(
                "model directory is missing: {}",
                path.display()
            )));
        }
        Ok((path, model.name))
    }

    async fn stop_current(&self) -> Result<()> {
        let taken = std::mem::take(&mut *self.slot());
        if let Slot::Loaded {
            model_id,
            mut supervisor,
            ..
        } = taken
        {
            tracing::info!(model_id, "stopping colibri");
            supervisor.stop().await?;
        }
        Ok(())
    }

    async fn await_healthy(&self, port: u16, supervisor: &RuntimeSupervisor) -> Result<()> {
        let deadline = Instant::now() + DEFAULT_LOAD_TIMEOUT;
        loop {
            if supervisor.state() == SupervisorState::GaveUp {
                return Err(colibri_err(
                    "colibri keeps crashing on startup — check the model directory and free RAM",
                ));
            }
            if self.client.health(port).await == Health::Healthy {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(colibri_err(format!(
                    "colibri did not become ready within {}s",
                    DEFAULT_LOAD_TIMEOUT.as_secs()
                )));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    /// Stream a completion from the resident model: a [`GenerationEvent`] per
    /// token chunk, then a final `Done`. Returns early if `tx`'s receiver is
    /// dropped (that is how the chat job cancels).
    pub async fn stream_completion(
        &self,
        prompt: &str,
        max_tokens: i32,
        tx: mpsc::Sender<GenerationEvent>,
    ) -> Result<()> {
        self.stream_completion_with(prompt, max_tokens, None, tx)
            .await
    }

    /// Like [`stream_completion`](Self::stream_completion), but with an optional
    /// system message in front of the user message — a resolved persona's prompt
    /// ([`crate::persona`]). Colibri speaks the same OpenAI-compatible
    /// `messages` array llama.cpp does, so personas work here too.
    pub async fn stream_completion_with(
        &self,
        prompt: &str,
        max_tokens: i32,
        system: Option<&str>,
        tx: mpsc::Sender<GenerationEvent>,
    ) -> Result<()> {
        let (port, model_id, api_key) = match &*self.slot() {
            Slot::Loaded {
                port,
                model_id,
                api_key,
                ..
            } => (*port, model_id.clone(), api_key.clone()),
            _ => return Err(colibri_err("no model is loaded")),
        };
        self.client
            .complete_stream(port, &api_key, &model_id, prompt, max_tokens, system, tx)
            .await
    }
}

#[async_trait]
impl RuntimeAdapter for ColibriAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Colibri
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

    /// `vram_mb` is always treated as `0` here — Colibri's real constraint is
    /// system RAM, checked by the capability layer before this is ever
    /// called, not the VRAM-shaped scheduler this trait was built for.
    async fn load_model(&self, model_id: &str, _vram_mb: u64) -> Result<()> {
        let _op = self.op_lock.lock().await;

        if let Resident::Same = self.resident(model_id) {
            return Ok(());
        }

        let bin = self
            .launcher_bin()
            .ok_or_else(|| colibri_err("colibri is not installed — run Colibri setup first"))?;
        let (model_dir, label) = self.resolve_model(model_id).await?;
        let port = free_loopback_port()?;
        let api_key = Uuid::now_v7().to_string();

        // One server per model: loading a different model here means swapping
        // the resident one out.
        if let Resident::Other = self.resident(model_id) {
            self.stop_current().await?;
        }

        let spec = build_spawn_spec(&bin, &model_dir, port, model_id, &api_key);
        let supervisor = RuntimeSupervisor::start(RUNTIME_ID, spec)?;
        *self.slot() = Slot::Loading {
            label: label.clone(),
        };

        if let Err(e) = self.await_healthy(port, &supervisor).await {
            *self.slot() = Slot::Empty; // `supervisor` drops -> process killed
            return Err(e);
        }

        tracing::info!(model_id, label = %label, port, "colibri is serving");
        *self.slot() = Slot::Loaded {
            model_id: model_id.to_string(),
            label,
            port,
            api_key,
            supervisor,
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

    /// The resident model, if any — always at `vram_mb: 0` (see the
    /// `load_model` doc comment). Still reported, not an empty list: other
    /// code (e.g. "refuse to delete a model that is currently loaded") keys
    /// off this regardless of which runtime is serving it.
    fn loaded_models(&self) -> Vec<LoadedModel> {
        match &*self.slot() {
            Slot::Loaded { model_id, .. } => vec![LoadedModel {
                model_id: model_id.clone(),
                vram_mb: 0,
            }],
            _ => Vec::new(),
        }
    }

    fn detail(&self) -> Option<String> {
        Some(match self.install_state() {
            InstallState::Running {
                phase: InstallPhase::Downloading,
                done_bytes,
                total_bytes,
            } => {
                let pct = done_bytes
                    .saturating_mul(100)
                    .checked_div(total_bytes)
                    .unwrap_or(0);
                return Some(format!("downloading Colibri — {pct}%"));
            }
            InstallState::Running {
                phase: InstallPhase::Extracting,
                ..
            } => return Some("unpacking Colibri…".to_string()),
            InstallState::Failed { error } => return Some(format!("setup failed: {error}")),
            InstallState::Idle => match &*self.slot() {
                Slot::Empty if !self.is_installed() => "not installed".to_string(),
                Slot::Empty => "installed · idle".to_string(),
                Slot::Loading { label } => format!("loading {label}…"),
                Slot::Loaded { label, port, .. } => format!("serving {label} on :{port}"),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn adapter() -> ColibriAdapter {
        let db = Database::connect_in_memory().await.unwrap();
        ColibriAdapter::with_binary(db, None)
    }

    #[tokio::test]
    async fn detail_shows_not_installed_when_nothing_is_installed() {
        let a = adapter().await;
        assert_eq!(a.detail().as_deref(), Some("not installed"));
    }

    #[tokio::test]
    async fn detail_shows_download_progress_while_installing() {
        let a = adapter().await;
        a.set_install_state(InstallState::Running {
            phase: InstallPhase::Downloading,
            done_bytes: 50,
            total_bytes: 100,
        });
        assert_eq!(a.detail().as_deref(), Some("downloading Colibri — 50%"));
    }

    #[tokio::test]
    async fn detail_shows_extracting_while_unpacking() {
        let a = adapter().await;
        a.set_install_state(InstallState::Running {
            phase: InstallPhase::Extracting,
            done_bytes: 100,
            total_bytes: 100,
        });
        assert_eq!(a.detail().as_deref(), Some("unpacking Colibri…"));
    }

    #[tokio::test]
    async fn detail_shows_the_failed_reason() {
        let a = adapter().await;
        a.set_install_state(InstallState::Failed {
            error: "boom".into(),
        });
        assert_eq!(a.detail().as_deref(), Some("setup failed: boom"));
    }
}
