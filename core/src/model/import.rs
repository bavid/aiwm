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
                let model =
                    repair_pinned_copy(db, existing, &source, &dest, &sha256, req.keep_original)
                        .await?;
                prune_pinned_folder(db, store_root, kind).await?;
                model
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
    let is_pinned = pinned_file.is_some();
    tokio::task::spawn_blocking(move || {
        if is_pinned {
            // Same rule as the repair path: never write through a link.
            remove_link_at(&dst)?;
        }
        place_file(&src, &dst, keep)
    })
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
    if is_pinned {
        prune_pinned_folder(db, store_root, kind).await?;
    }
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
        // `symlink_metadata`: a link to a correct file elsewhere is not an
        // intact copy (the load-time check refuses links too).
        let is_regular = std::fs::symlink_metadata(&dst).is_ok_and(|m| m.file_type().is_file());
        let intact = is_regular && sha256_file(&dst).is_ok_and(|h| h.eq_ignore_ascii_case(&want));
        if intact {
            return Ok(());
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Config(format!("create {}: {e}", parent.display())))?;
        }
        tracing::warn!(path = %dst.display(), "restoring a pinned captioner file");
        remove_link_at(&dst)?;
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

/// Remove a symlink or junction sitting where a pinned file is about to be
/// restored -- the link itself, never its target -- so the restore can
/// never write through it. A regular file is left for `place_file` to
/// replace; a real directory is refused rather than deleted.
fn remove_link_at(dst: &Path) -> Result<()> {
    let Ok(meta) = std::fs::symlink_metadata(dst) else {
        return Ok(()); // nothing there
    };
    let kind = meta.file_type();
    if kind.is_file() {
        return Ok(());
    }
    if !kind.is_symlink() {
        return Err(CoreError::Config(format!(
            "{} is a folder, not the pinned file -- remove it and reinstall the stack",
            dst.display()
        )));
    }
    // A directory link (junction / dir symlink) is removed with remove_dir,
    // a file symlink with remove_file; neither touches the target.
    std::fs::remove_dir(dst)
        .or_else(|_| std::fs::remove_file(dst))
        .map_err(|e| CoreError::Config(format!("remove link {}: {e}", dst.display())))
}

/// After a pinned captioner file of `kind` landed (install or repair): bring
/// its store folder back to "only catalog files". An install of an earlier
/// catalog revision (Plan 8: the `microsoft/Florence-2-large` remote-code
/// snapshot) leaves files the current catalog does not list, and the
/// load-time check refuses any extra file -- so they are removed here, and
/// every library row under that folder whose content is not a current
/// catalog file of this kind is dropped (the rows of removed files and of
/// files a new download overwrote), so stale rows can never make the
/// captioner read as installed.
///
/// Only entries directly inside the exact store folder are touched: a folder
/// that resolves elsewhere (junction) is left alone, links inside it are
/// removed as links (never followed), real subfolders are left in place.
async fn prune_pinned_folder(db: &Database, store_root: &Path, kind: ModelKind) -> Result<()> {
    let root = store_root.to_path_buf();
    let in_store = tokio::task::spawn_blocking(move || prune_non_catalog_entries(&root, kind))
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("prune worker panicked: {e}")))??;
    if !in_store {
        return Ok(());
    }
    let dir = store_root.join(kind.store_subdir());
    for m in db.models().list().await? {
        let path = Path::new(&m.file_path);
        if path.parent() != Some(dir.as_path()) || is_current_catalog_row(kind, &m) {
            continue;
        }
        tracing::warn!(id = %m.id, path = %m.file_path, "dropping the row of a stale captioner file");
        db.models().delete(&m.id).await?;
    }
    Ok(())
}

/// The row's content is a current catalog file of `kind`, under its name.
fn is_current_catalog_row(kind: ModelKind, m: &Model) -> bool {
    let name = Path::new(&m.file_path).file_name().and_then(|n| n.to_str());
    m.sha256
        .as_deref()
        .and_then(catalog::find_by_sha256)
        .is_some_and(|known| known.kind == kind.as_str() && Some(known.file) == name)
}

