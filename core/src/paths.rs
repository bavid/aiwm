//! Application directory layout.
//!
//! Config, database and logs live under `%APPDATA%\AIWorkstationManager\`;
//! bulky managed-runtime installs live under `%LOCALAPPDATA%\…\runtimes\` so
//! they never roam. `AIWM_DATA_DIR` collapses both onto one directory. The model
//! store is configured separately ([`crate::config::Config::store_path`],
//! default `E:\AI\models`) and is created lazily on first use, not here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

const APP_DIR_NAME: &str = "AIWorkstationManager";
/// Overrides the data-directory root (portable installs, tests).
const DATA_DIR_ENV: &str = "AIWM_DATA_DIR";

/// Resolved locations for this app's local state. Cheap to clone.
///
/// `root` (roaming `%APPDATA%`) holds small config + the database; `local_root`
/// (`%LOCALAPPDATA%`) holds bulky machine-specific data like the managed runtime
/// installs, which have no business roaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
    local_root: PathBuf,
}

impl AppPaths {
    /// The effective location: `AIWM_DATA_DIR` if set and non-empty (both roots
    /// collapse to it), otherwise `%APPDATA%` / `%LOCALAPPDATA%` +
    /// `AIWorkstationManager`.
    pub fn for_app() -> Result<Self> {
        Self::resolve(
            std::env::var_os(DATA_DIR_ENV),
            dirs::data_dir(),
            dirs::data_local_dir(),
        )
    }

    fn resolve(
        env_override: Option<std::ffi::OsString>,
        data_dir: Option<PathBuf>,
        data_local_dir: Option<PathBuf>,
    ) -> Result<Self> {
        if let Some(dir) = env_override.filter(|d| !d.is_empty()) {
            return Ok(Self::rooted(dir));
        }
        let base = data_dir
            .ok_or_else(|| CoreError::Config("cannot resolve the user data directory".into()))?;
        let local = data_local_dir.unwrap_or_else(|| base.clone());
        Ok(Self {
            root: base.join(APP_DIR_NAME),
            local_root: local.join(APP_DIR_NAME),
        })
    }

    /// Root the whole layout at one directory (tests, or a portable install).
    pub fn rooted(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            local_root: root.clone(),
            root,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn db_file(&self) -> PathBuf {
        self.root.join("aiwm.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Where the tool installs the runtimes it manages (llama.cpp, ComfyUI, …).
    /// One curated version per runtime lives under `<local_root>/runtimes/<id>/`
    /// — these can be gigabytes and must not roam.
    pub fn runtimes_dir(&self) -> PathBuf {
        self.local_root.join("runtimes")
    }

    /// ComfyUI's `--base-directory`: its `models/` (junctioned to the canonical
    /// store), `input/`, `temp/`. Machine-local, not roamed.
    pub fn comfyui_data_dir(&self) -> PathBuf {
        self.local_root.join("comfyui-data")
    }

    /// Where generated images / videos land (`jobs.output_path`). Local — this
    /// grows; a user-configurable path comes later.
    pub fn outputs_dir(&self) -> PathBuf {
        self.local_root.join("outputs")
    }

    /// Disposable machine-local caches (the registry index cache, 6.1). Safe to
    /// delete at any time; never roamed, never backed up.
    pub fn cache_dir(&self) -> PathBuf {
        self.local_root.join("cache")
    }

    /// Where the download manager (6.4) stages in-flight files, one dir per
    /// download id; a finished download is moved into the model store.
    pub fn downloads_dir(&self) -> PathBuf {
        self.local_root.join(".downloads")
    }

    /// Where `POST /export` writes backup archives (roamed — small).
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// A staging dir an import drops its `aiwm.db` / `config.toml` into; the
    /// next [`crate::backup::apply_pending_import`] on startup swaps them in.
    pub fn pending_import_dir(&self) -> PathBuf {
        self.root.join(".pending-import")
    }

    /// Create the root and logs directories if missing. Idempotent.
    pub fn ensure(&self) -> Result<()> {
        for dir in [self.root.clone(), self.logs_dir()] {
            fs::create_dir_all(&dir)
                .map_err(|e| CoreError::Config(format!("creating {}: {e}", dir.display())))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_paths_sit_under_root() {
        let p = AppPaths::rooted("/data/aiwm");
        assert_eq!(p.root(), Path::new("/data/aiwm"));
        assert!(p.config_file().ends_with("config.toml"));
        assert!(p.db_file().ends_with("aiwm.db"));
        assert!(p.logs_dir().ends_with("logs"));
        assert!(p.runtimes_dir().ends_with("runtimes"));
        assert!(p.comfyui_data_dir().ends_with("comfyui-data"));
        assert!(p.outputs_dir().ends_with("outputs"));
        assert!(p.config_file().starts_with(p.root()));
        // `rooted` collapses both roots, so runtimes still land under it.
        assert!(p.runtimes_dir().starts_with(p.root()));
    }

    #[test]
    fn runtimes_dir_uses_localappdata_not_roaming() {
        let p = AppPaths::resolve(
            None,
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Roaming")),
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Local")),
        )
        .unwrap();
        assert!(p
            .config_file()
            .starts_with("C:\\Users\\x\\AppData\\Roaming"));
        assert!(p.runtimes_dir().starts_with("C:\\Users\\x\\AppData\\Local"));
        assert!(p.runtimes_dir().ends_with("runtimes"));
    }

    #[test]
    fn ensure_creates_root_and_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::rooted(tmp.path().join("nested").join("aiwm"));
        assert!(!p.root().exists());

        p.ensure().unwrap();

        assert!(p.root().is_dir());
        assert!(p.logs_dir().is_dir());
    }

    #[test]
    fn ensure_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::rooted(tmp.path());
        p.ensure().unwrap();
        p.ensure().unwrap();
        assert!(p.logs_dir().is_dir());
    }

    #[test]
    fn resolve_uses_appdata_when_no_override() {
        let p = AppPaths::resolve(
            None,
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Roaming")),
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Local")),
        )
        .unwrap();
        assert!(p.root().ends_with(APP_DIR_NAME));
        assert!(p.root().starts_with("C:\\Users\\x\\AppData\\Roaming"));
    }

    #[test]
    fn resolve_prefers_env_override() {
        let p = AppPaths::resolve(
            Some("D:\\portable\\aiwm".into()),
            Some(PathBuf::from("C:\\ignored")),
            Some(PathBuf::from("C:\\ignored\\local")),
        )
        .unwrap();
        assert_eq!(p.root(), Path::new("D:\\portable\\aiwm"));
        assert_eq!(p.runtimes_dir(), Path::new("D:\\portable\\aiwm\\runtimes"));
    }

    #[test]
    fn resolve_ignores_empty_override() {
        let p = AppPaths::resolve(
            Some(String::new().into()),
            Some(PathBuf::from("C:\\base")),
            Some(PathBuf::from("C:\\base-local")),
        )
        .unwrap();
        assert!(p.root().starts_with("C:\\base"));
    }

    #[test]
    fn resolve_errors_without_any_base() {
        assert!(AppPaths::resolve(None, None, None).is_err());
    }

    #[test]
    fn resolve_falls_back_to_roaming_when_no_local_dir() {
        let p = AppPaths::resolve(None, Some(PathBuf::from("C:\\base")), None).unwrap();
        assert!(p.runtimes_dir().starts_with("C:\\base"));
    }

    #[test]
    fn for_app_resolves_a_directory() {
        // Uses the real environment; just checks it produces something.
        assert!(AppPaths::for_app().is_ok());
    }
}
