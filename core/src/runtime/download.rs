//! Verified downloads + the `uv` toolchain, shared by the runtime and agent
//! installers (llama.cpp, ComfyUI, Hermes).
//!
//! Stream a file while hashing it, reject on a size or SHA-256 mismatch (and
//! delete the partial), then unpack the zip. These hit GitHub / PyPI — not
//! `127.0.0.1` — but only ever behind an explicit "set up <runtime>" action,
//! and every caller refuses first in offline mode (ADR-009).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::{CoreError, Result};

/// Pinned `uv` (astral-sh) — a single static binary, `0.12.11`. Bump this URL
/// together with [`UV_ARCHIVE`]'s digest + size.
pub(crate) const UV_RELEASE_BASE: &str =
    "https://github.com/astral-sh/uv/releases/download/0.12.11";

/// `uv-x86_64-pc-windows-msvc.zip` for [`UV_RELEASE_BASE`]. SHA-256 from the
/// release's published `.sha256` sidecar; size from the release asset.
pub(crate) const UV_ARCHIVE: Archive<'static> = Archive {
    name: "uv-x86_64-pc-windows-msvc.zip",
    sha256: "e94225dea91e051472847bd6d146d7d66c4f54ffcd1f106678866a99580845f9",
    size: 16_996_332,
};

const UV_EXE: &str = if cfg!(windows) { "uv.exe" } else { "uv" };

/// One archive to fetch: URL basename, expected SHA-256 (lowercase hex) and
/// exact byte size. Both are checked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Archive<'a> {
    pub name: &'a str,
    pub sha256: &'a str,
    pub size: u64,
}

fn dl_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("download: {msg}"))
}

/// GET `url`, streaming to `dest` while hashing. Verifies size + SHA-256 and
/// deletes `dest` on any mismatch. `on_bytes` reports cumulative bytes written.
pub(crate) async fn download_verified(
    url: &str,
    archive: &Archive<'_>,
    dest: &Path,
    on_bytes: impl Fn(u64),
) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| dl_err(format!("build http client: {e}")))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| dl_err(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| dl_err(format!("GET {url}: {e}")))?;

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| dl_err(format!("create {}: {e}", dest.display())))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| dl_err(format!("download {url}: {e}")))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| dl_err(format!("write {}: {e}", dest.display())))?;
        downloaded += chunk.len() as u64;
        on_bytes(downloaded);
    }
    file.flush().await.ok();
    drop(file);

    if downloaded != archive.size {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(dl_err(format!(
            "{}: expected {} bytes, got {downloaded}",
            archive.name, archive.size
        )));
    }
    let got = hex(&hasher.finalize());
    if got != archive.sha256 {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(dl_err(format!(
            "{}: SHA-256 mismatch — expected {}, got {got}",
            archive.name, archive.sha256
        )));
    }
    Ok(())
}

/// Download + verify + unpack the pinned `uv` into `<dir>/uv/`, returning the
/// executable path. Idempotent — skips the fetch when it is already there.
/// `base` / `archive` are parameters so the installers' tests can substitute a
/// local server (production passes [`UV_RELEASE_BASE`] / [`UV_ARCHIVE`]).
pub(crate) async fn ensure_uv(
    base: &str,
    archive: &Archive<'_>,
    dir: &Path,
    on_bytes: impl Fn(u64),
) -> Result<PathBuf> {
    let exe = dir.join("uv").join(UV_EXE);
    if exe.is_file() {
        return Ok(exe);
    }
    let staging = dir.join(".uv-download");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| dl_err(format!("create {}: {e}", staging.display())))?;
    let zip = staging.join(archive.name);
    download_verified(&format!("{base}/{}", archive.name), archive, &zip, on_bytes).await?;
    extract_zip(&zip, &dir.join("uv")).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    Ok(exe)
}

/// Unpack `zip_path` into `target`, rejecting entries with unsafe paths.
pub(crate) async fn extract_zip(zip_path: &Path, target: &Path) -> Result<()> {
    unzip(zip_path, target, 0).await
}

/// Like [`extract_zip`], but drops the first path segment of every entry — for
/// GitHub source archives, which wrap everything in `<repo>-<ref>/`.
pub(crate) async fn extract_zip_flat(zip_path: &Path, target: &Path) -> Result<()> {
    unzip(zip_path, target, 1).await
}

