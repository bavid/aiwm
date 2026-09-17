//! Installing the pinned `ai-toolkit` trainer (spec "Task 6"). One curated
//! commit, one self-contained venv, exactly like ComfyUI's installer
//! (`runtime::comfyui::install`) — the pipeline, the verified downloads and
//! the [`CmdRunner`] subprocess boundary are all shared with it.
//!
//! Everything lands under `<runtimes_dir>/ai-toolkit/` so a "repair" is a
//! single delete:
//!
//! ```text
//! ai-toolkit/
//!   uv/uv.exe            the bootstrapped uv
//!   src/run.py           the pinned ai-toolkit checkout
//!   venv/Scripts/…       its own Python 3.12 environment
//!   .installed-<commit>  written last — the completion marker
//! ```
//!
//! The marker carries the commit id, so bumping [`PINNED_COMMIT`] makes an
//! existing install look incomplete and it is rebuilt from the new archive.

use std::path::{Path, PathBuf};

use crate::runtime::download::{
    download_verified, ensure_uv, extract_zip_flat, Archive, CmdRunner, UV_ARCHIVE, UV_RELEASE_BASE,
};
use crate::{CoreError, Result};

/// The pinned `ostris/ai-toolkit` commit. Bump together with
/// [`PINNED_ARCHIVE`] below — and with `training::config` /
/// `training::progress`, which parse this commit's output format.
pub const PINNED_COMMIT: &str = "e65c4d0fb69251e692390574c49873297dc4bae5";

/// Python the venv is built with (`uv python install` fetches it).
pub const PYTHON_VERSION: &str = "3.12";

/// The pinned PyTorch trio, installed before `requirements.txt` so the CUDA
/// build wins over whatever plain `torch` the requirements would resolve.
pub const TORCH_SPEC: &[&str] = &["torch==2.13.0", "torchvision==0.28.0", "torchaudio==2.11.0"];

/// PyTorch wheel index — CUDA 13.0, the same build ComfyUI's installer uses.
pub const TORCH_INDEX_URL: &str = "https://download.pytorch.org/whl/cu130";

/// `<runtimes_dir>/ai-toolkit` — also the registry/runtime id.
const RUNTIME_DIR: &str = "ai-toolkit";

const ARCHIVE_BASE: &str = "https://github.com/ostris/ai-toolkit/archive";

/// The GitHub source archive for [`PINNED_COMMIT`]. GitHub publishes no digest
/// for source archives, so this SHA-256 + size are the ones we computed while
/// curating the pin; a regeneration on GitHub's side surfaces as a
/// `SHA-256 mismatch` — verify the new archive by hand, then bump this.
const PINNED_ARCHIVE: Archive<'static> = Archive {
    name: "e65c4d0fb69251e692390574c49873297dc4bae5.zip",
    sha256: "6d4c67fa49b37d7dc2d5b237e571ae09620c4416605d1ef88aafd03ab5e957e5",
    size: 35_740_120,
};

const VENV_PYTHON: &str = if cfg!(windows) {
    "Scripts\\python.exe"
} else {
    "bin/python"
};

/// `<runtimes_dir>/ai-toolkit/` — everything trainer-related lives under here.
pub fn install_root(runtimes_dir: &Path) -> PathBuf {
    runtimes_dir.join(RUNTIME_DIR)
}

/// The bootstrapped `uv` executable.
pub fn uv_bin(runtimes_dir: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "uv.exe" } else { "uv" };
    install_root(runtimes_dir).join("uv").join(exe)
}

/// The `ai-toolkit` checkout: `run.py`, `requirements.txt`, `config/`, …
pub fn source_dir(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join("src")
}

/// The trainer's own virtual environment.
pub fn venv_dir(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join("venv")
}

/// The venv's Python interpreter — what a run is actually launched with.
pub fn venv_python(runtimes_dir: &Path) -> PathBuf {
    venv_dir(runtimes_dir).join(VENV_PYTHON)
}

/// `ai-toolkit`'s headless entry point.
pub fn run_py(runtimes_dir: &Path) -> PathBuf {
    source_dir(runtimes_dir).join("run.py")
}

/// Written last, once every step succeeded: `.installed-<commit>`.
pub fn marker_file(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join(format!(".installed-{PINNED_COMMIT}"))
}

