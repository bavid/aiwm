//! The text/image-to-video capability: drive one `job_type=video` body.
//!
//! By the time this runs the engine has put the video model's VRAM slot on the
//! GPU (scheduler → `ComfyUiAdapter::load_model`, which starts the server). Here
//! we turn the job's params into a fixed workflow ([`crate::pipeline`]) — Wan 2.2
//! or LTX-Video, picked from the model's family — hand it to ComfyUI, wait for
//! the clip, and write it to `<outputs>/<job_id>.mp4`.
//!
//! With `init_image` (a finished job's id, or a path to an image) the frame is
//! copied into ComfyUI's `input/` folder and wired as the start frame
//! (image→video, 4.2); the copy is removed after the render.
//!
//! Video is **slow** — minutes per clip. The event trail says so, and the UI
//! (4.3) sets expectations up front.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::media::{comfy_err as video_err, file_name, resolve_seed, round_to, write_output};
use crate::db::{Database, EventLevel, Model};
use crate::pipeline::{self, LtxModels, VideoInputs, VideoRecipe, WanModels};
use crate::runtime::ComfyUiAdapter;
use crate::Result;

/// Image extensions a start frame may have (ComfyUI's `LoadImage` reads these).
const FRAME_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

const DEFAULT_WIDTH: u32 = 832;
const DEFAULT_HEIGHT: u32 = 480;
const MIN_DIM: u32 = 128;
/// Hard cap — 720p-ish. Bigger is much slower and closer to the VRAM edge.
const MAX_DIM: u32 = 1280;
const DIM_STEP: u32 = 16;

/// Frames. Wan wants `(length - 1) % 4 == 0`, i.e. `4k + 1`.
const DEFAULT_LENGTH: u32 = 81; // ~3.4 s @ 24 fps
const MIN_LENGTH: u32 = 5;
const MAX_LENGTH: u32 = 121; // ~5 s @ 24 fps

const DEFAULT_FPS: u32 = 24;
const MIN_FPS: u32 = 8;
const MAX_FPS: u32 = 30;

const DEFAULT_STEPS: u32 = 30;
const MAX_STEPS: u32 = 60;

const DEFAULT_CFG: f64 = 5.0;
const MIN_CFG: f64 = 1.0;
const MAX_CFG: f64 = 15.0;

/// A video render takes minutes; this is the ceiling before the job fails
/// rather than hanging forever.
const VIDEO_TIMEOUT: Duration = Duration::from_secs(1800);

/// ComfyUI offloads the text encoder (~6 GB) and spills model weights to system
/// RAM. If free RAM is below `model size + this`, the render will thrash the
/// pagefile — warn the user (non-blocking).
const VIDEO_RAM_SLACK_MB: u64 = 6144;

/// A resolved text/image-to-video request, pulled from a job's `params`. Every
/// field has a default; only a non-empty `prompt` is required.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoRequest {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub length: u32,
    pub fps: u32,
    pub steps: u32,
    pub cfg: f64,
    pub seed: i64,
    /// A start frame for image→video: a completed job's id, or a path to an
    /// image file. `None` → text→video. Resolved + staged in [`run`].
    pub init_image: Option<String>,
}

