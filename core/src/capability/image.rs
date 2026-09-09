//! The text-to-image capability: drive one `job_type=image` body.
//!
//! By the time this runs the engine has put the checkpoint's VRAM slot on the
//! GPU (scheduler → `ComfyUiAdapter::load_model`, which starts the server).
//! Here we turn the job's params into a fixed SDXL workflow
//! ([`crate::pipeline`]), hand it to ComfyUI, wait for the image, and write it
//! to `<outputs>/<job_id>.<ext>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::watch;

use crate::db::{Database, EventLevel, Model};
use crate::pipeline::{self, Txt2ImgInputs};
use crate::runtime::ComfyUiAdapter;
use crate::{CoreError, Result};

const DEFAULT_DIM: u32 = 1024;
const MIN_DIM: u32 = 256;
const MAX_DIM: u32 = 2048;
const DIM_MULTIPLE: u32 = 8;

const DEFAULT_STEPS: u32 = 25;
const MAX_STEPS: u32 = 150;

const DEFAULT_CFG: f64 = 7.0;
const MIN_CFG: f64 = 1.0;
const MAX_CFG: f64 = 30.0;

const DEFAULT_SAMPLER: &str = "euler";
const DEFAULT_SCHEDULER: &str = "normal";

/// JSON stays lossless below 2^53, so random seeds are drawn from that range —
/// the UI can show and re-submit them without precision loss.
const SEED_CEILING: u64 = 1 << 53;

fn image_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "comfyui".into(),
        message: msg.to_string(),
    }
}

/// A resolved text-to-image request, pulled from a job's `params`. Every field
/// has a default; only a non-empty `prompt` is required.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f64,
    pub sampler: String,
    pub scheduler: String,
    /// Always concrete here — a missing or negative seed was resolved to a
    /// random one.
    pub seed: i64,
}

impl ImageRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| image_err("image job has no `prompt`"))?
            .to_string();

        let negative = params
            .get("negative")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let dim = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_u64)
                .map_or(DEFAULT_DIM, |v| {
                    round_to(v, DIM_MULTIPLE).clamp(MIN_DIM, MAX_DIM)
                })
        };
        let steps = params
            .get("steps")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .map_or(DEFAULT_STEPS, |v| v.clamp(1, MAX_STEPS));
        let cfg = params
            .get("cfg")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_CFG, |v| v.clamp(MIN_CFG, MAX_CFG));

        let seed = params
            .get("seed")
            .and_then(Value::as_i64)
            .filter(|s| *s >= 0)
            .unwrap_or_else(random_seed);

        Ok(Self {
            prompt,
            negative,
            width: dim("width"),
            height: dim("height"),
            steps,
            cfg,
            sampler: str_param(params, "sampler", DEFAULT_SAMPLER),
            scheduler: str_param(params, "scheduler", DEFAULT_SCHEDULER),
            seed,
        })
    }

    /// Write the resolved values back over a job's `params` object so a random
    /// seed becomes reproducible and the gallery (3.5) has concrete numbers.
    pub fn apply_to(&self, params: &mut Value) {
        let Some(obj) = params.as_object_mut() else {
            return;
        };
        obj.insert("prompt".into(), self.prompt.clone().into());
        obj.insert("negative".into(), self.negative.clone().into());
        obj.insert("width".into(), self.width.into());
        obj.insert("height".into(), self.height.into());
        obj.insert("steps".into(), self.steps.into());
        obj.insert("cfg".into(), self.cfg.into());
        obj.insert("sampler".into(), self.sampler.clone().into());
        obj.insert("scheduler".into(), self.scheduler.clone().into());
        obj.insert("seed".into(), self.seed.into());
    }
}

/// A finished image body.
#[derive(Debug, Clone)]
pub struct ImageDone {
    pub output_path: PathBuf,
    pub seed: i64,
    pub width: u32,
    pub height: u32,
}

/// How the image body came to rest.
#[derive(Debug, Clone)]
pub enum ImageOutcome {
    Done(ImageDone),
    /// The user cancelled while ComfyUI was rendering.
    Cancelled,
}