/// A complete install of exactly [`PINNED_COMMIT`]: entry point, interpreter
/// and the commit marker. Any one missing means "install again".
pub fn is_installed(runtimes_dir: &Path) -> bool {
    run_py(runtimes_dir).is_file()
        && venv_python(runtimes_dir).is_file()
        && marker_file(runtimes_dir).is_file()
}

/// Progress of [`install`]. Mirrors `comfyui::install::InstallPhase` (same
/// variants, same UI treatment) but is its own type: the trainer has an
/// explicit `uv python install` step ComfyUI has no equivalent of, and none of
/// ComfyUI's custom-node phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainerInstallPhase {
    Downloading,
    Extracting,
    InstallingPython,
    CreatingVenv,
    InstallingTorch,
    InstallingDeps,
}

/// Bytes to fetch for the toolchain — surfaced in the UI. The venv build adds
/// gigabytes of wheels whose size we cannot know up front.
pub const TOOLCHAIN_DOWNLOAD_BYTES: u64 = UV_ARCHIVE.size + PINNED_ARCHIVE.size;

/// The pinned archives + their base URLs; tests substitute a local server.
struct FetchSpec<'a> {
    uv_base: &'a str,
    src_base: &'a str,
    uv: &'a Archive<'a>,
    src: &'a Archive<'a>,
}

const PINNED_SPEC: FetchSpec<'static> = FetchSpec {
    uv_base: UV_RELEASE_BASE,
    src_base: ARCHIVE_BASE,
    uv: &UV_ARCHIVE,
    src: &PINNED_ARCHIVE,
};

pub(crate) fn trainer_install_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("trainer setup: {msg}"))
}

/// Install the pinned `ai-toolkit` into `runtimes_dir`. Idempotent — a
/// complete existing install returns immediately. `on_progress` gets
/// `(phase, done_bytes, total_bytes)` for the download phases; the `uv`
/// phases report `(phase, 0, 0)` (no byte total available).
pub async fn install<F>(
    runtimes_dir: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    on_progress: F,
) -> Result<()>
where
    F: Fn(TrainerInstallPhase, u64, u64) + Send + Sync,
{
    install_with(runtimes_dir, offline, runner, &PINNED_SPEC, on_progress).await
}

async fn install_with<F>(
    runtimes_dir: &Path,
    offline: bool,
    runner: &dyn CmdRunner,
    spec: &FetchSpec<'_>,
    on_progress: F,
) -> Result<()>
where
    F: Fn(TrainerInstallPhase, u64, u64) + Send + Sync,
{
    if is_installed(runtimes_dir) {
        return Ok(());
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download ai-toolkit. Turn it off, then set the \
             trainer up again."
                .into(),
        ));
    }

    let root = install_root(runtimes_dir);
    let uv = uv_bin(runtimes_dir);
    let src = source_dir(runtimes_dir);
    let py = venv_python(runtimes_dir);

    fetch_sources(&root, &src, spec, &on_progress).await?;
    build_venv(
        &root,
        &uv,
        &src,
        &venv_dir(runtimes_dir),
        &py,
        runner,
        &on_progress,
    )
    .await?;

    if !py.is_file() || !run_py(runtimes_dir).is_file() {
        return Err(trainer_install_err(
            "install finished but run.py or the venv python is missing — the steps did not \
             complete",
        ));
    }
    // Written last: the marker is what makes `is_installed` true, so a crash
    // anywhere above leaves the install visibly incomplete rather than broken.
    tokio::fs::write(marker_file(runtimes_dir), PINNED_COMMIT)
        .await
        .map_err(|e| trainer_install_err(format!("write the install marker: {e}")))?;
    tracing::info!(commit = PINNED_COMMIT, src = %src.display(), "ai-toolkit installed");
    Ok(())
}

