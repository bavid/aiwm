//! ComfyUI runtime adapter — the image / diffusion runtime (Phase 3).
//!
//! Unlike `llama-server` (one process per model), ComfyUI is **one long-lived
//! server that loads models on demand**. So this adapter runs a single
//! supervised process, started lazily on the first
//! [`load_model`](ComfyUiAdapter::load_model) and kept up until the app exits
//! (Python boot is expensive). `load_model` here means "make sure the server is
//! up, reserve the VRAM slot, and free the previous model" — ComfyUI loads the
//! actual checkpoint when the image workflow runs (`capability::image`, 3.4).
//!
//! It can also *attach* to a ComfyUI the user started themselves (ADR-002
//! fallback) — an attached server's lifetime is not ours to end.
//!
//! Installing the pinned ComfyUI (venv + one custom node) lands in 3.2.

mod client;
mod install;
mod launch;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{watch, Mutex as AsyncMutex};

pub use self::client::SystemStats;
use self::client::{ComfyClient, PromptOutcome};
use self::install::InstallPhase;
use self::launch::{build_spawn_spec, resolve_launch};
pub use self::launch::{ComfyDirs, ComfyLaunch};
use super::{
    free_loopback_port, Health, LoadedModel, RuntimeAdapter, RuntimeKind, RuntimeSupervisor,
    SpawnSpec, SupervisorState,
};
use crate::db::{runtime_state, Database};
use crate::{CoreError, Result};

use serde::Serialize;

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "comfyui";
/// `runtimes.kind` value (matches `RuntimeKind::ComfyUi`'s serde name).
const RUNTIME_KIND: &str = "comfy_ui";

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// ComfyUI + torch import can take a while on a cold start.
const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(120);

/// How often [`ComfyUiAdapter::generate_media`] polls `GET /history`.
const MEDIA_POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Progress of [`ComfyUiAdapter::install`], mirrored to the UI status line.
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

pub(super) fn comfy_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

/// A rendered file (image or video), straight from ComfyUI's `/view` — the
/// caller writes it to the outputs directory.
#[derive(Debug, Clone)]
pub struct GeneratedMedia {
    pub bytes: Vec<u8>,
    /// File extension from the returned filename (`png` / `mp4` / …).
    pub extension: String,
}

/// ComfyUI's VRAM-management mode — the `--<mode>vram` flag. `Auto` passes no
/// flag and lets ComfyUI decide from the detected card; `LowVram` offloads more
/// aggressively (useful for Flux on 16 GB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VramMode {
    #[default]
    Auto,
    HighVram,
    NormalVram,
    LowVram,
    NoVram,
}

impl VramMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "highvram" => Self::HighVram,
            "normalvram" => Self::NormalVram,
            "lowvram" => Self::LowVram,
            "novram" => Self::NoVram,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::HighVram => "highvram",
            Self::NormalVram => "normalvram",
            Self::LowVram => "lowvram",
            Self::NoVram => "novram",
        }
    }

    fn flag(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::HighVram => Some("--highvram"),
            Self::NormalVram => Some("--normalvram"),
            Self::LowVram => Some("--lowvram"),
            Self::NoVram => Some("--novram"),
        }
    }
}

/// Launch options for the ComfyUI server. The Settings UI exposes `vram_mode`
/// via the `[comfyui]` table; a change needs an app restart (ADR-017).
#[derive(Debug, Clone, Default)]
pub struct ComfyOptions {
    pub vram_mode: VramMode,
    /// Extra raw args, appended verbatim.
    pub extra_args: Vec<String>,
}

impl ComfyOptions {
    /// The CLI args these options add, in order.
    fn args(&self) -> Vec<String> {
        self.vram_mode
            .flag()
            .map(str::to_string)
            .into_iter()
            .chain(self.extra_args.iter().cloned())
            .collect()
    }
}

/// How the adapter finds the ComfyUI entrypoint.
#[derive(Debug)]
enum LaunchSource {
    /// Exactly this, or nothing (tests / an explicit config override).
    Fixed(Option<ComfyLaunch>),
    /// Re-scan `<runtimes_dir>/comfyui/` + the env overrides on every call, so a
    /// fresh install is picked up without a restart.
    Scan(PathBuf),
}

#[derive(Debug, Default)]
enum Server {
    #[default]
    Down,
    Starting,
    Up {
        port: u16,
        /// `Some` when we manage the process; `None` when attached.
        supervisor: Option<RuntimeSupervisor>,
    },
}

