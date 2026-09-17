//! The trainer runtime adapter: the pinned `ai-toolkit` install, an import
//! probe for its venv, and the GPU reservation a live training run holds.
//!
//! Unlike every other adapter here, this one owns **no process**: a training
//! run is launched detached (`launcher::spawn::launch_detached_quiet`) and
//! outlives the app, so [`spawn_spec`](RuntimeAdapter::spawn_spec) is `None`
//! and the supervisor never touches it. What it does own is the *reservation*:
//! while a run is alive the adapter reports exactly **one synthetic loaded
//! model**, [`TRAINING_MODEL_ID`], charged with the run's VRAM estimate, so the
//! scheduler sees the GPU as occupied and queues image/video jobs behind it.
//! Task 7 pins that id, which also makes it immune to eviction — the run is a
//! detached process the scheduler could not unload even if it wanted to.
//!
//! The install pipeline itself lives in [`install`]; this module wraps it with
//! the same `InstallState` shape the ComfyUI adapter exposes, so the Settings
//! install card can render both without a second code path.

pub(crate) mod install;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;

pub use self::install::TrainerInstallPhase;
use super::download::{CmdRunner, SystemRunner};
use super::{Health, LoadedModel, RuntimeAdapter, RuntimeKind, SpawnSpec};
use crate::training::{training_err, TRAINING_MODEL_ID};
use crate::Result;

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "training";

/// Progress of [`TrainingAdapter::install`], mirrored to the UI status line.
/// Same serialised shape as `runtime::comfyui::InstallState`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallState {
    Idle,
    Running {
        phase: TrainerInstallPhase,
        done_bytes: u64,
        total_bytes: u64,
    },
    Failed {
        error: String,
    },
}

/// What the trainer venv reports about its own PyTorch — see
/// [`TrainingAdapter::probe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Probe {
    pub torch_version: String,
    pub cuda: bool,
    pub vram_total_mb: u64,
}

/// The run currently holding the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AliveRun {
    run_id: String,
    vram_mb: u64,
}

/// What the probe script prints — kept private, [`Probe`] is the public shape.
#[derive(serde::Deserialize)]
struct ProbeJson {
    torch: String,
    cuda: bool,
    vram: u64,
}

/// How long [`TrainingAdapter::probe`] waits for the venv's `python -c` to
/// answer. Far longer than the 3–5 s the llama.cpp / ComfyUI adapters allow
/// their HTTP probes: this one starts a Python interpreter and imports torch,
/// which on a cold disk is several seconds before it prints anything.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// One line, so the exact string the probe runs is visible at a glance.
const PROBE_SCRIPT: &str = "import torch,json;print(json.dumps({'torch':torch.__version__,'cuda':torch.cuda.is_available(),'vram':torch.cuda.get_device_properties(0).total_memory if torch.cuda.is_available() else 0}))";

pub struct TrainingAdapter {
    runtimes_dir: PathBuf,
    install: Mutex<InstallState>,
    /// Held for the duration of an install.
    install_lock: AsyncMutex<()>,
    /// Set when the last [`probe`](TrainingAdapter::probe) failed: the venv is
    /// there but unusable (a half-finished install, a broken driver update).
    env_broken: AtomicBool,
    alive: Mutex<Option<AliveRun>>,
    /// The subprocess boundary — swapped in tests.
    runner: Arc<dyn CmdRunner>,
    /// [`PROBE_TIMEOUT`] in production; tests shorten it.
    probe_timeout: Duration,
}

impl std::fmt::Debug for TrainingAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrainingAdapter")
            .field("runtimes_dir", &self.runtimes_dir)
            .field("installed", &self.is_installed())
            .field("env_broken", &self.env_broken())
            .field("alive_run", &self.alive_run())
            .finish()
    }
}

impl TrainingAdapter {
    /// Re-scans the managed install dir on demand — a fresh install needs no
    /// restart. Registers even when nothing is found.
    pub fn discover(runtimes_dir: &Path) -> Self {
        Self::with_runner(runtimes_dir, Arc::new(SystemRunner))
    }