impl VideoRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| video_err("video job has no `prompt`"))?
            .to_string();
        let negative = params
            .get("negative")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let dim = |key: &str, default: u32| {
            params
                .get(key)
                .and_then(Value::as_u64)
                .map_or(default, |v| round_to(v, DIM_STEP).clamp(MIN_DIM, MAX_DIM))
        };
        let clamped = |key: &str, default: u32, lo: u32, hi: u32| {
            params
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .map_or(default, |v| v.clamp(lo, hi))
        };
        let length = params
            .get("length")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .map_or(DEFAULT_LENGTH, round_video_length);
        let cfg = params
            .get("cfg")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_CFG, |v| v.clamp(MIN_CFG, MAX_CFG));

        let init_image = params
            .get("init_image")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        Ok(Self {
            prompt,
            negative,
            width: dim("width", DEFAULT_WIDTH),
            height: dim("height", DEFAULT_HEIGHT),
            length,
            fps: clamped("fps", DEFAULT_FPS, MIN_FPS, MAX_FPS),
            steps: clamped("steps", DEFAULT_STEPS, 1, MAX_STEPS),
            cfg,
            seed: resolve_seed(params),
            init_image,
        })
    }

    /// Write the resolved values back over the job's `params` so a random seed
    /// becomes reproducible and the gallery has concrete numbers.
    pub fn apply_to(&self, params: &mut Value) {
        let Some(obj) = params.as_object_mut() else {
            return;
        };
        obj.insert("prompt".into(), self.prompt.clone().into());
        obj.insert("negative".into(), self.negative.clone().into());
        obj.insert("width".into(), self.width.into());
        obj.insert("height".into(), self.height.into());
        obj.insert("length".into(), self.length.into());
        obj.insert("fps".into(), self.fps.into());
        obj.insert("steps".into(), self.steps.into());
        obj.insert("cfg".into(), self.cfg.into());
        obj.insert("seed".into(), self.seed.into());
        if let Some(src) = &self.init_image {
            obj.insert("init_image".into(), src.clone().into());
        }
    }

    /// Clip length in seconds (for UI copy).
    pub fn seconds(&self) -> f64 {
        f64::from(self.length) / f64::from(self.fps.max(1))
    }
}

/// A finished video body.
#[derive(Debug, Clone)]
pub struct VideoDone {
    pub output_path: PathBuf,
    pub seed: i64,
    pub width: u32,
    pub height: u32,
    pub length: u32,
    pub fps: u32,
}

/// How the video body came to rest.
#[derive(Debug, Clone)]
pub enum VideoOutcome {
    Done(VideoDone),
    /// The user cancelled while ComfyUI was rendering.
    Cancelled,
}

/// Render `req` on `comfyui` with `model` as the Wan diffusion model, save the
/// clip under `outputs_dir`. Returns when the file is written, ComfyUI errors,
/// or `cancel` flips to `true`.
pub async fn run(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    model: &Model,
    req: VideoRequest,
    cancel: watch::Receiver<bool>,
) -> Result<VideoOutcome> {
    let base_model = file_name(&model.file_path)?;
    let recipe = VideoRecipe::for_family(model.family.as_deref());

    // image→video: resolve the start frame and stage it in ComfyUI's `input/`
    // folder. The guard removes the copy when this function returns. Do it first
    // so a bad `init_image` fails before the "takes several minutes" event.
    let frame = match &req.init_image {
        Some(spec) => Some(stage_start_frame(db, comfyui, job_id, spec).await?),
        None => None,
    };

    let inputs = VideoInputs {
        positive: &req.prompt,
        negative: &req.negative,
        width: req.width,
        height: req.height,
        length: req.length,
        fps: req.fps,
        steps: req.steps,
        cfg: req.cfg,
        seed: req.seed,
        start_image: frame.as_ref().map(|f| f.name.as_str()),
        filename_prefix: job_id,
    };

    // Pick the template + resolve its companion files, and describe them for the
    // event trail.
    let (workflow, companions) = match recipe {
        VideoRecipe::Wan => {
            let c = resolve_wan_companions(db).await?;
            let g = pipeline::wan_ti2v(
                &inputs,
                &WanModels {
                    unet: base_model,
                    clip: &c.clip,
                    vae: &c.vae,
                },
            );
            (
                g,
                format!(
                    "Wan \u{2014} encoder \u{201c}{}\u{201d}, VAE \u{201c}{}\u{201d}",
                    c.clip, c.vae
                ),
            )
        }
        VideoRecipe::Ltx => {
            let t5 = resolve_ltx_encoder(db).await?;
            let g = pipeline::ltx_video(
                &inputs,
                &LtxModels {
                    checkpoint: base_model,
                    t5: &t5,
                },
            );
            (
                g,
                format!(
                    "LTX-Video \u{2014} T5 encoder \u{201c}{t5}\u{201d}, VAE from the checkpoint"
                ),
            )
        }
    };

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "rendering {}\u{00d7}{} video, {} frames @ {} fps (~{:.1}s), {} steps, cfg {:.1}, \
                 seed {} \u{2014} {}",
                req.width,
                req.height,
                req.length,
                req.fps,
                req.seconds(),
                req.steps,
                req.cfg,
                req.seed,
                model.name
            ),
        )
        .await?;
    if let Some(frame) = &frame {
        db.jobs()
            .append_event(
                job_id,
                EventLevel::Info,
                &format!(
                    "image\u{2192}video \u{2014} start frame from {}",
                    frame.source
                ),
            )
            .await?;
    }
    if let Some(short_by) = ram_shortfall_mb(model) {
        db.jobs()
            .append_event(
                job_id,
                EventLevel::Warn,
                &format!(
                    "system RAM is tight (~{short_by} MB short of the offload budget) \u{2014} \
                     ComfyUI spills the encoder + model here, so a long clip may swap to disk \
                     (very slow). Close other apps or lower the resolution / length.",
                ),
            )
            .await?;
    }
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!("{companions} \u{2014} this takes several minutes"),
        )
        .await?;

    let Some(media) = comfyui
        .generate_media(&workflow, cancel, VIDEO_TIMEOUT)
        .await?
    else {
        return Ok(VideoOutcome::Cancelled);
    };

    let ext = if media.extension.is_empty() {
        "mp4"
    } else {
        &media.extension
    };
    let output_path = write_output(outputs_dir, job_id, ext, &media.bytes).await?;

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

    Ok(VideoOutcome::Done(VideoDone {
        output_path,
        seed: req.seed,
        width: req.width,
        height: req.height,
        length: req.length,
        fps: req.fps,
    }))
}

