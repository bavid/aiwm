//! Sampler fragments: the plain `KSampler` chain, FLUX.2 Klein's
//! `CFGGuider`/`SamplerCustomAdvanced` chain (`custom_advanced`) and
//! LTX-Video's `LTXVScheduler` + `SamplerCustom` pair — exact node shapes from
//! `pipeline::{checkpoint_txt2img, flux2_klein_txt2img, ltx_video}`.

use serde_json::json;

use crate::pipeline::graph::{Dim, Graph, OwnedLink};

/// Parameters a plain `KSampler` node needs.
#[derive(Debug, Clone, Copy)]
pub struct SamplerParams<'a> {
    pub seed: i64,
    pub steps: u32,
    pub cfg: f64,
    pub sampler: &'a str,
    pub scheduler: &'a str,
    pub denoise: f64,
}

/// `KSampler` — wires model/positive/negative/latent plus the sampling
/// knobs. Exact keys from `checkpoint_txt2img`.
pub fn ksampler(
    g: &mut Graph,
    id: &str,
    model: &OwnedLink,
    positive: &OwnedLink,
    negative: &OwnedLink,
    latent: &OwnedLink,
    p: &SamplerParams,
) -> OwnedLink {
    g.node(
        id,
        "KSampler",
        json!({
            "seed": p.seed,
            "steps": p.steps,
            "cfg": p.cfg,
            "sampler_name": p.sampler,
            "scheduler": p.scheduler,
            "denoise": p.denoise,
            "model": model.json(),
            "positive": positive.json(),
            "negative": negative.json(),
            "latent_image": latent.json()
        }),
    );
    OwnedLink::new(id, 0)
}

/// `KSamplerSelect` — picks the sampling algorithm by name, as a SAMPLER
/// object a `SamplerCustom*` node consumes. Exact keys from
/// `flux2_klein_txt2img` (and `ltx_video`, which pins it to `euler`).
pub fn ksampler_select(g: &mut Graph, id: &str, sampler: &str) -> OwnedLink {
    g.node(id, "KSamplerSelect", json!({ "sampler_name": sampler }));
    OwnedLink::new(id, 0)
}

/// Explicit node ids for the five nodes in FLUX.2 \[klein\]'s
/// `CFGGuider`/`SamplerCustomAdvanced` chain.
#[derive(Debug, Clone, Copy)]
pub struct CustomAdvancedIds<'a> {
    pub select: &'a str,
    pub scheduler: &'a str,
    pub noise: &'a str,
    pub guider: &'a str,
    pub sampler: &'a str,
}

/// Parameters the chain needs. `width`/`height` are [`Dim`]s because
/// `flux2_klein_edit` sizes its `Flux2Scheduler` from a `GetImageSize` node
/// rather than from fixed inputs. `sigmas_override` lets a caller (e.g. a
/// future Hi-Res-Fix second pass) feed a different sigmas source than this
/// chain's own `Flux2Scheduler`; `None` reproduces today's graph exactly.
#[derive(Debug, Clone)]
pub struct CustomAdvancedParams<'a> {
    pub seed: i64,
    pub steps: u32,
    pub width: Dim,
    pub height: Dim,
    pub sampler: &'a str,
    pub cfg: f64,
    pub sigmas_override: Option<OwnedLink>,
}

/// The three links a `SamplerCustomAdvanced` consumes besides its sigmas and
/// latent. [`custom_advanced`] hands them back so a second pass can reuse the
/// same noise source, guider and sampler instead of rebuilding them — which
/// is also what makes a LoRA chain reach the second pass for free, since
/// [`super::loras::apply`] repoints the *guider's* `model` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAdvancedLinks {
    pub noise: OwnedLink,
    pub guider: OwnedLink,
    pub sampler: OwnedLink,
}

/// What [`custom_advanced`] built: the sampled latent plus the chain links
/// behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAdvancedChain {
    /// `SamplerCustomAdvanced`'s output slot 0.
    pub sampled: OwnedLink,
    pub links: CustomAdvancedLinks,
}

