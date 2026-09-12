//! Installing the pinned Hermes Agent (Phase 5.4b) into `<hermes_root>/`.
//!
//! Lighter than ComfyUI — no torch, no source checkout: bootstrap `uv`, then
//! `uv venv` + `uv pip install hermes-agent==<ver>`, then a best-effort
//! `hermes postinstall` (pulls node / a headless browser / ripgrep / ffmpeg —
//! big; only `ripgrep` matters for a coding agent, so a failure here does not
//! fail the install). Everything under `<hermes_root>/` so a repair is one
//! delete.
//!
//! The `uv` steps go through the shared [`CmdRunner`] so the orchestration is
//! unit-tested with a recording fake; a real end-to-end run is the smoke test.

use std::path::{Path, PathBuf};

use crate::runtime::download::{ensure_uv, Archive, CmdRunner, UV_ARCHIVE, UV_RELEASE_BASE};
use crate::{CoreError, Result};

/// Pinned `hermes-agent` (PyPI). Bump deliberately — `postinstall` asset
/// versions move with it.
pub const PINNED_VERSION: &str = "0.19.0";
/// Python the venv is built with (`uv venv --python` downloads it).
const PYTHON_VERSION: &str = "3.13";

fn venv_bin(root: &Path) -> PathBuf {
    root.join(".venv")
        .join(if cfg!(windows) { "Scripts" } else { "bin" })
}

/// The venv's Python — the "is it installed?" marker.
pub fn venv_python(root: &Path) -> PathBuf {
    venv_bin(root).join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    })
}

/// The `hermes` console script inside the venv.
pub fn hermes_bin(root: &Path) -> PathBuf {
    venv_bin(root).join(if cfg!(windows) {
        "hermes.exe"
    } else {
        "hermes"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    /// Fetching the pinned `uv` toolchain.
    Downloading,
    CreatingVenv,
    InstallingHermes,
    /// `hermes postinstall` — node / browser / ripgrep / ffmpeg.
    PostInstall,
}

/// Progress of a Hermes install. Serialized for the `runtimes/hermes` status.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallStatus {
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

fn install_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("Hermes setup: {msg}"))
}

