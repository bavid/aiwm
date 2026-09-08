//! Import a local model file into the canonical store and register it.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{read_gguf_info, slugify, MIB};
use crate::db::{Database, Model, NewModel};
use crate::{CoreError, Result};

/// Headroom added to the on-disk size for a first VRAM estimate. The real KV
/// cache depends on the runtime and context length (refined by the compatibility
/// engine later).
const VRAM_HEADROOM_MB: i64 = 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    pub source_path: PathBuf,
    #[serde(default)]
    pub roles: Vec<String>,
    /// `false` (default) moves the file into the store; `true` copies it.
    #[serde(default)]
    pub keep_original: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub model: Model,
    /// True when the file's hash already matched a registered model (no-op).
    pub already_present: bool,
}

pub async fn import_model(
    db: &Database,
    store_root: &Path,
    req: ImportRequest,
) -> Result<ImportOutcome> {
    let source = req.source_path.clone();
    let meta = std::fs::metadata(&source)
        .map_err(|e| CoreError::Config(format!("cannot read {}: {e}", source.display())))?;
    if !meta.is_file() {
        return Err(CoreError::Config(format!(
            "{} is not a file",
            source.display()
        )));
    }
    if !has_ext(&source, "gguf") {
        return Err(CoreError::Config(
            "only .gguf files are supported for now".into(),
        ));
    }
    let size_bytes = meta.len();

    // Hash + inspect off the async runtime (multi-GB reads).
    let src = source.clone();
    let (sha256, gguf) = tokio::task::spawn_blocking(move || -> Result<_> {
        let sha = sha256_file(&src)?;
        let info = read_gguf_info(&src)?;
        Ok((sha, info))
    })
    .await
    .map_err(|e| CoreError::Other(anyhow::anyhow!("import worker panicked: {e}")))??;

    if let Some(existing) = db.models().find_by_sha256(&sha256).await? {
        return Ok(ImportOutcome {
            model: existing,
            already_present: true,
        });
    }

    let name = gguf
        .name
        .clone()
        .or_else(|| file_stem(&source))
        .unwrap_or_else(|| "model".to_string());

    let dest = unique_destination(store_root, &name, &sha256, &source);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
    }

    let (src, dst, keep) = (source.clone(), dest.clone(), req.keep_original);
    tokio::task::spawn_blocking(move || place_file(&src, &dst, keep))
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("copy worker panicked: {e}")))??;

    let size_mb = i64::try_from(size_bytes / MIB).unwrap_or(i64::MAX);
    let new = NewModel {
        publisher: None,
        name,
        family: gguf.architecture.clone(),
        format: "gguf".to_string(),
        quant: gguf.quantization.clone(),
        arch: gguf.architecture.clone(),
        param_count: gguf.parameter_count.and_then(|v| i64::try_from(v).ok()),
        file_path: dest.to_string_lossy().into_owned(),
        sha256: Some(sha256),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        ctx_max: gguf.context_length.and_then(|v| i64::try_from(v).ok()),
        vram_estimate_mb: Some(size_mb.saturating_add(VRAM_HEADROOM_MB)),
        ram_estimate_mb: None,
        source: "manual".to_string(),
        source_revision: None,
        roles: req.roles,
    };

    let model = db.models().insert(new).await?;
    tracing::info!(id = %model.id, name = %model.name, path = %model.file_path, "model imported");
    Ok(ImportOutcome {
        model,
        already_present: false,
    })
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| CoreError::Config(format!("open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; MIB as usize];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| CoreError::Config(format!("read: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// `<store>/llm/<slug>/<original-filename>`, disambiguated with a hash suffix if
/// something already sits at that path.
fn unique_destination(store_root: &Path, name: &str, sha256: &str, source: &Path) -> PathBuf {
    let filename = source
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model.gguf".to_string());
    let base = store_root.join("llm").join(slugify(name));
    let candidate = base.join(&filename);
    if !candidate.exists() {
        return candidate;
    }
    store_root
        .join("llm")
        .join(format!(
            "{}-{}",
            slugify(name),
            &sha256[..8.min(sha256.len())]
        ))
        .join(&filename)
}

fn place_file(src: &Path, dst: &Path, keep_original: bool) -> Result<()> {
    if !keep_original {
        match std::fs::rename(src, dst) {
            Ok(()) => return Ok(()),
            // Cross-volume move: fall through to copy + delete.
            Err(e)
                if e.raw_os_error() == Some(17)
                    || e.kind() == std::io::ErrorKind::CrossesDevices => {}
            Err(e) => return Err(CoreError::Config(format!("move into store: {e}"))),
        }
    }
    std::fs::copy(src, dst).map_err(|e| CoreError::Config(format!("copy into store: {e}")))?;
    if !keep_original {
        let _ = std::fs::remove_file(src);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::db::Database;

    /// A tiny but valid GGUF v3 with one metadata string and two tensors.
    fn gguf_bytes(name: &str) -> Vec<u8> {
        let mut kv = Vec::new();
        let gstr = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u64).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        gstr(&mut kv, "general.architecture");
        kv.extend_from_slice(&8u32.to_le_bytes());
        gstr(&mut kv, "llama");
        gstr(&mut kv, "general.name");
        kv.extend_from_slice(&8u32.to_le_bytes());
        gstr(&mut kv, name);
        gstr(&mut kv, "general.file_type");
        kv.extend_from_slice(&4u32.to_le_bytes());
        kv.extend_from_slice(&15u32.to_le_bytes()); // Q4_K_M

        let mut tensors = Vec::new();
        for (n, dims) in [
            ("token_embd.weight", [64u64, 100]),
            ("blk.0.ffn.weight", [64, 64]),
        ] {
            gstr(&mut tensors, n);
            tensors.extend_from_slice(&2u32.to_le_bytes());
            for d in dims {
                tensors.extend_from_slice(&d.to_le_bytes());
            }
            tensors.extend_from_slice(&0u32.to_le_bytes());
            tensors.extend_from_slice(&0u64.to_le_bytes());
        }

        let mut out = Vec::new();
        out.extend_from_slice(&0x4655_4747u32.to_le_bytes());
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&2u64.to_le_bytes()); // tensor count
        out.extend_from_slice(&3u64.to_le_bytes()); // kv count
        out.extend_from_slice(&kv);
        out.extend_from_slice(&tensors);
        out
    }

    fn write(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::File::create(&p)
            .unwrap()
            .write_all(&gguf_bytes("Test Model 7B"))
            .unwrap();
        p
    }

    #[tokio::test]
    async fn import_moves_file_and_registers_model() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write(tmp.path(), "qwen.Q4_K_M.gguf");

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                source_path: src.clone(),
                roles: vec!["coding".into()],
                keep_original: false,
            },
        )
        .await
        .unwrap();

        assert!(!out.already_present);
        assert_eq!(out.model.name, "Test Model 7B");
        assert_eq!(out.model.arch.as_deref(), Some("llama"));
        assert_eq!(out.model.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(out.model.param_count, Some(64 * 100 + 64 * 64));
        assert_eq!(out.model.roles, ["coding"]);
        assert!(out.model.vram_estimate_mb.unwrap() >= VRAM_HEADROOM_MB);

        assert!(!src.exists(), "source should have been moved");
        assert!(Path::new(&out.model.file_path).is_file());
        assert!(out.model.file_path.contains("test-model-7b"));
    }

    #[tokio::test]
    async fn import_copy_keeps_the_original() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let src = write(tmp.path(), "m.gguf");

        let out = import_model(
            &db,
            &tmp.path().join("store"),
            ImportRequest {
                source_path: src.clone(),
                roles: vec![],
                keep_original: true,
            },
        )
        .await
        .unwrap();

        assert!(src.exists());
        assert!(Path::new(&out.model.file_path).is_file());
    }

    #[tokio::test]
    async fn re_importing_the_same_bytes_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let a = write(tmp.path(), "a.gguf");
        let first = import_model(
            &db,
            &store,
            ImportRequest {
                source_path: a,
                roles: vec![],
                keep_original: true,
            },
        )
        .await
        .unwrap();

        let b = write(tmp.path(), "b.gguf"); // identical content
        let second = import_model(
            &db,
            &store,
            ImportRequest {
                source_path: b.clone(),
                roles: vec![],
                keep_original: false,
            },
        )
        .await
        .unwrap();

        assert!(second.already_present);
        assert_eq!(second.model.id, first.model.id);
        assert!(b.exists(), "a duplicate import must not move the file");
        assert_eq!(db.models().list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_non_gguf_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let bad = tmp.path().join("model.safetensors");
        std::fs::write(&bad, b"x").unwrap();

        let err = import_model(
            &db,
            &tmp.path().join("s"),
            ImportRequest {
                source_path: bad,
                roles: vec![],
                keep_original: true,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains(".gguf"));
    }
}
