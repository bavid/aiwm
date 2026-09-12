//! Downloading and installing the pinned Colibri CPU-only release.
//!
//! One curated version. The Windows release archive is a flat zip: `coli.cmd`
//! (the launcher — the real entry point; the per-family `.exe` engines do
//! nothing useful started on their own), one engine binary per model family,
//! the Python helper scripts the launcher shells out to, and the web
//! dashboard. No CUDA DLL is bundled — that needs a from-source build, not
//! automated here. Verified against the SHA-256 GitHub publishes for it, then
//! extracted into `<runtimes_dir>/colibri/<PINNED_BUILD>/`.

use std::path::{Path, PathBuf};

use super::{colibri_err, RUNTIME_ID};
use crate::runtime::download::{download_verified, extract_zip, Archive};
use crate::{CoreError, Result};

/// Pinned Colibri release tag. Bump this together with the digest below; it
/// comes from the GitHub Releases API (`assets[].digest`) / the release's own
/// `SHA256SUMS.txt`.
pub const PINNED_BUILD: &str = "v1.10.2";
const RELEASE_BASE: &str = "https://github.com/JustVugg/colibri/releases/download";

const PINNED_ARCHIVE: Archive<'static> = Archive {
    name: "colibri-v1.10.2-windows-x86_64.zip",
    sha256: "8dcad7b5806b9c54e85f04e54a65f280b43583e5eca0c59512ae23b43d19e051",
    size: 4_514_496,
};

/// Total bytes the archive weighs — surfaced in the UI ("~4.3 MB").
pub const TOTAL_DOWNLOAD_BYTES: u64 = PINNED_ARCHIVE.size;

/// The launcher script the archive ships — not an `.exe`: it picks the right
/// per-family engine binary from a model's `config.json` and needs Python 3
/// on `PATH` (the engines themselves are pure C, no Python at runtime).
const LAUNCHER: &str = if cfg!(windows) { "coli.cmd" } else { "coli" };

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

/// Install the pinned build. Returns the resolved launcher path (`coli.cmd`
/// on Windows). Idempotent — a complete existing install is returned
/// untouched. `on_progress` is called with `(phase, done_bytes, total_bytes)`
/// as the download proceeds.
pub async fn install<F>(runtimes_dir: &Path, offline: bool, on_progress: F) -> Result<PathBuf>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let base = format!("{RELEASE_BASE}/{PINNED_BUILD}");
    install_from(runtimes_dir, offline, &base, &PINNED_ARCHIVE, on_progress).await
}

/// The download → verify → extract → resolve pipeline, parameterised on the
/// archive + base URL so tests can drive it against a local server.
async fn install_from<F>(
    runtimes_dir: &Path,
    offline: bool,
    base_url: &str,
    archive: &Archive<'_>,
    on_progress: F,
) -> Result<PathBuf>
where
    F: Fn(InstallPhase, u64, u64) + Send + Sync,
{
    let target = install_root(runtimes_dir);
    let launcher = target.join(LAUNCHER);
    if launcher.is_file() {
        return Ok(launcher);
    }
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot download Colibri. Turn it off, or install it manually \
             and point AIWM_COLIBRI_PATH at coli.cmd."
                .into(),
        ));
    }

    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| colibri_err(format!("create {}: {e}", target.display())))?;
    let staging = target.join(".download");
    let _ = tokio::fs::remove_dir_all(&staging).await; // clear any earlier partial
    tokio::fs::create_dir_all(&staging).await.ok();

    let url = format!("{base_url}/{}", archive.name);
    let zip_path = staging.join(archive.name);
    download_verified(&url, archive, &zip_path, |done| {
        on_progress(InstallPhase::Downloading, done, archive.size);
    })
    .await?;
    on_progress(InstallPhase::Extracting, archive.size, archive.size);
    extract_zip(&zip_path, &target).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    if !launcher.is_file() {
        return Err(colibri_err(format!(
            "install finished but {} is missing — the archive layout was not what we expected",
            launcher.display()
        )));
    }
    tracing::info!(build = PINNED_BUILD, path = %launcher.display(), "colibri installed");
    Ok(launcher)
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
        std::fs::write(root.join(LAUNCHER), b"@echo off").unwrap();

        // offline + no launcher would normally fail; the early-return wins.
        let got = install(tmp.path(), true, |_, _, _| {}).await.unwrap();
        assert_eq!(got, root.join(LAUNCHER));
    }

    #[tokio::test]
    async fn install_from_downloads_verifies_extracts_and_resolves() {
        use std::sync::Mutex;

        use sha2::{Digest, Sha256};

        let zip = make_zip(&[
            ("coli.cmd", b"@echo off\r\npython coli %*"),
            ("colibri.exe", b"MZ real-ish"),
            ("olmoe.exe", b"MZ olmoe"),
            ("web/dist/index.html", b"<html></html>"),
        ]);
        let digest = |b: &[u8]| hex(&Sha256::digest(b));
        let sha = digest(&zip);

        let body = zip.clone();
        let app = Router::new().route(
            "/colibri.zip",
            get(move || {
                let b = body.clone();
                async move { Body::from(b) }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let archive = Archive {
            name: "colibri.zip",
            sha256: &sha,
            size: zip.len() as u64,
        };

        let tmp = tempfile::tempdir().unwrap();
        let phases: Mutex<Vec<(InstallPhase, u64, u64)>> = Mutex::new(Vec::new());
        let launcher = install_from(
            tmp.path(),
            false,
            &format!("http://127.0.0.1:{port}"),
            &archive,
            |phase, done, total| phases.lock().unwrap().push((phase, done, total)),
        )
        .await
        .unwrap();

        assert_eq!(launcher, install_root(tmp.path()).join(LAUNCHER));
        assert_eq!(
            std::fs::read(&launcher).unwrap(),
            b"@echo off\r\npython coli %*"
        );
        assert!(install_root(tmp.path()).join("olmoe.exe").is_file());
        assert!(install_root(tmp.path())
            .join("web/dist/index.html")
            .is_file());
        assert!(!install_root(tmp.path()).join(".download").exists());
        let phases = phases.into_inner().unwrap();
        assert!(phases.iter().any(|(p, ..)| *p == InstallPhase::Extracting));
        assert_eq!(
            phases.last().map(|&(_, d, t)| (d, t)),
            Some((archive.size, archive.size))
        );
    }
}
