//! The image recipes, each composed from [`crate::pipeline::fragments`].
//!
//! Every function here is re-exported from [`crate::pipeline`] under its
//! original name and signature, and produces exactly the graph its inline
//! `json!` predecessor did — the golden fixtures in
//! `core/tests/pipeline_goldens.rs` pin that byte-for-byte.

use serde_json::Value;

use crate::pipeline::fragments::{conditioning, latent, loaders, loras, output, sampling};
use crate::pipeline::graph::Graph;
use crate::pipeline::recipes::{
    finish, DENOISE_FULL, DISTILLED_SAMPLER_CFG, FLUX_GUIDANCE_RANGE, SOURCE_MEGAPIXELS,
    SOURCE_RESOLUTION_STEPS, SOURCE_UPSCALE_METHOD,
};
use crate::pipeline::{EditInputs, Flux2KleinModels, FluxModels, LoraSpec, Txt2ImgInputs};

/// The canonical ComfyUI default graph: load a single-file checkpoint, encode
/// both prompts, sample, VAE-decode, save. No custom nodes.
pub fn checkpoint_txt2img(i: &Txt2ImgInputs, checkpoint: &str, loras: &[LoraSpec]) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::checkpoint(&mut g, "4", checkpoint);
    let canvas = latent::empty(&mut g, "5", i.width, i.height);
    let cond = conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &loaded.model,
        &cond.positive,
        &cond.negative,
        &canvas,
        &sampling::SamplerParams {
            seed: i.seed,
            steps: i.steps,
            cfg: i.cfg,
            sampler: i.sampler,
            scheduler: i.scheduler,
            denoise: DENOISE_FULL,
        },
    );
    output::decode_and_save(&mut g, "8", "9", &sampled, &loaded.vae, i.filename_prefix);
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, "3", &["6", "7"]);
    finish(g, applied)
}

/// FLUX.1-dev via `ComfyUI-GGUF`: `UnetLoaderGGUF` + `DualCLIPLoaderGGUF`
/// (type `flux`, T5 + CLIP-L) + `VAELoader`, a `FluxGuidance` on the positive
/// conditioning, an SD3-format latent, then the sampler at CFG 1.
pub fn flux_txt2img(i: &Txt2ImgInputs, m: &FluxModels, loras: &[LoraSpec]) -> Value {
    let guidance = i.cfg.clamp(FLUX_GUIDANCE_RANGE.0, FLUX_GUIDANCE_RANGE.1);
    let mut g = Graph::default();
    let loaded = loaders::flux_gguf(
        &mut g,
        &loaders::SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        },
        m.unet,
        m.clip_l,
        m.t5,
        m.vae,
    );
    let canvas = latent::empty_sd3(&mut g, "5", i.width, i.height);
    let cond = conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
    let guided = conditioning::flux_guidance(&mut g, "26", &cond.positive, guidance);
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &loaded.model,
        &guided,
        &cond.negative,
        &canvas,
        &sampling::SamplerParams {
            seed: i.seed,
            steps: i.steps,
            cfg: DISTILLED_SAMPLER_CFG,
            sampler: "euler",
            scheduler: "simple",
            denoise: DENOISE_FULL,
        },
    );
    output::decode_and_save(&mut g, "8", "9", &sampled, &loaded.vae, i.filename_prefix);
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, "3", &["6", "7"]);
    finish(g, applied)
}

/// FLUX.2 \[klein\] via `ComfyUI-GGUF`: `UnetLoaderGGUF` plus a single
/// `CLIPLoader` (`type: "flux2"`, one Qwen3 encoder — not a T5 + CLIP-L pair)
/// plus `VAELoader`. No real negative prompt: FLUX.2's own template zeroes it
/// out (`ConditioningZeroOut`) rather than encoding one. Sampling goes through
/// `CFGGuider` + `SamplerCustomAdvanced` (driven by `Flux2Scheduler`'s sigmas
/// and `RandomNoise`) instead of a plain `KSampler` — genuinely FLUX.2's own
/// graph shape, verified against Comfy-Org's own
/// `image_flux2_klein_text_to_image` workflow template rather than assumed
/// from FLUX.1's.
pub fn flux2_klein_txt2img(i: &Txt2ImgInputs, m: &Flux2KleinModels, loras: &[LoraSpec]) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::flux2_klein_gguf(
        &mut g,
        &loaders::SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        },
        m.unet,
        m.clip,
        m.vae,
    );
    let positive = conditioning::encode_single(&mut g, "6", &loaded.clip, i.positive);
    let negative = conditioning::zero_out(&mut g, "27", &positive);
    let canvas = latent::empty_flux2(&mut g, "32", i.width, i.height);
    let sampled = sampling::custom_advanced(
        &mut g,
        &sampling::CustomAdvancedIds {
            select: "28",
            scheduler: "29",
            noise: "30",
            guider: "31",
            sampler: "3",
        },
        &loaded.model,
        &positive,
        &negative,
        &canvas,
        &sampling::CustomAdvancedParams {
            seed: i.seed,
            steps: i.steps,
            width: i.width.into(),
            height: i.height.into(),
            sampler: i.sampler,
            cfg: i.cfg,
            sigmas_override: None,
        },
    );
    output::decode_and_save(&mut g, "8", "9", &sampled, &loaded.vae, i.filename_prefix);
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, "31", &["6"]);
    finish(g, applied)
}

