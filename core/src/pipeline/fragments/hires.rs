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
use crate::pipeline::{latent_upscaled_px, HiresFix};

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

/// FLUX.2's token grid is 16 pixels to a tile (`Flux2Scheduler` derives its
/// sequence length as `width * height / 256`, one token per 16×16 tile).
const FLUX2_TILE: u32 = 16;

/// The size `Flux2Scheduler` is built at for the second pass: the real
/// post-upscale size ([`latent_upscaled_px`] — the one formula both the graph
/// and `ImageRequest::final_size` use) snapped **up** to FLUX.2's 16-pixel
/// token grid, so the schedule is never computed for fewer tokens than the
/// upscaled latent actually has.
fn scheduler_px(dim: u32, scale: f64) -> u32 {
    latent_upscaled_px(dim, scale).div_ceil(FLUX2_TILE) * FLUX2_TILE
}

/// The `Flux2Scheduler.steps` that leaves `hires.steps` steps *executed* after
/// `SplitSigmasDenoise` cuts the schedule.
///
/// `hires.steps` means executed steps in every family (that's what
/// `KSampler.steps` is, even at a denoise below 1). Here the sampler runs the
/// low-sigma tail `SplitSigmasDenoise` keeps — the last `round(steps *
/// denoise)` of them — so the schedule has to be built `steps / denoise` long
/// for the tail to come out at `hires.steps`.
fn schedule_steps(hires: &HiresFix) -> u32 {
    // The request parser clamps `denoise` to ≥ 0.2; this guard is the
    // fragment's own, so a hand-built `HiresFix` cannot divide by zero.
    if hires.denoise <= 0.0 {
        return hires.steps.max(1);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // `steps` is clamped to ≤ 60 and `denoise` to ≥ 0.2, so the quotient is
    // a few hundred at worst and never negative.
    let total = (f64::from(hires.steps) / hires.denoise).round() as u32;
    // A zero-length schedule is not a graph ComfyUI can run.
    total.max(1)
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
/// at [`scheduler_px`], because its sequence length is derived from the size
/// it is handed and the second pass runs on a bigger latent.
///
/// Note what `steps` becomes here. `hires.steps` is *executed* steps —
/// whatever `KSampler.steps` means for the other family — but
/// `Flux2Scheduler.steps` is the length of the *whole* schedule, of which
/// `SplitSigmasDenoise` keeps only the `denoise` tail. So this pass asks the
/// scheduler for the longer total ([`schedule_steps`]); handing it
/// `hires.steps` directly would silently run `steps × denoise` steps (5 of 12
/// at denoise 0.45) and make the knob mean two different things.
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
            "steps": schedule_steps(hires),
            "width": scheduler_px(width, hires.scale_by),
            "height": scheduler_px(height, hires.scale_by)
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

    /// The scheduler size starts from the *same* latent-unit formula the
    /// decoded image uses ([`latent_upscaled_px`]) and only then snaps up to
    /// FLUX.2's 16-pixel token grid. The old fragment-local formula scaled in
    /// pixels instead and came out 16 px larger at 1000 × 1.25 (1264 vs 1248).
    #[test]
    fn scheduler_px_starts_from_the_shared_latent_formula() {
        // The case the two formulas used to disagree on: 1000 px = 125 latent
        // units, 125 × 1.25 = 156.25 → 156 → 1248 px, already a multiple of
        // 16, so the scheduler sees exactly the decoded size.
        assert_eq!(latent_upscaled_px(1000, 1.25), 1248);
        assert_eq!(scheduler_px(1000, 1.25), 1248);
        // Already a multiple: unchanged.
        assert_eq!(scheduler_px(1024, 1.5), 1536);
        assert_eq!(scheduler_px(1024, 1.25), 1280);
        // Rounding up to 16 kicks in: 125 × 1.35 = 168.75 → 169 → 1352 px,
        // which is not a multiple of 16, so the schedule is computed for the
        // next whole token row (1360) rather than for fewer tokens than the
        // latent really has.
        assert_eq!(latent_upscaled_px(1000, 1.35), 1352);
        assert_eq!(scheduler_px(1000, 1.35), 1360);
        // A 1.0 scale still snaps, so the size is always legal.
        assert_eq!(scheduler_px(1000, 1.0), 1008);
        assert_eq!(scheduler_px(1024, 1.0), 1024);
    }

    /// `hires.steps` is EXECUTED steps in both families, so the klein-GGUF
    /// schedule has to be the longer total the denoise cut is taken from.
    #[test]
    fn schedule_steps_is_the_total_a_denoise_cut_leaves_hires_steps_of() {
        // 12 executed at denoise 0.45 needs a 27-step schedule
        // (12 / 0.45 = 26.67 → 27; 27 × 0.45 = 12.15 → 12 kept).
        assert_eq!(schedule_steps(&hires()), 27);
        // denoise 1.0 asks for the whole schedule.
        assert_eq!(
            schedule_steps(&HiresFix {
                denoise: 1.0,
                ..hires()
            }),
            12
        );
        // The fragment must not divide by zero even though the parser clamps
        // denoise to ≥ 0.2.
        assert_eq!(
            schedule_steps(&HiresFix {
                denoise: 0.0,
                ..hires()
            }),
            12
        );
        // Never a zero-length schedule.
        assert_eq!(
            schedule_steps(&HiresFix {
                denoise: 0.7,
                steps: 0,
                ..hires()
            }),
            1
        );
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
                // 12 EXECUTED steps at denoise 0.45 = a 27-step schedule.
                "inputs": { "steps": 27, "width": 1536, "height": 1536 }
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
