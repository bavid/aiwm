//! Fixed generation pipelines: a workflow-JSON template plus parameter
//! substitution into known node slots.
//!
//! The MVP ships **fixed** pipelines only (PHASE_3_PLAN decision C): no graph
//! editor, no custom-workflow upload — that would be arbitrary node execution
//! again. Each recipe returns a ComfyUI *API-format* prompt graph
//! (`{ "<node id>": { "class_type", "inputs" } }`, links are
//! `["<node id>", <output slot>]`) ready for `POST /prompt`.
//!
//! This module is the *surface*: the input structs every recipe takes, the
//! [`Recipe`]/[`VideoRecipe`] pickers, and the re-exports that keep
//! `pipeline::checkpoint_txt2img` and friends importable from one place. The
//! graph-building itself lives in [`graph`] (the builder), [`fragments`]
//! (node-shaped building blocks) and [`recipes`] (one file per family).
//!
//! Image: [`checkpoint_txt2img`] (SDXL and any single-file SD checkpoint —
//! core nodes) and [`flux_txt2img`] (FLUX.1-dev GGUF + dual CLIP + VAE, via
//! `ComfyUI-GGUF`); [`Recipe::for_family`] picks. Video: [`wan_ti2v`] (Wan 2.2
//! TI2V — three separate files) and [`ltx_video`] (LTX-Video 0.9.x — a bundled
//! checkpoint + a T5); [`VideoRecipe::for_family`] picks. Both do text→video and
//! image→video (a start frame in ComfyUI's `input/`). A TOML pipeline *registry*
//! stays out of the MVP.

// The graph builder, the fragment layer and the recipe bodies are
// implementation detail: callers go through the re-exported recipe functions
// below. One deliberate exception: `capability::media` imports
// `fragments::loras::MAX_LORAS`, because the request parser's cap on how many
// LoRAs a job may carry is exactly what makes that fragment's reserved node-id
// window finite — the two have to move together, so the cap lives with the
// window rather than being restated at the boundary.
pub(crate) mod fragments;
pub(crate) mod graph;
pub(crate) mod recipes;

pub use recipes::image::{
    checkpoint_txt2img, flux2_klein_edit, flux2_klein_txt2img, flux2_klein_txt2img_safetensors,
    flux_txt2img, krea2_txt2img,
};
pub use recipes::story::{
    checkpoint_ipadapter_txt2img, flux2_klein_reference_txt2img,
    flux2_klein_reference_txt2img_safetensors,
};
pub use recipes::upscale::{rtx_upscale_image, rtx_upscale_video};
pub use recipes::video::{ltx_video, wan_ti2v};

/// The prompt + sampling parameters a text-to-image workflow needs, already
/// resolved (no `Auto`, no negative seeds). Model file names are passed
/// separately — they differ per template.
#[derive(Debug, Clone, Copy)]
pub struct Txt2ImgInputs<'a> {
    pub positive: &'a str,
    pub negative: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    /// `checkpoint_txt2img`: the KSampler CFG scale. `flux_txt2img`: the
    /// `FluxGuidance` value (Flux is guidance-distilled, so KSampler CFG is
    /// pinned to 1).
    pub cfg: f64,
    pub sampler: &'a str,
    pub scheduler: &'a str,
    pub seed: i64,
    /// `SaveImage` prefix — the job id, so the output is easy to find.
    pub filename_prefix: &'a str,
    /// `Some` → run a [`HiresFix`] second pass between the first sampler and
    /// the decode. `None` → the graph is exactly what it was before Hi-Res-Fix
    /// existed (the golden fixtures pin that).
    pub hires: Option<HiresFix>,
}