/// `KSamplerSelect` + `Flux2Scheduler` + `RandomNoise` + `CFGGuider` +
/// `SamplerCustomAdvanced` — FLUX.2 \[klein\]'s sampling chain. Exact keys
/// from `flux2_klein_txt2img`; `sigmas` is the override when given, else the
/// chain's own scheduler node's output slot 0.
pub fn custom_advanced(
    g: &mut Graph,
    ids: &CustomAdvancedIds,
    model: &OwnedLink,
    positive: &OwnedLink,
    negative: &OwnedLink,
    latent: &OwnedLink,
    p: &CustomAdvancedParams,
) -> CustomAdvancedChain {
    let sampler = ksampler_select(g, ids.select, p.sampler);
    g.node(
        ids.scheduler,
        "Flux2Scheduler",
        json!({ "steps": p.steps, "width": p.width.json(), "height": p.height.json() }),
    );
    g.node(ids.noise, "RandomNoise", json!({ "noise_seed": p.seed }));
    g.node(
        ids.guider,
        "CFGGuider",
        json!({
            "model": model.json(),
            "positive": positive.json(),
            "negative": negative.json(),
            "cfg": p.cfg
        }),
    );
    let sigmas = match &p.sigmas_override {
        Some(link) => link.json(),
        None => OwnedLink::new(ids.scheduler, 0).json(),
    };
    let links = CustomAdvancedLinks {
        noise: OwnedLink::new(ids.noise, 0),
        guider: OwnedLink::new(ids.guider, 0),
        sampler,
    };
    g.node(
        ids.sampler,
        "SamplerCustomAdvanced",
        json!({
            "noise": links.noise.json(),
            "guider": links.guider.json(),
            "sampler": links.sampler.json(),
            "sigmas": sigmas,
            "latent_image": latent.json()
        }),
    );
    CustomAdvancedChain {
        sampled: OwnedLink::new(ids.sampler, 0),
        links,
    }
}

/// `SamplerCustom` always adds noise in the recipes here — it samples from
/// scratch, never continuing a partially denoised latent.
const ADD_NOISE: bool = true;

/// `LTXVScheduler`'s shift/stretch knobs. Defaults live with the recipe that
/// uses them; the fragment only carries the node's wire shape.
#[derive(Debug, Clone, Copy)]
pub struct LtxvSchedulerParams {
    pub steps: u32,
    pub max_shift: f64,
    pub base_shift: f64,
    pub stretch: bool,
    pub terminal: f64,
}

/// `LTXVScheduler` — LTX-Video's sigma schedule, sized from the latent it will
/// denoise. Exact keys from `ltx_video`.
pub fn ltxv_scheduler(
    g: &mut Graph,
    id: &str,
    p: &LtxvSchedulerParams,
    latent: &OwnedLink,
) -> OwnedLink {
    g.node(
        id,
        "LTXVScheduler",
        json!({
            "steps": p.steps,
            "max_shift": p.max_shift,
            "base_shift": p.base_shift,
            "stretch": p.stretch,
            "terminal": p.terminal,
            "latent": latent.json()
        }),
    );
    OwnedLink::new(id, 0)
}

/// Every link a `SamplerCustom` node consumes.
#[derive(Debug, Clone, Copy)]
pub struct CustomLinks<'a> {
    pub model: &'a OwnedLink,
    pub positive: &'a OwnedLink,
    pub negative: &'a OwnedLink,
    pub sampler: &'a OwnedLink,
    pub sigmas: &'a OwnedLink,
    pub latent: &'a OwnedLink,
}

