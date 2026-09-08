//! Downloading and installing the pinned `llama.cpp` CUDA build.
//!
//! One curated version (ADR-002, ADR-014). The two release archives are verified
//! against the SHA-256 digests GitHub publishes for them, then extracted side by
//! side into `<runtimes_dir>/llamacpp/<PINNED_BUILD>/` so `llama-server.exe`
//! finds its CUDA runtime DLLs next to it.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use super::{llama_err, RUNTIME_ID};
use crate::{CoreError, Result};

/// Pinned llama.cpp release tag. Bump this together with the two digests below;
/// they come from the GitHub Releases API (`assets[].digest`).
pub const PINNED_BUILD: &str = "b10855";
const RELEASE_BASE: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

struct Archive<'a> {
    name: &'a str,
    sha256: &'a str,
    size: u64,
}

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

async fn download_verified(
    url: &str,
    archive: &Archive<'_>,
    dest: &Path,
    on_bytes: impl Fn(u64),
) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| llama_err(format!("build http client: {e}")))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| llama_err(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| llama_err(format!("GET {url}: {e}")))?;

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| llama_err(format!("create {}: {e}", dest.display())))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| llama_err(format!("download {url}: {e}")))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| llama_err(format!("write {}: {e}", dest.display())))?;
        downloaded += chunk.len() as u64;
        on_bytes(downloaded);
    }
    file.flush().await.ok();
    drop(file);

    if downloaded != archive.size {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(llama_err(format!(
            "{}: expected {} bytes, got {downloaded}",
            archive.name, archive.size
        )));
    }
    let got = hex(&hasher.finalize());
    if got != archive.sha256 {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(llama_err(format!(
            "{}: SHA-256 mismatch — expected {}, got {got}",
            archive.name, archive.sha256
        )));
    }
    Ok(())
}

async fn extract_zip(zip_path: &Path, target: &Path) -> Result<()> {
    let (zip_path, target) = (zip_path.to_path_buf(), target.to_path_buf());
    tokio::task::spawn_blocking(move || extract_zip_blocking(&zip_path, &target))
        .await
        .map_err(|e| llama_err(format!("extract worker panicked: {e}")))?
}

fn extract_zip_blocking(zip_path: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| llama_err(format!("open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| llama_err(format!("read {}: {e}", zip_path.display())))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| llama_err(format!("zip entry {i}: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(llama_err(format!(
                "zip entry {:?} has an unsafe path",
                entry.name()
            )));
        };
        let out = target.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| llama_err(format!("create {}: {e}", out.display())))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| llama_err(format!("create {}: {e}", parent.display())))?;
        }
        let mut w = std::fs::File::create(&out)
            .map_err(|e| llama_err(format!("create {}: {e}", out.display())))?;
        std::io::copy(&mut entry, &mut w)
            .map_err(|e| llama_err(format!("write {}: {e}", out.display())))?;
        w.flush().ok();
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    use super::*;

    /// A minimal stored (uncompressed) zip with the given `(name, contents)`.
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[tokio::test]
    async fn extract_unpacks_every_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("a.zip");
        std::fs::write(
            &zip,
            make_zip(&[("llama-server.exe", b"MZ fake"), ("ggml.dll", b"dll")]),
        )
        .unwrap();

        extract_zip(&zip, tmp.path()).await.unwrap();

        assert_eq!(
            std::fs::read(tmp.path().join("llama-server.exe")).unwrap(),
            b"MZ fake"
        );
        assert!(tmp.path().join("ggml.dll").is_file());
    }

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
    async fn download_verified_rejects_a_bad_hash() {
        let body = b"not the real archive".to_vec();
        let app = Router::new().route(
            "/x.zip",
            get(move || {
                let body = body.clone();
                async move {
                    (
                        [(header::CONTENT_TYPE, "application/zip")],
                        Body::from(body),
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("x.zip");
        let archive = Archive {
            name: "x.zip",
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            size: 20,
        };
        let err = download_verified(
            &format!("http://127.0.0.1:{port}/x.zip"),
            &archive,
            &dest,
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256 mismatch"));
        assert!(!dest.exists(), "a failed download must not leave the file");
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
