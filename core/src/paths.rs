//! Application directory layout.
//!
//! By default the app is **portable**: everything lives in a `data/` folder
//! next to the running executable (see [`app_root_from_exe`]) — run it from
//! `E:\AI` and its config, database, logs, generated outputs, and managed
//! runtime installs all stay on `E:`, no `%APPDATA%` involved. `AIWM_DATA_DIR`
//! overrides the whole layout to one directory of your choosing (portable
//! installs, tests). Three of the bulkier folders — outputs, runtimes, cache —
//! can additionally be pointed elsewhere individually via `config.toml`'s
//! `[paths]` table ([`AppPaths::with_outputs_override`] and friends, wired in
//! `App::load` once the config is read). The model store is configured
//! separately ([`crate::config::Config::store_path`]) and is created lazily on
//! first use, not here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

const APP_DIR_NAME: &str = "AIWorkstationManager";
/// Overrides the data-directory root (portable installs, tests).
const DATA_DIR_ENV: &str = "AIWM_DATA_DIR";

/// Resolved locations for this app's local state. Cheap to clone.
///
/// `root` and `local_root` are the same directory by default (the portable
/// `data/` folder, or `AIWM_DATA_DIR`); they exist as separate fields so a
/// caller *could* still split them (e.g. a future "roam config, keep bulky
/// data local" mode), and because `outputs_dir` / `runtimes_dir` / `cache_dir`
/// hang off `local_root` specifically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
    local_root: PathBuf,
    outputs_override: Option<PathBuf>,
    runtimes_override: Option<PathBuf>,
    cache_override: Option<PathBuf>,
}

impl AppPaths {
    /// The effective location: `AIWM_DATA_DIR` if set and non-empty, otherwise
    /// `<app_root>/data` where `app_root` is derived from the running
    /// executable's path ([`app_root_from_exe`]), falling back to the OS user
    /// data directory only if the executable's own path can't be read.
    pub fn for_app() -> Result<Self> {
        let exe_root = std::env::current_exe()
            .ok()
            .map(|exe| app_root_from_exe(&exe));
        Self::resolve(std::env::var_os(DATA_DIR_ENV), exe_root, dirs::data_dir())
    }

    fn resolve(
        env_override: Option<std::ffi::OsString>,
        exe_root: Option<PathBuf>,
        os_data_dir: Option<PathBuf>,
    ) -> Result<Self> {
        if let Some(dir) = env_override.filter(|d| !d.is_empty()) {
            return Ok(Self::rooted(dir));
        }
        let base = match exe_root {
            Some(root) => root.join("data"),
            None => os_data_dir
                .ok_or_else(|| CoreError::Config("cannot resolve the user data directory".into()))?
                .join(APP_DIR_NAME),
        };
        Ok(Self::rooted(base))
    }