/// Remove every entry of `<store>/<kind.store_subdir()>` that is not named
/// after a catalog file of `kind`. Returns `false` (and touches nothing)
/// when the folder is missing or resolves outside the store.
fn prune_non_catalog_entries(store_root: &Path, kind: ModelKind) -> Result<bool> {
    let subdir = kind.store_subdir();
    let dir = store_root.join(subdir);
    let (Ok(canonical_dir), Ok(canonical_root)) = (dir.canonicalize(), store_root.canonicalize())
    else {
        return Ok(false);
    };
    if canonical_dir != canonical_root.join(subdir) {
        tracing::warn!(dir = %dir.display(), "pinned folder resolves outside the store; not pruning");
        return Ok(false);
    }
    let keep: Vec<&str> = catalog::KNOWN_MODELS
        .iter()
        .filter(|m| m.kind == kind.as_str())
        .map(|m| m.file)
        .collect();
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| CoreError::Config(format!("cannot list {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| CoreError::Config(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name.as_str()) {
            continue;
        }
        remove_stale_entry(&entry.path())?;
    }
    Ok(true)
}

/// Delete one stale folder entry: a regular file, or a link as a link.
fn remove_stale_entry(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| CoreError::Config(format!("{}: {e}", path.display())))?;
    let file_type = meta.file_type();
    if file_type.is_file() {
        tracing::warn!(path = %path.display(), "removing a file that is not in the captioner catalog");
        std::fs::remove_file(path)
            .map_err(|e| CoreError::Config(format!("remove {}: {e}", path.display())))
    } else if file_type.is_symlink() {
        remove_link_at(path)
    } else {
        tracing::warn!(path = %path.display(), "leaving a subfolder in a captioner folder");
        Ok(())
    }
}

/// Extensions of a captioner snapshot directory that can steer what gets
/// loaded: any `.json` config can carry an `auto_map` that would point
/// `trust_remote_code` at an arbitrary Hub repo's Python. Neither captioner
/// loads remote code, but their configs are gated so no `auto_map` can ever
/// be slipped in.
fn is_code_bearing(kind: ModelKind, ext: &str) -> bool {
    matches!(kind, ModelKind::Florence2Engine | ModelKind::QwenVlEngine)
        && ext.eq_ignore_ascii_case("json")
}