/// A Hi-Res-Fix second pass: upscale the first pass's *latent*, then re-sample
/// it at a low denoise. The model fills in detail at the larger size instead
/// of re-composing the image, which is what makes it different from running
/// the whole render at that resolution (faster, and no second composition to
/// fight the first).
///
/// Clamping belongs to the request parser, not here — a recipe takes whatever
/// numbers it is handed and wires them verbatim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HiresFix {
    /// `LatentUpscaleBy`'s `scale_by`: how much bigger the second pass runs.
    pub scale_by: f64,
    /// The second pass's denoise. Low (≈0.2–0.7) — high values throw the
    /// first pass's composition away.
    pub denoise: f64,
    /// Steps the second pass actually **executes** — the same meaning in every
    /// family. Usually about half the first pass's: the pass only fills in
    /// detail, it does not re-compose.
    ///
    /// That is what `KSampler.steps` already means at a denoise below 1 (it
    /// builds a `steps / denoise` schedule and runs the last `steps` of it,
    /// ComfyUI v0.34.0). FLUX.2 \[klein\] GGUF has no such node, so its
    /// fragment does the same arithmetic by hand — `Flux2Scheduler.steps =
    /// round(steps / denoise)`, cut back down by `SplitSigmasDenoise` — rather
    /// than letting this number mean "schedule length" there and "executed
    /// steps" everywhere else. See `fragments::hires::custom_advanced_pass`.
    pub steps: u32,
    /// `LatentUpscaleBy`'s `upscale_method` — one of ComfyUI's
    /// `nearest-exact`, `bilinear`, `area`, `bicubic`, `bislerp`.
    pub upscale_method: &'static str,
}

impl HiresFix {
    /// What ComfyUI's own Hi-Res-Fix templates reach for: the latent is about
    /// to be re-denoised anyway, so a cheap, artifact-free resample beats a
    /// smart one.
    pub const DEFAULT_METHOD: &str = "nearest-exact";
}

/// Pixels per latent unit. Every image family we render encodes 8×8 pixels
/// into one latent cell, which is what makes [`latent_upscaled_px`] the same
/// arithmetic for all of them.
const LATENT_SCALE: u32 = 8;

/// One dimension, in pixels, after `LatentUpscaleBy` — **the** formula for the
/// post-upscale size, used by both the graph builders and the capability layer
/// (`ImageRequest::final_size`), so a request's advertised output size and the
/// size the graph actually produces can never drift apart.
///
/// `LatentUpscaleBy` scales the *latent* and rounds there — `width =
/// round(samples.shape[-1] * scale_by)` in latent units (ComfyUI v0.34.0,
/// `nodes.py:1384-1385`) — so the decoded image is `round(px / 8 * scale) * 8`,
/// not `round(px * scale)`. Those differ: 1000 px × 1.25 is 1248, not 1250.
///
/// Note on halves: ComfyUI rounds with Python's `round()`, which breaks ties to
/// the *even* number, while Rust's `f64::round` breaks them away from zero. The
/// two disagree only when a tie would land on an odd result (a latent width of
/// 162.5: Python 162, Rust 163 → 8 px apart). Nothing pins the Python
/// behaviour today and matching it exactly would need a bespoke rounding, so
/// the Rust semantics stand.
#[must_use]
pub fn latent_upscaled_px(dim: u32, scale: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // `dim` is a clamped image size (≤ 2048) and `scale` a clamped factor
    // (≤ 2.0), so the product is small and never negative.
    let latent = (f64::from(dim / LATENT_SCALE) * scale).round() as u32;
    latent.saturating_mul(LATENT_SCALE)
}

/// FLUX needs its diffusion model, both text encoders and the VAE as separate
/// files (bare names as ComfyUI sees them in `diffusion_models` / `text_encoders`
/// / `vae`).
#[derive(Debug, Clone, Copy)]
pub struct FluxModels<'a> {
    pub unet: &'a str,
    pub t5: &'a str,
    pub clip_l: &'a str,
    pub vae: &'a str,
}

