//! The text-to-image capability: drive one `job_type=image` body.
//!
//! By the time this runs the engine has put the checkpoint's VRAM slot on the
//! GPU (scheduler → `ComfyUiAdapter::load_model`, which starts the server).
//! Here we turn the job's params into a fixed SDXL workflow
//! ([`crate::pipeline`]), hand it to ComfyUI, wait for the image, and write it
//! to `<outputs>/<job_id>.<ext>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::media::{
    comfy_err as image_err, file_name, parse_loras, resolve_loras, resolve_seed, round_to,
    str_param, write_output, LoraRef,
};
use crate::db::{Database, EventLevel, Model};
use crate::pipeline::{
    self, EditInputs, Flux2KleinModels, FluxModels, LoraSpec, Recipe, Txt2ImgInputs,
};
use crate::runtime::ComfyUiAdapter;
use crate::Result;

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

/// Upper bound on one image render — a slow first checkpoint load plus a large,
/// high-step render. Past this the job fails rather than hanging forever.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(600);

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
    /// Library references, not yet resolved to files — [`run`] does that once
    /// it has a `Database` handle.
    pub loras: Vec<LoraRef>,
    /// A finished job's id, or a path to an image file — when set, this is an
    /// instruction-based *edit* of that image (`prompt` is the instruction)
    /// rather than a fresh generation. Only FLUX.2 [klein] supports it today.
    pub source_image: Option<String>,
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

        let source_image = params
            .get("source_image")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        Ok(Self {
            prompt,
            negative,
            width: dim("width"),
            height: dim("height"),
            steps,
            cfg,
            sampler: str_param(params, "sampler", DEFAULT_SAMPLER),
            scheduler: str_param(params, "scheduler", DEFAULT_SCHEDULER),
            seed: resolve_seed(params),
            loras: parse_loras(params),
            source_image,
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
        if let Some(src) = &self.source_image {
            obj.insert("source_image".into(), src.clone().into());
        }
        obj.insert(
            "loras".into(),
            serde_json::to_value(&self.loras).unwrap_or(Value::Array(Vec::new())),
        );
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
    let model_file = file_name(&model.file_path)?;

    if let Some(spec) = &req.source_image {
        return run_edit(
            db,
            comfyui,
            outputs_dir,
            job_id,
            model,
            model_file,
            &req,
            spec,
            cancel,
        )
        .await;
    }

    let recipe = Recipe::for_family(model.family.as_deref(), model_file);
    let resolved_loras = resolve_loras(db, &req.loras).await?;
    if !resolved_loras.is_empty() {
        let summary = resolved_loras
            .iter()
            .map(|l| format!("{} @ {:.2}", l.file, l.strength))
            .collect::<Vec<_>>()
            .join(", ");
        db.jobs()
            .append_event(job_id, EventLevel::Info, &format!("LoRA: {summary}"))
            .await?;
    }
    let lora_specs: Vec<LoraSpec> = resolved_loras
        .iter()
        .map(|l| LoraSpec {
            file: &l.file,
            strength: l.strength,
        })
        .collect();

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "rendering {}\u{00d7}{}, {} steps, {} {:.1}, seed {} \u{2014} {}",
                req.width,
                req.height,
                req.steps,
                if matches!(recipe, Recipe::FluxGguf | Recipe::Flux2KleinSafetensors) {
                    "guidance"
                } else {
                    "cfg"
                },
                req.cfg,
                req.seed,
                model.name
            ),
        )
        .await?;

    let inputs = Txt2ImgInputs {
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
    };
    let workflow = match recipe {
        Recipe::Checkpoint => pipeline::checkpoint_txt2img(&inputs, model_file, &lora_specs),
        Recipe::FluxGguf => {
            let c = resolve_flux_companions(db).await?;
            db.jobs()
                .append_event(
                    job_id,
                    EventLevel::Info,
                    &format!(
                        "Flux — T5 “{}”, CLIP-L “{}”, VAE “{}”",
                        c.t5, c.clip_l, c.vae
                    ),
                )
                .await?;
            pipeline::flux_txt2img(
                &inputs,
                &FluxModels {
                    unet: model_file,
                    t5: &c.t5,
                    clip_l: &c.clip_l,
                    vae: &c.vae,
                },
                &lora_specs,
            )
        }
        Recipe::Flux2KleinGguf => {
            let c = resolve_flux2_klein_companions(db).await?;
            db.jobs()
                .append_event(
                    job_id,
                    EventLevel::Info,
                    &format!("FLUX.2 — text encoder “{}”, VAE “{}”", c.clip, c.vae),
                )
                .await?;
            pipeline::flux2_klein_txt2img(
                &inputs,
                &Flux2KleinModels {
                    unet: model_file,
                    clip: &c.clip,
                    vae: &c.vae,
                },
                &lora_specs,
            )
        }
        Recipe::Flux2KleinSafetensors => {
            let c = resolve_flux2_klein_companions(db).await?;
            db.jobs()
                .append_event(
                    job_id,
                    EventLevel::Info,
                    &format!("FLUX.2 — text encoder “{}”, VAE “{}”", c.clip, c.vae),
                )
                .await?;
            pipeline::flux2_klein_txt2img_safetensors(
                &inputs,
                &Flux2KleinModels {
                    unet: model_file,
                    clip: &c.clip,
                    vae: &c.vae,
                },
                &lora_specs,
            )
        }
    };

    let Some(image) = comfyui
        .generate_media(job_id, &workflow, cancel, IMAGE_TIMEOUT)
        .await?
    else {
        return Ok(ImageOutcome::Cancelled);
    };

    let output_path = write_output(outputs_dir, job_id, &image.extension, &image.bytes).await?;

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

