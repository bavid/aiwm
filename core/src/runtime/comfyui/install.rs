//! Installing the pinned ComfyUI (ADR-002, ADR-018). One curated version.
//!
//! The pipeline: bootstrap `uv` (a single verified static binary), fetch the
//! ComfyUI source at a pinned tag, then build a self-contained venv with `uv`
//! (`uv venv` downloads Python 3.13; then the CUDA `torch` build and ComfyUI's
//! `requirements.txt`). Everything lands under `<runtimes_dir>/comfyui/` so a
//! "repair" is a single delete. The custom node (`ComfyUI-GGUF`) is added in
//! 3.2b.
//!
//! The `uv` subprocess steps go through a [`CmdRunner`] so the orchestration is
//! unit-tested with a recording fake; a real end-to-end run is the smoke test.

use std::path::{Path, PathBuf};

use crate::runtime::download::{download_verified, extract_zip, extract_zip_flat, Archive};
use crate::{CoreError, Result};

use super::RUNTIME_ID;

/// Pinned ComfyUI release tag. Bump together with [`COMFYUI_SRC`] below.
pub const PINNED_TAG: &str = "v0.34.0";
/// Pinned `uv` (astral-sh) version — a single static binary.
pub const UV_VERSION: &str = "0.12.11";
/// Python the venv is built with (`uv venv --python` downloads it).
const PYTHON_VERSION: &str = "3.13";
/// PyTorch wheel index — CUDA 13.0, ComfyUI's current recommendation for
/// RTX 20-series and newer. Needs a recent NVIDIA driver.
const TORCH_INDEX_URL: &str = "https://download.pytorch.org/whl/cu130";

const UV_RELEASE_BASE: &str = "https://github.com/astral-sh/uv/releases/download";
const COMFYUI_ARCHIVE_BASE: &str = "https://github.com/comfyanonymous/ComfyUI/archive/refs/tags";

/// `uv-x86_64-pc-windows-msvc.zip` for [`UV_VERSION`]. SHA-256 from the release's
/// published `.sha256` sidecar; size from the release asset.
const UV_ARCHIVE: Archive<'static> = Archive {
    name: "uv-x86_64-pc-windows-msvc.zip",
    sha256: "e94225dea91e051472847bd6d146d7d66c4f54ffcd1f106678866a99580845f9",
    size: 16_996_332,
};

/// The GitHub source archive for [`PINNED_TAG`]. GitHub does **not** publish a
/// digest for source archives, so this SHA-256 is the one we computed while
/// curating the pin. A regeneration on GitHub's side surfaces as a
/// `SHA-256 mismatch` — verify the new archive by hand, then bump this.
const COMFYUI_SRC: Archive<'static> = Archive {
    name: "v0.34.0.zip",
    sha256: "7e0d0afd8e9b18d6df2b1401bc579187e36cebb5a0be69c3b2580bc48eca8a2a",
    size: 12_613_058,
};

const UV_EXE: &str = if cfg!(windows) { "uv.exe" } else { "uv" };
const VENV_PYTHON: &str = if cfg!(windows) {
    ".venv\\Scripts\\python.exe"
} else {
    ".venv/bin/python"
};

/// `<runtimes_dir>/comfyui/` — everything ComfyUI-related lives under here so a
/// "repair" is a single `rm -rf`.
pub fn install_root(runtimes_dir: &Path) -> PathBuf {
    runtimes_dir.join(RUNTIME_ID)
}

/// The bootstrapped `uv` executable.
pub fn uv_bin(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join("uv").join(UV_EXE)
}

/// The ComfyUI checkout: `main.py`, `requirements.txt`, `.venv/`, `custom_nodes/`.
pub fn comfy_home(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join(PINNED_TAG)
}

fn venv_python(runtimes_dir: &Path) -> PathBuf {
    comfy_home(runtimes_dir).join(VENV_PYTHON)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Downloading,
    Extracting,
    CreatingVenv,
    InstallingTorch,
    InstallingDeps,
}

/// Bytes to fetch for the toolchain — surfaced in the UI. The venv build adds
/// gigabytes of wheels whose size we cannot know up front.
pub const TOOLCHAIN_DOWNLOAD_BYTES: u64 = UV_ARCHIVE.size + COMFYUI_SRC.size;

// --- subprocess boundary ----------------------------------------------------

/// Runs one external command. Injected so the install orchestration is testable
/// without a real Python toolchain.
#[async_trait::async_trait]
pub trait CmdRunner: Send + Sync {
    async fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()>;
}

