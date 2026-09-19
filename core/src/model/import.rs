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
pub(crate) fn media_headroom_mb(family: Option<&str>) -> i64 {
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

    let pinned_file = pinned_file_for(kind, &ext, &sha256)
        .map_err(|e| CoreError::Config(format!("{}: {e}", source.display())))?;

    if let Some(existing) = db.models().find_by_sha256(&sha256).await? {
        // A pinned captioner file is loaded from its catalog location, so a
        // re-import ("reinstall the stack") must be able to repair a copy
        // that was tampered with or deleted there -- not just report the
        // row it matched.
        let model = match pinned_file {
            Some(file) => {
                let dest = store_root.join(kind.store_subdir()).join(file);
                repair_pinned_copy(db, existing, &source, &dest, &sha256, req.keep_original).await?
            }
            None => existing,
        };
        return Ok(ImportOutcome {
            model,
            already_present: true,
        });
    }

    let name = gguf
        .as_ref()
        .and_then(|g| g.name.clone())
        .or_else(|| file_stem(&source))
        .unwrap_or_else(|| "model".to_string());

    let dest = match pinned_file {
        Some(file) => store_root.join(kind.store_subdir()).join(file),
        None => unique_destination(store_root, kind, &name, &sha256, &source),
    };
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

/// Make sure the pinned file `existing` stands for really sits at `dest`
/// with the verified content `sha256` (the just-hashed `source`): when the
/// copy there is missing or no longer matches, `source` is placed there, and
/// the row is re-pointed at `dest` if it recorded another path.
async fn repair_pinned_copy(
    db: &Database,
    existing: Model,
    source: &Path,
    dest: &Path,
    sha256: &str,
    keep_original: bool,
) -> Result<Model> {
    let (src, dst, want) = (source.to_path_buf(), dest.to_path_buf(), sha256.to_string());
    tokio::task::spawn_blocking(move || -> Result<()> {
        let intact =
            dst.is_file() && sha256_file(&dst).is_ok_and(|h| h.eq_ignore_ascii_case(&want));
        if intact {
            return Ok(());
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
        }
        tracing::warn!(path = %dst.display(), "restoring a pinned captioner file");
        place_file(&src, &dst, keep_original)
    })
    .await
    .map_err(|e| CoreError::Other(anyhow::anyhow!("repair worker panicked: {e}")))??;

    let dest_str = dest.to_string_lossy();
    if existing.file_path == dest_str {
        return Ok(existing);
    }
    db.models().set_file_path(&existing.id, &dest_str).await
}

/// Extensions of a captioner snapshot directory that can steer code
/// execution: Florence-2's `.py` runs via `trust_remote_code=True`, and any
/// `.json` config can carry an `auto_map` pointing `trust_remote_code` at an
/// arbitrary Hub repo's Python. Qwen2.5-VL has no remote code, but its
/// configs are gated the same way so no `auto_map` can ever be slipped in.
fn is_code_bearing(kind: ModelKind, ext: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    match kind {
        ModelKind::Florence2Engine => matches!(ext.as_str(), "py" | "json"),
        ModelKind::QwenVlEngine => ext == "json",
        _ => false,
    }
}

/// For the Florence-2 / Qwen2.5-VL directory kinds: the catalog file name
/// this content is pinned as (`Some`), so the destination is named after the
/// catalog entry rather than whatever the source was called -- a pinned
/// `processing_florence2.py` renamed to `modeling_florence2.py` can never
/// overwrite the real sibling. A code-bearing file ([`is_code_bearing`])
/// that is not a pinned catalog entry of this exact kind is refused. `None`
/// for every other kind, and for uncatalogued weights/tokenizer data (the
/// load-time `model::integrity` check still verifies those).
fn pinned_file_for(kind: ModelKind, ext: &str, sha256: &str) -> Result<Option<&'static str>> {
    if !matches!(kind, ModelKind::Florence2Engine | ModelKind::QwenVlEngine) {
        return Ok(None);
    }
    let pinned = catalog::find_by_sha256(sha256)
        .filter(|known| known.kind == kind.as_str())
        .map(|known| known.file);
    if pinned.is_none() && is_code_bearing(kind, ext) {
        return Err(CoreError::Config(format!(
            "this .{ext} file is not one of the pinned catalog files for a {} (it could make \
             the sidecar run unreviewed code via `trust_remote_code`) — refusing to import it",
            kind.as_str()
        )));
    }
    Ok(pinned)
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
    // The headroom (sampler activations + VAE decode + compute buffers) is a
    // cost the *base* model incurs while it drives a render -- a VAE/text-
    // encoder/LoRA companion doesn't carry it, and adding it there inflated a
    // 235 MB CLIP-L encoder's estimate to ~2.8 GB.
    let is_base_model = matches!(
        kind,
        ModelKind::Checkpoint | ModelKind::DiffusionModel | ModelKind::VideoModel
    );
    let headroom_mb = if is_base_model {
        media_headroom_mb(family.as_deref())
    } else {
        0
    };
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
/// A no-op for a kind no runtime scans itself (voice files: the Python
/// sidecar is handed `file_path` directly, never a folder to search).
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
    } else if kind.comfy_folder().is_some() {
        // ComfyUI is pointed at the *directory* via extra_model_paths.yaml (3.3).
        let dir = store_root.join(kind.store_subdir());
        db.models()
            .link_runtime(&model.id, "comfyui", "extra_path", &dir.to_string_lossy())
            .await
    } else {
        Ok(())
    }
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

