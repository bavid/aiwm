//! Hi-Res-Fix fragment: the second pass a txt2img recipe inserts between its
//! first sampler and the decode.
//!
//! The shape is always the same — upscale the first pass's *latent*, then
//! re-sample it at a low denoise — but the sampler half differs by family:
//!
//! - [`ksampler_pass`] for everything that samples with a plain `KSampler`
//!   (checkpoint/SDXL, FLUX.1, FLUX.2 \[klein\] `.safetensors`): the second
//!   `KSampler` simply takes `denoise` below 1.
//! - [`custom_advanced_pass`] for FLUX.2 \[klein\] GGUF, which samples through
//!   `SamplerCustomAdvanced`. That node has no `denoise` knob, and
//!   `Flux2Scheduler` has no `denoise` input either (verified against ComfyUI
//!   v0.34.0, `comfy_extras/nodes_flux.py:213-232`), so the second pass
//!   shortens the *schedule* instead: a fresh `Flux2Scheduler` at the upscaled
//!   size, cut by `SplitSigmasDenoise`, whose output slot 1 is the low-sigma
//!   tail (`comfy_extras/nodes_custom_sampler.py:228-249`).
//!
//! Both reuse the first pass's model/conditioning links, which is why a LoRA
//! chain applies to both passes without the fragment knowing LoRAs exist.

use serde_json::json;

use crate::pipeline::fragments::latent;
use crate::pipeline::fragments::sampling::{self, CustomAdvancedLinks, SamplerParams};
use crate::pipeline::graph::{Graph, OwnedLink};
use crate::pipeline::HiresFix;

/// The first node id this fragment uses. It owns `HIRES_ID_BASE ..=
/// HIRES_ID_BASE + 4`; see the node-id map in
/// [`crate::pipeline::recipes::ids`] for why that window is free in every
/// recipe that can take a `hires` input.
pub const HIRES_ID_BASE: u32 = 40;

/// `LatentUpscaleBy` — where both second-pass shapes start.
const UPSCALE: u32 = 0;
/// The KSampler-family second pass.
const KSAMPLER: u32 = 1;
/// klein-GGUF only: the second `Flux2Scheduler`, …
const SCHEDULER: u32 = 2;
/// … the `SplitSigmasDenoise` that shortens its schedule, …
const SPLIT_SIGMAS: u32 = 3;
/// … and the second `SamplerCustomAdvanced`.
const SAMPLER: u32 = 4;

/// ComfyUI node ids are strings, so the offsets above become ids here — one
/// place that turns [`HIRES_ID_BASE`] into the window this fragment writes to.
fn id(offset: u32) -> String {
    (HIRES_ID_BASE + offset).to_string()
}

/// `SplitSigmasDenoise`'s output slot 1 — the *low* sigmas, i.e. the short
/// late part of the schedule a low-denoise pass runs. Slot 0 is the high
/// sigmas, which would re-compose the image from scratch.
const LOW_SIGMAS_SLOT: u32 = 1;

/// FLUX.2's latent grid is 16 pixels to a tile (`Flux2Scheduler` derives its
/// sequence length as `width * height / 256`, one token per 16×16 tile), so a
/// scaled size that is not a multiple of 16 does not describe a real latent.
/// Round the scaled dimension **up** — `(scaled + 15) / 16 * 16` — so the
/// schedule is never computed for fewer tokens than the upscaled latent
/// actually has.
fn round16(dim: u32, scale: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Dimensions are image sizes (≤ 8192) times a scale of at most a few, so
    // the product is far inside u32 and never negative.
    let scaled = (f64::from(dim) * scale).round() as u32;
    scaled.div_ceil(16) * 16
}