    /// Root the whole layout at one directory (tests, or an explicit portable
    /// install). No per-folder overrides.
    pub fn rooted(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            local_root: root.clone(),
            root,
            outputs_override: None,
            runtimes_override: None,
            cache_override: None,
        }
    }

    /// Point [`outputs_dir`](Self::outputs_dir) at a specific directory
    /// instead of the default under `local_root`. `None` restores the default.
    pub fn with_outputs_override(mut self, dir: Option<PathBuf>) -> Self {
        self.outputs_override = dir;
        self
    }

    /// Point [`runtimes_dir`](Self::runtimes_dir) at a specific directory
    /// instead of the default under `local_root`. `None` restores the default.
    pub fn with_runtimes_override(mut self, dir: Option<PathBuf>) -> Self {
        self.runtimes_override = dir;
        self
    }

    /// Point [`cache_dir`](Self::cache_dir) at a specific directory instead of
    /// the default under `local_root`. `None` restores the default.
    pub fn with_cache_override(mut self, dir: Option<PathBuf>) -> Self {
        self.cache_override = dir;
        self
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
    /// One curated version per runtime lives under `<runtimes_dir>/<id>/` —
    /// these can be gigabytes. Overridable via `config.toml`'s `[paths]` table.
    pub fn runtimes_dir(&self) -> PathBuf {
        self.runtimes_override
            .clone()
            .unwrap_or_else(|| self.local_root.join("runtimes"))
    }

    /// ComfyUI's `--base-directory`: its `models/` (junctioned to the canonical
    /// store), `input/`, `temp/`. Not independently overridable — small and
    /// tied to wherever ComfyUI itself is installed.
    pub fn comfyui_data_dir(&self) -> PathBuf {
        self.local_root.join("comfyui-data")
    }

    /// Where generated images / videos land (`jobs.output_path`). Grows over
    /// time. Overridable via `config.toml`'s `[paths]` table.
    pub fn outputs_dir(&self) -> PathBuf {
        self.outputs_override
            .clone()
            .unwrap_or_else(|| self.local_root.join("outputs"))
    }

    /// Disposable caches (the registry index cache, 6.1). Safe to delete at
    /// any time; never backed up. Overridable via `config.toml`'s `[paths]`
    /// table.
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_override
            .clone()
            .unwrap_or_else(|| self.local_root.join("cache"))
    }

    /// Where the download manager (6.4) stages in-flight files, one dir per
    /// download id; a finished download is moved into the model store.
    pub fn downloads_dir(&self) -> PathBuf {
        self.local_root.join(".downloads")
    }

    /// Optional Hugging Face token (6.9) — a bare file, machine-local only so
    /// it is **never roamed and never in a backup export** (ADR-022).
    pub fn hf_token_file(&self) -> PathBuf {
        self.local_root.join("hf_token.txt")
    }

    /// Where `POST /export` writes backup archives.
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// A staging dir an import drops its `aiwm.db` / `config.toml` into; the
    /// next [`crate::backup::apply_pending_import`] on startup swaps them in.
    pub fn pending_import_dir(&self) -> PathBuf {
        self.root.join(".pending-import")
    }

    /// Create the root and logs directories if missing. Idempotent. The
    /// overridable folders (outputs/runtimes/cache) are created lazily by
    /// their own consumers on first use.
    pub fn ensure(&self) -> Result<()> {
        for dir in [self.root.clone(), self.logs_dir()] {
            fs::create_dir_all(&dir)
                .map_err(|e| CoreError::Config(format!("creating {}: {e}", dir.display())))?;
        }
        Ok(())
    }
}

