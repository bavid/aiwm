//! The upscale capability: drive one `job_type=upscale` body.
//!
//! Takes an already-finished `image` or `video` job's output (or a raw file
//! path) and runs it through NVIDIA's real, GA **RTX Video Super
//! Resolution** ComfyUI node (`Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`, installed
//! alongside `ComfyUI-GGUF` — see `runtime::comfyui::install`). It sharpens,
//! denoises and resizes; unlike a diffusion upscaler it does not hallucinate
//! new detail, and unlike `capability::audio_clean` this is a real scheduled
//! ComfyUI render (VRAM, queueing) — not a fast synchronous DSP pass — so it
//! goes through the normal job engine exactly like `image`/`video`.
//!
//! There is no checkpoint to pick: the node is a custom-node install, not a
//! downloadable weights file, so `job.model_id` is a fixed synthetic id (see
//! `orchestrator::engine`'s `UPSCALE_MODEL_ID`) rather than a real library
//! `Model` row.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::media::{comfy_err as upscale_err, media_kind, stage_image, stage_video, MediaKind};
use crate::db::{Database, EventLevel};
use crate::pipeline::{self, UpscaleImageInputs, UpscaleResize, UpscaleVideoInputs};
use crate::runtime::ComfyUiAdapter;
use crate::Result;

const DEFAULT_RESIZE_MODE: &str = "scale";
const DEFAULT_SCALE: f64 = 2.0;
const MIN_SCALE: f64 = 1.0;
const MAX_SCALE: f64 = 4.0;

const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
const MIN_DIM: u32 = 64;
const MAX_DIM: u32 = 8192;
const DIM_MULTIPLE: u32 = 8;

const DEFAULT_QUALITY: &str = "ULTRA";
const QUALITIES: [&str; 4] = ["LOW", "MEDIUM", "HIGH", "ULTRA"];

/// Upper bound on one upscale pass. RTX Video Super Resolution is a fixed-
/// function effect, not a diffusion sampler, so even a full video's worth of
/// frames should be much faster than a video *render* — but a very long clip
/// still has a lot of frames to run through it one at a time.
const UPSCALE_TIMEOUT: Duration = Duration::from_secs(900);

/// A resolved upscale request, pulled from a job's `params`. Only `source` is
/// required; everything else falls back to the RTX node's own defaults
/// (`scale` × 2.0, quality `ULTRA`).
#[derive(Debug, Clone, PartialEq)]
pub struct UpscaleRequest {
    /// A finished `image`/`video` job's id, or a path to a file on disk.
    pub source: String,
    pub resize: UpscaleResize,
    /// `"LOW"` | `"MEDIUM"` | `"HIGH"` | `"ULTRA"`.
    pub quality: String,
}

impl UpscaleRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let source = params
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| upscale_err("upscale job has no `source`"))?
            .to_string();

        let mode = params
            .get("resize_mode")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_RESIZE_MODE);

        let resize = if mode.eq_ignore_ascii_case("dimensions") {
            let dim = |key: &str, default: u32| {
                params
                    .get(key)
                    .and_then(Value::as_u64)
                    .map_or(default, |v| {
                        round_to_multiple(v, DIM_MULTIPLE).clamp(MIN_DIM, MAX_DIM)
                    })
            };
            UpscaleResize::Target {
                width: dim("width", DEFAULT_WIDTH),
                height: dim("height", DEFAULT_HEIGHT),
            }
        } else {
            let scale = params
                .get("scale")
                .and_then(Value::as_f64)
                .map_or(DEFAULT_SCALE, |v| v.clamp(MIN_SCALE, MAX_SCALE));
            UpscaleResize::ScaleBy(scale)
        };

        let quality = params
            .get("quality")
            .and_then(Value::as_str)
            .map(str::to_ascii_uppercase)
            .filter(|q| QUALITIES.contains(&q.as_str()))
            .unwrap_or_else(|| DEFAULT_QUALITY.to_string());

        Ok(Self {
            source,
            resize,
            quality,
        })
    }
}

