//! The Story Studio character-consistency recipes, each composed from
//! [`crate::pipeline::fragments`].
//!
//! Three ways to keep the same character across renders: IP-Adapter
//! conditioning for SDXL-family checkpoints, and FLUX.2 \[klein\]'s
//! `ReferenceLatent` mechanism for both its GGUF and its plain `.safetensors`
//! form. Every function here is re-exported from [`crate::pipeline`] under its
//! original name and signature, and produces exactly the graph its inline
//! `json!` predecessor did — the golden fixtures in
//! `core/tests/pipeline_goldens.rs` pin that byte-for-byte.

use serde_json::Value;

use crate::pipeline::fragments::{
    conditioning, ipadapter, latent, loaders, loras, output, reference, sampling,
};
use crate::pipeline::graph::Graph;
use crate::pipeline::recipes::{
    finish, DENOISE_FULL, DISTILLED_SAMPLER_CFG, FLUX_GUIDANCE_RANGE, SOURCE_MEGAPIXELS,
    SOURCE_RESOLUTION_STEPS, SOURCE_UPSCALE_METHOD,
};
use crate::pipeline::{Flux2KleinModels, IpAdapterSpec, LoraSpec, Txt2ImgInputs};

/// How the reference portrait is rescaled before it is encoded — the same
/// budget and filter the edit recipe uses.
const REFERENCE_RESCALE: reference::Rescale<'static> = reference::Rescale {
    method: SOURCE_UPSCALE_METHOD,
    megapixels: SOURCE_MEGAPIXELS,
    resolution_steps: SOURCE_RESOLUTION_STEPS,
};

/// Node ids for the reference portrait's load → rescale → encode chain, shared
/// by both FLUX.2 \[klein\] reference recipes.
const REFERENCE_IDS: reference::GuidingLatentIds<'static> = reference::GuidingLatentIds {
    load: "50",
    scale: "51",
    encode: "52",
};

/// Node ids for the pair of `ReferenceLatent` nodes and the
/// `ConditioningZeroOut` between them, likewise shared.
const ANCHOR_IDS: reference::AnchorIds<'static> = reference::AnchorIds {
    positive: "53",
    zero_out: "27",
    negative: "54",
};

/// Node ids for the three FLUX.2 \[klein\] loaders.
const KLEIN_LOADER_IDS: loaders::SplitModelIds<'static> = loaders::SplitModelIds {
    unet: "12",
    clip: "11",
    vae: "10",
};

/// SDXL (and any single-file SD checkpoint) txt2img with IP-Adapter
/// character/style conditioning spliced in: the same graph as
/// [`super::image::checkpoint_txt2img`], plus a `CLIPVisionLoader` +
/// `IPAdapterModelLoader` + `LoadImage` feeding an `IPAdapterAdvanced` node
/// between the checkpoint (and any LoRAs) and the sampler.
///
/// LoRAs patch the checkpoint's model/CLIP *before* IP-Adapter sees it (the
/// common community ordering: checkpoint → LoRA → IPAdapter → sampler), so the
/// chain is routed into the `IPAdapterAdvanced` node rather than into the
/// sampler directly.
pub fn checkpoint_ipadapter_txt2img(
    i: &Txt2ImgInputs,
    checkpoint: &str,
    ip: &IpAdapterSpec,
    loras: &[LoraSpec],
) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::checkpoint(&mut g, "4", checkpoint);
    let canvas = latent::empty(&mut g, "5", i.width, i.height);
    let cond = conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
    let clip_vision = ipadapter::clip_vision_loader(&mut g, "40", ip.clip_vision);
    let ip_model = ipadapter::model_loader(&mut g, "41", ip.ipadapter_model);
    let portrait = output::load_image(&mut g, "42", ip.reference_image);
    let patched = ipadapter::advanced(
        &mut g,
        "43",
        &loaded.model,
        &ipadapter::AdvancedInputs {
            ipadapter: &ip_model,
            image: &portrait,
            clip_vision: &clip_vision,
            weight: ip.weight,
        },
    );
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &patched,
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
    let applied = loras::apply(
        &mut g,
        loras,
        &loaded.model,
        &loaded.clip,
        &["43"],
        &["6", "7"],
    );
    finish(g, applied)
}

