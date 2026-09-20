//! Application bootstrap: tie together paths, config, database, telemetry,
//! runtimes, scheduler and the job engine into a live [`App`] handle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing_appender::non_blocking::WorkerGuard;

use crate::agent::{HermesAgentAdapter, OpenCodeAdapter};
use crate::capability::agent::{AgentSessions, LlamaCodingRuntime};
use crate::config::{Config, FALLBACK_VRAM_BUDGET_MB};
use crate::db::{now_rfc3339, Database};
use crate::download::DownloadManager;
use crate::launcher::{AdapterBinaries, Launcher};
use crate::orchestrator::JobEngine;
use crate::paths::AppPaths;
use crate::progress::ProgressHub;
use crate::registry::{CivitaiSource, HuggingFaceSource, Registry};
use crate::runtime::{
    ColibriAdapter, ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry, TrainingAdapter,
    TtsAdapter, VisionAdapter,
};
use crate::scheduler::HybridScheduler;
use crate::telemetry::{GpuStatus, Sampler};
use crate::training::runner::{spawn_poller, Runner as TrainingRunner};
use crate::Result;

/// Settings seeded on first run. `config.toml` remains the source of truth for
/// startup configuration; these are app-managed markers.
const SCHEMA_VERSION: &str = "1";

/// A bootstrapped core. Held behind an `Arc` by the host process and shared with
/// the API server.
#[derive(Debug)]
pub struct App {
    pub paths: AppPaths,
    /// Startup configuration snapshot. Fields the Settings UI can change are
    /// written straight to `config.toml` (a restart re-reads them); the live
    /// exception is offline mode — [`offline`](Self::offline).
    pub config: Config,
    pub db: Database,
    pub telemetry: Arc<Sampler>,
    pub runtimes: RuntimeRegistry,
    /// The llama.cpp and ComfyUI adapters, also registered in
    /// [`runtimes`](Self::runtimes). Held typed so handlers can drive their
    /// installers.
    pub llama: Arc<LlamaCppAdapter>,
    pub comfyui: Arc<ComfyUiAdapter>,
    /// CPU-only Colibri runtime — optional in spirit (RAM-hungry, only one
    /// curated model fits this project's hardware ceiling), but always
    /// constructed and registered like every other runtime so it shows up in
    /// `GET /runtimes` and the Settings install card the same way.
    pub colibri: Arc<ColibriAdapter>,
    /// The narrator's local text-to-speech runtime (Story Studio, WP-9) —
    /// always constructed and registered the same way `colibri` is; resolving
    /// `uv` and spawning the sidecar is deferred to first real use.
    pub tts: Arc<TtsAdapter>,
    /// Live per-job render progress (`ComfyUiAdapter::generate_media` ->
    /// ComfyUI's own `/ws`), shared with `comfyui` so `api::http`'s
    /// `GET /ws/jobs/{id}` route reads the same readings it publishes.
    pub progress: Arc<ProgressHub>,
    /// The dataset-prep captioning pipeline's runtime (Florence-2/Qwen2.5-VL)
    /// — same lazy-sidecar shape as `tts`, but tracks real (non-zero) VRAM.
    pub vision: Arc<VisionAdapter>,
    /// The `ai-toolkit` LoRA trainer. Registered like every other runtime so
    /// it shows up in `GET /runtimes` and the Settings install card, but it
    /// supervises no process: a training run is detached and reports its GPU
    /// hold as one synthetic loaded model (`training::TRAINING_MODEL_ID`).
    pub training: Arc<TrainingAdapter>,
    /// Drives training runs: preflight, the detached launch, the 3 s poller,
    /// pause/resume/cancel and the import of the finished LoRA. Separate from
    /// [`jobs`](Self::jobs) on purpose — a run outlives the app (see
    /// `crate::training`).
    pub training_runner: Arc<TrainingRunner>,
    pub scheduler: Arc<HybridScheduler>,
    pub jobs: Arc<JobEngine>,
    /// Long-running agent sessions (Phase 5.1c) — its own subsystem, not a job.
    pub agents: Arc<AgentSessions>,
    /// The agent adapters, also registered on [`agents`](Self::agents). Held
    /// typed so handlers can report install state and drive Hermes' installer.
    pub opencode: Arc<OpenCodeAdapter>,
    pub hermes: Arc<HermesAgentAdapter>,
    /// Opens a real, independent terminal running OpenCode/Hermes against a
    /// pinned local model — deliberately NOT Job-Object-supervised, unlike
    /// everything else in this list (see `launcher` module docs).
    pub launcher: Arc<Launcher>,
    /// Online model discovery (Phase 6.1). Wraps the Hugging Face source with a
    /// disposable cache and the same `offline` switch. `Arc` so the job engine
    /// (upgrade check, 6.7) shares it.
    pub registry: Arc<Registry>,
    /// The Civitai source (image/video checkpoints + LoRAs) — its own
    /// `Registry` (own disposable cache subdirectory), the same `offline`
    /// switch. Not shared with the job engine: the upgrade-check (6.7) is a
    /// Hugging-Face-only concept (it walks `base_model:` quant/fine-tune
    /// chains, which Civitai has no equivalent of).
    pub civitai_registry: Arc<Registry>,
    /// The model download queue (Phase 6.4). Its worker is spawned by
    /// [`crate::api::spawn`].
    pub downloads: Arc<DownloadManager>,
    /// Live offline switch (ADR-009). Seeded from `config.offline_mode`; the
    /// Settings UI flips it without a restart, and every outbound-call site
    /// checks [`offline`](Self::offline) rather than `config.offline_mode`.
    offline: Arc<AtomicBool>,
}