/// Edit `req.source_image` per `req.prompt` (the instruction) instead of
/// generating from nothing. Only FLUX.2 [klein] supports this today — a
/// plain-language config error names the requirement rather than silently
/// misapplying a text-to-image recipe to a model that can't do it.
#[allow(clippy::too_many_arguments)]
async fn run_edit(
    db: &Database,
    comfyui: &Arc<ComfyUiAdapter>,
    outputs_dir: &Path,
    job_id: &str,
    model: &Model,
    model_file: &str,
    req: &ImageRequest,
    source_spec: &str,
    cancel: watch::Receiver<bool>,
) -> Result<ImageOutcome> {
    if !model
        .family
        .as_deref()
        .is_some_and(|f| f.eq_ignore_ascii_case("flux2"))
    {
        return Err(image_err(
            "editing an image needs the FLUX.2 [klein] 9B stack \u{2014} pick it from the \
             Model dropdown, or import it from the Models tab first",
        ));
    }

    let staged = super::media::stage_image(db, &comfyui.input_dir(), job_id, source_spec).await?;
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!("editing {}", staged.source),
        )
        .await?;

    let resolved_loras = resolve_loras(db, &req.loras).await?;
    let lora_specs: Vec<LoraSpec> = resolved_loras
        .iter()
        .map(|l| LoraSpec {
            file: &l.file,
            strength: l.strength,
        })
        .collect();

    let c = resolve_flux2_klein_edit_companions(db).await?;
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "FLUX.2 edit — text encoder \u{201c}{}\u{201d}, VAE \u{201c}{}\u{201d}",
                c.clip, c.vae
            ),
        )
        .await?;

    let inputs = EditInputs {
        instruction: &req.prompt,
        source_image: &staged.name,
        steps: req.steps,
        cfg: req.cfg,
        sampler: &req.sampler,
        seed: req.seed,
        filename_prefix: job_id,
    };
    let workflow = pipeline::flux2_klein_edit(
        &inputs,
        &Flux2KleinModels {
            unet: model_file,
            clip: &c.clip,
            vae: &c.vae,
        },
        &lora_specs,
    );

    let Some(image) = comfyui
        .generate_media(job_id, &workflow, cancel, IMAGE_TIMEOUT)
        .await?
    else {
        return Ok(ImageOutcome::Cancelled);
    };

    let output_path = write_output(outputs_dir, job_id, &image.extension, &image.bytes).await?;
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

    // The edited output's real size follows the source image, not
    // `req.width`/`req.height` (an edit request has no dimension inputs of
    // its own) -- these are only used for a display line, not a correctness
    // path, so the slight imprecision doesn't need a second image-decode
    // just to read the real pixel size back out.
    Ok(ImageOutcome::Done(ImageDone {
        output_path,
        seed: req.seed,
        width: req.width,
        height: req.height,
    }))
}