/// The KSampler-family second pass: `LatentUpscaleBy` → a second `KSampler`
/// that keeps the first pass's seed, cfg, sampler and scheduler and changes
/// only `steps` and `denoise`. Returns the second pass's latent, which the
/// recipe decodes instead of the first pass's.
///
/// Same seed on purpose: the second pass continues the image the first pass
/// composed rather than rolling a different one.
pub fn ksampler_pass(
    g: &mut Graph,
    first_pass: &OwnedLink,
    model: &OwnedLink,
    positive: &OwnedLink,
    negative: &OwnedLink,
    first: &SamplerParams,
    hires: &HiresFix,
) -> OwnedLink {
    let upscaled = latent::upscale_by(
        g,
        &id(UPSCALE),
        first_pass,
        hires.upscale_method,
        hires.scale_by,
    );
    sampling::ksampler(
        g,
        &id(KSAMPLER),
        model,
        positive,
        negative,
        &upscaled,
        &SamplerParams {
            steps: hires.steps,
            denoise: hires.denoise,
            ..*first
        },
    )
}

/// FLUX.2 \[klein\] GGUF's second pass: `LatentUpscaleBy` → a fresh
/// `Flux2Scheduler` at the upscaled size → `SplitSigmasDenoise` → a second
/// `SamplerCustomAdvanced` reusing `chain`'s noise, guider and sampler.
/// Returns the second pass's latent.
///
/// `width`/`height` are the *first* pass's pixel size; the scheduler is built
/// at `round16(dim, scale_by)`, because its sequence length is derived from
/// the size it is handed and the second pass runs on a bigger latent.
pub fn custom_advanced_pass(
    g: &mut Graph,
    first_pass: &OwnedLink,
    chain: &CustomAdvancedLinks,
    width: u32,
    height: u32,
    hires: &HiresFix,
) -> OwnedLink {
    let upscaled = latent::upscale_by(
        g,
        &id(UPSCALE),
        first_pass,
        hires.upscale_method,
        hires.scale_by,
    );
    g.node(
        &id(SCHEDULER),
        "Flux2Scheduler",
        json!({
            "steps": hires.steps,
            "width": round16(width, hires.scale_by),
            "height": round16(height, hires.scale_by)
        }),
    );
    g.node(
        &id(SPLIT_SIGMAS),
        "SplitSigmasDenoise",
        json!({
            "sigmas": OwnedLink::new(&id(SCHEDULER), 0).json(),
            "denoise": hires.denoise
        }),
    );
    g.node(
        &id(SAMPLER),
        "SamplerCustomAdvanced",
        json!({
            "noise": chain.noise.json(),
            "guider": chain.guider.json(),
            "sampler": chain.sampler.json(),
            "sigmas": OwnedLink::new(&id(SPLIT_SIGMAS), LOW_SIGMAS_SLOT).json(),
            "latent_image": upscaled.json()
        }),
    );
    OwnedLink::new(&id(SAMPLER), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::fragments::sampling::{CustomAdvancedLinks, SamplerParams};
    use crate::pipeline::graph::{Graph, OwnedLink};
    use crate::pipeline::HiresFix;
    use serde_json::json;

    fn hires() -> HiresFix {
        HiresFix {
            scale_by: 1.5,
            denoise: 0.45,
            steps: 12,
            upscale_method: HiresFix::DEFAULT_METHOD,
        }
    }

    /// Both passes together must stay inside the window the node-id map
    /// reserves for this fragment — nothing may leak past `HIRES_ID_BASE + 4`.
    #[test]
    fn both_passes_write_only_inside_the_reserved_id_window() {
        let window: Vec<String> = (HIRES_ID_BASE..=HIRES_ID_BASE + 4)
            .map(|n| n.to_string())
            .collect();

        let mut g = Graph::default();
        ksampler_pass(
            &mut g,
            &OwnedLink::new("3", 0),
            &OwnedLink::new("4", 0),
            &OwnedLink::new("6", 0),
            &OwnedLink::new("7", 0),
            &SamplerParams {
                seed: 42,
                steps: 25,
                cfg: 7.0,
                sampler: "euler",
                scheduler: "normal",
                denoise: 1.0,
            },
            &hires(),
        );
        let chain = CustomAdvancedLinks {
            noise: OwnedLink::new("30", 0),
            guider: OwnedLink::new("31", 0),
            sampler: OwnedLink::new("28", 0),
        };
        custom_advanced_pass(
            &mut g,
            &OwnedLink::new("3", 0),
            &chain,
            1024,
            1024,
            &hires(),
        );

        let v = g.into_value();
        let mut written: Vec<String> = v
            .as_object()
            .expect("a graph is an object of nodes")
            .keys()
            .cloned()
            .collect();
        written.sort();
        assert_eq!(written, window);
    }

    #[test]
    fn round16_rounds_up_to_a_multiple_of_16() {
        // Already a multiple: unchanged.
        assert_eq!(round16(1024, 1.5), 1536);
        assert_eq!(round16(1024, 1.25), 1280);
        // Not a multiple: rounded up, never down.
        assert_eq!(round16(1000, 1.5), 1504);
        assert_eq!(round16(1000, 1.25), 1264);
        // A 1.0 scale still snaps, so the size is always legal.
        assert_eq!(round16(1000, 1.0), 1008);
        assert_eq!(round16(1024, 1.0), 1024);
    }

    #[test]
    fn ksampler_pass_upscales_then_resamples_at_the_hires_denoise() {
        let mut g = Graph::default();
        let first_pass = OwnedLink::new("3", 0);
        let model = OwnedLink::new("4", 0);
        let positive = OwnedLink::new("6", 0);
        let negative = OwnedLink::new("7", 0);
        let first = SamplerParams {
            seed: 42,
            steps: 25,
            cfg: 7.0,
            sampler: "dpmpp_2m",
            scheduler: "karras",
            denoise: 1.0,
        };
        let out = ksampler_pass(
            &mut g,
            &first_pass,
            &model,
            &positive,
            &negative,
            &first,
            &hires(),
        );
        assert_eq!(out, OwnedLink::new("41", 0));

        let v = g.into_value();
        assert_eq!(
            v["40"],
            json!({
                "class_type": "LatentUpscaleBy",
                "inputs": {
                    "samples": ["3", 0],
                    "upscale_method": "nearest-exact",
                    "scale_by": 1.5
                }
            })
        );
        assert_eq!(
            v["41"],
            json!({
                "class_type": "KSampler",
                "inputs": {
                    // Seed, sampler, scheduler and cfg come from the first
                    // pass; only steps and denoise are the hires knobs.
                    "seed": 42,
                    "steps": 12,
                    "cfg": 7.0,
                    "sampler_name": "dpmpp_2m",
                    "scheduler": "karras",
                    "denoise": 0.45,
                    "model": ["4", 0],
                    "positive": ["6", 0],
                    "negative": ["7", 0],
                    "latent_image": ["40", 0]
                }
            })
        );
    }

    #[test]
    fn custom_advanced_pass_reuses_the_first_chains_noise_guider_and_sampler() {
        let mut g = Graph::default();
        let first_pass = OwnedLink::new("3", 0);
        let chain = CustomAdvancedLinks {
            noise: OwnedLink::new("30", 0),
            guider: OwnedLink::new("31", 0),
            sampler: OwnedLink::new("28", 0),
        };
        let out = custom_advanced_pass(&mut g, &first_pass, &chain, 1024, 1024, &hires());
        assert_eq!(out, OwnedLink::new("44", 0));

        let v = g.into_value();
        assert_eq!(v["40"]["inputs"]["samples"], json!(["3", 0]));
        assert_eq!(
            v["42"],
            json!({
                "class_type": "Flux2Scheduler",
                "inputs": { "steps": 12, "width": 1536, "height": 1536 }
            })
        );
        assert_eq!(
            v["43"],
            json!({
                "class_type": "SplitSigmasDenoise",
                "inputs": { "sigmas": ["42", 0], "denoise": 0.45 }
            })
        );
        assert_eq!(
            v["44"],
            json!({
                "class_type": "SamplerCustomAdvanced",
                "inputs": {
                    "noise": ["30", 0],
                    "guider": ["31", 0],
                    "sampler": ["28", 0],
                    // Slot 1 is the LOW-sigma tail: the short, late part of
                    // the schedule a low-denoise pass runs.
                    "sigmas": ["43", 1],
                    "latent_image": ["40", 0]
                }
            })
        );
    }
}
