//! Locating the ComfyUI entrypoint and turning it into a [`SpawnSpec`]. Kept
//! apart from the adapter's state machine in `mod.rs`.
//!
//! Real ComfyUI is `<venv python> <checkout>/main.py`; the test fixture
//! (`aiwm-fake-comfy`) is a single self-contained executable. [`ComfyLaunch`]
//! covers both — `main` is `None` for the fixture.

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use super::RUNTIME_ID;
use crate::runtime::SpawnSpec;

/// Env override for the ComfyUI interpreter (or a standalone fixture exe).
pub(super) const PYTHON_ENV: &str = "AIWM_COMFYUI_PYTHON";
/// Env override for the ComfyUI checkout directory (the one holding `main.py`).
pub(super) const DIR_ENV: &str = "AIWM_COMFYUI_DIR";

const PYTHON_EXE: &str = if cfg!(windows) {
    ".venv\\Scripts\\python.exe"
} else {
    ".venv/bin/python"
};

/// How to start the ComfyUI server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComfyLaunch {
    /// The interpreter, or a self-contained executable when `main` is `None`.
    pub program: PathBuf,
    /// `main.py` for a real checkout; `None` for the fixture.
    pub main: Option<PathBuf>,
    /// Extra raw arguments, appended verbatim (Settings-configurable flags in
    /// 3.7; the test fixture's `--fake-ready-ms`).
    pub extra_args: Vec<String>,
}

/// The two directories ComfyUI is told to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfyDirs {
    /// `--base-directory` — ComfyUI's `models/`, `input/`, `temp/`.
    pub base: PathBuf,
    /// `--output-directory` — where generated images land.
    pub output: PathBuf,
}

impl ComfyDirs {
    /// Create both directories if missing. ComfyUI needs them to exist.
    pub(super) fn ensure(&self) -> crate::Result<()> {
        for dir in [&self.base, &self.output] {
            std::fs::create_dir_all(dir).map_err(|e| crate::CoreError::Runtime {
                runtime: RUNTIME_ID.into(),
                message: format!("creating {}: {e}", dir.display()),
            })?;
        }
        Ok(())
    }
}

/// Build the launch command. Always pins `--listen 127.0.0.1` (ADR-008) and
/// keeps ComfyUI headless + quiet.
pub(super) fn build_spawn_spec(launch: &ComfyLaunch, port: u16, dirs: &ComfyDirs) -> SpawnSpec {
    let mut spec = SpawnSpec::new(&launch.program);
    if let Some(main) = &launch.main {
        spec = spec.arg(main.to_string_lossy().into_owned());
        if let Some(parent) = main.parent() {
            spec.cwd = Some(parent.to_path_buf());
        }
    }
    spec = spec
        .arg("--listen")
        .arg(Ipv4Addr::LOCALHOST.to_string())
        .arg("--port")
        .arg(port.to_string())
        .arg("--base-directory")
        .arg(dirs.base.to_string_lossy().into_owned())
        .arg("--output-directory")
        .arg(dirs.output.to_string_lossy().into_owned())
        .arg("--disable-auto-launch")
        .arg("--dont-print-server");
    for extra in &launch.extra_args {
        spec = spec.arg(extra.clone());
    }
    spec
}