/// Flux's three companion files, resolved from the library by role + name.
#[derive(Debug)]
struct FluxCompanionFiles {
    t5: String,
    clip_l: String,
    vae: String,
}

/// Find the T5 encoder, the CLIP-L encoder and the VAE a Flux job needs. Each
/// missing piece is a plain-language "import this" error rather than a cryptic
/// ComfyUI node failure.
async fn resolve_flux_companions(db: &Database) -> Result<FluxCompanionFiles> {
    let encoders = db.models().for_role("text_encoder").await?;
    let t5 = encoders
        .iter()
        .find(|m| name_is_t5(&m.name) || name_is_t5(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "Flux needs a T5 text encoder — import a t5xxl file (safetensors or GGUF) as \
                 \u{201c}Text encoder / CLIP\u{201d} on the Models tab",
            )
        })?;
    let clip_l = encoders
        .iter()
        .find(|m| name_is_clip_l(&m.name) || name_is_clip_l(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "Flux needs the CLIP-L text encoder — import clip_l.safetensors as \
                 \u{201c}Text encoder / CLIP\u{201d}",
            )
        })?;
    // Excludes FLUX.2's VAEs (plain and edit) by name -- now that other
    // `vae`-role files can exist in the library, picking blindly
    // (`pick_for_role`, most-recently-used) risked handing a FLUX.1 job one
    // of FLUX.2's incompatible VAEs.
    let vae = db
        .models()
        .for_role("vae")
        .await?
        .into_iter()
        .find(|m| {
            !name_is_flux2(&m.name)
                && !name_is_flux2(&m.file_path)
                && !name_is_flux2_edit_vae(&m.name)
                && !name_is_flux2_edit_vae(&m.file_path)
        })
        .ok_or_else(|| {
            image_err("Flux needs a VAE — import ae.safetensors as \u{201c}VAE\u{201d}")
        })?;

    Ok(FluxCompanionFiles {
        t5: file_name(&t5.file_path)?.to_string(),
        clip_l: file_name(&clip_l.file_path)?.to_string(),
        vae: file_name(&vae.file_path)?.to_string(),
    })
}

fn name_is_t5(s: &str) -> bool {
    s.to_ascii_lowercase().contains("t5")
}

fn name_is_clip_l(s: &str) -> bool {
    let n = s.to_ascii_lowercase();
    n.contains("clip") && !n.contains("t5")
}

/// FLUX.2 [klein]'s two companion files, resolved from the library by role +
/// name. Unlike FLUX.1, one text encoder does the whole job — but it still
/// needs telling apart from FLUX.1's T5/CLIP-L (same `text_encoder` role) and
/// from FLUX.1's own VAE (same `vae` role).
#[derive(Debug)]
struct Flux2KleinCompanionFiles {
    clip: String,
    vae: String,
}

async fn resolve_flux2_klein_companions(db: &Database) -> Result<Flux2KleinCompanionFiles> {
    let clip = db
        .models()
        .for_role("text_encoder")
        .await?
        .into_iter()
        .find(|m| name_is_qwen(&m.name) || name_is_qwen(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "FLUX.2 needs its Qwen3 text encoder — import qwen_3_8b_fp8mixed.safetensors \
                 as \u{201c}Text encoder / CLIP\u{201d} on the Models tab",
            )
        })?;
    let vae = db
        .models()
        .for_role("vae")
        .await?
        .into_iter()
        .find(|m| name_is_flux2(&m.name) || name_is_flux2(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "FLUX.2 needs its own VAE — import flux2-vae.safetensors as \u{201c}VAE\u{201d}",
            )
        })?;

    Ok(Flux2KleinCompanionFiles {
        clip: file_name(&clip.file_path)?.to_string(),
        vae: file_name(&vae.file_path)?.to_string(),
    })
}

fn name_is_qwen(s: &str) -> bool {
    s.to_ascii_lowercase().contains("qwen")
}

fn name_is_flux2(s: &str) -> bool {
    s.to_ascii_lowercase().contains("flux2")
}

/// FLUX.2's editing-only VAE (`full_encoder_small_decoder.safetensors`) --
/// its name doesn't contain "flux2" like the plain generation VAE does, so it
/// needs its own heuristic, distinct from [`name_is_flux2`], to keep the two
/// apart now that both share the `vae` role.
fn name_is_flux2_edit_vae(s: &str) -> bool {
    let n = s.to_ascii_lowercase();
    n.contains("small_decoder") || n.contains("small-decoder") || n.contains("small decoder")
}