pub(super) fn sha256_file(path: &Path) -> Result<String> {
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
///
/// Dia's two directory-shaped kinds, the WD tagger's `model.onnx` +
/// `selected_tags.csv` pair, and the Florence-2 / Qwen2.5-VL snapshot
/// directories are the exception: they always keep their exact
/// original filename with no hash-suffix, even on a "collision" (see
/// [`unique_destination`]'s doc below for why that's actually safe).
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

    // `DiaForConditionalGeneration::from_pretrained` (and `AutoProcessor` for
    // the codec) read a *directory* of siblings by their real Hugging Face
    // filenames -- renaming one to `config-a1b2c3d4.json` on a "collision"
    // would just make the directory unloadable. The WD tagger's Python
    // sidecar (`vision.tag_frame`) is the same shape: it opens `model.onnx`
    // and `selected_tags.csv` by their fixed names in one directory, so a
    // hash-suffixed `model-a1b2c3d4.onnx` from a re-import would silently
    // strand the sidecar with no model to find. There is no real collision
    // to avoid here: each kind gets its own fixed subdirectory
    // (`ModelKind::store_subdir`), so nothing else's file ever lands next to
    // it under the same name. A leftover file already at the destination (a
    // stale partial import that never reached the DB insert below) is
    // deliberately overwritten by `place_file`, not renamed around.
    if matches!(
        kind,
        ModelKind::DiaEngine
            | ModelKind::DiaCodec
            | ModelKind::WdTagger
            | ModelKind::Florence2Engine
            | ModelKind::QwenVlEngine
    ) {
        return type_dir.join(&filename);
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
    async fn a_text_encoder_does_not_get_the_diffusion_model_headroom() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        // The diffusion-model headroom (2.5-4 GB, see `media_headroom_mb`) is
        // sampler activations + VAE decode + compute buffers -- a cost the
        // *base* model incurs while it runs, not something a VAE/text-encoder
        // companion carries too. A ~10 MB companion file must not balloon to
        // multiple GB just because it went through the same import path.
        let src = write_safetensors(tmp.path(), "clip_l.safetensors", &vec![0u8; 10_000_000]);

        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("text_encoder".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap();

        let estimate = out.model.vram_estimate_mb.unwrap();
        assert!(
            estimate < 200,
            "expected the estimate to track the ~10 MB file size, got {estimate} MB"
        );
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
    async fn a_voice_model_and_its_voices_file_import_with_no_runtime_link() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let onnx = write_safetensors(tmp.path(), "kokoro-v1.0.int8.onnx", &vec![0u8; 1_000_000]);
        let model_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("voice_model".into()),
                ..req(&onnx)
            },
        )
        .await
        .unwrap();
        assert_eq!(model_out.model.roles, ["voice_model"]);
        assert!(
            model_out.model.runtimes.is_empty(),
            "no runtime scans this itself"
        );
        assert!(db
            .models()
            .links(&model_out.model.id)
            .await
            .unwrap()
            .is_empty());
        let p = model_out.model.file_path.replace('\\', "/");
        assert!(p.contains("/voice/kokoro-v1.0.int8.onnx"), "{p}");

        // `.bin` is normally refused as a Pickle risk when *inferred* -- an
        // explicit `voice_data` hint is the one case that's actually safe
        // (Kokoro's voices file is a packed float blob, not a pickle), and
        // must not fall through to that guard.
        let bin = write_safetensors(tmp.path(), "voices-v1.0.bin", &vec![1u8; 500_000]);
        let voices_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("voice_data".into()),
                ..req(&bin)
            },
        )
        .await
        .unwrap();
        assert_eq!(voices_out.model.roles, ["voice_data"]);
        assert!(voices_out.model.runtimes.is_empty());
        let p = voices_out.model.file_path.replace('\\', "/");
        assert!(p.contains("/voice/voices-v1.0.bin"), "{p}");
    }

    #[tokio::test]
    async fn an_unhinted_bin_file_is_still_refused_as_a_pickle_risk() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let bin = write_safetensors(tmp.path(), "mystery.bin", b"??");

        let err = import_model(&db, &tmp.path().join("s"), req(&bin))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Pickle"), "{err}");
    }

    #[tokio::test]
    async fn dia_engine_files_import_with_their_exact_original_filenames_into_a_dedicated_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let config = write_safetensors(tmp.path(), "config.json", b"{\"dia\":true}");
        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("dia_engine".into()),
                ..req(&config)
            },
        )
        .await
        .unwrap();

        assert_eq!(out.model.roles, ["dia_engine"]);
        assert!(
            out.model.runtimes.is_empty(),
            "no runtime scans this itself, same as the voice kinds"
        );
        let p = out.model.file_path.replace('\\', "/");
        assert!(p.ends_with("/voice/dia-engine/config.json"), "{p}");

        // A second, differently-named sibling lands right alongside it --
        // same directory, both keeping their real names.
        let shard = write_safetensors(
            tmp.path(),
            "model-00001-of-00002.safetensors",
            &vec![0u8; 1000],
        );
        let shard_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("dia_engine".into()),
                ..req(&shard)
            },
        )
        .await
        .unwrap();
        let shard_p = shard_out.model.file_path.replace('\\', "/");
        assert!(
            shard_p.ends_with("/voice/dia-engine/model-00001-of-00002.safetensors"),
            "{shard_p}"
        );
        assert_eq!(
            Path::new(&shard_out.model.file_path).parent(),
            Path::new(&out.model.file_path).parent(),
            "both Dia engine files must be siblings in the same directory"
        );
    }

    #[tokio::test]
    async fn dia_codec_files_land_in_their_own_subdir_distinct_from_the_dia_engine_one() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        // Both repos ship a same-named "config.json" -- they must not collide.
        let engine_config = write_safetensors(tmp.path(), "config.json", b"engine");
        let engine_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("dia_engine".into()),
                ..req(&engine_config)
            },
        )
        .await
        .unwrap();

        let codec_dir = tmp.path().join("codec");
        std::fs::create_dir_all(&codec_dir).unwrap();
        let codec_config = write_safetensors(&codec_dir, "config.json", b"codec");
        let codec_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("dia_codec".into()),
                ..req(&codec_config)
            },
        )
        .await
        .unwrap();

        assert_eq!(codec_out.model.roles, ["dia_codec"]);
        let engine_p = engine_out.model.file_path.replace('\\', "/");
        let codec_p = codec_out.model.file_path.replace('\\', "/");
        assert!(
            engine_p.ends_with("/voice/dia-engine/config.json"),
            "{engine_p}"
        );
        assert!(
            codec_p.ends_with("/voice/dia-codec/config.json"),
            "{codec_p}"
        );
        assert_ne!(engine_p, codec_p);
        assert!(Path::new(&engine_p).is_file());
        assert!(Path::new(&codec_p).is_file());
    }

    #[tokio::test]
    async fn wd_tagger_files_import_with_their_exact_original_filenames_into_a_dedicated_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let model = write_safetensors(tmp.path(), "model.onnx", b"onnx-bytes-v1");
        let model_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("wd_tagger".into()),
                ..req(&model)
            },
        )
        .await
        .unwrap();

        assert_eq!(model_out.model.roles, ["vision_wd_tagger"]);
        assert!(
            model_out.model.runtimes.is_empty(),
            "no runtime scans this itself, same as the voice/dia kinds"
        );
        let model_p = model_out.model.file_path.replace('\\', "/");
        assert!(
            model_p.ends_with("/vision/wd-tagger/model.onnx"),
            "{model_p}"
        );

        // The tag list is a differently-named sibling in the same directory.
        let tags = write_safetensors(
            tmp.path(),
            "selected_tags.csv",
            b"tag_id,name,category,count",
        );
        let tags_out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("wd_tagger".into()),
                ..req(&tags)
            },
        )
        .await
        .unwrap();
        let tags_p = tags_out.model.file_path.replace('\\', "/");
        assert!(
            tags_p.ends_with("/vision/wd-tagger/selected_tags.csv"),
            "{tags_p}"
        );
        assert_eq!(
            Path::new(&tags_out.model.file_path).parent(),
            Path::new(&model_out.model.file_path).parent(),
            "the model and its tag list must be siblings in the same directory"
        );
    }

    /// Byte-exact copies of two tiny pinned Florence-2 files (the catalog
    /// hashes them: `tokenizer_config.json` 34 B, `generation_config.json`
    /// 51 B at commit 21a599d4) -- real pinned JSON without a network fetch.
    const FLORENCE2_TOKENIZER_CONFIG: &[u8] = b"{\n    \"model_max_length\": 1024\n}\n\n";
    const FLORENCE2_GENERATION_CONFIG: &[u8] =
        b"{\n    \"num_beams\": 3,\n    \"early_stopping\": false\n}";

    async fn import_as(
        db: &Database,
        store: &Path,
        src: &Path,
        kind: &str,
    ) -> Result<ImportOutcome> {
        import_model(
            db,
            store,
            ImportRequest {
                model_type: Some(kind.into()),
                ..req(src)
            },
        )
        .await
    }

    #[tokio::test]
    async fn florence2_engine_files_land_side_by_side_under_their_original_names() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let mut dirs = Vec::new();
        for (name, body) in [
            ("tokenizer_config.json", FLORENCE2_TOKENIZER_CONFIG),
            ("model.safetensors", &b"not-a-real-header"[..]),
        ] {
            let src = write_safetensors(tmp.path(), name, body);
            let out = import_as(&db, &store, &src, "florence2_engine")
                .await
                .unwrap();
            assert_eq!(out.model.roles, ["vision_florence2"]);
            assert!(out.model.runtimes.is_empty(), "sidecar-only kind");
            let p = out.model.file_path.replace('\\', "/");
            assert!(
                p.ends_with(&format!("/vision/florence2-large/{name}")),
                "{p}"
            );
            dirs.push(
                Path::new(&out.model.file_path)
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            );
        }
        assert_eq!(dirs[0], dirs[1], "one directory for `from_pretrained`");
    }

    /// `auto_map` in a config JSON can point `trust_remote_code` at any Hub
    /// repo's Python -- so a Florence-2 JSON that is not a pinned catalog
    /// file is as dangerous as unpinned `.py` and must be refused.
    #[tokio::test]
    async fn an_unpinned_florence2_config_json_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(
            tmp.path(),
            "config.json",
            br#"{"auto_map": {"AutoModelForCausalLM": "evil/repo--modeling.X"}}"#,
        );
        let err = import_as(&db, &store, &src, "florence2_engine")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pinned"), "{err}");
        assert!(!store.join("vision/florence2-large/config.json").exists());
        assert!(db.models().list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unpinned_qwen_vl_json_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(tmp.path(), "tokenizer_config.json", b"{}");
        let err = import_as(&db, &store, &src, "qwen_vl_engine")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pinned"), "{err}");
        assert!(db.models().list().await.unwrap().is_empty());
    }

    /// "Reinstall the stack" must actually repair a pinned folder: a
    /// re-download matches the existing row by SHA-256, and instead of a
    /// bare `already_present` no-op it puts the verified bytes back where the
    /// integrity check will look for them.
    #[tokio::test]
    async fn re_importing_a_pinned_file_restores_a_corrupted_or_deleted_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let dest = store.join("vision/florence2-large/tokenizer_config.json");

        let first = write_safetensors(
            tmp.path(),
            "tokenizer_config.json",
            FLORENCE2_TOKENIZER_CONFIG,
        );
        let out = import_as(&db, &store, &first, "florence2_engine")
            .await
            .unwrap();
        assert!(!out.already_present);

        // Tampered in place, then re-downloaded.
        std::fs::write(&dest, b"{\"auto_map\": {}}\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n").unwrap();
        let again = write_safetensors(
            tmp.path(),
            "tokenizer_config.json",
            FLORENCE2_TOKENIZER_CONFIG,
        );
        let out = import_as(&db, &store, &again, "florence2_engine")
            .await
            .unwrap();
        assert!(out.already_present);
        assert_eq!(std::fs::read(&dest).unwrap(), FLORENCE2_TOKENIZER_CONFIG);
        assert_eq!(Path::new(&out.model.file_path), dest.as_path());

        // Deleted, then re-downloaded.
        std::fs::remove_file(&dest).unwrap();
        let third = write_safetensors(
            tmp.path(),
            "tokenizer_config.json",
            FLORENCE2_TOKENIZER_CONFIG,
        );
        import_as(&db, &store, &third, "florence2_engine")
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), FLORENCE2_TOKENIZER_CONFIG);
        assert_eq!(
            db.models().list().await.unwrap().len(),
            1,
            "no duplicate row"
        );
    }

    /// The row's recorded path is corrected when the repair lands the file
    /// somewhere else (e.g. the row was re-pointed by hand).
    #[tokio::test]
    async fn a_repaired_pinned_file_updates_the_rows_path() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let first = write_safetensors(
            tmp.path(),
            "tokenizer_config.json",
            FLORENCE2_TOKENIZER_CONFIG,
        );
        let out = import_as(&db, &store, &first, "florence2_engine")
            .await
            .unwrap();
        let elsewhere = tmp.path().join("elsewhere.json");
        db.models()
            .set_file_path(&out.model.id, &elsewhere.to_string_lossy())
            .await
            .unwrap();

        let again = write_safetensors(
            tmp.path(),
            "tokenizer_config.json",
            FLORENCE2_TOKENIZER_CONFIG,
        );
        let out = import_as(&db, &store, &again, "florence2_engine")
            .await
            .unwrap();
        let dest = store.join("vision/florence2-large/tokenizer_config.json");
        assert_eq!(Path::new(&out.model.file_path), dest.as_path());
        let stored = db.models().get(&out.model.id).await.unwrap().unwrap();
        assert_eq!(Path::new(&stored.file_path), dest.as_path());
    }

    /// A pinned file imported under another pinned file's name must land
    /// under its *own* catalog name -- never overwrite the real sibling.
    #[tokio::test]
    async fn a_pinned_file_lands_under_its_catalog_name_not_the_source_name() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let real = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        import_as(&db, &store, &real, "florence2_engine")
            .await
            .unwrap();

        let sub = tmp.path().join("renamed");
        std::fs::create_dir_all(&sub).unwrap();
        let renamed = write_safetensors(&sub, "generation_config.json", FLORENCE2_TOKENIZER_CONFIG);
        let out = import_as(&db, &store, &renamed, "florence2_engine")
            .await
            .unwrap();
        let p = out.model.file_path.replace('\\', "/");
        assert!(
            p.ends_with("/vision/florence2-large/tokenizer_config.json"),
            "{p}"
        );
        assert_eq!(
            std::fs::read(store.join("vision/florence2-large/generation_config.json")).unwrap(),
            FLORENCE2_GENERATION_CONFIG,
            "the real sibling is untouched"
        );
    }

    #[tokio::test]
    async fn qwen_vl_engine_files_land_side_by_side_under_their_original_names() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let mut dirs = Vec::new();
        for (name, body) in [
            ("merges.txt", &b"#version: 0.2\n"[..]),
            ("model-00001-of-00005.safetensors", &b"shard-bytes"[..]),
        ] {
            let src = write_safetensors(tmp.path(), name, body);
            let out = import_model(
                &db,
                &store,
                ImportRequest {
                    model_type: Some("qwen_vl_engine".into()),
                    ..req(&src)
                },
            )
            .await
            .unwrap();
            assert_eq!(out.model.roles, ["vision_qwen2_5_vl"]);
            let p = out.model.file_path.replace('\\', "/");
            assert!(p.ends_with(&format!("/vision/qwen2.5-vl-7b/{name}")), "{p}");
            dirs.push(
                Path::new(&out.model.file_path)
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            );
        }
        assert_eq!(dirs[0], dirs[1]);
    }

    #[tokio::test]
    async fn a_remote_code_python_file_outside_the_catalog_is_refused() {
        // Florence-2 runs its `.py` files via `trust_remote_code=True` --
        // only the exact, pinned files the catalog lists may ever be placed
        // where the sidecar would execute them.
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(tmp.path(), "modeling_florence2.py", b"import os\n");
        let err = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("florence2_engine".into()),
                ..req(&src)
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("pinned"), "{err}");
        assert!(!store
            .join("vision/florence2-large/modeling_florence2.py")
            .exists());
        assert!(db.models().list().await.unwrap().is_empty());
    }

    #[test]
    fn code_bearing_files_are_allowed_only_as_catalogued_files_of_the_same_kind() {
        let pinned = crate::model::KNOWN_MODELS
            .iter()
            .find(|m| m.kind == "florence2_engine" && m.file.ends_with(".py"))
            .expect("the catalog pins Florence-2's remote code");
        let florence = ModelKind::Florence2Engine;
        let qwen = ModelKind::QwenVlEngine;
        let unknown = "0".repeat(64);

        assert_eq!(
            pinned_file_for(florence, "py", pinned.sha256).unwrap(),
            Some(pinned.file)
        );
        assert_eq!(
            pinned_file_for(florence, "PY", &pinned.sha256.to_ascii_uppercase()).unwrap(),
            Some(pinned.file)
        );
        assert!(pinned_file_for(florence, "py", &unknown).is_err());
        assert!(pinned_file_for(florence, "json", &unknown).is_err());
        // Another kind's pinned file is not this kind's.
        // (Qwen never accepts `.py` at all -- `resolve_kind` refuses it
        // before this gate -- so the cross-kind case is a pinned JSON.)
        let florence_json = crate::model::KNOWN_MODELS
            .iter()
            .find(|m| m.kind == "florence2_engine" && m.file == "config.json")
            .unwrap();
        assert!(pinned_file_for(qwen, "json", florence_json.sha256).is_err());
        assert!(pinned_file_for(qwen, "json", &unknown).is_err());
        // Weights / tokenizer data are not gated at import (the load-time
        // integrity check covers them) and keep their source name.
        assert_eq!(
            pinned_file_for(florence, "safetensors", &unknown).unwrap(),
            None
        );
        assert_eq!(pinned_file_for(qwen, "txt", &unknown).unwrap(), None);
        // Other kinds are untouched by the gate.
        assert_eq!(
            pinned_file_for(ModelKind::DiaEngine, "json", &unknown).unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_stale_leftover_wd_tagger_file_is_overwritten_not_hash_suffixed() {
        // Same reasoning as the Dia test below: a previous import that copied
        // `model.onnx` into the store but crashed before the DB insert (or a
        // corrected re-download) must land back on the exact same filename --
        // the sidecar looks for `model.onnx` by that literal name, never a
        // `model-a1b2c3d4.onnx` it would never find.
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let stale_dest = store.join("vision/wd-tagger/model.onnx");
        std::fs::create_dir_all(stale_dest.parent().unwrap()).unwrap();
        std::fs::write(&stale_dest, b"stale-leftover-bytes").unwrap();

        let fresh = write_safetensors(tmp.path(), "model.onnx", b"the-real-current-onnx-bytes");
        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("wd_tagger".into()),
                ..req(&fresh)
            },
        )
        .await
        .unwrap();

        let p = out.model.file_path.replace('\\', "/");
        assert!(
            p.ends_with("/vision/wd-tagger/model.onnx"),
            "must not have been hash-suffixed: {p}"
        );
        assert_eq!(
            std::fs::read(&out.model.file_path).unwrap(),
            b"the-real-current-onnx-bytes"
        );
    }

    #[tokio::test]
    async fn a_stale_leftover_dia_file_is_overwritten_not_hash_suffixed() {
        // Simulates a previous import that copied the file into the store
        // but crashed before the DB insert -- a retry must land back on the
        // exact same original filename (Dia needs it to be *that* name),
        // never a `config-a1b2c3d4.json` the loader would never look for.
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        let stale_dest = store.join("voice/dia-engine/config.json");
        std::fs::create_dir_all(stale_dest.parent().unwrap()).unwrap();
        std::fs::write(&stale_dest, b"stale-leftover-bytes").unwrap();

        let fresh = write_safetensors(tmp.path(), "config.json", b"the-real-current-config");
        let out = import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("dia_engine".into()),
                ..req(&fresh)
            },
        )
        .await
        .unwrap();

        let p = out.model.file_path.replace('\\', "/");
        assert!(
            p.ends_with("/voice/dia-engine/config.json"),
            "must not have been hash-suffixed: {p}"
        );
        assert_eq!(
            std::fs::read(&out.model.file_path).unwrap(),
            b"the-real-current-config"
        );
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