/// FLUX.2 [klein]'s three files: the GGUF diffusion model, its Qwen3 text
/// encoder (bare names as ComfyUI sees them in `diffusion_models` /
/// `text_encoders`), and its VAE (`vae`). Unlike FLUX.1, Klein uses a single
/// text encoder, not a T5 + CLIP-L pair.
#[derive(Debug, Clone, Copy)]
pub struct Flux2KleinModels<'a> {
    pub unet: &'a str,
    pub clip: &'a str,
    pub vae: &'a str,
}

/// Krea 2's three files (bare names as ComfyUI sees them in
/// `diffusion_models` / `text_encoders` / `vae`): the diffusion model — the
/// Turbo base or a Civitai fine-tune, which ship without an encoder or VAE —
/// its Qwen3-VL 4B text encoder, and the Qwen-Image VAE.
#[derive(Debug, Clone, Copy)]
pub struct Krea2Models<'a> {
    pub unet: &'a str,
    pub clip: &'a str,
    pub vae: &'a str,
}

/// One LoRA to splice into a graph before sampling, as a `LoraLoader` node.
/// Multiple entries chain in order. A single `strength` drives both the model
/// and CLIP patch — ComfyUI's default UI splits these into two knobs, but one
/// covers the overwhelming majority of real usage and keeps the picker simple.
#[derive(Debug, Clone, Copy)]
pub struct LoraSpec<'a> {
    pub file: &'a str,
    pub strength: f64,
}

/// Which template a model needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    /// One `.safetensors` file carries model + CLIP + VAE.
    Checkpoint,
    /// FLUX.1: a GGUF diffusion model plus separate T5 + CLIP-L encoders + VAE.
    FluxGguf,
    /// FLUX.2 \[klein\]: a GGUF diffusion model plus a single Qwen3 text
    /// encoder and VAE, sampled via `CFGGuider`/`SamplerCustomAdvanced`
    /// rather than a plain `KSampler`. A different enough graph shape from
    /// FLUX.1 to need its own recipe (see [`flux2_klein_txt2img`]).
    Flux2KleinGguf,
    /// FLUX.2 \[klein\] from a plain `.safetensors` checkpoint (no
    /// `ComfyUI-GGUF` custom node needed): the same single Qwen3 encoder and
    /// VAE, but sampled with a plain `KSampler` at CFG 1 plus a `FluxGuidance`
    /// node -- FLUX.1's graph shape, not the GGUF recipe's `CFGGuider` chain.
    /// Verified against a real exported ComfyUI workflow for this checkpoint
    /// (see [`flux2_klein_txt2img_safetensors`]).
    Flux2KleinSafetensors,
    /// Krea 2: a `.safetensors` diffusion model (`UNETLoader`) plus its
    /// Qwen3-VL text encoder (`CLIPLoader` type `krea2`) and the Qwen-Image
    /// VAE, sampled with a plain `KSampler` — Comfy-Org's own Turbo
    /// template (see [`krea2_txt2img`]).
    Krea2,
}

impl Recipe {
    /// `family == "flux"` → [`Recipe::FluxGguf`]; `"flux2"` → the GGUF or
    /// safetensors Klein recipe depending on `file_name`'s extension (both
    /// share the same companion files, they just load the diffusion model
    /// differently); `"krea2"` → [`Recipe::Krea2`]; everything else is a
    /// single-file checkpoint.
    pub fn for_family(family: Option<&str>, file_name: &str) -> Self {
        match family {
            Some(f) if f.eq_ignore_ascii_case("flux") => Self::FluxGguf,
            Some(f) if f.eq_ignore_ascii_case("krea2") => Self::Krea2,
            Some(f) if f.eq_ignore_ascii_case("flux2") => {
                if file_name.to_ascii_lowercase().ends_with(".gguf") {
                    Self::Flux2KleinGguf
                } else {
                    Self::Flux2KleinSafetensors
                }
            }
            _ => Self::Checkpoint,
        }
    }
}