/// Fetch + unpack the verified toolchain: `uv` (via [`ensure_uv`]) and the
/// pinned `ai-toolkit` source, flattened out of its `<repo>-<sha>/` wrapper.
async fn fetch_sources<F>(
    root: &Path,
    src: &Path,
    spec: &FetchSpec<'_>,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(TrainerInstallPhase, u64, u64) + Send + Sync,
{
    let staging = root.join(".download");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| trainer_install_err(format!("create {}: {e}", staging.display())))?;
    let total = spec.uv.size + spec.src.size;
    let mut done = 0u64;

    ensure_uv(spec.uv_base, spec.uv, root, |n| {
        on_progress(TrainerInstallPhase::Downloading, done + n, total);
    })
    .await?;
    done += spec.uv.size;

    if !src.join("run.py").is_file() {
        let zip = staging.join(spec.src.name);
        download_verified(
            &format!("{}/{}", spec.src_base, spec.src.name),
            spec.src,
            &zip,
            |n| on_progress(TrainerInstallPhase::Downloading, done + n, total),
        )
        .await?;
        on_progress(TrainerInstallPhase::Extracting, total, total);
        extract_zip_flat(&zip, src).await?;
    } else {
        on_progress(TrainerInstallPhase::Extracting, total, total);
    }

    let _ = tokio::fs::remove_dir_all(&staging).await;
    Ok(())
}

