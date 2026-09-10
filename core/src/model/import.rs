//! Import a local model file into the canonical store and register it.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    catalog, read_gguf_info, read_safetensors_info, slugify, ModelKind, SafetensorsInfo, MIB,
};
use crate::compat::{self, ModelDims};
use crate::db::{Database, Model, NewModel};
use crate::{CoreError, Result};

/// VRAM headroom over the on-disk weight size for a diffusion / video model —
/// sampler activations + VAE decode + CUDA/compute buffers, per family. Still a
/// documented heuristic (`docs/HARDWARE.md` ranges); real calibration waits on
/// the hands-on ComfyUI run (slice 4.0).
fn media_headroom_mb(family: Option<&str>) -> i64 {
    match family {
        // Wan / LTX video: temporal attention over a long latent is dear.
        Some("wan") => 6144,
        Some("ltx") => 4096,
        // Flux / SD3: a big DiT plus a partly-resident T5 encoder.
        Some("flux") | Some("sd3") => 4096,
        // SDXL / SD1.5: a comfortable UNet.
        Some("sdxl") => 2048,
        // Unknown → a middle guess.
        _ => 2560,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    pub source_path: PathBuf,
    #[serde(default)]
    pub roles: Vec<String>,
    /// `false` (default) moves the file into the store; `true` copies it.
    #[serde(default)]
    pub keep_original: bool,
    /// What the file is: `chat`, `checkpoint`, `vae`, `lora`, `diffusion_model`,
    /// `text_encoder`. Omitted → inferred from the extension (`.gguf` → chat,
    /// `.safetensors` → checkpoint).
    #[serde(default)]
    pub model_type: Option<String>,
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
    let size_bytes = meta.len();
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let kind = resolve_kind(req.model_type.as_deref(), &ext)?;

    // Hash off the async runtime (multi-GB reads); inspect the header by format.
    // A GGUF parse failure is fatal (the file claims to be a GGUF); a
    // `.safetensors` header we can't read just means "no extra metadata".
    let src = source.clone();
    let is_llm = kind.is_llm();
    let is_safetensors = ext == "safetensors";
    let (sha256, gguf, safet) = tokio::task::spawn_blocking(move || -> Result<_> {
        let sha = sha256_file(&src)?;
        let gguf = if is_llm {
            Some(read_gguf_info(&src)?)
        } else {
            None
        };
        let safet = if is_safetensors {
            read_safetensors_info(&src)
                .map_err(|e| tracing::debug!(error = %e, "safetensors header not readable"))
                .ok()
        } else {
            None
        };
        Ok((sha, gguf, safet))
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
        .as_ref()
        .and_then(|g| g.name.clone())
        .or_else(|| file_stem(&source))
        .unwrap_or_else(|| "model".to_string());

    let dest = unique_destination(store_root, kind, &name, &sha256, &source);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
    }

    let (src, dst, keep) = (source.clone(), dest.clone(), req.keep_original);
    tokio::task::spawn_blocking(move || place_file(&src, &dst, keep))
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("copy worker panicked: {e}")))??;

    let mut new = match &gguf {
        Some(g) => gguf_new_model(g, &name, &dest, sha256.clone(), size_bytes, req.roles),
        None => media_new_model(
            kind,
            &name,
            &dest,
            &ext,
            safet.as_ref(),
            sha256.clone(),
            size_bytes,
        ),
    };
    if let Some(known) = catalog::find_by_sha256(&sha256) {
        // A verified match to the curated list: take its authoritative metadata.
        tracing::info!(catalog = known.id, "import recognized a known model");
        new.publisher = Some(known.publisher.to_string());
        new.source_revision = Some(format!("catalog:{}", known.id));
        if let Some(family) = known.family {
            new.family = Some(family.to_string());
        }
    }

    let model = db.models().insert(new).await?;
    link_for_kind(db, &model, kind, store_root).await?;
    let model = db
        .models()
        .get(&model.id)
        .await?
        .ok_or_else(|| CoreError::Db("model vanished right after import".into()))?;

    tracing::info!(id = %model.id, name = %model.name, kind = kind.as_str(), path = %model.file_path, "model imported");
    Ok(ImportOutcome {
        model,
        already_present: false,
    })
}

/// The requested (or inferred) kind, validated against the file's extension.
fn resolve_kind(hint: Option<&str>, ext: &str) -> Result<ModelKind> {
    let kind = match hint.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => ModelKind::from_hint(h)
            .ok_or_else(|| CoreError::Config(format!("unknown model type {h:?}")))?,
        None => ModelKind::default_for_ext(ext).ok_or_else(|| {
            CoreError::Config(if matches!(ext, "ckpt" | "bin" | "pt" | "pth") {
                format!(
                    ".{ext} is a Pickle format (can execute code on load) — convert to \
                     .safetensors first"
                )
            } else {
                "only .gguf and .safetensors files are supported".into()
            })
        })?,
    };
    if !kind.accepts_ext(ext) {
        return Err(CoreError::Config(format!(
            "a {} cannot be a .{ext} file",
            kind.as_str()
        )));
    }
    Ok(kind)
}