/// The real runner: `tokio::process::Command`, non-zero exit → error with the
/// tail of stderr.
pub struct SystemRunner;

#[async_trait::async_trait]
impl CmdRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| comfy_install_err(format!("run {}: {e}", program.display())))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        Err(comfy_install_err(format!(
            "{} {} exited with {}:\n{tail}",
            program.file_name().and_then(|n| n.to_str()).unwrap_or("uv"),
            args.first().copied().unwrap_or(""),
            out.status
        )))
    }
}

// --- the installer --------------------------------------------------------

/// Install the pinned ComfyUI into `runtimes_dir`. Idempotent — a complete
/// existing install returns immediately. `on_progress` gets
/// `(phase, done_bytes, total_bytes)` for the download phases; the `uv` phases
/// report `(phase, 0, 0)` (no byte total available).
pub async fn install<F>(
    runtimes_dir: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    on_progress: F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    install_with(
        runtimes_dir,
        offline,
        runner,
        &format!("{UV_RELEASE_BASE}/{UV_VERSION}"),
        COMFYUI_ARCHIVE_BASE,
        &UV_ARCHIVE,
        &COMFYUI_SRC,
        on_progress,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn install_with<F>(
    runtimes_dir: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    uv_base: &str,
    comfy_base: &str,
    uv_archive: &Archive<'_>,
    comfy_archive: &Archive<'_>,
    on_progress: F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let root = install_root(runtimes_dir);
    let uv = uv_bin(runtimes_dir);
    let home = comfy_home(runtimes_dir);
    let py = venv_python(runtimes_dir);

    if py.is_file() && home.join("main.py").is_file() {
        return Ok(()); // already installed
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download ComfyUI. Turn it off, or install ComfyUI \
             manually and point AIWM_COMFYUI_PYTHON / AIWM_COMFYUI_DIR at it."
                .into(),
        ));
    }

    fetch_toolchain(
        &root,
        &uv,
        &home,
        uv_base,
        comfy_base,
        uv_archive,
        comfy_archive,
        &on_progress,
    )
    .await?;
    build_venv(&root, &uv, &home, &py, runner, &on_progress).await?;

    if !py.is_file() {
        return Err(comfy_install_err(
            "install finished but the venv python is missing — the uv steps did not complete",
        ));
    }
    tracing::info!(tag = PINNED_TAG, home = %home.display(), "ComfyUI installed");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn fetch_toolchain<F>(
    root: &Path,
    uv: &Path,
    home: &Path,
    uv_base: &str,
    comfy_base: &str,
    uv_archive: &Archive<'_>,
    comfy_archive: &Archive<'_>,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let staging = root.join(".download");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| comfy_install_err(format!("create {}: {e}", staging.display())))?;
    let total = uv_archive.size + comfy_archive.size;

    if !uv.is_file() {
        let zip = staging.join(uv_archive.name);
        download_verified(
            &format!("{uv_base}/{}", uv_archive.name),
            uv_archive,
            &zip,
            |n| {
                on_progress(InstallPhase::Downloading, n, total);
            },
        )
        .await?;
        on_progress(InstallPhase::Extracting, uv_archive.size, total);
        extract_zip(&zip, &root.join("uv")).await?;
    }

    if !home.join("main.py").is_file() {
        let zip = staging.join(comfy_archive.name);
        download_verified(
            &format!("{comfy_base}/{}", comfy_archive.name),
            comfy_archive,
            &zip,
            |n| on_progress(InstallPhase::Downloading, uv_archive.size + n, total),
        )
        .await?;
        on_progress(InstallPhase::Extracting, total, total);
        extract_zip_flat(&zip, home).await?;
    }

    let _ = tokio::fs::remove_dir_all(&staging).await;
    Ok(())
}

async fn build_venv<F>(
    root: &Path,
    uv: &Path,
    home: &Path,
    py: &Path,
    runner: &dyn CmdRunner,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    // Keep uv's managed Python + wheel cache inside our tree so a repair is clean.
    let py_dir = root.join("python").to_string_lossy().into_owned();
    let cache_dir = root.join("uv-cache").to_string_lossy().into_owned();
    let env = [
        ("UV_PYTHON_INSTALL_DIR", py_dir.as_str()),
        ("UV_CACHE_DIR", cache_dir.as_str()),
        ("UV_NO_PROGRESS", "1"),
    ];
    let venv = home.join(".venv");
    let (venv_s, home_s, reqs_s) = (
        venv.to_string_lossy().into_owned(),
        home.to_string_lossy().into_owned(),
        home.join("requirements.txt").to_string_lossy().into_owned(),
    );
    let py_s = py.to_string_lossy().into_owned();

    if !py.is_file() {
        on_progress(InstallPhase::CreatingVenv, 0, 0);
        runner
            .run(uv, &["venv", "--python", PYTHON_VERSION, &venv_s], &env)
            .await?;
    }

    on_progress(InstallPhase::InstallingTorch, 0, 0);
    runner
        .run(
            uv,
            &[
                "pip",
                "install",
                "--python",
                &py_s,
                "--index-url",
                TORCH_INDEX_URL,
                "torch",
                "torchvision",
                "torchaudio",
            ],
            &env,
        )
        .await?;

    on_progress(InstallPhase::InstallingDeps, 0, 0);
    runner
        .run(
            uv,
            &["pip", "install", "--python", &py_s, "-r", &reqs_s],
            &env,
        )
        .await?;

    let _ = home_s; // reserved for the custom-node step in 3.2b
    Ok(())
}

fn comfy_install_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("ComfyUI setup: {msg}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use sha2::{Digest, Sha256};

    use crate::runtime::download::hex;
    use crate::runtime::download::test_support::make_zip;

    use super::*;

    /// Records every command it is asked to run; always succeeds.
    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl CmdRunner for RecordingRunner {
        async fn run(&self, program: &Path, args: &[&str], _env: &[(&str, &str)]) -> Result<()> {
            let mut call = vec![program.file_name().unwrap().to_string_lossy().into_owned()];
            call.extend(args.iter().map(|s| s.to_string()));
            self.calls.lock().unwrap().push(call);
            Ok(())
        }
    }

    struct FailingRunner;

    #[async_trait::async_trait]
    impl CmdRunner for FailingRunner {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            Err(comfy_install_err(format!(
                "boom running uv {}",
                args.first().unwrap()
            )))
        }
    }

    fn archives(uv_zip: &[u8], src_zip: &[u8]) -> (String, String) {
        (hex(&Sha256::digest(uv_zip)), hex(&Sha256::digest(src_zip)))
    }

    async fn serve(uv_zip: Vec<u8>, src_zip: Vec<u8>) -> u16 {
        let app = Router::new()
            .route(
                "/uv/uv.zip",
                get(move || {
                    let b = uv_zip.clone();
                    async move { Body::from(b) }
                }),
            )
            .route(
                "/src/comfy.zip",
                get(move || {
                    let b = src_zip.clone();
                    async move { Body::from(b) }
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        port
    }

    #[tokio::test]
    async fn refuses_in_offline_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install(tmp.path(), true, &RecordingRunner::default(), |_, _, _| {})
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
        std::fs::write(comfy_home(tmp.path()).join("main.py"), b"# comfy").unwrap();

        let runner = RecordingRunner::default();
        install(tmp.path(), true, &runner, |_, _, _| {})
            .await
            .unwrap();
        assert!(runner.calls.lock().unwrap().is_empty(), "nothing to do");
    }

    #[tokio::test]
    async fn full_install_fetches_then_runs_the_uv_steps_in_order() {
        let uv_zip = make_zip(&[("uv.exe", b"MZ uv")]);
        let src_zip = make_zip(&[
            ("ComfyUI-0.34.0/main.py", b"# comfy"),
            ("ComfyUI-0.34.0/requirements.txt", b"torch\ntorchsde\n"),
        ]);
        let (uv_sha, src_sha) = archives(&uv_zip, &src_zip);
        let port = serve(uv_zip.clone(), src_zip.clone()).await;

        let uv_archive = Archive {
            name: "uv.zip",
            sha256: &uv_sha,
            size: uv_zip.len() as u64,
        };
        let src_archive = Archive {
            name: "comfy.zip",
            sha256: &src_sha,
            size: src_zip.len() as u64,
        };

        let tmp = tempfile::tempdir().unwrap();
        // The fake runner does not create the venv python, so add it after the
        // "venv" call would have run — emulate by pre-creating it before deps.
        let runner = VenvCreatingRunner {
            py: venv_python(tmp.path()),
            calls: Mutex::new(Vec::new()),
        };
        let phases: Mutex<Vec<InstallPhase>> = Mutex::new(Vec::new());

        install_with(
            tmp.path(),
            false,
            &runner,
            &format!("http://127.0.0.1:{port}/uv"),
            &format!("http://127.0.0.1:{port}/src"),
            &uv_archive,
            &src_archive,
            |p, _, _| phases.lock().unwrap().push(p),
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(uv_bin(tmp.path())).unwrap(), b"MZ uv");
        assert!(comfy_home(tmp.path()).join("main.py").is_file());

        let calls = runner.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 3, "venv, torch, requirements: {calls:?}");
        assert_eq!(calls[0][0], "venv");
        assert!(
            calls[1].contains(&"--index-url".to_string())
                && calls[1].contains(&"torch".to_string())
        );
        assert!(calls[2].contains(&"-r".to_string()));

        let phases = phases.into_inner().unwrap();
        for expected in [
            InstallPhase::Extracting,
            InstallPhase::CreatingVenv,
            InstallPhase::InstallingTorch,
            InstallPhase::InstallingDeps,
        ] {
            assert!(
                phases.contains(&expected),
                "missing {expected:?} in {phases:?}"
            );
        }
    }

    /// Like [`RecordingRunner`] but drops a fake venv python on the first call,
    /// so the "already there?" guards in `build_venv` behave.
    struct VenvCreatingRunner {
        py: PathBuf,
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl CmdRunner for VenvCreatingRunner {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            if args.first() == Some(&"venv") {
                std::fs::create_dir_all(self.py.parent().unwrap()).unwrap();
                std::fs::write(&self.py, b"py").unwrap();
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_failing_uv_step_surfaces_the_error() {
        let uv_zip = make_zip(&[("uv.exe", b"MZ")]);
        let src_zip = make_zip(&[
            ("ComfyUI-0.34.0/main.py", b"x"),
            ("ComfyUI-0.34.0/requirements.txt", b"t"),
        ]);
        let (uv_sha, src_sha) = archives(&uv_zip, &src_zip);
        let port = serve(uv_zip.clone(), src_zip.clone()).await;
        let tmp = tempfile::tempdir().unwrap();

        let err = install_with(
            tmp.path(),
            false,
            &FailingRunner,
            &format!("http://127.0.0.1:{port}/uv"),
            &format!("http://127.0.0.1:{port}/src"),
            &Archive {
                name: "uv.zip",
                sha256: &uv_sha,
                size: uv_zip.len() as u64,
            },
            &Archive {
                name: "comfy.zip",
                sha256: &src_sha,
                size: src_zip.len() as u64,
            },
            |_, _, _| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("boom running uv"));
    }

    #[tokio::test]
    async fn a_bad_source_hash_aborts_before_the_uv_steps() {
        let uv_zip = make_zip(&[("uv.exe", b"MZ")]);
        let (uv_sha, _) = archives(&uv_zip, b"");
        let port = serve(uv_zip.clone(), b"corrupt".to_vec()).await;
        let tmp = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::default();

        let err = install_with(
            tmp.path(),
            false,
            &runner,
            &format!("http://127.0.0.1:{port}/uv"),
            &format!("http://127.0.0.1:{port}/src"),
            &Archive {
                name: "uv.zip",
                sha256: &uv_sha,
                size: uv_zip.len() as u64,
            },
            &Archive {
                name: "comfy.zip",
                sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                size: 7,
            },
            |_, _, _| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256 mismatch"));
        assert!(runner.calls.lock().unwrap().is_empty(), "no uv steps ran");
        assert!(uv_bin(tmp.path()).is_file(), "uv still landed");
    }

    /// The real thing: run the whole pinned install against GitHub + PyPI.
    /// `#[ignore]` — downloads gigabytes. Run explicitly for the slice smoke:
    /// `cargo test -p aiwm-core --lib comfyui::install::tests::real -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "network + disk: downloads uv, ComfyUI source and a CUDA torch build"]
    async fn real_pinned_install() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path(), false, &SystemRunner, |phase, done, total| {
            eprintln!("  {phase:?} {done}/{total}");
        })
        .await
        .expect("the pinned ComfyUI install should succeed on this machine");

        let home = comfy_home(tmp.path());
        assert!(venv_python(tmp.path()).is_file());
        assert!(home.join("main.py").is_file());
        // torch imports (proves the wheel + Python match; not the GPU driver).
        SystemRunner
            .run(
                &venv_python(tmp.path()),
                &["-c", "import torch, sys; print(torch.__version__)"],
                &[],
            )
            .await
            .expect("torch should import in the fresh venv");
    }
}