    pub(crate) fn with_runner(runtimes_dir: &Path, runner: Arc<dyn CmdRunner>) -> Self {
        Self {
            runtimes_dir: runtimes_dir.to_path_buf(),
            install: Mutex::new(InstallState::Idle),
            install_lock: AsyncMutex::new(()),
            env_broken: AtomicBool::new(false),
            alive: Mutex::new(None),
            runner,
            probe_timeout: PROBE_TIMEOUT,
        }
    }

    /// Shorten the probe's patience — tests only; production waits
    /// [`PROBE_TIMEOUT`].
    #[must_use]
    pub fn with_probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    /// Whether the pinned trainer is completely installed.
    pub fn is_installed(&self) -> bool {
        install::is_installed(&self.runtimes_dir)
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

    /// Install the pinned `ai-toolkit`: bootstrap `uv`, fetch the source at
    /// [`install::PINNED_COMMIT`], then build a self-contained venv (Python
    /// 3.12 + the pinned CUDA torch + `requirements.txt`). Long-running —
    /// callers spawn it and poll [`install_state`](Self::install_state).
    /// A successful install also clears [`env_broken`](Self::env_broken): the
    /// environment that failed the last probe has just been rebuilt.
    pub async fn install(&self, offline: bool) -> Result<()> {
        let Ok(_guard) = self.install_lock.try_lock() else {
            return Err(training_err("a trainer install is already running"));
        };
        self.set_install_state(InstallState::Running {
            phase: TrainerInstallPhase::Downloading,
            done_bytes: 0,
            total_bytes: install::TOOLCHAIN_DOWNLOAD_BYTES,
        });

        let result = install::install(
            &self.runtimes_dir,
            offline,
            self.runner.as_ref(),
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
                self.clear_env_broken();
            }
            Err(e) => self.set_install_state(InstallState::Failed {
                error: e.to_string(),
            }),
        }
        result
    }

    /// Ask the trainer venv what PyTorch it has and whether CUDA is live —
    /// the one check that distinguishes "the files are there" from "a run
    /// would actually start". A failure sets [`env_broken`](Self::env_broken)
    /// so the UI can offer a repair instead of launching a doomed run.
    pub async fn probe(&self) -> Result<Probe> {
        if !self.is_installed() {
            return Err(training_err(
                "the trainer is not installed yet — set it up first",
            ));
        }
        let python = self.python_bin();
        // A hung import (a broken CUDA driver is the usual cause) must not
        // hang the caller: past the timeout the environment counts as broken.
        let probe = self.runner.run_capture(&python, &["-c", PROBE_SCRIPT], &[]);
        let Ok(captured) = tokio::time::timeout(self.probe_timeout, probe).await else {
            self.env_broken.store(true, Ordering::Relaxed);
            return Err(training_err(format!(
                "trainer environment probe timed out after {} s — set it up again",
                self.probe_timeout.as_secs()
            )));
        };
        let raw = match captured {
            Ok(raw) => raw,
            Err(e) => {
                self.env_broken.store(true, Ordering::Relaxed);
                return Err(training_err(format!(
                    "trainer environment is broken — set it up again: {e}"
                )));
            }
        };
        let parsed: ProbeJson = serde_json::from_str(raw.trim()).map_err(|e| {
            self.env_broken.store(true, Ordering::Relaxed);
            training_err(format!(
                "trainer environment is broken — set it up again: its probe printed \
                 something unreadable ({e}): {}",
                raw.trim()
            ))
        })?;
        self.clear_env_broken();
        Ok(Probe {
            torch_version: parsed.torch,
            cuda: parsed.cuda,
            vram_total_mb: parsed.vram / (1024 * 1024),
        })
    }

    /// Whether the last probe found the environment unusable.
    pub fn env_broken(&self) -> bool {
        self.env_broken.load(Ordering::Relaxed)
    }

    /// Clear the flag — after a repair install, or when the user retries.
    pub fn clear_env_broken(&self) {
        self.env_broken.store(false, Ordering::Relaxed);
    }