/// Resolve how to launch ComfyUI: `AIWM_COMFYUI_PYTHON` (+ optional
/// `AIWM_COMFYUI_DIR`), then a managed install under
/// `<runtimes_dir>/comfyui/<version>/`. `lookup` reads env vars (injected for
/// tests). `None` when nothing is installed.
pub(super) fn resolve_launch(
    runtimes_dir: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<ComfyLaunch> {
    if let Some(raw) = lookup(PYTHON_ENV).filter(|s| !s.is_empty()) {
        let program = PathBuf::from(raw);
        if program.is_file() {
            let main = lookup(DIR_ENV)
                .filter(|s| !s.is_empty())
                .map(|d| PathBuf::from(d).join("main.py"))
                .filter(|p| p.is_file());
            return Some(ComfyLaunch {
                program,
                main,
                extra_args: Vec::new(),
            });
        }
    }

    // Managed: <runtimes_dir>/comfyui/<version>/main.py + .venv python.
    let managed_root = runtimes_dir.join(RUNTIME_ID);
    let entries = std::fs::read_dir(&managed_root).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let main = dir.join("main.py");
        let python = dir.join(PYTHON_EXE);
        if main.is_file() && python.is_file() {
            return Some(ComfyLaunch {
                program: python,
                main: Some(main),
                extra_args: Vec::new(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> ComfyDirs {
        ComfyDirs {
            base: PathBuf::from("C:\\aiwm\\comfyui-data"),
            output: PathBuf::from("C:\\aiwm\\outputs"),
        }
    }

    #[test]
    fn spawn_spec_for_a_real_checkout_runs_python_with_main_and_flags() {
        let launch = ComfyLaunch {
            program: PathBuf::from("C:\\c\\.venv\\Scripts\\python.exe"),
            main: Some(PathBuf::from("C:\\c\\ComfyUI\\main.py")),
            extra_args: vec!["--lowvram".into()],
        };
        let spec = build_spawn_spec(&launch, 48311, &dirs());
        assert_eq!(spec.program, launch.program);
        assert_eq!(spec.cwd, Some(PathBuf::from("C:\\c\\ComfyUI")));
        let joined = spec.args.join(" ");
        assert!(joined.starts_with("C:\\c\\ComfyUI\\main.py"));
        assert!(joined.contains("--listen 127.0.0.1"));
        assert!(joined.contains("--port 48311"));
        assert!(joined.contains("--base-directory C:\\aiwm\\comfyui-data"));
        assert!(joined.contains("--output-directory C:\\aiwm\\outputs"));
        assert!(joined.contains("--disable-auto-launch"));
        assert!(joined.contains("--dont-print-server"));
        assert!(
            joined.ends_with("--lowvram"),
            "extra args come last: {joined}"
        );
    }

    #[test]
    fn spawn_spec_for_the_fixture_has_no_main_arg() {
        let launch = ComfyLaunch {
            program: PathBuf::from("aiwm-fake-comfy.exe"),
            ..ComfyLaunch::default()
        };
        let spec = build_spawn_spec(&launch, 1, &dirs());
        assert!(spec.cwd.is_none());
        assert!(!spec.args.iter().any(|a| a.ends_with("main.py")));
        assert!(spec.args.join(" ").contains("--port 1"));
    }

    #[test]
    fn resolve_prefers_python_env_and_pairs_it_with_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let python = tmp.path().join("python.exe");
        std::fs::write(&python, b"x").unwrap();
        let comfy = tmp.path().join("ComfyUI");
        std::fs::create_dir_all(&comfy).unwrap();
        std::fs::write(comfy.join("main.py"), b"x").unwrap();

        let got = resolve_launch(Path::new("Z:\\nope"), |k| match k {
            PYTHON_ENV => Some(python.clone().into()),
            DIR_ENV => Some(comfy.clone().into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(got.program, python);
        assert_eq!(got.main, Some(comfy.join("main.py")));
    }

    #[test]
    fn resolve_allows_a_standalone_exe_without_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("fake-comfy.exe");
        std::fs::write(&exe, b"x").unwrap();

        let got = resolve_launch(Path::new("Z:\\nope"), |k| {
            (k == PYTHON_ENV).then(|| exe.clone().into())
        })
        .unwrap();
        assert_eq!(got.program, exe);
        assert_eq!(got.main, None);
    }

    #[test]
    fn resolve_finds_a_managed_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(RUNTIME_ID).join("0.34.0");
        std::fs::create_dir_all(dir.join(".venv").join(if cfg!(windows) {
            "Scripts"
        } else {
            "bin"
        }))
        .unwrap();
        std::fs::write(dir.join("main.py"), b"x").unwrap();
        std::fs::write(dir.join(PYTHON_EXE), b"x").unwrap();

        let got = resolve_launch(tmp.path(), |_| None).unwrap();
        assert_eq!(got.main, Some(dir.join("main.py")));
    }

    #[test]
    fn resolve_is_none_when_nothing_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_launch(tmp.path(), |_| None), None);
    }
}