#[derive(Debug)]
pub struct ComfyUiAdapter {
    launch: LaunchSource,
    dirs: ComfyDirs,
    opts: ComfyOptions,
    #[allow(dead_code)] // model-name lookups land with capability::image (3.4)
    db: Database,
    client: ComfyClient,
    server: Mutex<Server>,
    /// The single model the scheduler thinks is resident (ADR-003).
    resident: Mutex<Option<LoadedModel>>,
    /// Last `GET /system_stats` — cached on every `health()` so `detail()` can
    /// show ComfyUI's version + VRAM without an async call.
    stats: Mutex<Option<SystemStats>>,
    /// Serialises whole `load` / `unload` / `attach` / `stop` operations.
    op_lock: AsyncMutex<()>,
    install: Mutex<InstallState>,
    /// Held for the duration of an install.
    install_lock: AsyncMutex<()>,
}

impl ComfyUiAdapter {
    /// Re-scans the managed install dir + `AIWM_COMFYUI_*` on demand — a fresh
    /// install needs no restart. Registers even when nothing is found.
    pub fn discover(db: Database, runtimes_dir: &Path, dirs: ComfyDirs) -> Self {
        Self::new(db, LaunchSource::Scan(runtimes_dir.to_path_buf()), dirs)
    }

    /// Construct with an explicit (or absent) launch command — tests, or a
    /// config override.
    pub fn with_launch(db: Database, launch: Option<ComfyLaunch>, dirs: ComfyDirs) -> Self {
        Self::new(db, LaunchSource::Fixed(launch), dirs)
    }

    fn new(db: Database, launch: LaunchSource, dirs: ComfyDirs) -> Self {
        Self {
            launch,
            dirs,
            opts: ComfyOptions::default(),
            db,
            client: ComfyClient::new(),
            server: Mutex::new(Server::Down),
            resident: Mutex::new(None),
            stats: Mutex::new(None),
            op_lock: AsyncMutex::new(()),
            install: Mutex::new(InstallState::Idle),
            install_lock: AsyncMutex::new(()),
        }
    }

