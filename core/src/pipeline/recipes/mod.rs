//! Concrete recipes, each composed from [`crate::pipeline::fragments`] on top
//! of a [`crate::pipeline::graph::Graph`].
//!
//! The public entry points stay on [`crate::pipeline`] itself (`pipeline::
//! checkpoint_txt2img` and friends) — this module holds the bodies, so a
//! recipe file stays about one model family rather than about every family at
//! once. Shared knobs and the `finish` helper live here because every recipe
//! file needs them.

use serde_json::Value;

use crate::pipeline::graph::{Graph, PipelineError};

pub mod image;
pub mod story;
pub mod upscale;
pub mod video;

/// Full denoise — every recipe starts from noise or from an encoded source
/// image, never from a partially-denoised latent.
pub(crate) const DENOISE_FULL: f64 = 1.0;

/// FLUX.1 and FLUX.2 \[klein\] are guidance-distilled: the sampler itself runs
/// at CFG 1 and the knob the user reaches for lives in `FluxGuidance`.
pub(crate) const DISTILLED_SAMPLER_CFG: f64 = 1.0;

/// The range `FluxGuidance` is meaningful over; a user-supplied CFG is clamped
/// into it rather than rejected.
pub(crate) const FLUX_GUIDANCE_RANGE: (f64, f64) = (1.0, 10.0);

/// The edit and reference recipes rescale their source image to roughly one
/// megapixel before encoding it — FLUX.2 \[klein\] 9B's own template's budget.
pub(crate) const SOURCE_MEGAPIXELS: f64 = 1.0;

/// `1` = no snapping, i.e. whatever size the megapixel budget lands on.
pub(crate) const SOURCE_RESOLUTION_STEPS: u32 = 1;

/// The rescale filter FLUX.2 \[klein\]'s edit template uses.
pub(crate) const SOURCE_UPSCALE_METHOD: &str = "lanczos";

/// Finish a recipe: surface a LoRA-splicing failure, then hand back the graph.
///
/// [`crate::pipeline::fragments::loras::apply`] can only fail when a consumer
/// node id is absent, which means the recipe above asked for an id it never
/// built. That is a bug in this module, not in anything a user supplied, so
/// release builds log it and still return the graph (just without the LoRA
/// chain wired through) rather than killing a render — the repo bans
/// `unwrap`/`expect` in production. Dev and test builds fail loudly instead:
/// the `debug_assert!` below turns a latent wiring bug into a visible one long
/// before it can ship. The unit tests and golden fixtures pin every consumer
/// id.
pub(crate) fn finish(g: Graph, applied: Result<(), PipelineError>) -> Value {
    debug_assert!(
        applied.is_ok(),
        "recipe named a LoRA consumer id it never built: {applied:?}"
    );
    if let Err(err) = applied {
        tracing::error!(
            error = %err,
            "LoRA chain not spliced: recipe named a consumer node it did not build"
        );
    }
    g.into_value()
}

/// The node-id map: which id ranges belong to which concern, so a new fragment
/// can pick ids without reading every recipe first.
///
/// ComfyUI node ids are arbitrary strings; every recipe here uses decimal
/// numbers. The ranges below are what a single rendered *graph* reserves — not
/// what this module reserves globally. Two recipes that can never appear in
/// the same graph may reuse an id: Story Studio's IP-Adapter chain and the
/// Hi-Res-Fix fragment both sit at 40–43, which is fine precisely because no
/// txt2img recipe emits an IP-Adapter chain and no Story Studio recipe takes a
/// `hires` input.
///
/// | Range | Concern | Recipe file |
/// |-------|---------|-------------|
/// | 1–5 | RTX upscale: load → `RTXVideoSuperResolution` → save | [`upscale`] |
/// | 3–12 | The shared txt2img skeleton: sampler, checkpoint, latent, both encodes, decode, save, and the three split loaders | [`image`], [`story`] |
/// | 26–32 | `FluxGuidance`/`ConditioningZeroOut` plus FLUX.2 \[klein\]'s `CFGGuider` chain and latent | [`image`], [`story`] |
/// | 37–39, 44, 48, 55, 58–60, 69–73, 77–78 | The Wan and LTX video chains | [`video`] |
/// | 40–43 | Story Studio's IP-Adapter chain (`CLIPVisionLoader`, `IPAdapterModelLoader`, `LoadImage`, `IPAdapterAdvanced`) | [`story`] |
/// | 40–44 | **Hi-Res-Fix** (`HIRES_ID_BASE` = 40, five ids) — reserved in the four txt2img recipes only, which is why none of them uses 40–44 for anything else | [`crate::pipeline::fragments::hires`] |
/// | 50–54 | FLUX.2 \[klein\]'s reference-portrait chain | [`story`] |
/// | 61–66, 70–76, 80, 82, 99, 123–125 | The `flux2_klein_edit` graph | [`image`] |
/// | 90–94 | **LoRA chain** (`LORA_ID_BASE` = 90, `MAX_LORAS` = 5 ids) — reserved in *every* recipe | [`crate::pipeline::fragments::loras`] |
///
/// `tests::no_recipe_puts_a_node_in_another_concerns_reserved_range` renders
/// every recipe (with a full five-LoRA chain) and pins the two cross-cutting
/// reservations — the table above is documentation, that test is the
/// enforcement.
///
/// The consts below are therefore test-only: production code allocates
/// *forward* from a fragment's own base (`LORA_ID_BASE`) and never needs the
/// end of a range. Naming the closed ranges here is what lets the test assert
/// the reservation instead of trusting it.
pub(crate) mod ids {
    #[cfg(test)]
    use std::ops::RangeInclusive;

    #[cfg(test)]
    pub(crate) use crate::pipeline::fragments::hires::HIRES_ID_BASE;
    #[cfg(test)]
    pub(crate) use crate::pipeline::fragments::loras::{LORA_ID_BASE, MAX_LORAS};

    /// The ids a LoRA chain can occupy: `90 ..= 94`.
    #[cfg(test)]
    pub(crate) const LORA_IDS: RangeInclusive<u32> =
        LORA_ID_BASE..=LORA_ID_BASE + MAX_LORAS as u32 - 1;

    /// The ids the Hi-Res-Fix fragment can occupy: `40 ..= 44` — the latent
    /// upscale, the KSampler-family second pass, and klein-GGUF's
    /// `Flux2Scheduler` + `SplitSigmasDenoise` + `SamplerCustomAdvanced`.
    #[cfg(test)]
    pub(crate) const HIRES_IDS: RangeInclusive<u32> = HIRES_ID_BASE..=HIRES_ID_BASE + 4;
}

#[cfg(test)]
mod tests;