    /// The venv interpreter a run is launched with.
    pub fn python_bin(&self) -> PathBuf {
        install::venv_python(&self.runtimes_dir)
    }

    /// `ai-toolkit`'s headless entry point.
    pub fn run_py(&self) -> PathBuf {
        install::run_py(&self.runtimes_dir)
    }

    /// The `ai-toolkit` checkout (the run's working directory).
    pub fn source_dir(&self) -> PathBuf {
        install::source_dir(&self.runtimes_dir)
    }

    /// Record that `run_id` now holds the GPU, charging `vram_mb`. The runner
    /// calls this right after the scheduler's
    /// [`load_model`](RuntimeAdapter::load_model), which can only name the
    /// synthetic [`TRAINING_MODEL_ID`] — this is what puts the *real* run id
    /// on the reservation. The reservation itself still reports
    /// [`TRAINING_MODEL_ID`] to the scheduler either way.
    pub fn mark_alive(&self, run_id: &str, vram_mb: u64) {
        *self.alive() = Some(AliveRun {
            run_id: run_id.to_string(),
            vram_mb,
        });
    }

    /// Release the reservation.
    pub fn clear_alive(&self) {
        *self.alive() = None;
    }

    /// The id of the run currently holding the GPU.
    pub fn alive_run(&self) -> Option<String> {
        self.alive().as_ref().map(|r| r.run_id.clone())
    }