/// FLUX.2 \[klein\] from a plain `.safetensors` checkpoint: `UNETLoader` (no
/// custom node) + the same single-encoder `CLIPLoader` (`type: "flux2"`) +
/// `VAELoader`. Same "no real negative prompt" behavior as the GGUF recipe
/// (`ConditioningZeroOut`), but sampled FLUX.1-style: a `FluxGuidance` on the
/// positive conditioning and a plain `KSampler` pinned to CFG 1, rather than
/// `CFGGuider`/`SamplerCustomAdvanced`. Verified against a real exported
/// ComfyUI workflow for `flux-2-klein-9b-fp8.safetensors`.
pub fn flux2_klein_txt2img_safetensors(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    loras: &[LoraSpec],
) -> Value {
    let guidance = i.cfg.clamp(FLUX_GUIDANCE_RANGE.0, FLUX_GUIDANCE_RANGE.1);
    let mut g = Graph::default();
    let loaded = loaders::flux2_klein_safetensors(
        &mut g,
        &loaders::SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        },
        m.unet,
        m.clip,
        m.vae,
    );
    let encoded = conditioning::encode_single(&mut g, "6", &loaded.clip, i.positive);
    let negative = conditioning::zero_out(&mut g, "27", &encoded);
    let positive = conditioning::flux_guidance(&mut g, "26", &encoded, guidance);
    let canvas = latent::empty_flux2(&mut g, "32", i.width, i.height);
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &loaded.model,
        &positive,
        &negative,
        &canvas,
        &sampling::SamplerParams {
            seed: i.seed,
            steps: i.steps,
            cfg: DISTILLED_SAMPLER_CFG,
            sampler: i.sampler,
            scheduler: i.scheduler,
            denoise: DENOISE_FULL,
        },
    );
    output::decode_and_save(&mut g, "8", "9", &sampled, &loaded.vae, i.filename_prefix);
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, "3", &["6"]);
    finish(g, applied)
}

/// FLUX.2 \[klein\] 9B's own image-editing graph — the same model unifies
/// generation and editing, it just needs its own VAE
/// (`full_encoder_small_decoder.safetensors`, not the plain generation one)
/// and a different graph shape: `LoadImage` the source, rescale to ~1
/// megapixel, encode it into a latent, and inject that latent into *both* the
/// positive and the zeroed-out negative conditioning via `ReferenceLatent` —
/// the model edits from the real image instead of generating from nothing.
/// Output size matches the (rescaled) input image, not a fixed square.
/// Verified against Comfy-Org's own
/// `image_flux2_klein_image_edit_9b_distilled` workflow template.
pub fn flux2_klein_edit(i: &EditInputs, m: &Flux2KleinModels, loras: &[LoraSpec]) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::flux2_klein_safetensors(
        &mut g,
        &loaders::SplitModelIds {
            unet: "70",
            clip: "71",
            vae: "72",
        },
        m.unet,
        m.clip,
        m.vae,
    );
    let source = output::load_image(&mut g, "76", i.source_image);
    let scaled = output::scale_to_total_pixels(
        &mut g,
        "80",
        &source,
        SOURCE_UPSCALE_METHOD,
        SOURCE_MEGAPIXELS,
        SOURCE_RESOLUTION_STEPS,
    );
    // Both the output canvas and the sampler's sigma schedule are sized from
    // the rescaled source, not from fixed inputs.
    let size = output::image_size(&mut g, "99", &scaled);
    // Encoded once, then referenced by *both* conditionings below.
    let source_latent = latent::vae_encode(&mut g, "124", &scaled, &loaded.vae);
    let instruction = conditioning::encode_single(&mut g, "74", &loaded.clip, i.instruction);
    let positive = conditioning::reference_latent(&mut g, "123", &instruction, &source_latent);
    let zeroed = conditioning::zero_out(&mut g, "82", &instruction);
    let negative = conditioning::reference_latent(&mut g, "125", &zeroed, &source_latent);
    let canvas = latent::empty_flux2(&mut g, "66", size.width.clone(), size.height.clone());
    let sampled = sampling::custom_advanced(
        &mut g,
        &sampling::CustomAdvancedIds {
            select: "61",
            scheduler: "62",
            noise: "73",
            guider: "63",
            sampler: "64",
        },
        &loaded.model,
        &positive,
        &negative,
        &canvas,
        &sampling::CustomAdvancedParams {
            seed: i.seed,
            steps: i.steps,
            width: size.width,
            height: size.height,
            sampler: i.sampler,
            cfg: i.cfg,
            sigmas_override: None,
        },
    );
    output::decode_and_save(&mut g, "65", "9", &sampled, &loaded.vae, i.filename_prefix);
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, "63", &["74"]);
    finish(g, applied)
}

#[cfg(test)]
mod tests;