/// Install the pinned Hermes into `hermes_root`. Idempotent — a complete install
/// returns immediately. `on_progress` gets `(phase, done_bytes, total_bytes)`;
/// only the `Downloading` phase has a byte total.
pub async fn install<F>(
    hermes_root: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    on_progress: F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    install_with(
        hermes_root,
        offline,
        runner,
        UV_RELEASE_BASE,
        &UV_ARCHIVE,
        PINNED_VERSION,
        &on_progress,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn install_with<F>(
    root: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    uv_base: &str,
    uv_archive: &Archive<'_>,
    version: &str,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let py = venv_python(root);
    let hermes = hermes_bin(root);
    if py.is_file() && hermes.is_file() {
        return Ok(()); // already installed
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download Hermes. Turn it off, or install \
             `hermes-agent` yourself and point AIWM_HERMES_PATH at the `hermes` binary."
                .into(),
        ));
    }

    let total = uv_archive.size;
    let uv = ensure_uv(uv_base, uv_archive, root, |n| {
        on_progress(InstallPhase::Downloading, n, total);
    })
    .await?;

    // Keep uv's managed Python + wheel cache inside our tree so a repair is clean.
    let py_dir = root.join("python").to_string_lossy().into_owned();
    let cache_dir = root.join("uv-cache").to_string_lossy().into_owned();
    let env = [
        ("UV_PYTHON_INSTALL_DIR", py_dir.as_str()),
        ("UV_CACHE_DIR", cache_dir.as_str()),
        ("UV_NO_PROGRESS", "1"),
    ];
    let venv_s = root.join(".venv").to_string_lossy().into_owned();
    let py_s = py.to_string_lossy().into_owned();

    if !py.is_file() {
        on_progress(InstallPhase::CreatingVenv, 0, 0);
        runner
            .run(&uv, &["venv", "--python", PYTHON_VERSION, &venv_s], &env)
            .await?;
    }

    on_progress(InstallPhase::InstallingHermes, 0, 0);
    let pkg = format!("hermes-agent=={version}");
    // `aiohttp` alongside it: AIWM drives Hermes over its HTTP "API Server"
    // component (API_SERVER_ENABLED=1), which Hermes's own install does not
    // pull in by default -- without it the API server never starts, so our
    // `GET /health` readiness check never responds and every session open
    // times out (found via the first real hands-on Hermes run).
    runner
        .run(
            &uv,
            &["pip", "install", "--python", &py_s, &pkg, "aiohttp"],
            &env,
        )
        .await?;

    if !py.is_file() || !hermes.is_file() {
        return Err(install_err(
            "pip finished but the venv python or the `hermes` entrypoint is missing",
        ));
    }

    // Best effort — the browser / ffmpeg assets are disabled by our forced
    // config anyway; only ripgrep would be missed, and the shell fallback covers
    // search. `HERMES_HOME` keeps the assets inside our tree.
    on_progress(InstallPhase::PostInstall, 0, 0);
    let post_home = root.join("postinstall-home").to_string_lossy().into_owned();
    if let Err(e) = runner
        .run(
            &hermes,
            &["postinstall"],
            &[("HERMES_HOME", post_home.as_str())],
        )
        .await
    {
        tracing::warn!(error = %e, "hermes postinstall failed — search / browser tools may be limited");
    }

    tracing::info!(version, root = %root.display(), "Hermes installed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::runtime::download::{hex, test_support::make_zip};

    const UV_EXE: &str = if cfg!(windows) { "uv.exe" } else { "uv" };
    const HERMES_EXE: &str = if cfg!(windows) {
        "hermes.exe"
    } else {
        "hermes"
    };

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<Vec<String>>>,
        fail_on: Option<&'static str>,
        drop_venv_at: Option<PathBuf>,
    }

    #[async_trait::async_trait]
    impl CmdRunner for Recorder {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            if self.fail_on == args.first().copied() {
                return Err(install_err(format!("boom on {}", args.first().unwrap())));
            }
            if args.first() == Some(&"venv") {
                if let Some(py) = &self.drop_venv_at {
                    std::fs::create_dir_all(py.parent().unwrap()).unwrap();
                    std::fs::write(py, b"py").unwrap();
                    std::fs::write(py.with_file_name(HERMES_EXE), b"hermes").unwrap();
                }
            }
            Ok(())
        }
    }

    /// Serve one `uv.zip` from a local server; returns `(base_url, sha_hex, size)`.
    async fn serve_uv() -> (String, String, u64) {
        let zip = make_zip(&[(UV_EXE, b"MZ uv")]);
        let (sha, size) = (hex(&Sha256::digest(&zip)), zip.len() as u64);
        let app = Router::new().route(
            "/uv/uv.zip",
            get(move || {
                let b = zip.clone();
                async move { Body::from(b) }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}/uv"), sha, size)
    }

    #[tokio::test]
    async fn refuses_in_offline_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install(tmp.path(), true, &Recorder::default(), |_, _, _| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("offline mode"));
    }

    #[tokio::test]
    async fn idempotent_when_already_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let py = venv_python(tmp.path());
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, b"py").unwrap();
        std::fs::write(hermes_bin(tmp.path()), b"hermes").unwrap();

        let r = Recorder::default();
        install(tmp.path(), true, &r, |_, _, _| {}).await.unwrap();
        assert!(r.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn full_install_bootstraps_uv_then_venv_then_pip_and_postinstall() {
        let (base, sha, size) = serve_uv().await;
        let archive = Archive {
            name: "uv.zip",
            sha256: &sha,
            size,
        };
        let tmp = tempfile::tempdir().unwrap();
        let r = Recorder {
            drop_venv_at: Some(venv_python(tmp.path())),
            ..Recorder::default()
        };
        let phases = Mutex::new(Vec::new());

        install_with(
            tmp.path(),
            false,
            &r,
            &base,
            &archive,
            "0.19.0",
            &|p, _, _| phases.lock().unwrap().push(p),
        )
        .await
        .unwrap();

        assert!(hermes_bin(tmp.path()).is_file());
        let calls = r.calls.into_inner().unwrap();
        assert_eq!(calls[0][0], "venv");
        assert!(
            calls[1].contains(&"hermes-agent==0.19.0".to_string()),
            "{calls:?}"
        );
        // AIWM talks to Hermes over its HTTP "API Server" component
        // (API_SERVER_ENABLED=1), which needs `aiohttp` -- Hermes's own
        // install doesn't pull it in by default, and without it the API
        // server never starts, so `GET /health` never responds and every
        // session open times out with "hermes gateway did not become ready".
        assert!(calls[1].contains(&"aiohttp".to_string()), "{calls:?}");
        assert_eq!(calls[2], vec!["postinstall".to_string()]);
        let phases = phases.into_inner().unwrap();
        assert!(phases.contains(&InstallPhase::CreatingVenv));
        assert!(phases.contains(&InstallPhase::InstallingHermes));
        assert!(phases.contains(&InstallPhase::PostInstall));
    }

    #[tokio::test]
    async fn a_failing_pip_step_surfaces_the_error() {
        let (base, sha, size) = serve_uv().await;
        let archive = Archive {
            name: "uv.zip",
            sha256: &sha,
            size,
        };
        let tmp = tempfile::tempdir().unwrap();
        let r = Recorder {
            fail_on: Some("pip"),
            ..Recorder::default()
        };
        let err = install_with(
            tmp.path(),
            false,
            &r,
            &base,
            &archive,
            "0.19.0",
            &|_, _, _| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("boom on pip"), "{err}");
    }

    #[tokio::test]
    async fn a_failing_postinstall_does_not_fail_the_install() {
        let (base, sha, size) = serve_uv().await;
        let archive = Archive {
            name: "uv.zip",
            sha256: &sha,
            size,
        };
        let tmp = tempfile::tempdir().unwrap();
        let r = Recorder {
            fail_on: Some("postinstall"),
            drop_venv_at: Some(venv_python(tmp.path())),
            ..Recorder::default()
        };
        install_with(
            tmp.path(),
            false,
            &r,
            &base,
            &archive,
            "0.19.0",
            &|_, _, _| {},
        )
        .await
        .expect("postinstall is best-effort");
    }

    /// The real thing: install the pinned Hermes against PyPI. `#[ignore]` —
    /// ~120 deps + `hermes postinstall` pulls node / a browser / ffmpeg.
    /// `cargo test -p aiwm-core --lib agent::hermes::install::tests::real_pinned_install -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "network + disk: ~120 PyPI deps + hermes postinstall assets"]
    async fn real_pinned_install() {
        use crate::runtime::download::SystemRunner;
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path(), false, &SystemRunner, |phase, done, total| {
            eprintln!("  {phase:?} {done}/{total}");
        })
        .await
        .expect("the pinned Hermes install should succeed on this machine");
        assert!(venv_python(tmp.path()).is_file());
        assert!(hermes_bin(tmp.path()).is_file());
    }
}