/// Where the *portable* default should live: next to the running executable —
/// but `cargo run` / `cargo test` put the executable several levels deep
/// (`target/{debug,release}[/deps]/<exe>`), and a `data/` folder nested inside
/// `target/` would be wiped by `cargo clean`. Unwrap that known layout to
/// reach the workspace root; a "real" install (the exe sitting directly in its
/// own folder, e.g. a portable copy or a Tauri bundle) is used as-is.
fn app_root_from_exe(exe: &Path) -> PathBuf {
    let mut dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).map(str::to_string);

    if name(&dir).as_deref() == Some("deps") {
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        }
    }
    let is_profile = matches!(name(&dir).as_deref(), Some("debug") | Some("release"));
    if is_profile {
        if let Some(parent) = dir.parent() {
            if name(parent).as_deref() == Some("target") {
                if let Some(workspace) = parent.parent() {
                    dir = workspace.to_path_buf();
                }
            }
        }
    }
    dir
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
    fn for_app_resolves_a_directory() {
        // Uses the real environment; just checks it produces something.
        assert!(AppPaths::for_app().is_ok());
    }

    // --- app_root_from_exe: unwrapping cargo's build layouts ----------------

    #[test]
    fn app_root_from_exe_unwraps_a_cargo_run_layout() {
        let exe = Path::new("E:/AI/target/debug/aiwm-tauri.exe");
        assert_eq!(app_root_from_exe(exe), Path::new("E:/AI"));
    }

    #[test]
    fn app_root_from_exe_unwraps_a_release_build() {
        let exe = Path::new("E:/AI/target/release/aiwm-cored.exe");
        assert_eq!(app_root_from_exe(exe), Path::new("E:/AI"));
    }

    #[test]
    fn app_root_from_exe_unwraps_a_cargo_test_layout() {
        let exe = Path::new("E:/AI/target/debug/deps/aiwm_core-abcd1234.exe");
        assert_eq!(app_root_from_exe(exe), Path::new("E:/AI"));
    }

    #[test]
    fn app_root_from_exe_keeps_a_plain_install_directory() {
        // No target/{debug,release} above it -- the exe's own folder is the root
        // (a portable copy, or a real Tauri bundle install).
        let exe = Path::new("E:/Programs/AI Workstation Manager/aiwm-tauri.exe");
        assert_eq!(
            app_root_from_exe(exe),
            Path::new("E:/Programs/AI Workstation Manager")
        );
    }

    // --- resolve / for_app defaults ------------------------------------------

    #[test]
    fn resolve_defaults_next_to_the_running_exe() {
        let p = AppPaths::resolve(None, Some(PathBuf::from("E:\\AI")), None).unwrap();
        assert_eq!(p.root(), Path::new("E:\\AI\\data"));
        assert_eq!(p.runtimes_dir(), Path::new("E:\\AI\\data\\runtimes"));
        assert_eq!(p.outputs_dir(), Path::new("E:\\AI\\data\\outputs"));
    }

    #[test]
    fn resolve_falls_back_to_os_data_dir_when_the_exe_path_is_unknown() {
        let p = AppPaths::resolve(
            None,
            None,
            Some(PathBuf::from("C:\\Users\\x\\AppData\\Roaming")),
        )
        .unwrap();
        assert!(p.root().starts_with("C:\\Users\\x\\AppData\\Roaming"));
        assert!(p.root().ends_with(APP_DIR_NAME));
    }

    #[test]
    fn resolve_prefers_env_override_over_the_exe_root() {
        let p = AppPaths::resolve(
            Some("D:\\portable\\aiwm".into()),
            Some(PathBuf::from("E:\\ignored")),
            Some(PathBuf::from("C:\\ignored")),
        )
        .unwrap();
        assert_eq!(p.root(), Path::new("D:\\portable\\aiwm"));
        assert_eq!(p.runtimes_dir(), Path::new("D:\\portable\\aiwm\\runtimes"));
    }

    #[test]
    fn resolve_ignores_empty_override() {
        let p = AppPaths::resolve(
            Some(String::new().into()),
            Some(PathBuf::from("E:\\AI")),
            None,
        )
        .unwrap();
        assert_eq!(p.root(), Path::new("E:\\AI\\data"));
    }

    #[test]
    fn resolve_errors_without_any_base() {
        assert!(AppPaths::resolve(None, None, None).is_err());
    }

    // --- per-folder overrides -------------------------------------------------

    #[test]
    fn outputs_override_wins_over_the_default() {
        let p = AppPaths::rooted("/data/aiwm")
            .with_outputs_override(Some(PathBuf::from("/mnt/media/outputs")));
        assert_eq!(p.outputs_dir(), Path::new("/mnt/media/outputs"));
        // Untouched folders stay on the default root.
        assert!(p.cache_dir().starts_with("/data/aiwm"));
    }

    #[test]
    fn runtimes_override_wins_over_the_default() {
        let p = AppPaths::rooted("/data/aiwm")
            .with_runtimes_override(Some(PathBuf::from("/mnt/fast/runtimes")));
        assert_eq!(p.runtimes_dir(), Path::new("/mnt/fast/runtimes"));
    }

    #[test]
    fn cache_override_wins_over_the_default() {
        let p = AppPaths::rooted("/data/aiwm")
            .with_cache_override(Some(PathBuf::from("/mnt/fast/cache")));
        assert_eq!(p.cache_dir(), Path::new("/mnt/fast/cache"));
    }

    #[test]
    fn a_none_override_restores_the_default() {
        let p = AppPaths::rooted("/data/aiwm")
            .with_outputs_override(Some(PathBuf::from("/mnt/media")))
            .with_outputs_override(None);
        assert_eq!(p.outputs_dir(), Path::new("/data/aiwm/outputs"));
    }
}
