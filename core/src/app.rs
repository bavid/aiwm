//! Application bootstrap: tie together paths, config and logging into a live
//! [`App`] handle. Services (DB, telemetry, runtimes, scheduler) are attached in
//! later work packages.

use tracing_appender::non_blocking::WorkerGuard;

use crate::config::Config;
use crate::paths::AppPaths;
use crate::Result;

/// A bootstrapped core: resolved layout plus effective configuration. Cheap to
/// clone the parts you need; the value itself is held by the host process.
#[derive(Debug, Clone)]
pub struct App {
    pub paths: AppPaths,
    pub config: Config,
}

impl App {
    /// Ensure directories exist and load configuration. Does **not** install
    /// logging — that is a process concern (see [`bootstrap_process`]). Safe to
    /// call repeatedly in tests.
    pub fn load(paths: AppPaths) -> Result<Self> {
        paths.ensure()?;
        let config = Config::load(&paths)?;
        Ok(Self { paths, config })
    }
}

/// Full process bootstrap for a binary: resolve `%APPDATA%`, load config,
/// install logging, emit a startup line. Returns the [`App`] and the logging
/// guard, which the caller must keep alive.
pub fn bootstrap_process() -> Result<(App, WorkerGuard)> {
    let app = App::load(AppPaths::for_app()?)?;
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

    #[test]
    fn load_creates_dirs_and_config() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));

        let app = App::load(paths.clone()).unwrap();

        assert!(paths.root().is_dir());
        assert!(paths.logs_dir().is_dir());
        assert!(paths.config_file().is_file());
        assert_eq!(app.config, Config::default());
    }

    #[test]
    fn load_is_repeatable() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        let first = App::load(paths.clone()).unwrap();
        let second = App::load(paths).unwrap();
        assert_eq!(first.config, second.config);
    }

    #[test]
    fn load_surfaces_a_broken_config() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "core_api_port = 1").unwrap();

        assert!(App::load(paths).is_err());
    }
}