    /// Set the server launch options (VRAM mode, extra args). Applies on the
    /// next server start.
    #[must_use]
    pub fn with_options(mut self, opts: ComfyOptions) -> Self {
        self.opts = opts;
        self
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, Option<SystemStats>> {
        self.stats.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Current install progress (drives the UI status line).
    pub fn install_state(&self) -> InstallState {
        self.install
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether an install is running right now.
    pub fn is_installing(&self) -> bool {
        matches!(self.install_state(), InstallState::Running { .. })
    }

    fn set_install_state(&self, s: InstallState) {
        *self.install.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }

    /// Install the pinned ComfyUI into the managed runtimes dir: bootstrap `uv`,
    /// fetch the source at the pinned tag, then build a self-contained venv
    /// (Python 3.13 + the CUDA torch build + `requirements.txt`). Long-running —
    /// callers spawn it and poll [`install_state`](Self::install_state). Errors
    /// immediately when the adapter was built with a fixed launch (tests / a
    /// config override).
    pub async fn install(&self, offline: bool) -> Result<()> {
        let LaunchSource::Scan(dir) = &self.launch else {
            return Err(comfy_err(
                "this adapter uses a fixed launch command; nothing to install",
            ));
        };
        let dir = dir.clone();
        let Ok(_guard) = self.install_lock.try_lock() else {
            return Err(comfy_err("a ComfyUI install is already running"));
        };

        self.set_install_state(InstallState::Running {
            phase: InstallPhase::Downloading,
            done_bytes: 0,
            total_bytes: install::TOOLCHAIN_DOWNLOAD_BYTES,
        });
        let _ = self
            .db
            .runtimes()
            .set_state(RUNTIME_ID, RUNTIME_KIND, runtime_state::INSTALLING, None)
            .await;

        let result = install::install(
            &dir,
            offline,
            &install::SystemRunner,
            |phase, done_bytes, total_bytes| {
                self.set_install_state(InstallState::Running {
                    phase,
                    done_bytes,
                    total_bytes,
                });
            },
        )
        .await;

        match &result {
            Ok(()) => {
                self.set_install_state(InstallState::Idle);
                let _ = self
                    .db
                    .runtimes()
                    .record_install(
                        RUNTIME_ID,
                        RUNTIME_KIND,
                        install::PINNED_TAG,
                        &install::comfy_home(&dir).to_string_lossy(),
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
        result
    }

    fn server(&self) -> std::sync::MutexGuard<'_, Server> {
        self.server.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn resident(&self) -> std::sync::MutexGuard<'_, Option<LoadedModel>> {
        self.resident.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resolve the launch command now (may hit the filesystem).
    fn server_launch(&self) -> Option<ComfyLaunch> {
        match &self.launch {
            LaunchSource::Fixed(l) => l.clone(),
            LaunchSource::Scan(dir) => resolve_launch(dir, |k| std::env::var_os(k)),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.server_launch().is_some()
    }

    /// The port of the running server, if it is up. Locks briefly; safe to hold
    /// across `.await` afterwards (the guard is dropped here).
    fn up_port(&self) -> Option<u16> {
        match &*self.server() {
            Server::Up { port, .. } => Some(*port),
            _ => None,
        }
    }

    /// ComfyUI's reported version + VRAM, when the server is up. For Diagnostics.
    pub async fn system_stats(&self) -> Option<SystemStats> {
        let port = self.up_port()?;
        self.client.system_stats(port).await.ok()
    }

    /// Run one `workflow` (an API-format graph from [`crate::pipeline`]) on the
    /// running server: queue it, poll `GET /history` until it finishes, then
    /// fetch the output file (image or video). `Ok(None)` means `cancel` flipped
    /// mid-render — the workflow was interrupted. The server and its resident
    /// model stay up. `timeout` bounds one render (image: minutes; video: much
    /// longer).
    pub async fn generate_media(
        &self,
        workflow: &serde_json::Value,
        mut cancel: watch::Receiver<bool>,
        timeout: Duration,
    ) -> Result<Option<GeneratedMedia>> {
        let port = self
            .up_port()
            .ok_or_else(|| comfy_err("the ComfyUI server is not running"))?;
        let client_id = uuid::Uuid::now_v7().to_string();
        let prompt_id = self
            .client
            .submit_prompt(port, workflow, &client_id)
            .await?;

        let deadline = Instant::now() + timeout;
        loop {
            if *cancel.borrow_and_update() {
                let _ = self.client.interrupt(port).await;
                return Ok(None);
            }
            match self.client.history(port, &prompt_id).await? {
                PromptOutcome::Pending => {}
                PromptOutcome::Failed(msg) => return Err(comfy_err(msg)),
                PromptOutcome::Done(files) => {
                    let file = files
                        .into_iter()
                        .next()
                        .ok_or_else(|| comfy_err("ComfyUI finished but produced no output"))?;
                    let extension = file
                        .filename
                        .rsplit_once('.')
                        .map_or_else(|| "png".to_string(), |(_, ext)| ext.to_ascii_lowercase());
                    let bytes = self.client.view(port, &file).await?;
                    return Ok(Some(GeneratedMedia { bytes, extension }));
                }
            }
            if Instant::now() >= deadline {
                let _ = self.client.interrupt(port).await;
                return Err(comfy_err(format!(
                    "the render did not finish within {}s",
                    timeout.as_secs()
                )));
            }
            tokio::time::sleep(MEDIA_POLL_INTERVAL).await;
        }
    }

    /// Start the supervised process if it is down and wait for `/system_stats`.
    /// Assumes `op_lock` is held. Returns the live port.
    async fn ensure_server_locked(&self) -> Result<u16> {
        if let Some(port) = self.up_port() {
            return Ok(port);
        }

        let launch = self
            .server_launch()
            .ok_or_else(|| comfy_err("ComfyUI is not installed — run ComfyUI setup first"))?;
        self.dirs.ensure()?;
        let port = free_loopback_port()?;
        let spec = build_spawn_spec(&launch, port, &self.dirs, &self.opts);

        let supervisor = RuntimeSupervisor::start(RUNTIME_ID, spec)?;
        *self.server() = Server::Starting;

        if let Err(e) = self.await_healthy(port, &supervisor).await {
            *self.server() = Server::Down; // `supervisor` drops → process killed
            return Err(e);
        }

        tracing::info!(port, "ComfyUI is up");
        *self.server() = Server::Up {
            port,
            supervisor: Some(supervisor),
        };
        Ok(port)
    }

    async fn await_healthy(&self, port: u16, supervisor: &RuntimeSupervisor) -> Result<()> {
        let deadline = Instant::now() + SERVER_READY_TIMEOUT;
        loop {
            if supervisor.state() == SupervisorState::GaveUp {
                return Err(comfy_err(
                    "ComfyUI keeps crashing on startup — check the Python env, GPU driver and free VRAM",
                ));
            }
            if self.client.health(port).await == Health::Healthy {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(comfy_err(format!(
                    "ComfyUI did not become ready within {}s",
                    SERVER_READY_TIMEOUT.as_secs()
                )));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    /// Adopt a ComfyUI the user already started on `port`. Probes
    /// `/system_stats`; on success it becomes resident but its process is left
    /// alone by [`stop`](Self::stop).
    pub async fn attach(&self, port: u16) -> Result<()> {
        let _op = self.op_lock.lock().await;
        if self.client.health(port).await != Health::Healthy {
            return Err(comfy_err(format!(
                "no healthy ComfyUI found on 127.0.0.1:{port}"
            )));
        }
        tracing::info!(port, "attached to a user-managed ComfyUI");
        *self.server() = Server::Up {
            port,
            supervisor: None,
        };
        Ok(())
    }

    /// Stop the server we manage (a runtime restart / app teardown). Attached
    /// servers are just forgotten. Drop does the same, so this is only needed for
    /// an explicit restart.
    pub async fn stop(&self) -> Result<()> {
        let _op = self.op_lock.lock().await;
        let taken = std::mem::take(&mut *self.server());
        *self.resident() = None;
        if let Server::Up {
            supervisor: Some(mut sup),
            ..
        } = taken
        {
            tracing::info!("stopping ComfyUI");
            sup.stop().await?;
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeAdapter for ComfyUiAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ComfyUi
    }

    fn spawn_spec(&self) -> Option<SpawnSpec> {
        let launch = self.server_launch()?;
        Some(build_spawn_spec(&launch, 0, &self.dirs, &self.opts))
    }

    async fn health(&self) -> Health {
        let port = {
            match &*self.server() {
                Server::Down => return Health::Unknown,
                Server::Starting => return Health::Starting,
                Server::Up { port, .. } => *port,
            }
        };
        // `/system_stats` doubles as the health probe (ComfyUI has no /health);
        // cache the payload for `detail()`.
        match self.client.system_stats(port).await {
            Ok(s) => {
                *self.stats() = Some(s);
                Health::Healthy
            }
            Err(_) => {
                *self.stats() = None;
                Health::Unhealthy
            }
        }
    }

    /// Ensure the server is up and reserve the VRAM slot for `model_id`. Frees
    /// the previously-resident model first (ADR-003 single slot). Does **not**
    /// load the checkpoint — ComfyUI does that when the workflow runs.
    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()> {
        let _op = self.op_lock.lock().await;
        let port = self.ensure_server_locked().await?;

        let swap = matches!(&*self.resident(), Some(m) if m.model_id != model_id);
        if swap {
            tracing::info!(port, "freeing ComfyUI VRAM before the next model");
            let _ = self.client.free(port).await;
        }
        *self.resident() = Some(LoadedModel {
            model_id: model_id.to_string(),
            vram_mb,
        });
        Ok(())
    }

    /// Drop the resident model via `POST /free`. The server itself stays up.
    async fn unload_model(&self, model_id: &str) -> Result<()> {
        let _op = self.op_lock.lock().await;
        let mine = matches!(&*self.resident(), Some(m) if m.model_id == model_id);
        if !mine {
            return Ok(());
        }
        if let Some(port) = self.up_port() {
            let _ = self.client.free(port).await;
        }
        *self.resident() = None;
        Ok(())
    }

    fn loaded_models(&self) -> Vec<LoadedModel> {
        self.resident().clone().into_iter().collect()
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
                return Some(format!("downloading ComfyUI — {pct}%"));
            }
            InstallState::Running { phase, .. } => {
                let step = match phase {
                    InstallPhase::Downloading => unreachable!(),
                    InstallPhase::Extracting => "unpacking ComfyUI",
                    InstallPhase::CreatingVenv => "creating the Python environment",
                    InstallPhase::InstallingTorch => "installing PyTorch (this is a big download)",
                    InstallPhase::InstallingDeps => "installing ComfyUI dependencies",
                    InstallPhase::InstallingNode => "installing the GGUF node",
                };
                return Some(format!("{step}…"));
            }
            InstallState::Failed { error } => return Some(format!("setup failed: {error}")),
            InstallState::Idle => {}
        }

        Some(match &*self.server() {
            Server::Down if !self.is_installed() => "not installed".to_string(),
            Server::Down => "installed · idle".to_string(),
            Server::Starting => "starting…".to_string(),
            Server::Up { port, supervisor } => {
                let verb = if supervisor.is_some() {
                    "running on"
                } else {
                    "attached to"
                };
                let mut line = format!("{verb} :{port}");
                if let Some(v) = self.stats().as_ref().and_then(|s| s.version.as_deref()) {
                    line.push_str(&format!(" · ComfyUI {v}"));
                }
                if self.opts.vram_mode != VramMode::Auto {
                    line.push_str(&format!(" · {}", self.opts.vram_mode.as_str()));
                }
                if self.resident().is_some() {
                    line.push_str(" · model reserved");
                }
                line
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;

    fn dirs(tmp: &Path) -> ComfyDirs {
        ComfyDirs {
            base: tmp.join("comfyui-data"),
            output: tmp.join("outputs"),
            models_store: tmp.join("store"),
        }
    }

    async fn mock_comfy() -> u16 {
        let router = Router::new()
            .route(
                "/system_stats",
                get(|| async {
                    Json(serde_json::json!({
                        "system": { "comfyui_version": "0.34.0" },
                        "devices": [{ "name": "cuda:0 test", "vram_total": 17_000_000_000u64, "vram_free": 15_000_000_000u64 }]
                    }))
                }),
            )
            .route("/free", post(|| async { axum::http::StatusCode::OK }))
            .route("/interrupt", post(|| async { axum::http::StatusCode::OK }));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    async fn adapter(launch: Option<ComfyLaunch>) -> (ComfyUiAdapter, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let a = ComfyUiAdapter::with_launch(db, launch, dirs(tmp.path()));
        (a, tmp)
    }

    #[tokio::test]
    async fn reports_not_installed_without_a_launch() {
        let (a, _tmp) = adapter(None).await;
        assert!(!a.is_installed());
        assert_eq!(a.detail().as_deref(), Some("not installed"));
        assert_eq!(a.health().await, Health::Unknown);

        let err = a.load_model("m", 6_000).await.unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }

    #[tokio::test]
    async fn attach_adopts_a_running_server_and_tracks_a_model() {
        let port = mock_comfy().await;
        let (a, _tmp) = adapter(None).await;

        a.attach(port).await.unwrap();
        assert_eq!(a.health().await, Health::Healthy);
        // health() cached /system_stats, so detail() now names the version.
        let d = a.detail().unwrap();
        assert!(
            d.starts_with(&format!("attached to :{port}")) && d.contains("ComfyUI 0.34.0"),
            "{d}"
        );

        // load_model on an attached server just reserves the slot.
        a.load_model("sdxl", 7_000).await.unwrap();
        assert_eq!(a.loaded_models().len(), 1);
        assert_eq!(a.vram_used_mb(), 7_000);
        assert!(a.detail().unwrap().ends_with("· model reserved"));

        // A second model frees the first, still one resident.
        a.load_model("flux", 13_000).await.unwrap();
        assert_eq!(a.loaded_models()[0].model_id, "flux");
        assert_eq!(a.vram_used_mb(), 13_000);

        a.unload_model("flux").await.unwrap();
        assert!(a.loaded_models().is_empty());
        // Unload does not stop an attached server.
        assert_eq!(a.health().await, Health::Healthy);
    }

    #[tokio::test]
    async fn attach_fails_without_a_healthy_server() {
        let (a, _tmp) = adapter(None).await;
        let err = a.attach(59_999).await.unwrap_err();
        assert!(err.to_string().contains("no healthy ComfyUI"));
    }

    #[tokio::test]
    async fn unload_of_a_model_we_do_not_hold_is_a_noop() {
        let port = mock_comfy().await;
        let (a, _tmp) = adapter(None).await;
        a.attach(port).await.unwrap();
        a.load_model("sdxl", 7_000).await.unwrap();

        a.unload_model("something-else").await.unwrap();
        assert_eq!(a.loaded_models()[0].model_id, "sdxl");
    }
}