/// Build the trainer's own venv: managed Python, then the pinned CUDA torch
/// trio *before* `requirements.txt` so the CUDA wheels win over the plain
/// `torch` the requirements would otherwise resolve.
async fn build_venv<F>(
    root: &Path,
    uv: &Path,
    src: &Path,
    venv: &Path,
    py: &Path,
    runner: &dyn CmdRunner,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(TrainerInstallPhase, u64, u64) + Send + Sync,
{
    // Keep uv's managed Python + wheel cache inside our tree so a repair is
    // clean — same layout ComfyUI's installer uses.
    let py_dir = root.join("python").to_string_lossy().into_owned();
    let cache_dir = root.join("uv-cache").to_string_lossy().into_owned();
    let env = [
        ("UV_PYTHON_INSTALL_DIR", py_dir.as_str()),
        ("UV_CACHE_DIR", cache_dir.as_str()),
        ("UV_NO_PROGRESS", "1"),
    ];
    let venv_s = venv.to_string_lossy().into_owned();
    let py_s = py.to_string_lossy().into_owned();
    let reqs_s = src.join("requirements.txt").to_string_lossy().into_owned();

    on_progress(TrainerInstallPhase::InstallingPython, 0, 0);
    runner
        .run(uv, &["python", "install", PYTHON_VERSION], &env)
        .await?;

    if !py.is_file() {
        on_progress(TrainerInstallPhase::CreatingVenv, 0, 0);
        runner
            .run(uv, &["venv", "--python", PYTHON_VERSION, &venv_s], &env)
            .await?;
    }

    on_progress(TrainerInstallPhase::InstallingTorch, 0, 0);
    let mut torch = vec!["pip", "install", "--python", &py_s];
    torch.extend_from_slice(TORCH_SPEC);
    torch.extend_from_slice(&["--index-url", TORCH_INDEX_URL]);
    runner.run(uv, &torch, &env).await?;

    on_progress(TrainerInstallPhase::InstallingDeps, 0, 0);
    runner
        .run(
            uv,
            &["pip", "install", "--python", &py_s, "-r", &reqs_s],
            &env,
        )
        .await?;
    Ok(())
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
    use crate::runtime::download::SystemRunner;

    use super::*;

    /// Records every command; always succeeds, and drops a fake venv python on
    /// the `venv` call so the "already there?" guards behave.
    struct VenvCreatingRunner {
        py: PathBuf,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl VenvCreatingRunner {
        fn new(runtimes_dir: &Path) -> Self {
            Self {
                py: venv_python(runtimes_dir),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl CmdRunner for VenvCreatingRunner {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(args.iter().map(|s| s.to_string()).collect());
            if args.first() == Some(&"venv") {
                let parent = self.py.parent().expect("venv python has a parent");
                std::fs::create_dir_all(parent).expect("create the fake venv dir");
                std::fs::write(&self.py, b"py").expect("write the fake venv python");
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl CmdRunner for RecordingRunner {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(args.iter().map(|s| s.to_string()).collect());
            Ok(())
        }
    }

    struct FailingRunner;

    #[async_trait::async_trait]
    impl CmdRunner for FailingRunner {
        async fn run(&self, _p: &Path, args: &[&str], _e: &[(&str, &str)]) -> Result<()> {
            Err(trainer_install_err(format!(
                "boom running uv {}",
                args.first().copied().unwrap_or("?")
            )))
        }
    }

    fn sha(b: &[u8]) -> String {
        hex(&Sha256::digest(b))
    }

    /// The two archives, served from one local server — never the network.
    struct Fixtures {
        port: u16,
        uv_zip: Vec<u8>,
        src_zip: Vec<u8>,
        uv_sha: String,
        src_sha: String,
    }

    impl Fixtures {
        async fn serve() -> Self {
            let uv_zip = make_zip(&[("uv.exe", b"MZ uv")]);
            // The real archive's layout: everything under `<repo>-<sha>/`.
            let src_zip = make_zip(&[
                (
                    "ai-toolkit-e65c4d0fb69251e692390574c49873297dc4bae5/run.py",
                    b"# ai-toolkit",
                ),
                (
                    "ai-toolkit-e65c4d0fb69251e692390574c49873297dc4bae5/requirements.txt",
                    b"torch\nsafetensors\n",
                ),
            ]);
            let (u, s) = (uv_zip.clone(), src_zip.clone());
            let app = Router::new()
                .route(
                    "/uv/uv.zip",
                    get(move || {
                        let b = u.clone();
                        async move { Body::from(b) }
                    }),
                )
                .route(
                    "/src/trainer.zip",
                    get(move || {
                        let b = s.clone();
                        async move { Body::from(b) }
                    }),
                );
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind the fixture server");
            let port = listener
                .local_addr()
                .expect("fixture server address")
                .port();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });

            Self {
                port,
                uv_sha: sha(&uv_zip),
                src_sha: sha(&src_zip),
                uv_zip,
                src_zip,
            }
        }

        fn archives(&self) -> (String, String, [Archive<'_>; 2]) {
            (
                format!("http://127.0.0.1:{}/uv", self.port),
                format!("http://127.0.0.1:{}/src", self.port),
                [
                    Archive {
                        name: "uv.zip",
                        sha256: &self.uv_sha,
                        size: self.uv_zip.len() as u64,
                    },
                    Archive {
                        name: "trainer.zip",
                        sha256: &self.src_sha,
                        size: self.src_zip.len() as u64,
                    },
                ],
            )
        }
    }

    async fn run_install<F>(
        fx: &Fixtures,
        tmp: &Path,
        runner: &dyn CmdRunner,
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(TrainerInstallPhase, u64, u64) + Send + Sync,
    {
        let (ub, sb, a) = fx.archives();
        let spec = FetchSpec {
            uv_base: &ub,
            src_base: &sb,
            uv: &a[0],
            src: &a[1],
        };
        install_with(tmp, false, runner, &spec, on_progress).await
    }

    fn fake_complete_install(runtimes_dir: &Path) {
        let py = venv_python(runtimes_dir);
        std::fs::create_dir_all(py.parent().expect("venv python has a parent"))
            .expect("create the venv dir");
        std::fs::write(&py, b"py").expect("write the venv python");
        std::fs::create_dir_all(source_dir(runtimes_dir)).expect("create the source dir");
        std::fs::write(run_py(runtimes_dir), b"# ai-toolkit").expect("write run.py");
        std::fs::write(marker_file(runtimes_dir), PINNED_COMMIT).expect("write the marker");
    }

    #[tokio::test]
    async fn install_refuses_offline_when_not_installed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = RecordingRunner::default();

        let err = install(tmp.path(), true, &runner, |_, _, _| {})
            .await
            .expect_err("offline mode must refuse a fresh install");

        assert!(
            err.to_string().contains("offline mode"),
            "unexpected error: {err}"
        );
        assert!(runner.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn install_is_idempotent_via_the_commit_marker() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fake_complete_install(tmp.path());
        assert!(is_installed(tmp.path()));

        let runner = RecordingRunner::default();
        // Offline *and* already installed: returns Ok without touching uv.
        install(tmp.path(), true, &runner, |_, _, _| {})
            .await
            .expect("an existing install is a no-op");
        assert!(
            runner.calls.lock().expect("calls").is_empty(),
            "nothing to do"
        );

        // Remove just the marker (a half-finished earlier attempt): offline
        // now refuses again instead of pretending the trainer is ready.
        std::fs::remove_file(marker_file(tmp.path())).expect("remove the marker");
        assert!(!is_installed(tmp.path()));
        assert!(install(tmp.path(), true, &runner, |_, _, _| {})
            .await
            .is_err());
    }

    #[tokio::test]
    async fn install_runs_uv_steps_in_order_with_the_torch_pin() {
        let fx = Fixtures::serve().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = VenvCreatingRunner::new(tmp.path());
        let phases: Mutex<Vec<TrainerInstallPhase>> = Mutex::new(Vec::new());

        run_install(&fx, tmp.path(), &runner, |p, _, _| {
            phases.lock().expect("phases").push(p);
        })
        .await
        .expect("the install should succeed against the fixture server");

        // The source archive landed flattened: `run.py` at the top of `src/`.
        assert_eq!(std::fs::read(uv_bin(tmp.path())).expect("uv"), b"MZ uv");
        assert!(run_py(tmp.path()).is_file());
        assert!(
            !source_dir(tmp.path())
                .join("ai-toolkit-e65c4d0fb69251e692390574c49873297dc4bae5")
                .exists(),
            "the <repo>-<sha> prefix must be stripped"
        );
        assert!(is_installed(tmp.path()), "the marker is written last");
        assert_eq!(
            std::fs::read_to_string(marker_file(tmp.path())).expect("marker"),
            PINNED_COMMIT
        );

        let py = venv_python(tmp.path()).to_string_lossy().into_owned();
        let venv = venv_dir(tmp.path()).to_string_lossy().into_owned();
        let reqs = source_dir(tmp.path())
            .join("requirements.txt")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            runner.calls(),
            vec![
                vec!["python".to_string(), "install".into(), "3.12".into()],
                vec!["venv".to_string(), "--python".into(), "3.12".into(), venv],
                vec![
                    "pip".to_string(),
                    "install".into(),
                    "--python".into(),
                    py.clone(),
                    "torch==2.13.0".into(),
                    "torchvision==0.28.0".into(),
                    "torchaudio==2.11.0".into(),
                    "--index-url".into(),
                    "https://download.pytorch.org/whl/cu130".into(),
                ],
                vec![
                    "pip".to_string(),
                    "install".into(),
                    "--python".into(),
                    py,
                    "-r".into(),
                    reqs,
                ],
            ]
        );

        let phases = phases.into_inner().expect("phases");
        for expected in [
            TrainerInstallPhase::Downloading,
            TrainerInstallPhase::Extracting,
            TrainerInstallPhase::InstallingPython,
            TrainerInstallPhase::CreatingVenv,
            TrainerInstallPhase::InstallingTorch,
            TrainerInstallPhase::InstallingDeps,
        ] {
            assert!(
                phases.contains(&expected),
                "missing {expected:?} in {phases:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_failing_uv_step_leaves_no_marker_behind() {
        let fx = Fixtures::serve().await;
        let tmp = tempfile::tempdir().expect("tempdir");

        let err = run_install(&fx, tmp.path(), &FailingRunner, |_, _, _| {})
            .await
            .expect_err("a failing uv step must surface");

        assert!(
            err.to_string().contains("boom running uv"),
            "unexpected error: {err}"
        );
        assert!(!marker_file(tmp.path()).exists());
        assert!(!is_installed(tmp.path()));
    }

    /// The real thing: run the whole pinned install against GitHub + PyPI.
    /// `#[ignore]` — downloads gigabytes. Run explicitly for the slice smoke:
    /// `cargo test -p aiwm-core --lib runtime::training::install::tests::real -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "network + disk: downloads uv, the ai-toolkit source and a CUDA torch build"]
    async fn real_pinned_install() {
        let tmp = tempfile::tempdir().expect("tempdir");
        install(tmp.path(), false, &SystemRunner, |phase, done, total| {
            eprintln!("  {phase:?} {done}/{total}");
        })
        .await
        .expect("the pinned ai-toolkit install should succeed on this machine");

        assert!(is_installed(tmp.path()));
        SystemRunner
            .run(
                &venv_python(tmp.path()),
                &["-c", "import torch; print('ok', torch.__version__)"],
                &[],
            )
            .await
            .expect("torch should import in the fresh trainer venv");
    }
}
