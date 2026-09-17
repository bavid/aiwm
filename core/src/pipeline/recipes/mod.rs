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
/// built. That is a bug in this module, not in anything a user supplied, so it
/// is logged rather than panicked on — the graph is still returned, just
/// without the LoRA chain wired through. The unit tests and golden fixtures
/// pin every consumer id.
pub(crate) fn finish(g: Graph, applied: Result<(), PipelineError>) -> Value {
    if let Err(err) = applied {
        tracing::error!(
            error = %err,
            "LoRA chain not spliced: recipe named a consumer node it did not build"
        );
    }
    g.into_value()
}
