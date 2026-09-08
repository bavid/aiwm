//! Application bootstrap: tie together paths, config, database, telemetry and
//! logging into a live [`App`] handle. Runtimes and the scheduler are attached
//! in later work packages.

use tracing_appender::non_blocking::WorkerGuard;

use crate::config::Config;
use crate::db::{now_rfc3339, Database};
use crate::paths::AppPaths;
use crate::telemetry::Sampler;
use crate::Result;

/// Settings seeded on first run. `config.toml` remains the source of truth for
/// startup configuration; these are app-managed markers.
const SCHEMA_VERSION: &str = "1";

/// A bootstrapped core: resolved layout, effective configuration, an open
/// database and a running telemetry sampler.
#[derive(Debug)]
pub struct App {
    pub paths: AppPaths,
    pub config: Config,
    pub db: Database,
    pub telemetry: Sampler,
}

impl App {
    /// Ensure directories exist, load configuration, open the database, seed
    /// first-run settings and start telemetry sampling. Does **not** install
    /// logging — that is a process concern (see [`bootstrap_process`]). Requires
    /// a Tokio runtime.
    pub async fn load(paths: AppPaths) -> Result<Self> {
        paths.ensure()?;
        let config = Config::load(&paths)?;
        let db = Database::connect(&paths.db_file()).await?;
        Self::seed(&db).await?;
        let telemetry = Sampler::spawn();
        Ok(Self {
            paths,
            config,
            db,
            telemetry,
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

/// Full process bootstrap for a binary: resolve `%APPDATA%`, load config, open
/// the database, install logging, emit a startup line. Returns the [`App`] and
/// the logging guard, which the caller must keep alive.
pub async fn bootstrap_process() -> Result<(App, WorkerGuard)> {
    let app = App::load(AppPaths::for_app()?).await?;
    let guard = crate::logging::init(&app.config.log_filter, &app.paths.logs_dir())?;
    tracing::info!(
        version = crate::CORE_VERSION,
        data_dir = %app.paths.root().display(),
        store_path = %app.config.store_path.display(),
        core_api_port = app.config.core_api_port,
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