/// FLUX.2 \[klein\] GGUF txt2img with a reference portrait steering the render,
/// built on the same mechanism [`super::image::flux2_klein_edit`] uses for
/// instruction-based edits: `ReferenceLatent`. Unlike an edit, this is a
/// *fresh* generation from noise at the caller's own `width`/`height` (not the
/// reference's own size) — the reference only anchors identity/style via the
/// guiding latent injected into both the real and the zeroed-out conditioning,
/// the same "Kontext-style" technique real FLUX.2/Kontext character-consistency
/// workflows use.
///
/// Only meaningful for FLUX.2 \[klein\], which is edit-trained (that's what
/// makes `ReferenceLatent` actually do something — see its own doc string,
/// "sets the guiding latent for an edit model"). Plain FLUX.1-dev was never
/// trained to read `reference_latents` conditioning, so this recipe is not
/// offered for it (see `docs/TODO.md`'s Story Studio Phase 2 entry for the
/// FLUX.1 gap and what would close it — Flux Redux).
///
/// GGUF checkpoints only — see [`flux2_klein_reference_txt2img_safetensors`]
/// for a plain `.safetensors` FLUX.2 \[klein\] file (a real distinction, not a
/// pedantic one: `UnetLoaderGGUF` only ever lists `.gguf` files to ComfyUI, so
/// handing it a `.safetensors` name fails at `/prompt` submission — hit live
/// during this slice's real end-to-end smoke test, not a hypothetical).
pub fn flux2_klein_reference_txt2img(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    reference_image: &str,
    loras: &[LoraSpec],
) -> Value {
    let mut g = Graph::default();
    let loaded = loaders::flux2_klein_gguf(&mut g, &KLEIN_LOADER_IDS, m.unet, m.clip, m.vae);
    let guiding = reference::guiding_latent(
        &mut g,
        &REFERENCE_IDS,
        reference_image,
        &loaded.vae,
        &REFERENCE_RESCALE,
    );
    let encoded = conditioning::encode_single(&mut g, "6", &loaded.clip, i.positive);
    let cond = reference::anchor_both(&mut g, &ANCHOR_IDS, &encoded, &guiding);
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
        &cond.positive,
        &cond.negative,
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
    // Story Studio takes no Hi-Res-Fix: a reference render is about keeping a
    // character consistent, not about resolution (spec §3).
    output::decode_and_save(
        &mut g,
        "8",
        "9",
        &sampled.sampled,
        &loaded.vae,
        i.filename_prefix,
    );
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, &["31"], &["6"]);
    finish(g, applied)
}

/// Same as [`flux2_klein_reference_txt2img`] but for a plain `.safetensors`
/// FLUX.2 \[klein\] checkpoint (`UNETLoader`, no `ComfyUI-GGUF` custom node) —
/// [`crate::pipeline::Recipe::Flux2KleinSafetensors`]'s own graph shape
/// (`FluxGuidance` + a plain `KSampler` at CFG 1, like
/// [`super::image::flux2_klein_txt2img_safetensors`]), not the GGUF recipe's
/// `CFGGuider`/`SamplerCustomAdvanced` chain. A real bug hit live (Story Studio
/// Phase 2 smoke test): reusing the GGUF-only function for a safetensors
/// checkpoint fails at `/prompt` submission with `unet_name: '<file>' not in
/// ['<the-gguf-file>']` — `UnetLoaderGGUF` only ever lists `.gguf` files, so a
/// `.safetensors` FLUX.2 \[klein\] file (like the fp8 mixed-precision one) is
/// invisible to it. This function exists specifically so
/// [`crate::pipeline::Recipe::for_family`]'s file-extension split has a correct
/// home on both sides.
pub fn flux2_klein_reference_txt2img_safetensors(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    reference_image: &str,
    loras: &[LoraSpec],
) -> Value {
    let guidance = i.cfg.clamp(FLUX_GUIDANCE_RANGE.0, FLUX_GUIDANCE_RANGE.1);
    let mut g = Graph::default();
    let loaded = loaders::flux2_klein_safetensors(&mut g, &KLEIN_LOADER_IDS, m.unet, m.clip, m.vae);
    let guiding = reference::guiding_latent(
        &mut g,
        &REFERENCE_IDS,
        reference_image,
        &loaded.vae,
        &REFERENCE_RESCALE,
    );
    let encoded = conditioning::encode_single(&mut g, "6", &loaded.clip, i.positive);
    let cond = reference::anchor_both(&mut g, &ANCHOR_IDS, &encoded, &guiding);
    let positive = conditioning::flux_guidance(&mut g, "26", &cond.positive, guidance);
    let canvas = latent::empty_flux2(&mut g, "32", i.width, i.height);
    let sampled = sampling::ksampler(
        &mut g,
        "3",
        &loaded.model,
        &positive,
        &cond.negative,
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
    let applied = loras::apply(&mut g, loras, &loaded.model, &loaded.clip, &["3"], &["6"]);
    finish(g, applied)
}

#[cfg(test)]
mod tests;