/// An instruction-based edit of an existing image ("remove the blisters",
/// "make the hair blonde"), not a fresh generation from a prompt.
/// `source_image` is the bare file name of an image already placed in
/// ComfyUI's `input/` folder.
#[derive(Debug, Clone, Copy)]
pub struct EditInputs<'a> {
    pub instruction: &'a str,
    pub source_image: &'a str,
    pub steps: u32,
    pub cfg: f64,
    pub sampler: &'a str,
    pub seed: i64,
    pub filename_prefix: &'a str,
}

// --- character consistency (Story Studio Phase 2) ------------------------------

/// The two files SDXL-family IP-Adapter conditioning needs (`ComfyUI_IPAdapter_plus`'s
/// `IPAdapterModelLoader` + core ComfyUI's `CLIPVisionLoader`), plus the reference
/// portrait already staged in ComfyUI's `input/` folder and how strongly it should
/// steer the render.
#[derive(Debug, Clone, Copy)]
pub struct IpAdapterSpec<'a> {
    pub clip_vision: &'a str,
    pub ipadapter_model: &'a str,
    /// Bare file name of the reference image, already placed in ComfyUI's
    /// `input/` folder (same convention as [`EditInputs::source_image`]).
    pub reference_image: &'a str,
    /// `IPAdapterAdvanced`'s `weight`. The node's own default is `1.0`; the
    /// pack's README recommends lowering it (it suggests "at least 0.8") for
    /// better prompt adherence, so callers should default there, not to 1.0.
    pub weight: f64,
}

// --- video --------------------------------------------------------------------

/// A resolved text/image-to-video request. `start_image` is the bare file name
/// of a frame already placed in ComfyUI's `input/` folder (`None` → text→video).
#[derive(Debug, Clone, Copy)]
pub struct VideoInputs<'a> {
    pub positive: &'a str,
    pub negative: &'a str,
    pub width: u32,
    pub height: u32,
    /// Frames. Wan wants `(length - 1) % 4 == 0`; LTX-Video wants `% 8 == 0` and
    /// rounds down internally if it isn't.
    pub length: u32,
    pub fps: u32,
    pub steps: u32,
    pub cfg: f64,
    pub seed: i64,
    pub start_image: Option<&'a str>,
    pub filename_prefix: &'a str,
}

/// Wan 2.2's three files (bare names as ComfyUI sees them in `diffusion_models`
/// / `text_encoders` / `vae`).
#[derive(Debug, Clone, Copy)]
pub struct WanModels<'a> {
    pub unet: &'a str,
    /// The umt5 text encoder.
    pub clip: &'a str,
    pub vae: &'a str,
}

/// Which video template a model needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoRecipe {
    /// Wan 2.2 TI2V — three separate files ([`wan_ti2v`]).
    Wan,
    /// LTX-Video 0.9.x — a bundled checkpoint + a separate T5 ([`ltx_video`]).
    Ltx,
}

impl VideoRecipe {
    /// `family == "ltx"` → [`VideoRecipe::Ltx`]; everything else is Wan (the
    /// default video template).
    pub fn for_family(family: Option<&str>) -> Self {
        match family {
            Some(f) if f.eq_ignore_ascii_case("ltx") => Self::Ltx,
            _ => Self::Wan,
        }
    }
}

/// LTX-Video's two files: the 2B checkpoint (bundles the diffusion model **and**
/// the VAE) and a T5 text encoder loaded separately (`CLIPLoader type="ltxv"`).
#[derive(Debug, Clone, Copy)]
pub struct LtxModels<'a> {
    /// `ltx-video-2b-v0.9.x.safetensors` — model + VAE.
    pub checkpoint: &'a str,
    /// A `t5xxl` encoder (the fp8 one already in the catalogue works).
    pub t5: &'a str,
}

// --- upscale ------------------------------------------------------------------