/// A start frame copied into ComfyUI's `input/` folder. Dropping it removes the
/// copy — the frame is only needed for the one render.
struct StagedFrame {
    /// Bare file name, as `LoadImage` refers to it.
    name: String,
    /// Full path of the copy (removed on drop).
    path: PathBuf,
    /// What the user asked for — a job id or a path (for the event trail).
    source: String,
}

impl Drop for StagedFrame {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                path = %self.path.display(),
                "could not remove staged start frame: {e}"
            ),
        }
    }
}

/// Resolve `spec` (a completed job's id, or a path to an image) to a real file,
/// then copy it to `<comfyui input>/<job_id>.<ext>` for `LoadImage`.
async fn stage_start_frame(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    job_id: &str,
    spec: &str,
) -> Result<StagedFrame> {
    let source = resolve_start_frame(db, spec).await?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "png".to_string());

    let input_dir = comfyui.input_dir();
    tokio::fs::create_dir_all(&input_dir)
        .await
        .map_err(|e| video_err(format!("create {}: {e}", input_dir.display())))?;
    let name = format!("{job_id}.{ext}");
    let path = input_dir.join(&name);
    tokio::fs::copy(&source, &path).await.map_err(|e| {
        video_err(format!(
            "stage start frame {} \u{2192} {}: {e}",
            source.display(),
            path.display()
        ))
    })?;
    Ok(StagedFrame {
        name,
        path,
        source: spec.to_string(),
    })
}

/// A start frame is either the output of a finished job (the gallery hands us
/// its id) or a path to an image file on disk. Either way it must be an existing
/// image ComfyUI's `LoadImage` can read.
async fn resolve_start_frame(db: &Database, spec: &str) -> Result<PathBuf> {
    if let Some(job) = db.jobs().get(spec).await? {
        let out = job
            .output_path
            .ok_or_else(|| video_err(format!("job {spec} has no image output to start from")))?;
        return checked_frame(&out);
    }
    checked_frame(spec)
}

