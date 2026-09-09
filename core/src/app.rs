//! Application bootstrap: tie together paths, config, database, telemetry,
//! runtimes, scheduler and the job engine into a live [`App`] handle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing_appender::non_blocking::WorkerGuard;

use crate::agent::{HermesAgentAdapter, OpenCodeAdapter};
use crate::capability::agent::{AgentSessions, LlamaCodingRuntime};
use crate::config::{Config, FALLBACK_VRAM_BUDGET_MB};
use crate::db::{now_rfc3339, Database};
use crate::orchestrator::JobEngine;
use crate::paths::AppPaths;
use crate::runtime::{ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry};
use crate::scheduler::HybridScheduler;
use crate::telemetry::{GpuStatus, Sampler};
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
    pub scheduler: Arc<HybridScheduler>,
    pub jobs: Arc<JobEngine>,
    /// Long-running agent sessions (Phase 5.1c) — its own subsystem, not a job.
    pub agents: Arc<AgentSessions>,
    /// The agent adapters, also registered on [`agents`](Self::agents). Held
    /// typed so handlers can report install state and drive Hermes' installer.
    pub opencode: Arc<OpenCodeAdapter>,
    pub hermes: Arc<HermesAgentAdapter>,
    /// Live offline switch (ADR-009). Seeded from `config.offline_mode`; the
    /// Settings UI flips it without a restart, and every outbound-call site
    /// checks [`offline`](Self::offline) rather than `config.offline_mode`.
    offline: Arc<AtomicBool>,
}

impl App {
    /// Ensure directories exist, load configuration, open the database, seed
    /// first-run settings, start telemetry, and wire up the scheduler + job
    /// engine. Does **not** install logging (see [`bootstrap_process`]).
    /// Requires a Tokio runtime.
    pub async fn load(paths: AppPaths) -> Result<Self> {
        paths.ensure()?;
        if crate::backup::apply_pending_import(&paths)? {
            tracing::warn!("a backup import was applied on startup");
        }
        let config = Config::load(&paths)?;
        let db = Database::connect(&paths.db_file()).await?;
        Self::seed(&db).await?;

        let telemetry = Arc::new(Sampler::spawn());
        let runtimes = RuntimeRegistry::new();
        let llama = Arc::new(
            LlamaCppAdapter::discover(db.clone(), &paths.runtimes_dir())
                .with_options(config.llama.to_options()),
        );
        runtimes.register(llama.clone());
        let comfyui = Arc::new(
            ComfyUiAdapter::discover(
                db.clone(),
                &paths.runtimes_dir(),
                ComfyDirs {
                    base: paths.comfyui_data_dir(),
                    output: paths.outputs_dir(),
                    models_store: config.store_path.clone(),
                },
            )
            .with_options(config.comfyui.to_options()),
        );
        runtimes.register(comfyui.clone());
        let budget = resolve_vram_budget(&config, &telemetry);
        let scheduler = Arc::new(HybridScheduler::new(runtimes.clone(), budget));
        let jobs = Arc::new(JobEngine::new(
            db.clone(),
            runtimes.clone(),
            scheduler.clone(),
            llama.clone(),
            comfyui.clone(),
            paths.outputs_dir(),
        ));
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
                .with_adapter(hermes.clone()),
        );
        let offline = Arc::new(AtomicBool::new(config.offline_mode));

        Ok(Self {
            paths,
            config,
            db,
            telemetry,
            runtimes,
            llama,
            comfyui,
            scheduler,
            jobs,
            agents,
            opencode,
            hermes,
            offline,
        })
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

/// Full process bootstrap for a binary: resolve `%APPDATA%`, load config, open
/// the database, install logging, emit a startup line. Returns the [`App`] and
/// the logging guard, which the caller must keep alive.
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
        assert_eq!(app.config, Config::default());
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
    async fn load_surfaces_a_broken_config() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "core_api_port = 1").unwrap();

        assert!(App::load(paths).await.is_err());
    }
}