/// How to resize when running NVIDIA's RTX Video Super Resolution node's
/// `resize_type` `DynamicCombo` input — matches its two branches verbatim,
/// key strings included (`UpscaleType` in `Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`'s
/// own source). The wire shape those branches serialize to lives in
/// [`fragments::upscale`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpscaleResize {
    /// Multiply both dimensions by this factor (the node clamps to 1.0-4.0).
    ScaleBy(f64),
    /// Resize to these exact pixel dimensions (the node clamps 64-8192, step 8).
    Target { width: u32, height: u32 },
}

/// Inputs for an image upscale via NVIDIA's RTX Video Super Resolution node.
#[derive(Debug, Clone, Copy)]
pub struct UpscaleImageInputs<'a> {
    pub source_image: &'a str,
    pub resize: UpscaleResize,
    /// `"LOW"` | `"MEDIUM"` | `"HIGH"` | `"ULTRA"` — the node's own default is
    /// `"ULTRA"`.
    pub quality: &'a str,
    pub filename_prefix: &'a str,
}

/// Inputs for a video upscale via NVIDIA's RTX Video Super Resolution node.
#[derive(Debug, Clone, Copy)]
pub struct UpscaleVideoInputs<'a> {
    pub source_video: &'a str,
    pub resize: UpscaleResize,
    pub quality: &'a str,
    pub filename_prefix: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_selects_flux_for_the_family() {
        assert_eq!(
            Recipe::for_family(Some("flux"), "flux1-dev-Q8_0.gguf"),
            Recipe::FluxGguf
        );
        assert_eq!(
            Recipe::for_family(Some("FLUX"), "flux1-dev.safetensors"),
            Recipe::FluxGguf
        );
        assert_eq!(
            Recipe::for_family(Some("sdxl"), "sd_xl_base_1.0.safetensors"),
            Recipe::Checkpoint
        );
        // What an import of the SD 1.5 catalog checkpoint records.
        assert_eq!(
            Recipe::for_family(Some("sd15"), "v1-5-pruned-emaonly.safetensors"),
            Recipe::Checkpoint
        );
        assert_eq!(
            Recipe::for_family(None, "x.safetensors"),
            Recipe::Checkpoint
        );
    }

    #[test]
    fn recipe_picks_the_flux2_klein_variant_by_file_extension() {
        assert_eq!(
            Recipe::for_family(Some("flux2"), "flux-2-klein-9b-Q4_K_M.gguf"),
            Recipe::Flux2KleinGguf
        );
        assert_eq!(
            Recipe::for_family(Some("FLUX2"), "flux-2-klein-9b-Q4_K_M.GGUF"),
            Recipe::Flux2KleinGguf
        );
        assert_eq!(
            Recipe::for_family(Some("flux2"), "flux-2-klein-9b-fp8mixed.safetensors"),
            Recipe::Flux2KleinSafetensors
        );
    }

    #[test]
    fn recipe_selects_krea2_for_its_family() {
        assert_eq!(
            Recipe::for_family(Some("krea2"), "krea2_turbo_fp8_scaled.safetensors"),
            Recipe::Krea2
        );
        assert_eq!(
            Recipe::for_family(Some("KREA2"), "x.safetensors"),
            Recipe::Krea2
        );
        // FLUX.1 Krea [dev] is FLUX.1, not Krea 2.
        assert_eq!(
            Recipe::for_family(Some("flux"), "flux1-krea-dev-Q8_0.gguf"),
            Recipe::FluxGguf
        );
    }

    #[test]
    fn video_recipe_selects_ltx_for_the_family() {
        assert_eq!(VideoRecipe::for_family(Some("ltx")), VideoRecipe::Ltx);
        assert_eq!(VideoRecipe::for_family(Some("LTX")), VideoRecipe::Ltx);
        assert_eq!(VideoRecipe::for_family(Some("wan")), VideoRecipe::Wan);
        assert_eq!(VideoRecipe::for_family(None), VideoRecipe::Wan);
    }
}