    fn alive(&self) -> std::sync::MutexGuard<'_, Option<AliveRun>> {
        self.alive.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RuntimeAdapter for TrainingAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Training
    }

    /// Nothing to supervise: a run is a detached process that outlives the app.
    fn spawn_spec(&self) -> Option<SpawnSpec> {
        None
    }

    /// `Unknown` until the trainer is installed, then `Healthy` unless the
    /// last [`probe`](TrainingAdapter::probe) found the venv unusable. There
    /// is no process to ping — see the module docs.
    async fn health(&self) -> Health {
        match (self.is_installed(), self.env_broken()) {
            (false, _) => Health::Unknown,
            (true, true) => Health::Unhealthy,
            (true, false) => Health::Healthy,
        }
    }

    /// Takes the GPU reservation for a training run. Only
    /// [`TRAINING_MODEL_ID`] is accepted: this adapter loads no real model,
    /// it only stands in for the detached trainer process while it runs.
    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()> {
        if model_id != TRAINING_MODEL_ID {
            return Err(training_err(format!(
                "the trainer runtime only reserves {TRAINING_MODEL_ID}, not {model_id}"
            )));
        }
        self.mark_alive(model_id, vram_mb);
        Ok(())
    }

    /// Releases the reservation. Accepts both the synthetic id the scheduler
    /// knows and the real run id [`mark_alive`](TrainingAdapter::mark_alive)
    /// later put on it; anything else is a no-op, like every other adapter.
    async fn unload_model(&self, model_id: &str) -> Result<()> {
        let mut alive = self.alive();
        let matches = alive
            .as_ref()
            .is_some_and(|r| model_id == TRAINING_MODEL_ID || r.run_id == model_id);
        if matches {
            *alive = None;
        }
        Ok(())
    }

    /// One synthetic model while a run is alive — the GPU hold the scheduler
    /// queues image/video work behind (and pins, so it is never evicted: the
    /// run is a detached process an unload could not actually stop).
    fn loaded_models(&self) -> Vec<LoadedModel> {
        self.alive()
            .as_ref()
            .map(|r| {
                vec![LoadedModel {
                    model_id: TRAINING_MODEL_ID.to_string(),
                    vram_mb: r.vram_mb,
                }]
            })
            .unwrap_or_default()
    }

    /// The one-line status the Settings card shows — same phrasing shape as
    /// the ComfyUI adapter's, so the two install cards read alike.
    fn detail(&self) -> Option<String> {
        Some(match self.install_state() {
            InstallState::Running {
                phase: TrainerInstallPhase::Downloading,
                done_bytes,
                total_bytes,
            } => {
                let pct = done_bytes
                    .saturating_mul(100)
                    .checked_div(total_bytes)
                    .unwrap_or(0);
                format!("downloading ai-toolkit — {pct}%")
            }
            InstallState::Running { phase, .. } => {
                let step = match phase {
                    // Handled above, with its percentage; kept exhaustive
                    // rather than `unreachable!()` — a panic in a status line
                    // would be a poor trade for one match arm.
                    TrainerInstallPhase::Downloading => "downloading ai-toolkit".to_string(),
                    TrainerInstallPhase::Extracting => "unpacking ai-toolkit".to_string(),
                    TrainerInstallPhase::InstallingPython => {
                        format!("installing Python {}", install::PYTHON_VERSION)
                    }
                    TrainerInstallPhase::CreatingVenv => {
                        "creating the Python environment".to_string()
                    }
                    TrainerInstallPhase::InstallingTorch => {
                        "installing PyTorch (large download)".to_string()
                    }
                    TrainerInstallPhase::InstallingDeps => {
                        "installing trainer dependencies".to_string()
                    }
                };
                format!("{step}…")
            }
            InstallState::Failed { error } => format!("setup failed: {error}"),
            InstallState::Idle if !self.is_installed() => "not installed".to_string(),
            InstallState::Idle if self.env_broken() => {
                "the trainer environment is broken — set it up again".to_string()
            }
            InstallState::Idle => match self.alive_run() {
                Some(run_id) => format!("training run {run_id} is holding the GPU"),
                None => "idle".to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    /// Returns canned stdout for the probe (or a canned failure), recording
    /// what it was asked to run.
    struct ScriptedRunner {
        stdout: String,
        failure: Option<String>,
        calls: StdMutex<Vec<Vec<String>>>,
    }

    impl ScriptedRunner {
        fn ok(stdout: &str) -> Arc<Self> {
            Arc::new(Self {
                stdout: stdout.to_string(),
                failure: None,
                calls: StdMutex::new(Vec::new()),
            })
        }

        fn failing(message: &str) -> Arc<Self> {
            Arc::new(Self {
                stdout: String::new(),
                failure: Some(message.to_string()),
                calls: StdMutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl CmdRunner for ScriptedRunner {
        async fn run(&self, _p: &Path, _args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            Ok(())
        }

        async fn run_capture(
            &self,
            program: &Path,
            args: &[&str],
            _e: &[(&str, &str)],
        ) -> Result<String> {
            let mut call = vec![program.to_string_lossy().into_owned()];
            call.extend(args.iter().map(|s| s.to_string()));
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
            match &self.failure {
                Some(msg) => Err(crate::CoreError::Config(msg.clone())),
                None => Ok(self.stdout.clone()),
            }
        }
    }

    /// A complete-looking install so the probe/health paths get past their
    /// "is it installed?" guard without a real download.
    fn fake_install(runtimes_dir: &Path) {
        let py = install::venv_python(runtimes_dir);
        std::fs::create_dir_all(py.parent().expect("venv python has a parent"))
            .expect("create the venv dir");
        std::fs::write(&py, b"py").expect("write the venv python");
        std::fs::create_dir_all(install::source_dir(runtimes_dir)).expect("create the source dir");
        std::fs::write(install::run_py(runtimes_dir), b"# ai-toolkit").expect("write run.py");
        std::fs::write(install::marker_file(runtimes_dir), install::PINNED_COMMIT)
            .expect("write the marker");
    }

    #[tokio::test]
    async fn probe_parses_torch_json_and_clears_env_broken() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fake_install(tmp.path());
        let runner = ScriptedRunner::ok(
            r#"{"torch": "2.13.0+cu130", "cuda": true, "vram": 17179869184}
"#,
        );
        let adapter = TrainingAdapter::with_runner(tmp.path(), runner.clone());
        // A previous failure must not stick once the probe succeeds again.
        adapter.env_broken.store(true, Ordering::Relaxed);

        let probe = adapter.probe().await.expect("the probe should parse");

        assert_eq!(
            probe,
            Probe {
                torch_version: "2.13.0+cu130".to_string(),
                cuda: true,
                vram_total_mb: 16_384,
            }
        );
        assert!(!adapter.env_broken(), "a good probe clears the flag");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "one probe call: {calls:?}");
        assert_eq!(calls[0][0], adapter.python_bin().to_string_lossy());
        assert_eq!(calls[0][1], "-c");
        assert!(calls[0][2].contains("import torch"), "{:?}", calls[0][2]);
    }

    #[tokio::test]
    async fn probe_failure_marks_env_broken_with_the_stderr_tail() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fake_install(tmp.path());
        let runner = ScriptedRunner::failing(
            "python -c exited with 1:\nModuleNotFoundError: No module named 'torch'",
        );
        let adapter = TrainingAdapter::with_runner(tmp.path(), runner);

        let err = adapter
            .probe()
            .await
            .expect_err("a broken venv must surface");

        let msg = err.to_string();
        assert!(
            msg.contains("trainer environment is broken"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("No module named 'torch'"),
            "the stderr tail must survive: {msg}"
        );
        assert!(adapter.env_broken());

        adapter.clear_env_broken();
        assert!(!adapter.env_broken());
    }

    #[tokio::test]
    async fn probe_refuses_before_the_trainer_is_installed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));

        let err = adapter
            .probe()
            .await
            .expect_err("nothing to probe without an install");

        assert!(
            err.to_string().contains("not installed"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn adapter_reports_one_pinned_loaded_model_while_a_run_is_alive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));

        assert_eq!(adapter.id(), "training");
        assert_eq!(adapter.kind(), RuntimeKind::Training);
        assert!(adapter.spawn_spec().is_none());
        assert!(adapter.loaded_models().is_empty());
        assert_eq!(adapter.vram_used_mb(), 0);
        assert_eq!(adapter.alive_run(), None);

        adapter
            .load_model(TRAINING_MODEL_ID, 14_000)
            .await
            .expect("the scheduler's reservation must be accepted");

        assert_eq!(
            adapter.loaded_models(),
            vec![LoadedModel {
                model_id: TRAINING_MODEL_ID.to_string(),
                vram_mb: 14_000,
            }]
        );
        assert_eq!(adapter.vram_used_mb(), 14_000);

        // The runner names the real run right after the scheduler's load: the
        // reservation still reports the synthetic id, only `alive_run` changes.
        adapter.mark_alive("run-42", 14_000);
        assert_eq!(adapter.alive_run().as_deref(), Some("run-42"));
        assert_eq!(
            adapter.loaded_models(),
            vec![LoadedModel {
                model_id: TRAINING_MODEL_ID.to_string(),
                vram_mb: 14_000,
            }]
        );

        adapter
            .unload_model(TRAINING_MODEL_ID)
            .await
            .expect("unload releases the reservation");
        assert!(adapter.loaded_models().is_empty());
        assert_eq!(adapter.alive_run(), None);

        // Unloading again is a harmless no-op, like every other adapter.
        adapter
            .unload_model(TRAINING_MODEL_ID)
            .await
            .expect("a second unload is a no-op");

        // Anything that is not the reservation id is refused rather than
        // silently pretending a model loaded.
        assert!(adapter.load_model("flux2-klein", 14_000).await.is_err());
        assert!(adapter.loaded_models().is_empty());
    }

    #[tokio::test]
    async fn health_reflects_install_and_env_state() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));
        assert_eq!(adapter.health().await, Health::Unknown, "not installed yet");
        assert!(!adapter.is_installing());
        assert!(matches!(adapter.install_state(), InstallState::Idle));

        fake_install(tmp.path());
        assert!(adapter.is_installed());
        assert_eq!(adapter.health().await, Health::Healthy);

        adapter.env_broken.store(true, Ordering::Relaxed);
        assert_eq!(adapter.health().await, Health::Unhealthy);
    }

    #[test]
    fn runtime_kind_training_round_trips_through_serde() {
        let json = serde_json::to_string(&RuntimeKind::Training).expect("serialise");
        assert_eq!(json, "\"training\"");
        let back: RuntimeKind = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, RuntimeKind::Training);
    }

    #[tokio::test]
    async fn install_refuses_offline_when_nothing_is_installed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));

        let err = adapter
            .install(true)
            .await
            .expect_err("offline mode must refuse");

        assert!(
            err.to_string().contains("offline mode"),
            "unexpected error: {err}"
        );
        assert!(
            matches!(adapter.install_state(), InstallState::Failed { .. }),
            "a refused install is a failed install, not idle"
        );
    }

    /// A runner whose probe never answers — a hung `python -c` (an import
    /// deadlocked on a broken CUDA driver is the real-world shape of this).
    struct HangingRunner;

    #[async_trait::async_trait]
    impl CmdRunner for HangingRunner {
        async fn run(&self, _p: &Path, _args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            Ok(())
        }

        async fn run_capture(
            &self,
            _p: &Path,
            _args: &[&str],
            _e: &[(&str, &str)],
        ) -> Result<String> {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok("{}".to_string())
        }
    }

    #[tokio::test]
    async fn probe_times_out_and_marks_env_broken() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fake_install(tmp.path());
        let adapter = TrainingAdapter::with_runner(tmp.path(), Arc::new(HangingRunner))
            .with_probe_timeout(std::time::Duration::from_millis(30));

        let err = adapter
            .probe()
            .await
            .expect_err("a hung probe must not hang the caller");

        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
        assert!(adapter.env_broken(), "a timeout is a broken environment");
    }

    #[test]
    fn the_default_probe_timeout_leaves_room_for_a_cold_torch_import() {
        assert_eq!(PROBE_TIMEOUT, std::time::Duration::from_secs(20));
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));
        assert_eq!(adapter.probe_timeout, PROBE_TIMEOUT);
    }

    #[test]
    fn detail_reports_friendly_install_phases() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = TrainingAdapter::with_runner(tmp.path(), ScriptedRunner::ok("{}"));
        assert_eq!(adapter.detail().as_deref(), Some("not installed"));

        adapter.set_install_state(InstallState::Running {
            phase: TrainerInstallPhase::Downloading,
            done_bytes: 42,
            total_bytes: 100,
        });
        assert_eq!(
            adapter.detail().as_deref(),
            Some("downloading ai-toolkit — 42%")
        );

        // A zero total (nothing known yet) must not divide by zero.
        adapter.set_install_state(InstallState::Running {
            phase: TrainerInstallPhase::Downloading,
            done_bytes: 0,
            total_bytes: 0,
        });
        assert_eq!(
            adapter.detail().as_deref(),
            Some("downloading ai-toolkit — 0%")
        );

        for (phase, expected) in [
            (TrainerInstallPhase::Extracting, "unpacking ai-toolkit…"),
            (
                TrainerInstallPhase::InstallingPython,
                "installing Python 3.12…",
            ),
            (
                TrainerInstallPhase::CreatingVenv,
                "creating the Python environment…",
            ),
            (
                TrainerInstallPhase::InstallingTorch,
                "installing PyTorch (large download)…",
            ),
            (
                TrainerInstallPhase::InstallingDeps,
                "installing trainer dependencies…",
            ),
        ] {
            adapter.set_install_state(InstallState::Running {
                phase,
                done_bytes: 0,
                total_bytes: 0,
            });
            assert_eq!(adapter.detail().as_deref(), Some(expected), "{phase:?}");
        }

        adapter.set_install_state(InstallState::Failed {
            error: "boom".to_string(),
        });
        assert_eq!(adapter.detail().as_deref(), Some("setup failed: boom"));

        adapter.set_install_state(InstallState::Idle);
        fake_install(tmp.path());
        assert_eq!(adapter.detail().as_deref(), Some("idle"));

        adapter.mark_alive("run-7", 14_000);
        assert_eq!(
            adapter.detail().as_deref(),
            Some("training run run-7 is holding the GPU")
        );

        adapter.clear_alive();
        adapter.env_broken.store(true, Ordering::Relaxed);
        assert_eq!(
            adapter.detail().as_deref(),
            Some("the trainer environment is broken — set it up again")
        );
    }
}
