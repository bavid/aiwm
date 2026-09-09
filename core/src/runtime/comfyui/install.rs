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

use crate::runtime::download::{
    download_verified, ensure_uv, extract_zip_flat, Archive, UV_ARCHIVE, UV_RELEASE_BASE,
};
// Re-exported so `comfyui::mod` (and this module's tests) reach them via `install::`.
pub(crate) use crate::runtime::download::{CmdRunner, SystemRunner};
use crate::{CoreError, Result};

use super::RUNTIME_ID;

/// Pinned ComfyUI release tag. Bump together with [`COMFYUI_SRC`] below.
pub const PINNED_TAG: &str = "v0.34.0";
/// Python the venv is built with (`uv venv --python` downloads it).
const PYTHON_VERSION: &str = "3.13";
/// PyTorch wheel index — CUDA 13.0, ComfyUI's current recommendation for
/// RTX 20-series and newer. Needs a recent NVIDIA driver.
const TORCH_INDEX_URL: &str = "https://download.pytorch.org/whl/cu130";

const COMFYUI_ARCHIVE_BASE: &str = "https://github.com/comfyanonymous/ComfyUI/archive/refs/tags";
const GGUF_ARCHIVE_BASE: &str = "https://github.com/city96/ComfyUI-GGUF/archive";

/// The one custom node we ship (ADR-018): GGUF quantization support, needed to
/// run Flux / SD3.5 on 16 GB. Pinned to a commit — the repo has no tags — and
/// the archive SHA-256 we computed. Same caveat as [`COMFYUI_SRC`].
const GGUF_NODE_DIR: &str = "ComfyUI-GGUF";
const GGUF_NODE_ARCHIVE: Archive<'static> = Archive {
    name: "6ea2651e7df66d7585f6ffee804b20e92fb38b8a.zip",
    sha256: "aad273a0b774684285b4496f6edff7e2293bd29d71ef89926d6861bfdf4947ac",
    size: 38_051,
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
    let exe = if cfg!(windows) { "uv.exe" } else { "uv" };
    install_root(runtimes_dir).join("uv").join(exe)
}

/// The ComfyUI checkout: `main.py`, `requirements.txt`, `.venv/`, `custom_nodes/`.
pub fn comfy_home(runtimes_dir: &Path) -> PathBuf {
    install_root(runtimes_dir).join(PINNED_TAG)
}

fn venv_python(runtimes_dir: &Path) -> PathBuf {
    comfy_home(runtimes_dir).join(VENV_PYTHON)
}

fn gguf_node_dir(runtimes_dir: &Path) -> PathBuf {
    comfy_home(runtimes_dir)
        .join("custom_nodes")
        .join(GGUF_NODE_DIR)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Downloading,
    Extracting,
    CreatingVenv,
    InstallingTorch,
    InstallingDeps,
    InstallingNode,
}

/// Bytes to fetch for the toolchain — surfaced in the UI. The venv build adds
/// gigabytes of wheels whose size we cannot know up front.
pub const TOOLCHAIN_DOWNLOAD_BYTES: u64 =
    UV_ARCHIVE.size + COMFYUI_SRC.size + GGUF_NODE_ARCHIVE.size;

// --- the installer --------------------------------------------------------

/// The pinned archives + their base URLs. Bundled so the pipeline functions
/// stay readable; tests substitute a local server.
struct FetchSpec<'a> {
    uv_base: &'a str,
    comfy_base: &'a str,
    gguf_base: &'a str,
    uv: &'a Archive<'a>,
    comfy: &'a Archive<'a>,
    gguf: &'a Archive<'a>,
}

const PINNED_SPEC: FetchSpec<'static> = FetchSpec {
    uv_base: UV_RELEASE_BASE,
    comfy_base: COMFYUI_ARCHIVE_BASE,
    gguf_base: GGUF_ARCHIVE_BASE,
    uv: &UV_ARCHIVE,
    comfy: &COMFYUI_SRC,
    gguf: &GGUF_NODE_ARCHIVE,
};

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
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let root = install_root(runtimes_dir);
    let uv = uv_bin(runtimes_dir);
    let home = comfy_home(runtimes_dir);
    let py = venv_python(runtimes_dir);
    let node_marker = gguf_node_dir(runtimes_dir).join("__init__.py");

    if py.is_file() && home.join("main.py").is_file() && node_marker.is_file() {
        return Ok(()); // already installed
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download ComfyUI. Turn it off, or install ComfyUI \
             manually and point AIWM_COMFYUI_PYTHON / AIWM_COMFYUI_DIR at it."
                .into(),
        ));
    }

    fetch_sources(
        &root,
        &home,
        gguf_node_dir(runtimes_dir).as_path(),
        spec,
        &on_progress,
    )
    .await?;
    build_venv(&root, &uv, &home, &py, runner, &on_progress).await?;

    if !py.is_file() || !node_marker.is_file() {
        return Err(comfy_install_err(
            "install finished but the venv python or the GGUF node is missing — the steps \
             did not complete",
        ));
    }
    tracing::info!(tag = PINNED_TAG, home = %home.display(), "ComfyUI installed");
    Ok(())
}