/// `SamplerCustom` — the sigma-driven sampler LTX-Video uses instead of a
/// plain `KSampler`. Exact keys from `ltx_video`.
pub fn sampler_custom(
    g: &mut Graph,
    id: &str,
    links: &CustomLinks,
    seed: i64,
    cfg: f64,
) -> OwnedLink {
    g.node(
        id,
        "SamplerCustom",
        json!({
            "add_noise": ADD_NOISE,
            "noise_seed": seed,
            "cfg": cfg,
            "model": links.model.json(),
            "positive": links.positive.json(),
            "negative": links.negative.json(),
            "sampler": links.sampler.json(),
            "sigmas": links.sigmas.json(),
            "latent_image": links.latent.json()
        }),
    );
    OwnedLink::new(id, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn ksampler_select_names_the_algorithm() {
        let mut g = Graph::default();
        let out = ksampler_select(&mut g, "73", "euler");
        assert_eq!(out, OwnedLink::new("73", 0));
        assert_eq!(
            g.into_value()["73"],
            json!({ "class_type": "KSamplerSelect", "inputs": { "sampler_name": "euler" } })
        );
    }

    #[test]
    fn ltxv_scheduler_sizes_its_sigmas_from_the_latent() {
        let mut g = Graph::default();
        let latent = OwnedLink::new("70", 0);
        let out = ltxv_scheduler(
            &mut g,
            "71",
            &LtxvSchedulerParams {
                steps: 30,
                max_shift: 2.05,
                base_shift: 0.95,
                stretch: true,
                terminal: 0.1,
            },
            &latent,
        );
        assert_eq!(out, OwnedLink::new("71", 0));
        assert_eq!(
            g.into_value()["71"],
            json!({
                "class_type": "LTXVScheduler",
                "inputs": {
                    "steps": 30,
                    "max_shift": 2.05,
                    "base_shift": 0.95,
                    "stretch": true,
                    "terminal": 0.1,
                    "latent": ["70", 0]
                }
            })
        );
    }

    #[test]
    fn sampler_custom_wires_every_link_plus_seed_and_cfg() {
        let mut g = Graph::default();
        let (model, positive, negative) = (
            OwnedLink::new("44", 0),
            OwnedLink::new("69", 0),
            OwnedLink::new("69", 1),
        );
        let (sampler, sigmas, latent) = (
            OwnedLink::new("73", 0),
            OwnedLink::new("71", 0),
            OwnedLink::new("70", 0),
        );
        let out = sampler_custom(
            &mut g,
            "72",
            &CustomLinks {
                model: &model,
                positive: &positive,
                negative: &negative,
                sampler: &sampler,
                sigmas: &sigmas,
                latent: &latent,
            },
            7,
            5.0,
        );
        assert_eq!(out, OwnedLink::new("72", 0));
        assert_eq!(
            g.into_value()["72"],
            json!({
                "class_type": "SamplerCustom",
                "inputs": {
                    "add_noise": true,
                    "noise_seed": 7,
                    "cfg": 5.0,
                    "model": ["44", 0],
                    "positive": ["69", 0],
                    "negative": ["69", 1],
                    "sampler": ["73", 0],
                    "sigmas": ["71", 0],
                    "latent_image": ["70", 0]
                }
            })
        );
    }

    #[test]
    fn ksampler_wires_model_cond_latent_and_params() {
        let mut g = Graph::default();
        let model = OwnedLink::new("4", 0);
        let positive = OwnedLink::new("6", 0);
        let negative = OwnedLink::new("7", 0);
        let latent = OwnedLink::new("5", 0);
        let out = ksampler(
            &mut g,
            "3",
            &model,
            &positive,
            &negative,
            &latent,
            &SamplerParams {
                seed: 42,
                steps: 25,
                cfg: 7.0,
                sampler: "euler",
                scheduler: "normal",
                denoise: 1.0,
            },
        );
        assert_eq!(out, OwnedLink::new("3", 0));
        assert_eq!(
            g.into_value()["3"],
            json!({
                "class_type": "KSampler",
                "inputs": {
                    "seed": 42,
                    "steps": 25,
                    "cfg": 7.0,
                    "sampler_name": "euler",
                    "scheduler": "normal",
                    "denoise": 1.0,
                    "model": ["4", 0],
                    "positive": ["6", 0],
                    "negative": ["7", 0],
                    "latent_image": ["5", 0]
                }
            })
        );
    }

    #[test]
    fn custom_advanced_chain_wires_select_scheduler_noise_guider_sampler() {
        let mut g = Graph::default();
        let model = OwnedLink::new("12", 0);
        let positive = OwnedLink::new("6", 0);
        let negative = OwnedLink::new("27", 0);
        let latent = OwnedLink::new("32", 0);
        let ids = CustomAdvancedIds {
            select: "28",
            scheduler: "29",
            noise: "30",
            guider: "31",
            sampler: "3",
        };
        let out = custom_advanced(
            &mut g,
            &ids,
            &model,
            &positive,
            &negative,
            &latent,
            &CustomAdvancedParams {
                seed: 42,
                steps: 25,
                width: Dim::Fixed(1024),
                height: Dim::Fixed(1024),
                sampler: "euler",
                cfg: 7.0,
                sigmas_override: None,
            },
        );
        assert_eq!(out.sampled, OwnedLink::new("3", 0));
        // The chain hands its own links back so a Hi-Res-Fix second pass can
        // reuse them rather than build a second noise/guider/sampler trio.
        assert_eq!(
            out.links,
            CustomAdvancedLinks {
                noise: OwnedLink::new("30", 0),
                guider: OwnedLink::new("31", 0),
                sampler: OwnedLink::new("28", 0),
            }
        );

        let v = g.into_value();
        assert_eq!(
            v["28"],
            json!({ "class_type": "KSamplerSelect", "inputs": { "sampler_name": "euler" } })
        );
        assert_eq!(
            v["29"],
            json!({
                "class_type": "Flux2Scheduler",
                "inputs": { "steps": 25, "width": 1024, "height": 1024 }
            })
        );
        assert_eq!(
            v["30"],
            json!({ "class_type": "RandomNoise", "inputs": { "noise_seed": 42 } })
        );
        assert_eq!(
            v["31"],
            json!({
                "class_type": "CFGGuider",
                "inputs": {
                    "model": ["12", 0],
                    "positive": ["6", 0],
                    "negative": ["27", 0],
                    "cfg": 7.0
                }
            })
        );
        assert_eq!(
            v["3"],
            json!({
                "class_type": "SamplerCustomAdvanced",
                "inputs": {
                    "noise": ["30", 0],
                    "guider": ["31", 0],
                    "sampler": ["28", 0],
                    "sigmas": ["29", 0],
                    "latent_image": ["32", 0]
                }
            })
        );
    }

    #[test]
    fn custom_advanced_uses_the_sigmas_override_when_given() {
        let mut g = Graph::default();
        let model = OwnedLink::new("12", 0);
        let positive = OwnedLink::new("6", 0);
        let negative = OwnedLink::new("27", 0);
        let latent = OwnedLink::new("32", 0);
        let ids = CustomAdvancedIds {
            select: "28",
            scheduler: "29",
            noise: "30",
            guider: "31",
            sampler: "3",
        };
        let override_link = OwnedLink::new("60", 0);
        custom_advanced(
            &mut g,
            &ids,
            &model,
            &positive,
            &negative,
            &latent,
            &CustomAdvancedParams {
                seed: 42,
                steps: 25,
                width: Dim::Fixed(1024),
                height: Dim::Fixed(1024),
                sampler: "euler",
                cfg: 7.0,
                sigmas_override: Some(override_link),
            },
        );
        assert_eq!(g.input("3", "sigmas"), Some(&json!(["60", 0])));
    }

    #[test]
    fn custom_advanced_scheduler_takes_link_dimensions() {
        let mut g = Graph::default();
        let ids = CustomAdvancedIds {
            select: "61",
            scheduler: "62",
            noise: "73",
            guider: "63",
            sampler: "64",
        };
        custom_advanced(
            &mut g,
            &ids,
            &OwnedLink::new("70", 0),
            &OwnedLink::new("123", 0),
            &OwnedLink::new("125", 0),
            &OwnedLink::new("66", 0),
            &CustomAdvancedParams {
                seed: 42,
                steps: 8,
                width: Dim::Link(OwnedLink::new("99", 0)),
                height: Dim::Link(OwnedLink::new("99", 1)),
                sampler: "euler",
                cfg: 1.5,
                sigmas_override: None,
            },
        );
        assert_eq!(g.input("62", "width"), Some(&json!(["99", 0])));
        assert_eq!(g.input("62", "height"), Some(&json!(["99", 1])));
    }
}
