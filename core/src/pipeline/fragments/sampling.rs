//! Sampler fragments: the plain `KSampler` chain and FLUX.2 Klein's
//! `CFGGuider`/`SamplerCustomAdvanced` chain (`custom_advanced`), exact node
//! shapes from `pipeline::{checkpoint_txt2img, flux2_klein_txt2img}`.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

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

/// Parameters the chain needs. `sigmas_override` lets a caller (e.g. a
/// future Hi-Res-Fix second pass) feed a different sigmas source than this
/// chain's own `Flux2Scheduler`; `None` reproduces today's graph exactly.
#[derive(Debug, Clone)]
pub struct CustomAdvancedParams<'a> {
    pub seed: i64,
    pub steps: u32,
    pub width: u32,
    pub height: u32,
    pub sampler: &'a str,
    pub cfg: f64,
    pub sigmas_override: Option<OwnedLink>,
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
) -> OwnedLink {
    g.node(
        ids.select,
        "KSamplerSelect",
        json!({ "sampler_name": p.sampler }),
    );
    g.node(
        ids.scheduler,
        "Flux2Scheduler",
        json!({ "steps": p.steps, "width": p.width, "height": p.height }),
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
    g.node(
        ids.sampler,
        "SamplerCustomAdvanced",
        json!({
            "noise": OwnedLink::new(ids.noise, 0).json(),
            "guider": OwnedLink::new(ids.guider, 0).json(),
            "sampler": OwnedLink::new(ids.select, 0).json(),
            "sigmas": sigmas,
            "latent_image": latent.json()
        }),
    );
    OwnedLink::new(ids.sampler, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

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
                width: 1024,
                height: 1024,
                sampler: "euler",
                cfg: 7.0,
                sigmas_override: None,
            },
        );
        assert_eq!(out, OwnedLink::new("3", 0));

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
                width: 1024,
                height: 1024,
                sampler: "euler",
                cfg: 7.0,
                sigmas_override: Some(override_link),
            },
        );
        assert_eq!(g.input("3", "sigmas"), Some(&json!(["60", 0])));
    }
}