/// Startup switches for [`App::load_with`]. Every field defaults to what the
/// real application wants; a test flips only the one it needs to hold still.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppOptions {
    /// Spawn the training poller loop. On in production — it is what keeps a
    /// detached trainer's progress visible and settles a run whose process
    /// has gone away.
    ///
    /// A test that writes `training_runs` rows by hand wants it off: every
    /// few seconds the poller re-reads each `running` row, finds no PID
    /// behind it and reconciles it to `interrupted` — correctly — racing
    /// whatever the test asserts about that row in the meantime.
    ///
    /// This does **not** switch off [`crate::training::runner::Runner::
    /// recover`], which runs either way: recovery is startup state repair,
    /// and it sees only the rows a *previous* process left behind.
    pub training_poller: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            training_poller: true,
        }
    }
}

impl App {
    /// Ensure directories exist, load configuration, open the database, seed
    /// first-run settings, start telemetry, and wire up the scheduler + job
    /// engine. Does **not** install logging (see [`bootstrap_process`]).
    /// Requires a Tokio runtime.
    pub async fn load(paths: AppPaths) -> Result<Self> {
        Self::load_with(paths, AppOptions::default()).await
    }

    /// [`load`](Self::load) with the startup switches spelled out — see
    /// [`AppOptions`].
    pub async fn load_with(paths: AppPaths, options: AppOptions) -> Result<Self> {
        paths.ensure()?;
        if crate::backup::apply_pending_import(&paths)? {
            tracing::warn!("a backup import was applied on startup");
        }
        let config = Config::load(&paths)?;
        let paths = paths
            .with_outputs_override(config.paths.outputs_path.clone())
            .with_runtimes_override(config.paths.runtimes_path.clone())
            .with_cache_override(config.paths.cache_path.clone())
            .with_datasets_override(config.paths.datasets_path.clone())
            .with_training_override(config.paths.training_path.clone());
        let db = Database::connect(&paths.db_file()).await?;
        Self::seed(&db).await?;

        let telemetry = Arc::new(Sampler::spawn());
        let runtimes = RuntimeRegistry::new();
        let llama = Arc::new(
            LlamaCppAdapter::discover(db.clone(), &paths.runtimes_dir())
                .with_options(config.llama.to_options()),
        );
        runtimes.register(llama.clone());
        let progress = Arc::new(ProgressHub::new());
        let comfyui = Arc::new(
            ComfyUiAdapter::discover(
                db.clone(),
                &paths.runtimes_dir(),
                ComfyDirs {
                    base: paths.comfyui_data_dir(),
                    // ComfyUI's own scratch output folder -- never the
                    // canonical `outputs_dir` the job engine writes each
                    // job's fetched bytes into (`write_output`), or every
                    // render leaves two files behind: ComfyUI's own
                    // `<job_id>_00001_.<ext>` and AIWM's clean `<job_id>.<ext>`.
                    output: paths.comfyui_data_dir().join("output"),
                    models_store: config.store_path.clone(),
                },
            )
            .with_options(config.comfyui.to_options())
            .with_progress(progress.clone()),
        );
        runtimes.register(comfyui.clone());
        let colibri = Arc::new(ColibriAdapter::discover(db.clone(), &paths.runtimes_dir()));
        runtimes.register(colibri.clone());
        let tts = Arc::new(TtsAdapter::new());
        runtimes.register(tts.clone());
        let vision = Arc::new(VisionAdapter::new());
        runtimes.register(vision.clone());
        let training = Arc::new(TrainingAdapter::discover(&paths.runtimes_dir()));
        runtimes.register(training.clone());
        let budget = resolve_vram_budget(&config, &telemetry);
        let scheduler = Arc::new(HybridScheduler::new(runtimes.clone(), budget));
        let auto_pref = config.models.auto_preference;
        let offline = Arc::new(AtomicBool::new(config.offline_mode));
        let hf_token = std::fs::read_to_string(paths.hf_token_file())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let registry = Arc::new(Registry::new(
            Box::new(HuggingFaceSource::new()?.with_token(hf_token.clone())),
            paths.cache_dir().join("registry"),
            offline.clone(),
        ));
        let civitai_token = std::fs::read_to_string(paths.civitai_token_file())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let civitai_registry = Arc::new(Registry::new(
            Box::new(
                CivitaiSource::for_front_door(config.civitai.front_door)?
                    .with_token(civitai_token.clone()),
            ),
            paths.cache_dir().join("civitai_registry"),
            offline.clone(),
        ));
        let jobs = Arc::new(
            JobEngine::new(
                db.clone(),
                runtimes.clone(),
                scheduler.clone(),
                llama.clone(),
                comfyui.clone(),
                paths.outputs_dir(),
                paths.datasets_dir(),
            )
            .with_telemetry(telemetry.subscribe())
            .with_training_dir(paths.training_dir())
            .with_auto_preference(auto_pref)
            .with_registry(registry.clone())
            .with_colibri(colibri.clone())
            .with_tts(tts.clone())
            .with_vision(vision.clone(), config.store_path.clone()),
        );
        let coding = Arc::new(LlamaCodingRuntime::new(
            runtimes.clone(),
            scheduler.clone(),
            llama.clone(),
        ));
        let opencode = Arc::new(OpenCodeAdapter::discover(&paths.runtimes_dir()));
        let hermes = Arc::new(HermesAgentAdapter::discover(&paths.runtimes_dir()));
        let agents = Arc::new(
            AgentSessions::new(db.clone(), coding)
                .with_adapter(opencode.clone())
                .with_adapter(hermes.clone())
                .with_auto_preference(auto_pref),
        );
        // A second, independent `LlamaCodingRuntime` handle for the external
        // launcher -- cheap (just more `Arc` clones of the same registry /
        // scheduler / llama adapter `agents` already uses), and the actual
        // pin/unpin state lives in the shared scheduler either way, so two
        // wrapper instances stay consistent with each other.
        let launcher_coding = Arc::new(LlamaCodingRuntime::new(
            runtimes.clone(),
            scheduler.clone(),
            llama.clone(),
        ));
        let launcher = Arc::new(
            Launcher::new(
                db.clone(),
                launcher_coding,
                Arc::new(AdapterBinaries {
                    opencode: opencode.clone(),
                    hermes: hermes.clone(),
                }),
            )
            .with_auto_preference(auto_pref),
        );
        let downloads = Arc::new(
            DownloadManager::new(
                db.clone(),
                config.store_path.clone(),
                paths.downloads_dir(),
                offline.clone(),
            )
            // The same keys the registry sources use: searching is anonymous,
            // but a gated file's download answers 401 without one.
            .with_tokens(crate::download::HostTokens {
                civitai: civitai_token,
                huggingface: hf_token,
            }),
        );

        // The training runner, its startup recovery and its poller, in that
        // order and after the job engine's own recovery in [`Self::seed`]: a
        // detached trainer that survived the restart keeps its GPU
        // reservation, anything else becomes `interrupted`.
        //
        // `recover` is awaited rather than spawned. Every `running` row that
        // exists at this moment is by definition a leftover, which is what
        // lets recovery read "no PID anywhere" as "it did not survive"; once
        // the app is live that inference stops holding, because a run created
        // a millisecond ago looks exactly the same. Awaiting it here draws
        // that line where it belongs, and never fails startup over it.
        let training_runner = Arc::new(TrainingRunner::new(
            db.clone(),
            training.clone(),
            scheduler.clone(),
            runtimes.clone(),
            paths.training_dir(),
            config.store_path.clone(),
        ));
        // Recovery is state repair, not polling: it runs even with the poller
        // switched off, because a store left with `running` rows and no
        // process behind them is wrong whether or not anyone is watching.
        if let Err(e) = training_runner.recover().await {
            tracing::warn!(error = %e, "training-run recovery failed");
        }
        if options.training_poller {
            spawn_poller(training_runner.clone());
        }

        // Best-effort startup sweep (nice-to-have alongside the manual
        // "clean up now" button + a periodic timer isn't wired up separately):
        // only runs when a retention policy is actually configured, and never
        // blocks or fails startup either way.
        let retention_policy = config.retention.to_policy();
        if retention_policy.is_active() {
            let outputs_dir = paths.outputs_dir();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::cleanup::outputs::sweep(&outputs_dir, retention_policy)
                })
                .await;
                match result {
                    Ok(r) if r.deleted_files > 0 => tracing::info!(
                        deleted = r.deleted_files,
                        freed_bytes = r.freed_bytes,
                        "startup output-retention sweep"
                    ),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "startup output-retention sweep panicked"),
                }
            });
        }

        Ok(Self {
            paths,
            config,
            db,
            telemetry,
            runtimes,
            llama,
            comfyui,
            colibri,
            tts,
            progress,
            vision,
            training,
            training_runner,
            scheduler,
            jobs,
            agents,
            opencode,
            hermes,
            launcher,
            registry,
            civitai_registry,
            downloads,
            offline,
        })
    }

    /// Swap the discovery registry — tests point it at a fixture. Also re-points
    /// the job engine's copy (the upgrade check, 6.7).
    pub fn with_registry(mut self, registry: Registry) -> Self {
        let registry = Arc::new(registry);
        self.registry = registry.clone();
        if let Some(jobs) = Arc::get_mut(&mut self.jobs) {
            jobs.set_registry(registry);
        }
        self
    }

    /// Swap the Civitai registry — tests point it at a fixture. Unlike
    /// [`with_registry`](Self::with_registry), nothing else holds a copy to
    /// re-point (no job engine integration — see the field doc on
    /// [`civitai_registry`](Self::civitai_registry)).
    pub fn with_civitai_registry(mut self, registry: Registry) -> Self {
        self.civitai_registry = Arc::new(registry);
        self
    }

    /// Whether outbound network calls are currently forbidden (ADR-009). This is
    /// the live value — the Settings UI can change it mid-session.
    pub fn offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    /// Flip the live offline switch. The caller persists the new value to
    /// `config.toml` so it survives a restart.
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::Relaxed);
    }

    async fn seed(db: &Database) -> Result<()> {
        db.settings()
            .set_if_absent("schema_version", SCHEMA_VERSION)
            .await?;
        db.settings()
            .set_if_absent("first_run_at", &now_rfc3339())
            .await?;

        let recovered = db.jobs().recover_interrupted().await?;
        if recovered > 0 {
            tracing::warn!(count = recovered, "recovered interrupted jobs as failed");
        }
        let orphaned = db.agents().recover_orphaned().await?;
        if orphaned > 0 {
            tracing::warn!(
                count = orphaned,
                "closed orphaned agent sessions from a previous run"
            );
        }
        let requeued = db.downloads().recover_interrupted().await?;
        if requeued > 0 {
            tracing::warn!(count = requeued, "re-queued interrupted downloads");
        }
        Ok(())
    }
}

