//! Helpers shared by the ComfyUI media capabilities ([`image`](super::image),
//! [`video`](super::video)): parameter parsing, seeds, and writing the finished
//! file into the outputs directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Database;
use crate::{CoreError, Result};

/// A LoRA a job asked for, as submitted — a library reference, not yet a file
/// name (that needs a DB lookup, done in [`resolve_loras`] once the job
/// actually runs, the same way Flux/Wan's companion files are resolved late).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoraRef {
    pub model_id: String,
    /// ComfyUI's `LoraLoader` allows negative values (subtracting the LoRA);
    /// only the upper end is clamped against a fat-fingered `50` — see
    /// [`MAX_LORA_STRENGTH`].
    pub strength: f64,
}

/// Past this a LoRA is overwhelmingly likely to be a typo, not an intentional
/// choice — ComfyUI's own default UI caps its slider at 2.
const MAX_LORA_STRENGTH: f64 = 5.0;
const MIN_LORA_STRENGTH: f64 = -5.0;
const DEFAULT_LORA_STRENGTH: f64 = 1.0;
/// More than this and the graph (and the render) gets unwieldy for little
/// benefit — a soft ceiling, not a ComfyUI limitation.
const MAX_LORAS: usize = 5;

/// Read `params["loras"]` — an array of `{ model_id, strength }` — dropping
/// any entry with a blank/missing id and clamping strength. Absent or
/// malformed input just yields an empty list rather than an error: a LoRA
/// selection is always optional.
pub(super) fn parse_loras(params: &Value) -> Vec<LoraRef> {
    let Some(arr) = params.get("loras").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| {
            let model_id = v.get("model_id")?.as_str()?.trim();
            if model_id.is_empty() {
                return None;
            }
            let strength = v
                .get("strength")
                .and_then(Value::as_f64)
                .map_or(DEFAULT_LORA_STRENGTH, |s| {
                    s.clamp(MIN_LORA_STRENGTH, MAX_LORA_STRENGTH)
                });
            Some(LoraRef {
                model_id: model_id.to_string(),
                strength,
            })
        })
        .take(MAX_LORAS)
        .collect()
}

/// One LoRA, resolved from the library to the bare file name ComfyUI needs.
#[derive(Debug)]
pub(super) struct ResolvedLora {
    pub file: String,
    pub strength: f64,
}

/// Resolve every `LoraRef` to its on-disk file name. Errors with a plain
/// "which model" message if an id no longer exists (e.g. deleted after the
/// job was queued) rather than a cryptic ComfyUI node failure.
pub(super) async fn resolve_loras(db: &Database, refs: &[LoraRef]) -> Result<Vec<ResolvedLora>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let model =
            db.models().get(&r.model_id).await?.ok_or_else(|| {
                comfy_err(format!("LoRA {} is no longer in the library", r.model_id))
            })?;
        out.push(ResolvedLora {
            file: file_name(&model.file_path)?.to_string(),
            strength: r.strength,
        });
    }
    Ok(out)
}

/// JSON stays lossless below 2^53, so random seeds are drawn from that range —
/// the UI can show and re-submit them without precision loss.
pub(super) const SEED_CEILING: u64 = 1 << 53;

pub(super) fn comfy_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "comfyui".into(),
        message: msg.to_string(),
    }
}

/// The bare file name as ComfyUI sees it in its model folders.
pub(super) fn file_name(path: &str) -> Result<&str> {
    Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| comfy_err("the model file has no name"))
}

/// A trimmed string param, or `default` when it is absent / blank.
pub(super) fn str_param(params: &Value, key: &str, default: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_string()
}

/// Round `v` to the nearest multiple of `multiple`.
pub(super) fn round_to(v: u64, multiple: u32) -> u32 {
    let m = u64::from(multiple);
    let rounded = ((v + m / 2) / m).saturating_mul(m);
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

/// A non-crypto random seed: hash the current time with a process-random keyed
/// hasher. Good enough here — seeds only need to differ per call.
pub(super) fn random_seed() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    i64::try_from(h.finish() % SEED_CEILING).unwrap_or(0)
}

