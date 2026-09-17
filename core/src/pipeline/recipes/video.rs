//! The video recipes, each composed from [`crate::pipeline::fragments`].
//!
//! Both do text→video and image→video (a start frame already staged in
//! ComfyUI's `input/`); the start frame is the only branch in either graph.
//! Every function here is re-exported from [`crate::pipeline`] under its
//! original name and signature, and produces exactly the graph its inline
//! `json!` predecessor did — the golden fixtures in
//! `core/tests/pipeline_goldens.rs` pin that byte-for-byte.

use serde_json::Value;

use crate::pipeline::fragments::{conditioning, latent, loaders, loras, output, sampling, video};
use crate::pipeline::graph::Graph;
use crate::pipeline::recipes::{finish, DENOISE_FULL};
use crate::pipeline::{LoraSpec, LtxModels, VideoInputs, WanModels};

/// Wan 5B recommends a sampling shift around 8.
const WAN_SHIFT: f64 = 8.0;

/// Wan 2.2's sampler/scheduler pair, from ComfyUI's own Wan template. Not
/// caller-tunable: the recipe's `VideoInputs` carries no sampler knob.
const WAN_SAMPLER: &str = "uni_pc";
const WAN_SCHEDULER: &str = "simple";

/// `LTXVScheduler` defaults from ComfyUI's own LTXV template.
const LTX_MAX_SHIFT: f64 = 2.05;
const LTX_BASE_SHIFT: f64 = 0.95;
const LTX_TERMINAL: f64 = 0.1;
const LTX_STRETCH: bool = true;

/// `LTXVImgToVideo`: how strongly the start frame constrains the first frames.
const LTX_IMG_STRENGTH: f64 = 0.15;

/// LTX-Video samples through a `KSamplerSelect`, always with euler.
const LTX_SAMPLER: &str = "euler";

/// Wan 2.2 TI2V text/image-to-video. Core ComfyUI video nodes only:
/// `UNETLoader` + `CLIPLoader type=wan` + `VAELoader` → `ModelSamplingSD3`
/// (shift) → `WanImageToVideo` (the latent factory; `start_image` optional) →
/// `KSampler` → `VAEDecode` → `CreateVideo` → `SaveVideo` (mp4/h264).
pub fn wan_ti2v(i: &VideoInputs, m: &WanModels, loras: &[LoraSpec]) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::wan(
        &mut g,
        &loaders::SplitModelIds {
            unet: "37",
            clip: "38",
            vae: "39",
        },
        m.unet,
        m.clip,
        m.vae,
    );
    let cond = conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
    let model = video::model_sampling_sd3(&mut g, "48", &loaded.model, WAN_SHIFT);
    let start = i
        .start_image
        .map(|frame| output::load_image(&mut g, "60", frame));
    let clip = video::wan_image_to_video(
        &mut g,
        "55",
        &cond,
        &loaded.vae,
        &video::Frame {
            width: i.width,
            height: i.height,
            length: i.length,
        },
        start.as_ref(),
    );
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &model,
        &clip.cond.positive,
        &clip.cond.negative,
        &clip.latent,
        &sampling::SamplerParams {
            seed: i.seed,
            steps: i.steps,
            cfg: i.cfg,
            sampler: WAN_SAMPLER,
            scheduler: WAN_SCHEDULER,
            denoise: DENOISE_FULL,
        },
    );
    let frames = output::vae_decode(&mut g, "8", &sampled, &loaded.vae);
    let encoded = video::create_video(&mut g, "58", &frames, None, i.fps);
    video::save_video(&mut g, "59", &encoded, i.filename_prefix);
    let applied = loras::apply(
        &mut g,
        loras,
        &loaded.model,
        &loaded.clip,
        &["48"],
        &["6", "7"],
    );
    finish(g, applied)
}

/// LTX-Video 0.9.x text/image-to-video. All core ComfyUI nodes (no custom
/// pack): `CheckpointLoaderSimple` (model + VAE) + `CLIPLoader type="ltxv"` →
/// `CLIPTextEncode` ×2 → `LTXVConditioning` → `EmptyLTXVLatentVideo` (or
/// `LTXVImgToVideo` when a start frame is given, which replaces it and
/// produces the conditioning pair too) → `LTXVScheduler` sigmas →
/// `SamplerCustom` (`KSamplerSelect euler`) → `VAEDecode` → `CreateVideo` →
/// `SaveVideo`.
pub fn ltx_video(i: &VideoInputs, m: &LtxModels, loras: &[LoraSpec]) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::ltx(&mut g, "44", "38", m.checkpoint, m.t5);
    let encoded = conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
    let frame = video::Frame {
        width: i.width,
        height: i.height,
        length: i.length,
    };
    // I2V: LTXVImgToVideo produces the conditioning *and* the start latent,
    // replacing EmptyLTXVLatentVideo outright.
    let source = match i.start_image {
        Some(file) => {
            let start = output::load_image(&mut g, "78", file);
            video::ltxv_img_to_video(
                &mut g,
                "77",
                &encoded,
                &loaded.vae,
                &start,
                &frame,
                LTX_IMG_STRENGTH,
            )
        }
        None => video::VideoLatent {
            cond: encoded,
            latent: latent::empty_ltxv(&mut g, "70", i.width, i.height, i.length),
        },
    };
    let cond = video::ltxv_conditioning(&mut g, "69", &source.cond, i.fps);
    let sigmas = sampling::ltxv_scheduler(
        &mut g,
        "71",
        &sampling::LtxvSchedulerParams {
            steps: i.steps,
            max_shift: LTX_MAX_SHIFT,
            base_shift: LTX_BASE_SHIFT,
            stretch: LTX_STRETCH,
            terminal: LTX_TERMINAL,
        },
        &source.latent,
    );
    let sampler = sampling::ksampler_select(&mut g, "73", LTX_SAMPLER);
    let sampled = sampling::sampler_custom(
        &mut g,
        "72",
        &sampling::CustomLinks {
            model: &loaded.model,
            positive: &cond.positive,
            negative: &cond.negative,
            sampler: &sampler,
            sigmas: &sigmas,
            latent: &source.latent,
        },
        i.seed,
        i.cfg,
    );
    let frames = output::vae_decode(&mut g, "8", &sampled, &loaded.vae);
    let clip = video::create_video(&mut g, "58", &frames, None, i.fps);
    video::save_video(&mut g, "59", &clip, i.filename_prefix);
    let applied = loras::apply(
        &mut g,
        loras,
        &loaded.model,
        &loaded.clip,
        &["72"],
        &["6", "7"],
    );
    finish(g, applied)
}

#[cfg(test)]
mod tests;