/// Render `req` on `comfyui` with `model` as the checkpoint, save the result
/// under `outputs_dir`. Returns when the image is written, ComfyUI errors, or
/// `cancel` flips to `true`.
pub async fn run(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    model: &Model,
    req: ImageRequest,
    cancel: watch::Receiver<bool>,
) -> Result<ImageOutcome> {
    let checkpoint = Path::new(&model.file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| image_err("the model file has no name"))?;

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "rendering {}\u{00d7}{}, {} steps, cfg {:.1}, seed {} \u{2014} {}",
                req.width, req.height, req.steps, req.cfg, req.seed, model.name
            ),
        )
        .await?;

    let workflow = pipeline::sdxl_txt2img(&Txt2ImgInputs {
        checkpoint,
        positive: &req.prompt,
        negative: &req.negative,
        width: req.width,
        height: req.height,
        steps: req.steps,
        cfg: req.cfg,
        sampler: &req.sampler,
        scheduler: &req.scheduler,
        seed: req.seed,
        filename_prefix: job_id,
    });

    let Some(image) = comfyui.generate_image(&workflow, cancel).await? else {
        return Ok(ImageOutcome::Cancelled);
    };

    tokio::fs::create_dir_all(outputs_dir)
        .await
        .map_err(|e| image_err(format!("create {}: {e}", outputs_dir.display())))?;
    let output_path = outputs_dir.join(format!("{job_id}.{}", image.extension));
    tokio::fs::write(&output_path, &image.bytes)
        .await
        .map_err(|e| image_err(format!("write {}: {e}", output_path.display())))?;

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "saved {} ({} KB)",
                output_path.display(),
                image.bytes.len() / 1024
            ),
        )
        .await?;

    Ok(ImageOutcome::Done(ImageDone {
        output_path,
        seed: req.seed,
        width: req.width,
        height: req.height,
    }))
}

fn str_param(params: &Value, key: &str, default: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn round_to(v: u64, multiple: u32) -> u32 {
    let m = u64::from(multiple);
    let rounded = ((v + m / 2) / m).saturating_mul(m);
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

/// A non-crypto random seed: hash the current time with a process-random keyed
/// hasher. Good enough for image seeds — they only need to differ per call.
fn random_seed() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    i64::try_from(h.finish() % SEED_CEILING).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_params_fills_defaults_and_resolves_a_seed() {
        let r = ImageRequest::from_params(&serde_json::json!({ "prompt": "  a cat  " })).unwrap();
        assert_eq!(r.prompt, "a cat");
        assert_eq!(r.negative, "");
        assert_eq!(r.width, DEFAULT_DIM);
        assert_eq!(r.height, DEFAULT_DIM);
        assert_eq!(r.steps, DEFAULT_STEPS);
        assert_eq!(r.sampler, DEFAULT_SAMPLER);
        assert!(r.seed >= 0);
    }

    #[test]
    fn from_params_rejects_a_blank_prompt() {
        assert!(ImageRequest::from_params(&serde_json::json!({})).is_err());
        assert!(ImageRequest::from_params(&serde_json::json!({ "prompt": "   " })).is_err());
    }

    #[test]
    fn from_params_clamps_and_rounds() {
        let r = ImageRequest::from_params(&serde_json::json!({
            "prompt": "x",
            "width": 5000,       // over the max
            "height": 999,       // rounds to a multiple of 8
            "steps": 9000,       // over the max
            "cfg": 0.1,          // under the min
        }))
        .unwrap();
        assert_eq!(r.width, MAX_DIM);
        assert_eq!(r.height, 1000);
        assert_eq!(r.steps, MAX_STEPS);
        assert_eq!(r.cfg, MIN_CFG);
    }

    #[test]
    fn an_explicit_seed_is_kept() {
        let r = ImageRequest::from_params(&serde_json::json!({ "prompt": "x", "seed": 12345 }))
            .unwrap();
        assert_eq!(r.seed, 12345);
    }

    #[test]
    fn apply_to_pins_resolved_values_without_dropping_engine_metadata() {
        let mut params = serde_json::json!({ "prompt": "a dog", "vram_needed_mb": 8000 });
        let r = ImageRequest::from_params(&params).unwrap();
        r.apply_to(&mut params);

        assert_eq!(params["seed"], r.seed);
        assert_eq!(params["width"], DEFAULT_DIM);
        // The engine's own key survived.
        assert_eq!(params["vram_needed_mb"], 8000);
    }

    #[test]
    fn random_seeds_vary() {
        let a = random_seed();
        let b = random_seed();
        assert!(a >= 0 && b >= 0);
        assert_ne!(a, b, "two draws should differ");
    }
}