fn checked_frame(path: &str) -> Result<PathBuf> {
    let p = PathBuf::from(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !ext.as_deref().is_some_and(|e| FRAME_EXTS.contains(&e)) {
        return Err(video_err(format!(
            "start frame must be a {} image \u{2014} got {path}",
            FRAME_EXTS.join(" / ")
        )));
    }
    if !p.is_file() {
        return Err(video_err(format!("start frame not found: {path}")));
    }
    Ok(p)
}

/// Wan's two companion files, resolved from the library by role + name.
#[derive(Debug)]
struct WanCompanionFiles {
    clip: String,
    vae: String,
}

async fn resolve_wan_companions(db: &Database) -> Result<WanCompanionFiles> {
    let clip = db
        .models()
        .for_role("text_encoder")
        .await?
        .into_iter()
        .find(|m| name_is_umt5(&m.name) || name_is_umt5(&m.file_path))
        .ok_or_else(|| {
            video_err(
                "Wan needs the umt5 text encoder — import umt5_xxl_… as \
                 \u{201c}Text encoder / CLIP\u{201d} on the Models tab",
            )
        })?;
    let vae = db
        .models()
        .for_role("vae")
        .await?
        .into_iter()
        .find(|m| name_is_wan(&m.name) || name_is_wan(&m.file_path))
        .ok_or_else(|| {
            video_err("Wan needs its VAE — import wan2.2_vae.safetensors as \u{201c}VAE\u{201d}")
        })?;
    Ok(WanCompanionFiles {
        clip: file_name(&clip.file_path)?.to_string(),
        vae: file_name(&vae.file_path)?.to_string(),
    })
}

/// LTX-Video needs a plain `t5xxl` encoder (its VAE rides in the checkpoint).
/// Resolved by role + name — `t5` in the name, but not Wan's `umt5`.
async fn resolve_ltx_encoder(db: &Database) -> Result<String> {
    let t5 = db
        .models()
        .for_role("text_encoder")
        .await?
        .into_iter()
        .find(|m| name_is_t5(&m.name) || name_is_t5(&m.file_path))
        .ok_or_else(|| {
            video_err(
                "LTX-Video needs a T5 text encoder \u{2014} import t5xxl_… as \
                 \u{201c}Text encoder / CLIP\u{201d} on the Models tab",
            )
        })?;
    Ok(file_name(&t5.file_path)?.to_string())
}

fn name_is_umt5(s: &str) -> bool {
    s.to_ascii_lowercase().contains("umt5")
}

fn name_is_wan(s: &str) -> bool {
    s.to_ascii_lowercase().contains("wan")
}

fn name_is_t5(s: &str) -> bool {
    let n = s.to_ascii_lowercase();
    n.contains("t5") && !n.contains("umt5")
}

/// How many MB the video model's offload budget exceeds free system RAM by, or
/// `None` when there is enough headroom. Reads live RAM via `sysinfo`.
fn ram_shortfall_mb(model: &Model) -> Option<u64> {
    let model_mb = u64::try_from(model.size_bytes).unwrap_or(0) / (1024 * 1024);
    ram_shortfall(model_mb, available_ram_mb())
}

fn available_ram_mb() -> u64 {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.available_memory() / (1024 * 1024)
}

/// `model_mb + VIDEO_RAM_SLACK_MB` minus `available_mb`, when positive.
fn ram_shortfall(model_mb: u64, available_mb: u64) -> Option<u64> {
    model_mb
        .saturating_add(VIDEO_RAM_SLACK_MB)
        .checked_sub(available_mb)
        .filter(|short| *short > 0)
}

/// Round a requested frame count to Wan's `4k + 1` and clamp.
fn round_video_length(v: u32) -> u32 {
    let k = (v.saturating_sub(1) + 2) / 4;
    (4 * k + 1).clamp(MIN_LENGTH, MAX_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_params_fills_video_defaults() {
        let r = VideoRequest::from_params(&serde_json::json!({ "prompt": " a boat " })).unwrap();
        assert_eq!(r.prompt, "a boat");
        assert_eq!(r.width, DEFAULT_WIDTH);
        assert_eq!(r.height, DEFAULT_HEIGHT);
        assert_eq!(r.length, DEFAULT_LENGTH);
        assert_eq!(r.fps, DEFAULT_FPS);
        assert_eq!(r.steps, DEFAULT_STEPS);
        assert!(r.seed >= 0);
        assert!((r.seconds() - 3.375).abs() < 0.01);
    }

    #[test]
    fn from_params_rejects_a_blank_prompt() {
        assert!(VideoRequest::from_params(&serde_json::json!({})).is_err());
    }

    #[test]
    fn length_is_snapped_to_wan_grid_and_clamped() {
        assert_eq!(round_video_length(80), 81);
        assert_eq!(round_video_length(100), 101);
        assert_eq!(round_video_length(3), MIN_LENGTH);
        assert_eq!(round_video_length(9999), MAX_LENGTH);
        // every result is 4k + 1
        for v in [1, 7, 40, 81, 200] {
            assert_eq!(round_video_length(v) % 4, 1);
        }
    }

    #[test]
    fn from_params_clamps_size_length_steps() {
        let r = VideoRequest::from_params(&serde_json::json!({
            "prompt": "x", "width": 5000, "height": 470, "length": 400, "steps": 999, "fps": 120
        }))
        .unwrap();
        assert_eq!(r.width, MAX_DIM);
        assert_eq!(r.height, 464); // 470 → nearest 16
        assert_eq!(r.length, MAX_LENGTH);
        assert_eq!(r.steps, MAX_STEPS);
        assert_eq!(r.fps, MAX_FPS);
    }

    #[test]
    fn apply_to_pins_values_without_dropping_engine_metadata() {
        let mut params = serde_json::json!({ "prompt": "sea", "vram_needed_mb": 12000 });
        let r = VideoRequest::from_params(&params).unwrap();
        r.apply_to(&mut params);
        assert_eq!(params["seed"], r.seed);
        assert_eq!(params["length"], DEFAULT_LENGTH);
        assert_eq!(params["vram_needed_mb"], 12000);
    }

    #[test]
    fn from_params_reads_init_image_and_apply_to_round_trips_it() {
        let none = VideoRequest::from_params(&serde_json::json!({ "prompt": "x" })).unwrap();
        assert_eq!(none.init_image, None);
        let blank =
            VideoRequest::from_params(&serde_json::json!({ "prompt": "x", "init_image": "  " }))
                .unwrap();
        assert_eq!(blank.init_image, None);

        let mut params = serde_json::json!({ "prompt": "x", "init_image": "  C:\\shots\\a.png  " });
        let r = VideoRequest::from_params(&params).unwrap();
        assert_eq!(r.init_image.as_deref(), Some("C:\\shots\\a.png"));
        r.apply_to(&mut params);
        assert_eq!(params["init_image"], "C:\\shots\\a.png");
    }

    #[test]
    fn checked_frame_rejects_non_images_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("frame.png");
        std::fs::write(&good, b"x").unwrap();
        assert_eq!(checked_frame(&good.to_string_lossy()).unwrap(), good);

        let mp4 = tmp.path().join("clip.mp4");
        std::fs::write(&mp4, b"x").unwrap();
        assert!(checked_frame(&mp4.to_string_lossy())
            .unwrap_err()
            .to_string()
            .contains("must be a"));

        assert!(
            checked_frame(&tmp.path().join("gone.png").to_string_lossy())
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }

    #[tokio::test]
    async fn resolve_start_frame_takes_a_path_or_a_finished_jobs_output() {
        use crate::db::{Database, NewJob};
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();

        // A bare path to an image on disk.
        let ondisk = tmp.path().join("hand.jpg");
        std::fs::write(&ondisk, b"x").unwrap();
        assert_eq!(
            resolve_start_frame(&db, &ondisk.to_string_lossy())
                .await
                .unwrap(),
            ondisk
        );

        // An image job that has not produced anything yet → a clear error.
        let job = db.jobs().insert(NewJob::new("image")).await.unwrap();
        assert!(resolve_start_frame(&db, &job.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("no image output"));
        // (a job id that resolves to a real output is covered end-to-end in
        // tests/video_job.rs)
    }

    #[test]
    fn ram_shortfall_flags_a_tight_offload_budget() {
        // 10 GB model + 6 GB slack, 32 GB free → fits.
        assert_eq!(ram_shortfall(10_240, 32_768), None);
        // same model, only 8 GB free → 8 GB short.
        assert_eq!(ram_shortfall(10_240, 8_192), Some(10_240 + 6144 - 8_192));
        assert_eq!(ram_shortfall(0, 0), Some(6144));
    }

    #[test]
    fn companion_name_heuristics() {
        assert!(name_is_umt5("umt5_xxl_fp8_e4m3fn_scaled.safetensors"));
        assert!(!name_is_umt5("t5xxl_fp8.safetensors"));
        assert!(name_is_wan("wan2.2_vae.safetensors"));
        assert!(!name_is_wan("ae.safetensors"));
        // LTX's T5 is `t5…` but Wan's `umt5…` must not match it.
        assert!(name_is_t5("t5xxl_fp8_e4m3fn.safetensors"));
        assert!(!name_is_t5("umt5_xxl_fp8_e4m3fn_scaled.safetensors"));
        assert!(!name_is_t5("clip_l.safetensors"));
    }

    #[tokio::test]
    async fn resolve_ltx_encoder_finds_the_t5_but_not_wans_umt5() {
        use crate::db::{Database, NewModel};
        let db = Database::connect_in_memory().await.unwrap();
        let add = |name: &str| {
            let name = name.to_string();
            let db = db.clone();
            async move {
                db.models()
                    .insert(NewModel {
                        name: name.clone(),
                        format: "safetensors".into(),
                        file_path: format!("E:\\AI\\models\\image\\text_encoders\\{name}"),
                        size_bytes: 1,
                        source: "manual".into(),
                        roles: vec!["text_encoder".into()],
                        ..NewModel::default()
                    })
                    .await
                    .unwrap();
            }
        };

        add("umt5_xxl_fp8_e4m3fn_scaled.safetensors").await;
        assert!(resolve_ltx_encoder(&db)
            .await
            .unwrap_err()
            .to_string()
            .contains("T5 text encoder"));
        add("t5xxl_fp8_e4m3fn.safetensors").await;
        assert_eq!(
            resolve_ltx_encoder(&db).await.unwrap(),
            "t5xxl_fp8_e4m3fn.safetensors"
        );
    }

    #[tokio::test]
    async fn resolve_wan_companions_reports_what_is_missing() {
        use crate::db::{Database, NewModel};
        let db = Database::connect_in_memory().await.unwrap();
        let add = |name: &str, role: &str| {
            let (name, role) = (name.to_string(), role.to_string());
            let db = db.clone();
            async move {
                db.models()
                    .insert(NewModel {
                        name: name.clone(),
                        format: "safetensors".into(),
                        file_path: format!("E:\\AI\\models\\video\\x\\{name}"),
                        size_bytes: 1,
                        source: "manual".into(),
                        roles: vec![role],
                        ..NewModel::default()
                    })
                    .await
                    .unwrap();
            }
        };

        assert!(resolve_wan_companions(&db)
            .await
            .unwrap_err()
            .to_string()
            .contains("umt5"));
        add("umt5_xxl_fp8_e4m3fn_scaled.safetensors", "text_encoder").await;
        assert!(resolve_wan_companions(&db)
            .await
            .unwrap_err()
            .to_string()
            .contains("VAE"));
        add("wan2.2_vae.safetensors", "vae").await;
        let c = resolve_wan_companions(&db).await.unwrap();
        assert_eq!(c.clip, "umt5_xxl_fp8_e4m3fn_scaled.safetensors");
        assert_eq!(c.vae, "wan2.2_vae.safetensors");
    }
}