/// FLUX.2 [klein]'s companions for *editing* — the same Qwen3 text encoder as
/// generation, but its own edit-specific VAE (a fuller encoder path for
/// re-encoding a real photo), not the plain generation one.
async fn resolve_flux2_klein_edit_companions(db: &Database) -> Result<Flux2KleinCompanionFiles> {
    let clip = db
        .models()
        .for_role("text_encoder")
        .await?
        .into_iter()
        .find(|m| name_is_qwen(&m.name) || name_is_qwen(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "Editing needs FLUX.2's Qwen3 text encoder — import \
                 qwen_3_8b_fp8mixed.safetensors as \u{201c}Text encoder / CLIP\u{201d} on the \
                 Models tab",
            )
        })?;
    let vae = db
        .models()
        .for_role("vae")
        .await?
        .into_iter()
        .find(|m| name_is_flux2_edit_vae(&m.name) || name_is_flux2_edit_vae(&m.file_path))
        .ok_or_else(|| {
            image_err(
                "Editing needs FLUX.2's edit VAE — import full_encoder_small_decoder.safetensors \
                 as \u{201c}VAE\u{201d} (Models tab \u{2192} Discover)",
            )
        })?;

    Ok(Flux2KleinCompanionFiles {
        clip: file_name(&clip.file_path)?.to_string(),
        vae: file_name(&vae.file_path)?.to_string(),
    })
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
    fn encoder_name_heuristics_split_t5_from_clip_l() {
        assert!(name_is_t5("t5xxl_fp8_e4m3fn.safetensors"));
        assert!(name_is_t5("t5-v1_1-xxl-encoder-Q8_0.gguf"));
        assert!(!name_is_t5("clip_l.safetensors"));

        assert!(name_is_clip_l("clip_l.safetensors"));
        assert!(name_is_clip_l("CLIP-L.safetensors"));
        assert!(!name_is_clip_l("t5xxl_fp8.safetensors"));
        // A combined-name file must not be claimed as CLIP-L.
        assert!(!name_is_clip_l("clip_t5_combined.safetensors"));
    }

    #[tokio::test]
    async fn resolve_flux_companions_needs_all_three() {
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
                        file_path: format!("E:\\AI\\models\\image\\x\\{name}"),
                        size_bytes: 1_000,
                        source: "manual".into(),
                        roles: vec![role],
                        ..NewModel::default()
                    })
                    .await
                    .unwrap();
            }
        };

        let err = resolve_flux_companions(&db).await.unwrap_err();
        assert!(err.to_string().contains("T5"), "{err}");

        add("t5xxl_fp8.safetensors", "text_encoder").await;
        assert!(resolve_flux_companions(&db)
            .await
            .unwrap_err()
            .to_string()
            .contains("CLIP-L"));

        add("clip_l.safetensors", "text_encoder").await;
        assert!(resolve_flux_companions(&db)
            .await
            .unwrap_err()
            .to_string()
            .contains("VAE"));

        add("ae.safetensors", "vae").await;
        let c = resolve_flux_companions(&db).await.unwrap();
        assert_eq!(c.t5, "t5xxl_fp8.safetensors");
        assert_eq!(c.clip_l, "clip_l.safetensors");
        assert_eq!(c.vae, "ae.safetensors");

        // FLUX.2's VAE shares the same `vae` role -- must not be mistaken for
        // FLUX.1's own, regardless of which was imported more recently.
        add("flux2-vae.safetensors", "vae").await;
        let c = resolve_flux_companions(&db).await.unwrap();
        assert_eq!(c.vae, "ae.safetensors");
    }

    #[tokio::test]
    async fn resolve_flux2_klein_companions_needs_both_and_ignores_flux1s() {
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
                        file_path: format!("E:\\AI\\models\\image\\x\\{name}"),
                        size_bytes: 1_000,
                        source: "manual".into(),
                        roles: vec![role],
                        ..NewModel::default()
                    })
                    .await
                    .unwrap();
            }
        };

        // FLUX.1's own encoder/VAE are in the library too -- must be skipped.
        add("t5xxl_fp8.safetensors", "text_encoder").await;
        add("ae.safetensors", "vae").await;

        let err = resolve_flux2_klein_companions(&db).await.unwrap_err();
        assert!(err.to_string().contains("Qwen3"), "{err}");

        add("qwen_3_8b_fp8mixed.safetensors", "text_encoder").await;
        let err = resolve_flux2_klein_companions(&db).await.unwrap_err();
        assert!(err.to_string().contains("VAE"), "{err}");

        add("flux2-vae.safetensors", "vae").await;
        let c = resolve_flux2_klein_companions(&db).await.unwrap();
        assert_eq!(c.clip, "qwen_3_8b_fp8mixed.safetensors");
        assert_eq!(c.vae, "flux2-vae.safetensors");

        // FLUX.2's *edit* VAE shares the same `vae` role too -- generation
        // must not pick it up even though it's also flux2-family.
        add("full_encoder_small_decoder.safetensors", "vae").await;
        let c = resolve_flux2_klein_companions(&db).await.unwrap();
        assert_eq!(c.vae, "flux2-vae.safetensors");
    }

    #[tokio::test]
    async fn resolve_flux2_klein_edit_companions_needs_its_own_vae_not_the_generation_one() {
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
                        file_path: format!("E:\\AI\\models\\image\\x\\{name}"),
                        size_bytes: 1_000,
                        source: "manual".into(),
                        roles: vec![role],
                        ..NewModel::default()
                    })
                    .await
                    .unwrap();
            }
        };

        // FLUX.1's encoder/VAE and FLUX.2's plain generation VAE are all in
        // the library -- none of them satisfy an edit request.
        add("t5xxl_fp8.safetensors", "text_encoder").await;
        add("ae.safetensors", "vae").await;
        add("qwen_3_8b_fp8mixed.safetensors", "text_encoder").await;
        add("flux2-vae.safetensors", "vae").await;

        let err = resolve_flux2_klein_edit_companions(&db).await.unwrap_err();
        assert!(err.to_string().contains("edit VAE"), "{err}");

        add("full_encoder_small_decoder.safetensors", "vae").await;
        let c = resolve_flux2_klein_edit_companions(&db).await.unwrap();
        assert_eq!(c.clip, "qwen_3_8b_fp8mixed.safetensors");
        assert_eq!(c.vae, "full_encoder_small_decoder.safetensors");
    }

    #[test]
    fn source_image_round_trips_and_marks_the_request_as_an_edit() {
        let none = ImageRequest::from_params(&serde_json::json!({ "prompt": "x" })).unwrap();
        assert_eq!(none.source_image, None);

        let mut params = serde_json::json!({ "prompt": "  make the hair blonde  ", "source_image": "  C:\\shots\\a.png  " });
        let r = ImageRequest::from_params(&params).unwrap();
        assert_eq!(r.source_image.as_deref(), Some("C:\\shots\\a.png"));
        r.apply_to(&mut params);
        assert_eq!(params["source_image"], "C:\\shots\\a.png");
    }

    #[tokio::test]
    async fn run_edit_refuses_a_non_flux2_model() {
        use crate::db::{Database, NewJob, NewModel};
        use crate::runtime::ComfyUiAdapter;

        let db = Database::connect_in_memory().await.unwrap();
        let model = db
            .models()
            .insert(NewModel {
                name: "SDXL Base 1.0".into(),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\image\\sdxl.safetensors".into(),
                size_bytes: 1_000,
                source: "manual".into(),
                family: Some("sdxl".into()),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let job = db.jobs().insert(NewJob::new("image")).await.unwrap();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let comfy = std::sync::Arc::new(ComfyUiAdapter::with_launch(
            db.clone(),
            None,
            crate::runtime::ComfyDirs {
                base: std::env::temp_dir(),
                output: std::env::temp_dir(),
                models_store: std::env::temp_dir(),
            },
        ));
        let req = ImageRequest::from_params(&serde_json::json!({
            "prompt": "make the hair blonde",
            "source_image": "C:\\nope.png"
        }))
        .unwrap();

        let err = run(
            &db,
            &comfy,
            std::path::Path::new("/tmp/out"),
            &job.id,
            &model,
            req,
            cancel_rx,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("FLUX.2"), "{err}");
    }
}
