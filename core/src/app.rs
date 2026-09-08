//! Application bootstrap: tie together paths, config, database, telemetry,
//! runtimes, scheduler and the job engine into a live [`App`] handle.

use std::sync::Arc;

use tracing_appender::non_blocking::WorkerGuard;

use crate::config::{Config, FALLBACK_VRAM_BUDGET_MB};
use crate::db::{now_rfc3339, Database};
use crate::orchestrator::JobEngine;
use crate::paths::AppPaths;
use crate::runtime::RuntimeRegistry;
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
    pub config: Config,
    pub db: Database,
    pub telemetry: Arc<Sampler>,
    pub runtimes: RuntimeRegistry,
    pub scheduler: Arc<HybridScheduler>,
    pub jobs: Arc<JobEngine>,
}

impl App {
    /// Ensure directories exist, load configuration, open the database, seed
    /// first-run settings, start telemetry, and wire up the scheduler + job
    /// engine. Does **not** install logging (see [`bootstrap_process`]).
    /// Requires a Tokio runtime.
    pub async fn load(paths: AppPaths) -> Result<Self> {
        paths.ensure()?;
        let config = Config::load(&paths)?;
        let db = Database::connect(&paths.db_file()).await?;
        Self::seed(&db).await?;

        let telemetry = Arc::new(Sampler::spawn());
        let runtimes = RuntimeRegistry::new();
        let budget = resolve_vram_budget(&config, &telemetry);
        let scheduler = Arc::new(HybridScheduler::new(runtimes.clone(), budget));
        let jobs = Arc::new(JobEngine::new(
            db.clone(),
            runtimes.clone(),
            scheduler.clone(),
        ));

        Ok(Self {
            paths,
            config,
            db,
            telemetry,
            runtimes,
            scheduler,
            jobs,
        })
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
        offline_mode = app.config.offline_mode,
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