/// Read `params["seed"]` when it is a non-negative integer; otherwise a fresh
/// random seed.
pub(super) fn resolve_seed(params: &Value) -> i64 {
    params
        .get("seed")
        .and_then(Value::as_i64)
        .filter(|s| *s >= 0)
        .unwrap_or_else(random_seed)
}

/// Image extensions ComfyUI's `LoadImage` can read.
const STAGED_IMAGE_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// An image copied into ComfyUI's `input/` folder for a job to reference (a
/// video's start frame, an image edit's source). Dropping it removes the
/// copy — it is only needed for the one render.
pub(super) struct StagedImage {
    /// Bare file name, as `LoadImage` refers to it.
    pub name: String,
    /// Full path of the copy (removed on drop).
    path: PathBuf,
    /// What the caller asked for — a job id or a path (for the event trail).
    pub source: String,
}

impl Drop for StagedImage {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                path = %self.path.display(),
                "could not remove staged image: {e}"
            ),
        }
    }
}

/// Resolve `spec` (a completed job's id, or a path to an image) to a real
/// file, then copy it to `<comfyui input>/<job_id>.<ext>` for `LoadImage`.
pub(super) async fn stage_image(
    db: &Database,
    input_dir: &Path,
    job_id: &str,
    spec: &str,
) -> Result<StagedImage> {
    let source = resolve_staged_image(db, spec).await?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "png".to_string());

    tokio::fs::create_dir_all(input_dir)
        .await
        .map_err(|e| comfy_err(format!("create {}: {e}", input_dir.display())))?;
    let name = format!("{job_id}.{ext}");
    let path = input_dir.join(&name);
    tokio::fs::copy(&source, &path).await.map_err(|e| {
        comfy_err(format!(
            "stage image {} \u{2192} {}: {e}",
            source.display(),
            path.display()
        ))
    })?;
    Ok(StagedImage {
        name,
        path,
        source: spec.to_string(),
    })
}

/// The image is either the output of a finished job (the gallery hands us its
/// id) or a path to an image file on disk. Either way it must be an existing
/// image ComfyUI's `LoadImage` can read.
async fn resolve_staged_image(db: &Database, spec: &str) -> Result<PathBuf> {
    if let Some(job) = db.jobs().get(spec).await? {
        let out = job
            .output_path
            .ok_or_else(|| comfy_err(format!("job {spec} has no image output to use")))?;
        return checked_image_file(&out);
    }
    checked_image_file(spec)
}