/// For the Florence-2 / Qwen2.5-VL directory kinds: the catalog file name
/// this content is pinned as (`Some`), so the destination is named after the
/// catalog entry rather than whatever the source was called -- a pinned
/// `generation_config.json` renamed to `config.json` can never overwrite the
/// real sibling. **Any** file that is not a pinned catalog
/// entry of this exact kind is refused -- code-bearing files because they
/// could run unreviewed code, weights and tokenizer data because the folder
/// is loaded as a whole and would only fail the load-time
/// `model::integrity` check later. `None` for every other kind.
fn pinned_file_for(kind: ModelKind, ext: &str, sha256: &str) -> Result<Option<&'static str>> {
    if !matches!(kind, ModelKind::Florence2Engine | ModelKind::QwenVlEngine) {
        return Ok(None);
    }
    let pinned = catalog::find_by_sha256(sha256)
        .filter(|known| known.kind == kind.as_str())
        .map(|known| known.file);
    if pinned.is_none() {
        let why = if is_code_bearing(kind, ext) {
            "an unreviewed config could steer what the sidecar loads"
        } else {
            "the snapshot folder may only hold the pinned files, or it will not load"
        };
        return Err(CoreError::Config(format!(
            "this .{ext} file is not one of the pinned catalog files for a {} ({why}) — \
             refusing to import it",
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

    /// Byte-exact copies of two small pinned Florence-2 files (the catalog
    /// hashes them: `preprocessor_config.json` 603 B, `generation_config.json`
    /// 292 B of `florence-community/Florence-2-large` at commit 4271c66b) --
    /// real pinned JSON without a network fetch.
    const FLORENCE2_PREPROCESSOR_CONFIG: &[u8] = b"{\n  \"auto_map\": {\n    \"AutoProcessor\": \"processing_florence2.Florence2Processor\"\n  },\n  \"crop_size\": {\n    \"height\": 768,\n    \"width\": 768\n  },\n  \"do_center_crop\": false,\n  \"do_convert_rgb\": null,\n  \"do_normalize\": true,\n  \"do_rescale\": true,\n  \"do_resize\": true,\n  \"image_mean\": [\n    0.485,\n    0.456,\n    0.406\n  ],\n  \"image_processor_type\": \"CLIPImageProcessor\",\n  \"image_seq_length\": 577,\n  \"image_std\": [\n    0.229,\n    0.224,\n    0.225\n  ],\n  \"processor_class\": \"Florence2Processor\",\n  \"resample\": 3,\n  \"rescale_factor\": 0.00392156862745098,\n  \"size\": {\n    \"height\": 768,\n    \"width\": 768\n  }\n}\n";
    const FLORENCE2_GENERATION_CONFIG: &[u8] = b"{\n  \"_from_model_config\": true,\n  \"bos_token_id\": 0,\n  \"decoder_start_token_id\": 2,\n  \"early_stopping\": true,\n  \"eos_token_id\": 2,\n  \"forced_bos_token_id\": 0,\n  \"forced_eos_token_id\": 2,\n  \"no_repeat_ngram_size\": 3,\n  \"num_beams\": 3,\n  \"pad_token_id\": 1,\n  \"transformers_version\": \"4.56.1\"\n}\n";

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
        // Two real pinned files: an uncatalogued one would be refused.
        for (name, body) in [
            ("preprocessor_config.json", FLORENCE2_PREPROCESSOR_CONFIG),
            ("generation_config.json", FLORENCE2_GENERATION_CONFIG),
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
        let dest = store.join("vision/florence2-large/preprocessor_config.json");

        let first = write_safetensors(
            tmp.path(),
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        let out = import_as(&db, &store, &first, "florence2_engine")
            .await
            .unwrap();
        assert!(!out.already_present);

        // Tampered in place, then re-downloaded.
        std::fs::write(&dest, b"{\"auto_map\": {}}\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n").unwrap();
        let again = write_safetensors(
            tmp.path(),
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        let out = import_as(&db, &store, &again, "florence2_engine")
            .await
            .unwrap();
        assert!(out.already_present);
        assert_eq!(std::fs::read(&dest).unwrap(), FLORENCE2_PREPROCESSOR_CONFIG);
        assert_eq!(Path::new(&out.model.file_path), dest.as_path());

        // Deleted, then re-downloaded.
        std::fs::remove_file(&dest).unwrap();
        let third = write_safetensors(
            tmp.path(),
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        import_as(&db, &store, &third, "florence2_engine")
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), FLORENCE2_PREPROCESSOR_CONFIG);
        assert_eq!(
            db.models().list().await.unwrap().len(),
            1,
            "no duplicate row"
        );
    }

    /// A link planted where a pinned file belongs must be replaced, never
    /// written through: the restore removes the junction itself and puts a
    /// regular file there, leaving the link's target untouched.
    #[cfg(windows)]
    #[tokio::test]
    async fn the_restore_replaces_a_junction_at_the_destination_instead_of_writing_through_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let first = write_safetensors(
            tmp.path(),
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        import_as(&db, &store, &first, "florence2_engine")
            .await
            .unwrap();

        let dest = store
            .join("vision")
            .join("florence2-large")
            .join("preprocessor_config.json");
        std::fs::remove_file(&dest).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&dest)
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");

        // Copy mode (`keep_original`) -- the path a cross-volume move falls
        // back to, where the write would otherwise follow the link.
        let again = write_safetensors(
            tmp.path(),
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        import_model(
            &db,
            &store,
            ImportRequest {
                model_type: Some("florence2_engine".into()),
                keep_original: true,
                ..req(&again)
            },
        )
        .await
        .unwrap();

        let meta = std::fs::symlink_metadata(&dest).unwrap();
        assert!(meta.file_type().is_file(), "a regular file, not a link");
        assert_eq!(std::fs::read(&dest).unwrap(), FLORENCE2_PREPROCESSOR_CONFIG);
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing written through the link"
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
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
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
            "preprocessor_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        let out = import_as(&db, &store, &again, "florence2_engine")
            .await
            .unwrap();
        let dest = store.join("vision/florence2-large/preprocessor_config.json");
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
        let renamed = write_safetensors(
            &sub,
            "generation_config.json",
            FLORENCE2_PREPROCESSOR_CONFIG,
        );
        let out = import_as(&db, &store, &renamed, "florence2_engine")
            .await
            .unwrap();
        let p = out.model.file_path.replace('\\', "/");
        assert!(
            p.ends_with("/vision/florence2-large/preprocessor_config.json"),
            "{p}"
        );
        assert_eq!(
            std::fs::read(store.join("vision/florence2-large/generation_config.json")).unwrap(),
            FLORENCE2_GENERATION_CONFIG,
            "the real sibling is untouched"
        );
    }

    /// A library row for a file of an earlier catalog revision, the way the
    /// old importer left it (role and path of the pinned folder).
    async fn insert_stale_row(db: &Database, path: &Path, body: &[u8]) -> Model {
        let sha = {
            let tmp = tempfile::tempdir().unwrap();
            let p = tmp.path().join("x");
            std::fs::write(&p, body).unwrap();
            sha256_file(&p).unwrap()
        };
        db.models()
            .insert(NewModel {
                name: "stale".into(),
                format: "json".into(),
                file_path: path.to_string_lossy().into_owned(),
                sha256: Some(sha),
                size_bytes: body.len() as i64,
                source: "manual".into(),
                roles: vec!["vision_florence2".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
    }

    /// Plan 8 migration: an install of the old `microsoft/Florence-2-large`
    /// snapshot (remote-code `.py` files, other JSON) sits in the pinned
    /// folder. Installing the new catalog files must leave only catalog
    /// files there -- otherwise the "no extra file" integrity rule keeps the
    /// captioner unusable forever -- and drop the rows of the replaced
    /// files, never touching anything outside that exact folder.
    #[tokio::test]
    async fn a_pinned_import_prunes_files_and_rows_of_an_earlier_catalog_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let dir = store.join("vision").join("florence2-large");
        std::fs::create_dir_all(&dir).unwrap();

        // The old snapshot: remote code plus an old config under a name the
        // new catalog also uses.
        let old_py = dir.join("modeling_florence2.py");
        std::fs::write(&old_py, b"import os\n").unwrap();
        let old_config = dir.join("config.json");
        std::fs::write(&old_config, b"{\"auto_map\": {}}").unwrap();
        let py_row = insert_stale_row(&db, &old_py, b"import os\n").await;
        let config_row = insert_stale_row(&db, &old_config, b"{\"auto_map\": {}}").await;

        // Look-alikes outside the exact folder stay untouched.
        let sibling = store.join("vision").join("florence2-large-old");
        std::fs::create_dir_all(&sibling).unwrap();
        let sibling_py = sibling.join("modeling_florence2.py");
        std::fs::write(&sibling_py, b"import os\n").unwrap();
        let sibling_row = insert_stale_row(&db, &sibling_py, b"import os\n").await;
        let above = store.join("vision").join("modeling_florence2.py");
        std::fs::write(&above, b"import os\n").unwrap();

        let src = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        let out = import_as(&db, &store, &src, "florence2_engine")
            .await
            .unwrap();

        assert!(!old_py.exists(), "the non-catalog file is removed");
        assert!(
            old_config.exists(),
            "a catalog-named file stays for its own download to replace"
        );
        let models = db.models();
        assert!(
            models.get(&py_row.id).await.unwrap().is_none(),
            "row of the removed file"
        );
        assert!(
            models.get(&config_row.id).await.unwrap().is_none(),
            "row whose content is not a catalog entry"
        );
        assert!(
            models.get(&out.model.id).await.unwrap().is_some(),
            "the new file's row"
        );
        assert!(sibling_py.exists() && above.exists(), "outside the folder");
        assert!(
            models.get(&sibling_row.id).await.unwrap().is_some(),
            "outside the folder"
        );
    }

    /// The repair path (the file's hash already in the library) prunes the
    /// same way, so "Re-download" alone fixes a folder with a leftover file.
    #[tokio::test]
    async fn a_pinned_repair_also_prunes_a_leftover_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let first = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        import_as(&db, &store, &first, "florence2_engine")
            .await
            .unwrap();
        let leftover = store.join("vision/florence2-large/processing_florence2.py");
        std::fs::write(&leftover, b"x = 1\n").unwrap();

        let again = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        let out = import_as(&db, &store, &again, "florence2_engine")
            .await
            .unwrap();
        assert!(out.already_present);
        assert!(!leftover.exists());
        assert!(store
            .join("vision/florence2-large/generation_config.json")
            .is_file());
    }

    /// Pruning never follows a link: a junction inside the pinned folder is
    /// removed as a link, and its target keeps every file.
    #[cfg(windows)]
    #[tokio::test]
    async fn pruning_removes_a_junction_itself_never_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let dir = store.join("vision").join("florence2-large");
        std::fs::create_dir_all(&dir).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(dir.join("linked"))
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");

        let src = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        import_as(&db, &store, &src, "florence2_engine")
            .await
            .unwrap();
        assert!(std::fs::symlink_metadata(dir.join("linked")).is_err());
        assert_eq!(std::fs::read(outside.join("keep.txt")).unwrap(), b"keep");
    }

    /// A pinned folder that is itself redirected out of the store is never
    /// pruned: nothing outside the store's own folder is deleted.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_redirected_pinned_folder_is_not_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        std::fs::create_dir_all(store.join("vision")).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious.py"), b"keep").unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(store.join("vision").join("florence2-large"))
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");

        let src = write_safetensors(
            tmp.path(),
            "generation_config.json",
            FLORENCE2_GENERATION_CONFIG,
        );
        // Whatever the import does with the file itself, the prune must not
        // delete anything in the junction's target.
        let _ = import_as(&db, &store, &src, "florence2_engine").await;
        assert_eq!(std::fs::read(outside.join("precious.py")).unwrap(), b"keep");
    }

    /// Every file of a pinned snapshot kind must be a catalog entry of that
    /// kind -- weights and tokenizer data included, not only code-bearing
    /// files: the folder is loaded as a whole, and anything else in it only
    /// fails later, at load time. (Side-by-side placement is shared with
    /// Florence-2, covered above with real pinned files.)
    #[tokio::test]
    async fn uncatalogued_weights_or_tokenizer_files_of_a_pinned_kind_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();

        for (kind, name, body, subdir) in [
            (
                "qwen_vl_engine",
                "merges.txt",
                &b"#version: 0.2\n"[..],
                "vision/qwen2.5-vl-7b",
            ),
            (
                "qwen_vl_engine",
                "model-00001-of-00005.safetensors",
                &b"shard-bytes"[..],
                "vision/qwen2.5-vl-7b",
            ),
            (
                "florence2_engine",
                "model.safetensors",
                &b"not-a-real-header"[..],
                "vision/florence2-large",
            ),
        ] {
            let src = write_safetensors(tmp.path(), name, body);
            let err = import_as(&db, &store, &src, kind).await.unwrap_err();
            assert!(err.to_string().contains("pinned"), "{kind} {name}: {err}");
            assert!(!store.join(subdir).join(name).exists(), "{kind} {name}");
        }
        assert!(db.models().list().await.unwrap().is_empty());
    }

    /// Florence-2 loads natively now: no Python file is ever a captioner
    /// file, so a `.py` offered as one is refused outright.
    #[tokio::test]
    async fn a_python_file_is_never_imported_as_a_florence2_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let db = Database::connect_in_memory().await.unwrap();
        let src = write_safetensors(tmp.path(), "modeling_florence2.py", b"import os\n");
        let err = import_as(&db, &store, &src, "florence2_engine")
            .await
            .unwrap_err();
        assert!(err.to_string().contains(".py"), "{err}");
        assert!(!store
            .join("vision/florence2-large/modeling_florence2.py")
            .exists());
        assert!(db.models().list().await.unwrap().is_empty());
    }

    #[test]
    fn pinned_kind_files_are_allowed_only_as_catalogued_files_of_the_same_kind() {
        let pinned = crate::model::KNOWN_MODELS
            .iter()
            .find(|m| m.kind == "florence2_engine" && m.file == "config.json")
            .expect("the catalog pins Florence-2's config");
        let florence = ModelKind::Florence2Engine;
        let qwen = ModelKind::QwenVlEngine;
        let unknown = "0".repeat(64);

        assert_eq!(
            pinned_file_for(florence, "json", pinned.sha256).unwrap(),
            Some(pinned.file)
        );
        assert_eq!(
            pinned_file_for(florence, "JSON", &pinned.sha256.to_ascii_uppercase()).unwrap(),
            Some(pinned.file)
        );
        assert!(pinned_file_for(florence, "json", &unknown).is_err());
        assert!(pinned_file_for(florence, "txt", &unknown).is_err());
        // Another kind's pinned file is not this kind's.
        assert!(pinned_file_for(qwen, "json", pinned.sha256).is_err());
        assert!(pinned_file_for(qwen, "json", &unknown).is_err());
        // Weights / tokenizer data are gated too: only catalog entries of
        // the kind are imported.
        assert!(pinned_file_for(florence, "safetensors", &unknown).is_err());
        assert!(pinned_file_for(qwen, "txt", &unknown).is_err());
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