fn round_to_multiple(v: u64, multiple: u32) -> u32 {
    let m = u64::from(multiple);
    let rounded = ((v + m / 2) / m).saturating_mul(m);
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

/// A finished upscale body.
#[derive(Debug, Clone)]
pub struct UpscaleDone {
    pub output_path: PathBuf,
}

/// How the upscale body came to rest.
#[derive(Debug, Clone)]
pub enum UpscaleOutcome {
    Done(UpscaleDone),
    /// The user cancelled while ComfyUI was running the pass.
    Cancelled,
}

/// Run `req` on `comfyui`, save the result under `outputs_dir`. Dispatches to
/// an image or a video pipeline depending on what `req.source` actually is
/// (a finished job's own `job_type`, or the extension of a bare path).
pub async fn run(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    req: UpscaleRequest,
    cancel: watch::Receiver<bool>,
) -> Result<UpscaleOutcome> {
    match media_kind(db, &req.source).await? {
        MediaKind::Image => run_image(db, comfyui, outputs_dir, job_id, &req, cancel).await,
        MediaKind::Video => run_video(db, comfyui, outputs_dir, job_id, &req, cancel).await,
    }
}

fn resize_note(resize: &UpscaleResize) -> String {
    match resize {
        UpscaleResize::ScaleBy(scale) => format!("{scale:.2}\u{00d7}"),
        UpscaleResize::Target { width, height } => format!("{width}\u{00d7}{height}"),
    }
}

async fn run_image(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    req: &UpscaleRequest,
    cancel: watch::Receiver<bool>,
) -> Result<UpscaleOutcome> {
    let staged = stage_image(db, &comfyui.input_dir(), job_id, &req.source).await?;
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "upscaling {} \u{2014} {}, {} quality",
                staged.source,
                resize_note(&req.resize),
                req.quality
            ),
        )
        .await?;

    let inputs = UpscaleImageInputs {
        source_image: &staged.name,
        resize: req.resize,
        quality: &req.quality,
        filename_prefix: job_id,
    };
    let workflow = pipeline::rtx_upscale_image(&inputs);

    let Some(image) = comfyui
        .generate_media(&workflow, cancel, UPSCALE_TIMEOUT)
        .await?
    else {
        return Ok(UpscaleOutcome::Cancelled);
    };

    let output_path =
        write_upscale_output(outputs_dir, job_id, &image.extension, &image.bytes).await?;
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
    Ok(UpscaleOutcome::Done(UpscaleDone { output_path }))
}

async fn run_video(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    req: &UpscaleRequest,
    cancel: watch::Receiver<bool>,
) -> Result<UpscaleOutcome> {
    let staged = stage_video(db, &comfyui.input_dir(), job_id, &req.source).await?;
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "upscaling {} \u{2014} {}, {} quality \u{2014} this can take a while",
                staged.source,
                resize_note(&req.resize),
                req.quality
            ),
        )
        .await?;

    let inputs = UpscaleVideoInputs {
        source_video: &staged.name,
        resize: req.resize,
        quality: &req.quality,
        filename_prefix: job_id,
    };
    let workflow = pipeline::rtx_upscale_video(&inputs);

    let Some(media) = comfyui
        .generate_media(&workflow, cancel, UPSCALE_TIMEOUT)
        .await?
    else {
        return Ok(UpscaleOutcome::Cancelled);
    };

    let ext = if media.extension.is_empty() {
        "mp4"
    } else {
        &media.extension
    };
    let output_path = write_upscale_output(outputs_dir, job_id, ext, &media.bytes).await?;
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "saved {} ({} KB)",
                output_path.display(),
                media.bytes.len() / 1024
            ),
        )
        .await?;
    Ok(UpscaleOutcome::Done(UpscaleDone { output_path }))
}