fn gguf_new_model(
    gguf: &super::GgufInfo,
    name: &str,
    dest: &Path,
    sha256: String,
    size_bytes: u64,
    roles: Vec<String>,
) -> NewModel {
    let dims = ModelDims {
        size_bytes,
        ctx_max: gguf.context_length.and_then(|v| u32::try_from(v).ok()),
        param_count: gguf.parameter_count,
        n_layers: gguf.block_count.and_then(|v| u32::try_from(v).ok()),
        n_embd: gguf.embedding_length.and_then(|v| u32::try_from(v).ok()),
        n_heads: gguf.head_count.and_then(|v| u32::try_from(v).ok()),
        n_kv_heads: gguf.head_count_kv.and_then(|v| u32::try_from(v).ok()),
    };
    // Estimate for the context a chat actually runs at, not the trained maximum.
    let estimate = compat::estimate(&dims, compat::effective_ctx(dims.ctx_max));
    NewModel {
        name: name.to_string(),
        family: gguf.architecture.clone(),
        format: "gguf".to_string(),
        quant: gguf.quantization.clone(),
        arch: gguf.architecture.clone(),
        param_count: gguf.parameter_count.and_then(|v| i64::try_from(v).ok()),
        file_path: dest.to_string_lossy().into_owned(),
        sha256: Some(sha256),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        ctx_max: gguf.context_length.and_then(|v| i64::try_from(v).ok()),
        vram_estimate_mb: i64::try_from(estimate.total_mb).ok(),
        source: "manual".to_string(),
        n_layers: dims.n_layers.map(i64::from),
        n_embd: dims.n_embd.map(i64::from),
        n_heads: dims.n_heads.map(i64::from),
        n_kv_heads: dims.n_kv_heads.map(i64::from),
        roles,
        ..NewModel::default()
    }
}

/// A `.safetensors` / `.gguf` image or video model. The family still comes from
/// the file name (safetensors headers rarely say "flux"); the `.safetensors`
/// header, when readable, fills in the parameter count and weight precision and
/// firms up the VRAM estimate. The role comes from the typed [`ModelKind`] so
/// `Auto` selection can find it.
fn media_new_model(
    kind: ModelKind,
    name: &str,
    dest: &Path,
    ext: &str,
    safet: Option<&SafetensorsInfo>,
    sha256: String,
    size_bytes: u64,
) -> NewModel {
    let family = media_family(name);
    let headroom_mb = media_headroom_mb(family.as_deref());
    let size_mb = i64::try_from(size_bytes / MIB).unwrap_or(i64::MAX);
    let arch = safet
        .and_then(|s| s.metadata.get("modelspec.architecture").cloned())
        .or_else(|| safet.and_then(|s| s.metadata.get("architecture").cloned()));
    NewModel {
        name: name.to_string(),
        family,
        format: ext.to_string(),
        quant: safet.and_then(|s| s.precision.clone()),
        arch,
        param_count: safet
            .and_then(|s| s.parameter_count)
            .and_then(|n| i64::try_from(n).ok()),
        file_path: dest.to_string_lossy().into_owned(),
        sha256: Some(sha256),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        vram_estimate_mb: Some(size_mb.saturating_add(headroom_mb)),
        source: "manual".to_string(),
        roles: kind
            .default_role()
            .map(str::to_string)
            .into_iter()
            .collect(),
        ..NewModel::default()
    }
}

/// Guess the diffusion / video family from a model's file name. Flux / SD3 / Wan
/// / LTX carry a large T5/umt5 encoder — see [`media_headroom_mb`].
fn media_family(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    let f = if n.contains("flux") {
        "flux"
    } else if n.contains("sd3") || n.contains("sd35") {
        "sd3"
    } else if n.contains("wan") {
        "wan"
    } else if n.contains("ltx") {
        "ltx"
    } else if n.contains("xl") {
        "sdxl"
    } else {
        return None;
    };
    Some(f.to_string())
}

