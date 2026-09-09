//! Downloading and installing the pinned `llama.cpp` CUDA build.
//!
//! One curated version (ADR-002, ADR-014). The two release archives are verified
//! against the SHA-256 digests GitHub publishes for them, then extracted side by
//! side into `<runtimes_dir>/llamacpp/<PINNED_BUILD>/` so `llama-server.exe`
//! finds its CUDA runtime DLLs next to it.

use std::path::{Path, PathBuf};

use super::{llama_err, RUNTIME_ID};
use crate::runtime::download::{download_verified, extract_zip, Archive};
use crate::{CoreError, Result};

/// Pinned llama.cpp release tag. Bump this together with the two digests below;
/// they come from the GitHub Releases API (`assets[].digest`).
pub const PINNED_BUILD: &str = "b10855";
const RELEASE_BASE: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

const PINNED_ARCHIVES: [Archive<'static>; 2] = [
    Archive {
        name: "llama-b10855-bin-win-cuda-12.4-x64.zip",
        sha256: "4f1e2505e5c3ce0126b2f44c5b87af375960428a4ff98535dfeee8a5fbed8a5b",
        size: 254_069_159,
    },
    Archive {
        name: "cudart-llama-bin-win-cuda-12.4-x64.zip",
        sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
        size: 391_443_627,
    },
];

/// Total bytes the two archives weigh — surfaced in the UI ("~645 MB").
pub const TOTAL_DOWNLOAD_BYTES: u64 = PINNED_ARCHIVES[0].size + PINNED_ARCHIVES[1].size;

const SERVER_EXE: &str = if cfg!(windows) {
    "llama-server.exe"
} else {
    "llama-server"
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Downloading,
    Extracting,
}

/// Where the pinned build lives once installed.
pub fn install_root(runtimes_dir: &Path) -> PathBuf {
    runtimes_dir.join(RUNTIME_ID).join(PINNED_BUILD)
}

/// Install the pinned build. Returns the resolved `llama-server` path.
/// Idempotent — a complete existing install is returned untouched. `on_progress`
/// is called with `(phase, done_bytes, total_bytes)` as the download proceeds.
pub async fn install<F>(runtimes_dir: &Path, offline: bool, on_progress: F) -> Result<PathBuf>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let base = format!("{RELEASE_BASE}/{PINNED_BUILD}");
    install_from(runtimes_dir, offline, &base, &PINNED_ARCHIVES, on_progress).await
}

/// The download → verify → extract → resolve pipeline, parameterised on the
/// archive list + base URL so tests can drive it against a local server.
async fn install_from<F>(
    runtimes_dir: &Path,
    offline: bool,
    base_url: &str,
    archives: &[Archive<'_>],
    on_progress: F,
) -> Result<PathBuf>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let target = install_root(runtimes_dir);
    let server = target.join(SERVER_EXE);
    if server.is_file() {
        return Ok(server);
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download llama.cpp. Turn it off, or install the build \
             manually and point AIWM_LLAMACPP_PATH at llama-server.exe."
                .into(),
        ));
    }

    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| llama_err(format!("create {}: {e}", target.display())))?;
    let staging = target.join(".download");
    let _ = tokio::fs::remove_dir_all(&staging).await; // clear any earlier partial
    tokio::fs::create_dir_all(&staging).await.ok();

    let total: u64 = archives.iter().map(|a| a.size).sum();
    let mut done_before = 0u64;
    for archive in archives {
        let url = format!("{base_url}/{}", archive.name);
        let zip_path = staging.join(archive.name);
        download_verified(&url, archive, &zip_path, |chunk_total| {
            on_progress(InstallPhase::Downloading, done_before + chunk_total, total);
        })
        .await?;
        on_progress(InstallPhase::Extracting, done_before + archive.size, total);
        extract_zip(&zip_path, &target).await?;
        let _ = tokio::fs::remove_file(&zip_path).await;
        done_before += archive.size;
    }
    let _ = tokio::fs::remove_dir_all(&staging).await;

    if !server.is_file() {
        return Err(llama_err(format!(
            "install finished but {} is missing — the archive layout was not what we expected",
            server.display()
        )));
    }
    tracing::info!(build = PINNED_BUILD, path = %server.display(), "llama.cpp installed");
    Ok(server)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;

    use super::*;
    use crate::runtime::download::hex;
    use crate::runtime::download::test_support::make_zip;

    #[tokio::test]
    async fn install_refuses_in_offline_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install(tmp.path(), true, |_, _, _| {}).await.unwrap_err();
        assert!(err.to_string().contains("offline mode"));
    }

    #[tokio::test]
    async fn install_is_idempotent_when_already_present() {
        let tmp = tempfile::tempdir().unwrap();
        let root = install_root(tmp.path());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(SERVER_EXE), b"present").unwrap();

        // offline + no server would normally fail; the early-return wins.
        let got = install(tmp.path(), true, |_, _, _| {}).await.unwrap();
        assert_eq!(got, root.join(SERVER_EXE));
    }

    #[tokio::test]
    async fn install_from_downloads_verifies_extracts_and_resolves() {
        use std::sync::Mutex;

        use sha2::{Digest, Sha256};

        let bin_zip = make_zip(&[("llama-server.exe", b"MZ real-ish"), ("ggml.dll", b"g")]);
        let cudart_zip = make_zip(&[("cudart64_12.dll", b"cuda")]);
        let digest = |b: &[u8]| hex(&Sha256::digest(b));

        let bin_sha = digest(&bin_zip);
        let cudart_sha = digest(&cudart_zip);

        let (b1, b2) = (bin_zip.clone(), cudart_zip.clone());
        let app = Router::new()
            .route(
                "/bin.zip",
                get(move || {
                    let b = b1.clone();
                    async move { Body::from(b) }
                }),
            )
            .route(
                "/cudart.zip",
                get(move || {
                    let b = b2.clone();
                    async move { Body::from(b) }
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let archives = [
            Archive {
                name: "bin.zip",
                sha256: &bin_sha,
                size: bin_zip.len() as u64,
            },
            Archive {
                name: "cudart.zip",
                sha256: &cudart_sha,
                size: cudart_zip.len() as u64,
            },
        ];

        let tmp = tempfile::tempdir().unwrap();
        let phases: Mutex<Vec<(InstallPhase, u64, u64)>> = Mutex::new(Vec::new());
        let server = install_from(
            tmp.path(),
            false,
            &format!("http://127.0.0.1:{port}"),
            &archives,
            |phase, done, total| phases.lock().unwrap().push((phase, done, total)),
        )
        .await
        .unwrap();

        assert_eq!(server, install_root(tmp.path()).join(SERVER_EXE));
        assert_eq!(std::fs::read(&server).unwrap(), b"MZ real-ish");
        assert!(install_root(tmp.path()).join("cudart64_12.dll").is_file());
        assert!(!install_root(tmp.path()).join(".download").exists());
        let phases = phases.into_inner().unwrap();
        assert!(phases.iter().any(|(p, ..)| *p == InstallPhase::Extracting));
        assert_eq!(
            phases.last().map(|&(_, d, t)| (d, t)),
            Some((total_of(&archives), total_of(&archives)))
        );
    }

    fn total_of(archives: &[Archive<'_>]) -> u64 {
        archives.iter().map(|a| a.size).sum()
    }
}