fn checked_image_file(path: &str) -> Result<PathBuf> {
    let p = PathBuf::from(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !ext
        .as_deref()
        .is_some_and(|e| STAGED_IMAGE_EXTS.contains(&e))
    {
        return Err(comfy_err(format!(
            "must be a {} image \u{2014} got {path}",
            STAGED_IMAGE_EXTS.join(" / ")
        )));
    }
    if !p.is_file() {
        return Err(comfy_err(format!("image not found: {path}")));
    }
    Ok(p)
}

/// Write a finished render to `<outputs_dir>/<job_id>.<ext>`.
pub(super) async fn write_output(
    outputs_dir: &Path,
    job_id: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(outputs_dir)
        .await
        .map_err(|e| comfy_err(format!("create {}: {e}", outputs_dir.display())))?;
    let path = outputs_dir.join(format!("{job_id}.{ext}"));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| comfy_err(format!("write {}: {e}", path.display())))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_snaps_to_the_nearest_multiple() {
        assert_eq!(round_to(900, 64), 896);
        assert_eq!(round_to(833, 8), 832);
        assert_eq!(round_to(0, 64), 0);
    }

    #[test]
    fn resolve_seed_keeps_explicit_and_randomizes_otherwise() {
        assert_eq!(resolve_seed(&serde_json::json!({ "seed": 123 })), 123);
        assert!(resolve_seed(&serde_json::json!({ "seed": -1 })) >= 0);
        assert!(resolve_seed(&serde_json::json!({})) >= 0);
    }

    #[test]
    fn random_seeds_differ() {
        assert_ne!(random_seed(), random_seed());
    }

    #[test]
    fn parse_loras_reads_ids_and_clamps_strength() {
        let loras = parse_loras(&serde_json::json!({
            "loras": [
                { "model_id": "m-1", "strength": 0.8 },
                { "model_id": "m-2", "strength": 99.0 },
                { "model_id": "  " },
                { "strength": 1.0 },
                { "model_id": "m-3" },
            ]
        }));
        assert_eq!(
            loras,
            vec![
                LoraRef {
                    model_id: "m-1".into(),
                    strength: 0.8
                },
                LoraRef {
                    model_id: "m-2".into(),
                    strength: MAX_LORA_STRENGTH
                },
                LoraRef {
                    model_id: "m-3".into(),
                    strength: DEFAULT_LORA_STRENGTH
                },
            ]
        );
    }

    #[test]
    fn parse_loras_is_empty_when_absent_or_malformed() {
        assert!(parse_loras(&serde_json::json!({})).is_empty());
        assert!(parse_loras(&serde_json::json!({ "loras": "not-an-array" })).is_empty());
    }

    #[test]
    fn parse_loras_caps_the_count() {
        let many: Vec<_> = (0..10)
            .map(|i| serde_json::json!({ "model_id": format!("m-{i}"), "strength": 1.0 }))
            .collect();
        let loras = parse_loras(&serde_json::json!({ "loras": many }));
        assert_eq!(loras.len(), MAX_LORAS);
    }

    #[tokio::test]
    async fn resolve_loras_reports_a_missing_model_by_id() {
        let db = crate::db::Database::connect_in_memory().await.unwrap();
        let err = resolve_loras(
            &db,
            &[LoraRef {
                model_id: "ghost".into(),
                strength: 1.0,
            }],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn checked_image_file_rejects_non_images_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("frame.png");
        std::fs::write(&good, b"x").unwrap();
        assert_eq!(checked_image_file(&good.to_string_lossy()).unwrap(), good);

        let mp4 = tmp.path().join("clip.mp4");
        std::fs::write(&mp4, b"x").unwrap();
        assert!(checked_image_file(&mp4.to_string_lossy())
            .unwrap_err()
            .to_string()
            .contains("must be a"));

        assert!(
            checked_image_file(&tmp.path().join("gone.png").to_string_lossy())
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }

    #[tokio::test]
    async fn resolve_staged_image_takes_a_path_or_a_finished_jobs_output() {
        use crate::db::{Database, NewJob};
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();

        // A bare path to an image on disk.
        let ondisk = tmp.path().join("hand.jpg");
        std::fs::write(&ondisk, b"x").unwrap();
        assert_eq!(
            resolve_staged_image(&db, &ondisk.to_string_lossy())
                .await
                .unwrap(),
            ondisk
        );

        // A job that has not produced anything yet → a clear error.
        let job = db.jobs().insert(NewJob::new("image")).await.unwrap();
        assert!(resolve_staged_image(&db, &job.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("no image output"));
        // (a job id that resolves to a real output is covered end-to-end in
        // tests/video_job.rs)
    }

    #[tokio::test]
    async fn stage_image_copies_into_the_input_dir_and_cleans_up_on_drop() {
        let src_dir = tempfile::tempdir().unwrap();
        let input_dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::connect_in_memory().await.unwrap();

        let source = src_dir.path().join("edit-me.png");
        std::fs::write(&source, b"pretend png bytes").unwrap();

        let staged_path = {
            let staged = stage_image(&db, input_dir.path(), "job-123", &source.to_string_lossy())
                .await
                .unwrap();
            assert_eq!(staged.name, "job-123.png");
            let path = input_dir.path().join(&staged.name);
            assert!(path.is_file(), "copy should exist while staged is alive");
            path
        };
        assert!(
            !staged_path.is_file(),
            "dropping the guard removes the copy"
        );
    }

    #[tokio::test]
    async fn resolve_loras_resolves_the_bare_file_name() {
        use crate::db::{Database, NewModel};
        let db = Database::connect_in_memory().await.unwrap();
        let m = db
            .models()
            .insert(NewModel {
                name: "Add Detail XL".into(),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\image\\loras\\add-detail-xl.safetensors".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();

        let resolved = resolve_loras(
            &db,
            &[LoraRef {
                model_id: m.id,
                strength: 0.6,
            }],
        )
        .await
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].file, "add-detail-xl.safetensors");
        assert_eq!(resolved[0].strength, 0.6);
    }
}