async fn unzip(zip_path: &Path, target: &Path, strip: usize) -> Result<()> {
    let (zip_path, target) = (zip_path.to_path_buf(), target.to_path_buf());
    tokio::task::spawn_blocking(move || unzip_blocking(&zip_path, &target, strip))
        .await
        .map_err(|e| dl_err(format!("extract worker panicked: {e}")))?
}

fn unzip_blocking(zip_path: &Path, target: &Path, strip: usize) -> Result<()> {
    use std::io::Write as _;

    let file = std::fs::File::open(zip_path)
        .map_err(|e| dl_err(format!("open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| dl_err(format!("read {}: {e}", zip_path.display())))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| dl_err(format!("zip entry {i}: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(dl_err(format!(
                "zip entry {:?} has an unsafe path",
                entry.name()
            )));
        };
        let rel: std::path::PathBuf = rel.components().skip(strip).collect();
        if rel.as_os_str().is_empty() {
            continue; // the stripped top-level dir entry itself
        }
        let out = target.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| dl_err(format!("create {}: {e}", out.display())))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| dl_err(format!("create {}: {e}", parent.display())))?;
        }
        let mut w = std::fs::File::create(&out)
            .map_err(|e| dl_err(format!("create {}: {e}", out.display())))?;
        std::io::copy(&mut entry, &mut w)
            .map_err(|e| dl_err(format!("write {}: {e}", out.display())))?;
        w.flush().ok();
    }
    Ok(())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

// --- subprocess boundary ----------------------------------------------------

/// Runs one external command (a `uv` / `pip` step). Injected so the install
/// orchestration is testable without a real Python toolchain.
#[async_trait::async_trait]
pub(crate) trait CmdRunner: Send + Sync {
    async fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()>;

    /// Like [`run`](Self::run), but returns the command's stdout — the trainer
    /// adapter's import probe reads JSON the child prints
    /// (`runtime::training::TrainingAdapter::probe`). Defaults to an explicit
    /// error so a runner that only records arguments (every installer test
    /// fake) fails loudly instead of silently reporting empty output; runners
    /// used for probing override it.
    async fn run_capture(
        &self,
        program: &Path,
        _args: &[&str],
        _env: &[(&str, &str)],
    ) -> Result<String> {
        Err(dl_err(format!(
            "this runner cannot capture the output of {}",
            program.display()
        )))
    }
}

/// The real runner: `tokio::process::Command`, non-zero exit → error with the
/// tail of stderr.
pub(crate) struct SystemRunner;

#[async_trait::async_trait]
impl CmdRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()> {
        self.run_capture(program, args, env).await.map(|_| ())
    }

    async fn run_capture(
        &self,
        program: &Path,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<String> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).kill_on_drop(true);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| dl_err(format!("run {}: {e}", program.display())))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
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
        Err(dl_err(format!(
            "{} {} exited with {}:\n{tail}",
            program
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("cmd"),
            args.first().copied().unwrap_or(""),
            out.status
        )))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write as _;

    /// A minimal stored (uncompressed) zip of `(name, contents)` entries.
    pub(crate) fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
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
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::header;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    use super::test_support::make_zip;
    use super::*;

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[tokio::test]
    async fn extract_unpacks_every_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("a.zip");
        std::fs::write(&zip, make_zip(&[("bin/x.exe", b"MZ"), ("y.dll", b"dll")])).unwrap();

        extract_zip(&zip, tmp.path()).await.unwrap();

        assert_eq!(std::fs::read(tmp.path().join("bin/x.exe")).unwrap(), b"MZ");
        assert!(tmp.path().join("y.dll").is_file());
    }

    #[tokio::test]
    async fn extract_flat_drops_the_wrapping_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("src.zip");
        std::fs::write(
            &zip,
            make_zip(&[
                ("ComfyUI-0.34.0/", b""),
                ("ComfyUI-0.34.0/main.py", b"# comfy"),
                ("ComfyUI-0.34.0/comfy/model.py", b"x"),
            ]),
        )
        .unwrap();

        extract_zip_flat(&zip, tmp.path()).await.unwrap();

        assert_eq!(
            std::fs::read(tmp.path().join("main.py")).unwrap(),
            b"# comfy"
        );
        assert!(tmp.path().join("comfy/model.py").is_file());
        assert!(!tmp.path().join("ComfyUI-0.34.0").exists());
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
}