/// Record which runtime can reach the freshly-imported model, and how (ADR-007).
async fn link_for_kind(
    db: &Database,
    model: &Model,
    kind: ModelKind,
    store_root: &Path,
) -> Result<()> {
    if kind.is_llm() {
        // llama-server takes `-m <absolute path>`.
        db.models()
            .link_runtime(&model.id, "llamacpp", "passthrough", &model.file_path)
            .await
    } else {
        // ComfyUI is pointed at the *directory* via extra_model_paths.yaml (3.3).
        let dir = store_root.join(kind.store_subdir());
        db.models()
            .link_runtime(&model.id, "comfyui", "extra_path", &dir.to_string_lossy())
            .await
    }
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

/// Where the file lands in the store. Chat models get a per-model slug directory
/// (`<store>/llm/<slug>/<file>`); image models sit flat in their typed folder
/// (`<store>/image/checkpoints/<file>`) to match ComfyUI's own layout. A name
/// clash is broken with an 8-char hash.
fn unique_destination(
    store_root: &Path,
    kind: ModelKind,
    name: &str,
    sha256: &str,
    source: &Path,
) -> PathBuf {
    let filename = source
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model.bin".to_string());
    let hash8 = &sha256[..8.min(sha256.len())];
    let type_dir = store_root.join(kind.store_subdir());

    if kind.is_llm() {
        let by_slug = type_dir.join(slugify(name));
        let candidate = by_slug.join(&filename);
        return if candidate.exists() {
            type_dir
                .join(format!("{}-{hash8}", slugify(name)))
                .join(&filename)
        } else {
            candidate
        };
    }

    let candidate = type_dir.join(&filename);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, dot_ext) = filename
        .rsplit_once('.')
        .map_or((filename.as_str(), String::new()), |(s, e)| {
            (s, format!(".{e}"))
        });
    type_dir.join(format!("{stem}-{hash8}{dot_ext}"))
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

    /// A tiny but valid GGUF v3 with a few metadata keys and two tensors.
    fn gguf_bytes(name: &str) -> Vec<u8> {
        let mut kv = Vec::new();
        let gstr = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u64).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        let kv_str = |b: &mut Vec<u8>, key: &str, val: &str| {
            gstr(b, key);
            b.extend_from_slice(&8u32.to_le_bytes());
            gstr(b, val);
        };
        let kv_u32 = |b: &mut Vec<u8>, key: &str, val: u32| {
            gstr(b, key);
            b.extend_from_slice(&4u32.to_le_bytes());
            b.extend_from_slice(&val.to_le_bytes());
        };
        kv_str(&mut kv, "general.architecture", "llama");
        kv_str(&mut kv, "general.name", name);
        kv_u32(&mut kv, "general.file_type", 15); // Q4_K_M
        kv_u32(&mut kv, "llama.block_count", 2);
        kv_u32(&mut kv, "llama.embedding_length", 64);
        kv_u32(&mut kv, "llama.attention.head_count", 4);
        kv_u32(&mut kv, "llama.attention.head_count_kv", 2);

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
        out.extend_from_slice(&7u64.to_le_bytes()); // kv count
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

    /// A `.safetensors` whose header the importer *can't* parse (raw bytes).
    fn write_safetensors(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    /// A `.safetensors` with a real header: one BF16 weight of `params` elements.
    fn write_real_safetensors(dir: &Path, name: &str, params: u64) -> PathBuf {
        let header = serde_json::json!({
            "__metadata__": { "modelspec.architecture": "flux-1-dev" },
            "w": { "dtype": "BF16", "shape": [params], "data_offsets": [0, params * 2] }
        });
        let json = serde_json::to_vec(&header).unwrap();
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&json);
        bytes.extend(std::iter::repeat_n(0u8, 32));
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn req(source: &Path) -> ImportRequest {
        ImportRequest {
            source_path: source.to_path_buf(),
            roles: vec![],
            keep_original: false,
            model_type: None,
        }
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
                roles: vec!["coding".into()],
                ..req(&src)
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

        // Architecture dims came off the GGUF header and feed the estimate.
        assert_eq!(out.model.n_layers, Some(2));
        assert_eq!(out.model.n_kv_heads, Some(2));
        let est = crate::compat::estimate(
            &out.model.vram_dims(),
            crate::compat::effective_ctx(out.model.ctx_max.and_then(|v| u32::try_from(v).ok())),
        );
        assert!(!est.kv_is_rough);
        assert_eq!(out.model.vram_estimate_mb, i64::try_from(est.total_mb).ok());
        assert!(out.model.vram_estimate_mb.unwrap() >= est.overhead_mb as i64);

        // A GGUF is registered as reachable by llama.cpp (passthrough — ADR-007).
        assert_eq!(out.model.runtimes, ["llamacpp"]);
        let links = db.models().links(&out.model.id).await.unwrap();
        assert_eq!(links[0].strategy, "passthrough");
        assert_eq!(links[0].link_path, out.model.file_path);

        assert!(!src.exists(), "source should have been moved");
        assert!(Path::new(&out.model.file_path).is_file());
        assert!(out.model.file_path.contains("test-model-7b"));
        assert!(out.model.file_path.replace('\\', "/").contains("/llm/"));
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
                keep_original: true,
                ..req(&src)
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
                keep_original: true,
                ..req(&a)
            },
        )
        .await
        .unwrap();

        let b = write(tmp.path(), "b.gguf"); // identical content
        let second = import_model(&db, &store, req(&b)).await.unwrap();

        assert!(second.already_present);
        assert_eq!(second.model.id, first.model.id);
        assert!(b.exists(), "a duplicate import must not move the file");
        assert_eq!(db.models().list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn safetensors_default_to_a_checkpoint_reachable_by_comfyui() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(
            tmp.path(),
            "sd_xl_base_1.0.safetensors",
            &vec![0u8; 3_000_000],
        );

        let out = import_model(&db, &store, req(&src)).await.unwrap();

        assert_eq!(out.model.format, "safetensors");
        assert_eq!(out.model.arch, None, "the raw-bytes header is not readable");
        assert_eq!(out.model.param_count, None);
        assert_eq!(out.model.runtimes, ["comfyui"]);
        // Typed → carries the role so `Auto` image jobs find it.
        assert_eq!(out.model.roles, ["base_diffusion"]);
        assert_eq!(out.model.family.as_deref(), Some("sdxl"));
        let p = out.model.file_path.replace('\\', "/");
        assert!(
            p.contains("/image/checkpoints/sd_xl_base_1.0.safetensors"),
            "{p}"
        );
        // ~3 MB file + the SDXL headroom.
        assert!(out.model.vram_estimate_mb.unwrap() >= media_headroom_mb(Some("sdxl")));

        let links = db.models().links(&out.model.id).await.unwrap();
        assert_eq!(links[0].strategy, "extra_path");
        assert!(links[0]
            .link_path
            .replace('\\', "/")
            .ends_with("image/checkpoints"));
    }

    #[tokio::test]
    async fn a_real_safetensors_header_fills_in_params_and_precision() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_real_safetensors(tmp.path(), "flux1-dev.safetensors", 12_000_000_000);

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("diffusion_model".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap();

        assert_eq!(out.model.param_count, Some(12_000_000_000));
        assert_eq!(out.model.quant.as_deref(), Some("BF16"));
        assert_eq!(out.model.family.as_deref(), Some("flux"));
    }

    #[tokio::test]
    async fn a_flux_checkpoint_gets_a_bigger_headroom_and_the_family() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(tmp.path(), "flux1-dev.safetensors", &vec![0u8; 2_000_000]);

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("diffusion_model".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap();

        assert_eq!(out.model.family.as_deref(), Some("flux"));
        assert_eq!(out.model.roles, ["base_diffusion"]);
        assert!(out.model.vram_estimate_mb.unwrap() >= media_headroom_mb(Some("flux")));
        let p = out.model.file_path.replace('\\', "/");
        assert!(p.contains("/image/diffusion_models/"), "{p}");
    }

    #[tokio::test]
    async fn a_wan_video_model_routes_to_the_video_store_with_base_video_role() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(
            tmp.path(),
            "wan2.2_ti2v_5B_fp16.safetensors",
            &vec![0u8; 1_000_000],
        );

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("video".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap();

        assert_eq!(out.model.family.as_deref(), Some("wan"));
        assert_eq!(out.model.roles, ["base_video"]);
        assert_eq!(out.model.runtimes, ["comfyui"]);
        let p = out.model.file_path.replace('\\', "/");
        assert!(p.contains("/video/diffusion_models/"), "{p}");
        let links = db.models().links(&out.model.id).await.unwrap();
        assert!(links[0]
            .link_path
            .replace('\\', "/")
            .ends_with("video/diffusion_models"));
    }

    #[tokio::test]
    async fn explicit_model_type_routes_to_its_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(tmp.path(), "sdxl.vae.safetensors", b"vae-bytes");

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("vae".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap();

        assert!(out
            .model
            .file_path
            .replace('\\', "/")
            .contains("/image/vae/"));
    }

    #[tokio::test]
    async fn rejects_pickle_and_unknown_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();

        let ckpt = write_safetensors(tmp.path(), "old.ckpt", b"PK");
        let err = import_model(&db, &tmp.path().join("s"), req(&ckpt))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Pickle"), "{err}");

        let zip = write_safetensors(tmp.path(), "weights.zip", b"PK");
        let err = import_model(&db, &tmp.path().join("s"), req(&zip))
            .await
            .unwrap_err();
        assert!(err.to_string().contains(".safetensors"), "{err}");
    }

    #[tokio::test]
    async fn a_gguf_declared_as_a_vae_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let src = write(tmp.path(), "x.gguf");

        let err = import_model(
            &db,
            &tmp.path().join("s"),
            ImportRequest {
                model_type: Some("vae".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("cannot be a .gguf"), "{err}");
    }
}