async fn write_upscale_output(
    outputs_dir: &Path,
    job_id: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    super::media::write_output(outputs_dir, job_id, ext, bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_params_requires_a_source() {
        assert!(UpscaleRequest::from_params(&serde_json::json!({})).is_err());
        assert!(UpscaleRequest::from_params(&serde_json::json!({ "source": "   " })).is_err());
    }

    #[test]
    fn from_params_defaults_to_a_2x_scale_and_ultra_quality() {
        let r = UpscaleRequest::from_params(&serde_json::json!({ "source": "job-1" })).unwrap();
        assert_eq!(r.source, "job-1");
        assert_eq!(r.resize, UpscaleResize::ScaleBy(2.0));
        assert_eq!(r.quality, "ULTRA");
    }

    #[test]
    fn from_params_clamps_an_explicit_scale() {
        let too_big =
            UpscaleRequest::from_params(&serde_json::json!({ "source": "x", "scale": 9.0 }))
                .unwrap();
        assert_eq!(too_big.resize, UpscaleResize::ScaleBy(MAX_SCALE));
        let too_small =
            UpscaleRequest::from_params(&serde_json::json!({ "source": "x", "scale": 0.1 }))
                .unwrap();
        assert_eq!(too_small.resize, UpscaleResize::ScaleBy(MIN_SCALE));
    }

    #[test]
    fn from_params_reads_target_dimensions_and_rounds_to_a_multiple_of_8() {
        let r = UpscaleRequest::from_params(&serde_json::json!({
            "source": "x",
            "resize_mode": "dimensions",
            "width": 1921,
            "height": 1079,
        }))
        .unwrap();
        assert_eq!(
            r.resize,
            UpscaleResize::Target {
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn from_params_clamps_target_dimensions_to_the_nodes_own_range() {
        let r = UpscaleRequest::from_params(&serde_json::json!({
            "source": "x",
            "resize_mode": "dimensions",
            "width": 50,
            "height": 99999,
        }))
        .unwrap();
        assert_eq!(
            r.resize,
            UpscaleResize::Target {
                width: MIN_DIM,
                height: MAX_DIM
            }
        );
    }

    #[test]
    fn from_params_rejects_an_unknown_quality_and_falls_back_to_the_default() {
        let r = UpscaleRequest::from_params(&serde_json::json!({
            "source": "x",
            "quality": "potato",
        }))
        .unwrap();
        assert_eq!(r.quality, "ULTRA");
    }

    #[test]
    fn from_params_keeps_an_explicit_valid_quality_case_insensitively() {
        let r = UpscaleRequest::from_params(&serde_json::json!({
            "source": "x",
            "quality": "low",
        }))
        .unwrap();
        assert_eq!(r.quality, "LOW");
    }

    #[tokio::test]
    async fn run_reports_a_clear_error_when_the_source_job_has_no_output() {
        use crate::db::{Database, NewJob};

        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("image")).await.unwrap();
        let comfy = std::sync::Arc::new(ComfyUiAdapter::with_launch(
            db.clone(),
            None,
            crate::runtime::ComfyDirs {
                base: std::env::temp_dir(),
                output: std::env::temp_dir(),
                models_store: std::env::temp_dir(),
            },
        ));
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let req = UpscaleRequest::from_params(&serde_json::json!({ "source": job.id })).unwrap();

        let err = run(
            &db,
            &comfy,
            std::path::Path::new("/tmp/out"),
            "up-1",
            req,
            cancel_rx,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no image output"), "{err}");
    }

    #[tokio::test]
    async fn run_reports_a_clear_error_for_a_missing_file_source() {
        let db = crate::db::Database::connect_in_memory().await.unwrap();
        let comfy = std::sync::Arc::new(ComfyUiAdapter::with_launch(
            db.clone(),
            None,
            crate::runtime::ComfyDirs {
                base: std::env::temp_dir(),
                output: std::env::temp_dir(),
                models_store: std::env::temp_dir(),
            },
        ));
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let req = UpscaleRequest::from_params(&serde_json::json!({
            "source": "C:\\nope\\gone.png"
        }))
        .unwrap();

        let err = run(
            &db,
            &comfy,
            std::path::Path::new("/tmp/out"),
            "up-1",
            req,
            cancel_rx,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }
}