/// Effective VRAM budget: the config value if set, else the detected GPU's total,
/// else a conservative fallback.
fn resolve_vram_budget(config: &Config, telemetry: &Sampler) -> u64 {
    if config.vram_budget_mb > 0 {
        return config.vram_budget_mb;
    }
    match telemetry.latest().gpu {
        GpuStatus::Available(gpu) => gpu.vram_total_mb,
        GpuStatus::Unavailable { .. } => FALLBACK_VRAM_BUDGET_MB,
    }
}

/// Full process bootstrap for a binary: resolve the data directory (portable
/// by default, see [`AppPaths::for_app`]), load config, open the database,
/// install logging, emit a startup line. Returns the [`App`] and the logging
/// guard, which the caller must keep alive.
pub async fn bootstrap_process() -> Result<(Arc<App>, WorkerGuard)> {
    let app = Arc::new(App::load(AppPaths::for_app()?).await?);
    let guard = crate::logging::init(&app.config.log_filter, &app.paths.logs_dir())?;
    tracing::info!(
        version = crate::CORE_VERSION,
        data_dir = %app.paths.root().display(),
        store_path = %app.config.store_path.display(),
        core_api_port = app.config.core_api_port,
        vram_budget_mb = app.scheduler.budget_mb(),
        offline_mode = app.offline(),
        "aiwm-core bootstrapped"
    );
    Ok((app, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_creates_dirs_config_and_db() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));

        let app = App::load(paths.clone()).await.unwrap();

        assert!(paths.root().is_dir());
        assert!(paths.logs_dir().is_dir());
        assert!(paths.config_file().is_file());
        assert!(paths.db_file().is_file());
        // Defaults, except that a fresh data root keeps its model store inside
        // itself rather than adopting the machine-wide default path.
        assert_eq!(
            app.config,
            Config {
                store_path: paths.root().join("models"),
                ..Config::default()
            }
        );
    }

    #[tokio::test]
    async fn load_seeds_schema_version_once() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());

        let first = App::load(paths.clone()).await.unwrap();
        let seeded_at = first.db.settings().get("first_run_at").await.unwrap();
        assert_eq!(
            first
                .db
                .settings()
                .get("schema_version")
                .await
                .unwrap()
                .as_deref(),
            Some("1")
        );
        first.db.close().await;

        // Re-open: the seed timestamp must not change.
        let second = App::load(paths).await.unwrap();
        assert_eq!(
            second.db.settings().get("first_run_at").await.unwrap(),
            seeded_at
        );
    }

    #[tokio::test]
    async fn load_applies_paths_overrides_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        let outputs = tmp.path().join("elsewhere-outputs");
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(
            paths.config_file(),
            format!(
                "[paths]\noutputs_path = '{}'\n",
                outputs.display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let app = App::load(paths.clone()).await.unwrap();

        assert_eq!(app.paths.outputs_dir(), outputs);
        // Untouched folders (and the config/db location itself) stay put.
        assert_eq!(app.paths.runtimes_dir(), paths.runtimes_dir());
        assert_eq!(app.paths.root(), paths.root());
    }

    #[tokio::test]
    async fn load_applies_datasets_and_training_overrides_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        let datasets = tmp.path().join("elsewhere-datasets");
        let training = tmp.path().join("elsewhere-training");
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(
            paths.config_file(),
            format!(
                "[paths]\ndatasets_path = '{}'\ntraining_path = '{}'\n",
                datasets.display().to_string().replace('\\', "\\\\"),
                training.display().to_string().replace('\\', "\\\\"),
            ),
        )
        .unwrap();

        let app = App::load(paths.clone()).await.unwrap();

        assert_eq!(app.paths.datasets_dir(), datasets);
        assert_eq!(app.paths.training_dir(), training);
        // Untouched folders stay put.
        assert_eq!(app.paths.outputs_dir(), paths.outputs_dir());
    }

    #[tokio::test]
    async fn comfyui_gets_its_own_output_dir_distinct_from_the_canonical_one() {
        // ComfyUI's `SaveImage`/`SaveVideo` nodes always write their own copy
        // under `--output-directory` -- if that were the same folder the job
        // engine's own `write_output` writes the fetched bytes into (the
        // canonical `outputs_dir` the Gallery/API serve from), every
        // successful render would leave two files behind: ComfyUI's own
        // `<job_id>_00001_.png` and AIWM's clean `<job_id>.png`. ComfyUI needs
        // its own scratch folder instead.
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));

        let app = App::load(paths.clone()).await.unwrap();

        assert_ne!(app.comfyui.output_dir(), app.paths.outputs_dir());
        assert_eq!(
            app.comfyui.output_dir(),
            app.paths.comfyui_data_dir().join("output")
        );
    }

    #[tokio::test]
    async fn load_runs_a_best_effort_startup_retention_sweep_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(
            paths.config_file(),
            "[retention]\nmax_age_days = 30\nmax_total_mb = 0\n",
        )
        .unwrap();

        let outputs = paths.outputs_dir();
        std::fs::create_dir_all(&outputs).unwrap();
        let stale = outputs.join("job-ancient.png");
        std::fs::write(&stale, b"old bytes").unwrap();
        let ancient = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 86_400);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(ancient)
            .unwrap();

        let _app = App::load(paths).await.unwrap();
        // The sweep is spawned in the background -- give it a moment.
        for _ in 0..50 {
            if !stale.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!stale.exists(), "startup sweep should have removed it");
    }

    #[tokio::test]
    async fn load_never_touches_outputs_when_retention_is_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));
        let outputs = paths.outputs_dir();
        std::fs::create_dir_all(&outputs).unwrap();
        let file = outputs.join("job-a.png");
        std::fs::write(&file, b"bytes").unwrap();
        let ancient = std::time::SystemTime::now() - std::time::Duration::from_secs(365 * 86_400);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(ancient)
            .unwrap();

        let _app = App::load(paths).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(file.exists(), "no policy configured -- nothing should move");
    }

    #[tokio::test]
    async fn load_surfaces_a_broken_config() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "core_api_port = 1").unwrap();

        assert!(App::load(paths).await.is_err());
    }
}