/// Fetch + unpack the verified toolchain: `uv` (via [`ensure_uv`]), the ComfyUI
/// source, and the pinned GGUF node.
async fn fetch_sources<F>(
    root: &Path,
    home: &Path,
    node_dir: &Path,
    spec: &FetchSpec<'_>,
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
    let total = spec.uv.size + spec.comfy.size + spec.gguf.size;
    let mut done = 0u64;

    ensure_uv(spec.uv_base, spec.uv, root, |n| {
        on_progress(InstallPhase::Downloading, done + n, total);
    })
    .await?;
    done += spec.uv.size;

    if !home.join("main.py").is_file() {
        let zip = staging.join(spec.comfy.name);
        download_verified(
            &format!("{}/{}", spec.comfy_base, spec.comfy.name),
            spec.comfy,
            &zip,
            |n| on_progress(InstallPhase::Downloading, done + n, total),
        )
        .await?;
        extract_zip_flat(&zip, home).await?;
    }
    done += spec.comfy.size;

    if !node_dir.join("__init__.py").is_file() {
        let zip = staging.join(spec.gguf.name);
        download_verified(
            &format!("{}/{}", spec.gguf_base, spec.gguf.name),
            spec.gguf,
            &zip,
            |n| on_progress(InstallPhase::Downloading, done + n, total),
        )
        .await?;
        extract_zip_flat(&zip, node_dir).await?;
    }

    on_progress(InstallPhase::Extracting, total, total);
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
    let venv_s = home.join(".venv").to_string_lossy().into_owned();
    let reqs_s = home.join("requirements.txt").to_string_lossy().into_owned();
    let node_reqs_s = home
        .join("custom_nodes")
        .join(GGUF_NODE_DIR)
        .join("requirements.txt")
        .to_string_lossy()
        .into_owned();
    let py_s = py.to_string_lossy().into_owned();

    if !py.is_file() {
        on_progress(InstallPhase::CreatingVenv, 0, 0);
        runner
            .run(uv, &["venv", "--python", PYTHON_VERSION, &venv_s], &env)
            .await?;
    }

    on_progress(InstallPhase::InstallingTorch, 0, 0);
    let torch = &[
        "pip",
        "install",
        "--python",
        &py_s,
        "--index-url",
        TORCH_INDEX_URL,
        "torch",
        "torchvision",
        "torchaudio",
    ];
    runner.run(uv, torch, &env).await?;

    on_progress(InstallPhase::InstallingDeps, 0, 0);
    runner
        .run(
            uv,
            &["pip", "install", "--python", &py_s, "-r", &reqs_s],
            &env,
        )
        .await?;

    on_progress(InstallPhase::InstallingNode, 0, 0);
    runner
        .run(
            uv,
            &["pip", "install", "--python", &py_s, "-r", &node_reqs_s],
            &env,
        )
        .await?;

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

    /// Like [`RecordingRunner`] but drops a fake venv python on the `venv` call,
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

    fn sha(b: &[u8]) -> String {
        hex(&Sha256::digest(b))
    }

    struct Fixtures {
        port: u16,
        uv_zip: Vec<u8>,
        src_zip: Vec<u8>,
        node_zip: Vec<u8>,
        uv_sha: String,
        src_sha: String,
        node_sha: String,
    }

    impl Fixtures {
        /// The three archives, served from one local server. `src`/`node`
        /// contents can be overridden (`None` = a valid minimal one).
        async fn serve(src_body: Option<Vec<u8>>, node_body: Option<Vec<u8>>) -> Self {
            let uv_zip = make_zip(&[("uv.exe", b"MZ uv")]);
            let src_zip = src_body.unwrap_or_else(|| {
                make_zip(&[
                    ("ComfyUI-0.34.0/main.py", b"# comfy"),
                    ("ComfyUI-0.34.0/requirements.txt", b"torch\ntorchsde\n"),
                ])
            });
            let node_zip = node_body.unwrap_or_else(|| {
                make_zip(&[
                    ("ComfyUI-GGUF-abc/__init__.py", b"NODE_CLASS_MAPPINGS = {}"),
                    ("ComfyUI-GGUF-abc/requirements.txt", b"gguf>=0.13.0\n"),
                ])
            });
            let (u, s, n) = (uv_zip.clone(), src_zip.clone(), node_zip.clone());
            let app = Router::new()
                .route(
                    "/uv/uv.zip",
                    get(move || {
                        let b = u.clone();
                        async move { Body::from(b) }
                    }),
                )
                .route(
                    "/src/comfy.zip",
                    get(move || {
                        let b = s.clone();
                        async move { Body::from(b) }
                    }),
                )
                .route(
                    "/node/gguf.zip",
                    get(move || {
                        let b = n.clone();
                        async move { Body::from(b) }
                    }),
                );
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

            Self {
                port,
                uv_sha: sha(&uv_zip),
                src_sha: sha(&src_zip),
                node_sha: sha(&node_zip),
                uv_zip,
                src_zip,
                node_zip,
            }
        }

        fn spec(&self) -> (String, String, String, [Archive<'_>; 3]) {
            let (ub, cb, nb) = (
                format!("http://127.0.0.1:{}/uv", self.port),
                format!("http://127.0.0.1:{}/src", self.port),
                format!("http://127.0.0.1:{}/node", self.port),
            );
            let archives = [
                Archive {
                    name: "uv.zip",
                    sha256: &self.uv_sha,
                    size: self.uv_zip.len() as u64,
                },
                Archive {
                    name: "comfy.zip",
                    sha256: &self.src_sha,
                    size: self.src_zip.len() as u64,
                },
                Archive {
                    name: "gguf.zip",
                    sha256: &self.node_sha,
                    size: self.node_zip.len() as u64,
                },
            ];
            (ub, cb, nb, archives)
        }
    }

    async fn run_install_with<F>(
        fx: &Fixtures,
        tmp: &Path,
        runner: &dyn CmdRunner,
        on_progress: F,
    ) -> Result<()>
    where
        F: Fn(InstallPhase, u64, u64) + Send + Sync,
    {
        let (ub, cb, nb, a) = fx.spec();
        let spec = FetchSpec {
            uv_base: &ub,
            comfy_base: &cb,
            gguf_base: &nb,
            uv: &a[0],
            comfy: &a[1],
            gguf: &a[2],
        };
        install_with(tmp, false, runner, &spec, on_progress).await
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
        std::fs::create_dir_all(gguf_node_dir(tmp.path())).unwrap();
        std::fs::write(gguf_node_dir(tmp.path()).join("__init__.py"), b"x").unwrap();

        let runner = RecordingRunner::default();
        install(tmp.path(), true, &runner, |_, _, _| {})
            .await
            .unwrap();
        assert!(runner.calls.lock().unwrap().is_empty(), "nothing to do");
    }

    #[tokio::test]
    async fn full_install_fetches_all_three_then_runs_the_uv_steps_in_order() {
        let fx = Fixtures::serve(None, None).await;
        let tmp = tempfile::tempdir().unwrap();
        let runner = VenvCreatingRunner {
            py: venv_python(tmp.path()),
            calls: Mutex::new(Vec::new()),
        };
        let phases: Mutex<Vec<InstallPhase>> = Mutex::new(Vec::new());

        run_install_with(&fx, tmp.path(), &runner, |p, _, _| {
            phases.lock().unwrap().push(p)
        })
        .await
        .unwrap();

        assert_eq!(std::fs::read(uv_bin(tmp.path())).unwrap(), b"MZ uv");
        assert!(comfy_home(tmp.path()).join("main.py").is_file());
        assert!(gguf_node_dir(tmp.path()).join("__init__.py").is_file());
        assert!(
            !gguf_node_dir(tmp.path()).join("ComfyUI-GGUF-abc").exists(),
            "flattened"
        );

        let calls = runner.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 4, "venv, torch, requirements, node: {calls:?}");
        assert_eq!(calls[0][0], "venv");
        assert!(
            calls[1].contains(&"--index-url".to_string())
                && calls[1].contains(&"torch".to_string())
        );
        assert!(calls[2].last().unwrap().ends_with("requirements.txt"));
        assert!(calls[3]
            .last()
            .unwrap()
            .replace('\\', "/")
            .ends_with("custom_nodes/ComfyUI-GGUF/requirements.txt"));

        let phases = phases.into_inner().unwrap();
        for expected in [
            InstallPhase::Extracting,
            InstallPhase::CreatingVenv,
            InstallPhase::InstallingTorch,
            InstallPhase::InstallingDeps,
            InstallPhase::InstallingNode,
        ] {
            assert!(
                phases.contains(&expected),
                "missing {expected:?} in {phases:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_failing_uv_step_surfaces_the_error() {
        let fx = Fixtures::serve(None, None).await;
        let tmp = tempfile::tempdir().unwrap();
        let err = run_install_with(&fx, tmp.path(), &FailingRunner, |_, _, _| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom running uv"));
    }

    #[tokio::test]
    async fn a_bad_source_hash_aborts_before_the_uv_steps() {
        let fx = Fixtures::serve(Some(b"corrupt".to_vec()), None).await;
        let tmp = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::default();

        // Override the source sha to a wrong one.
        let (ub, cb, nb, a) = fx.spec();
        let spec = FetchSpec {
            uv_base: &ub,
            comfy_base: &cb,
            gguf_base: &nb,
            uv: &a[0],
            comfy: &Archive {
                name: "comfy.zip",
                sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                size: 7,
            },
            gguf: &a[2],
        };
        let err = install_with(tmp.path(), false, &runner, &spec, |_, _, _| {})
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
        assert!(gguf_node_dir(tmp.path()).join("__init__.py").is_file());
        // torch + the node's `gguf` dep both import (proves the wheels + Python
        // match; not the GPU driver — that waits for a real render in 3.4).
        SystemRunner
            .run(
                &venv_python(tmp.path()),
                &["-c", "import torch, gguf; print('ok', torch.__version__)"],
                &[],
            )
            .await
            .expect("torch and gguf should import in the fresh venv");
    }
}
