//! Fixed image-generation pipelines: a workflow-JSON template plus parameter
//! substitution into known node slots.
//!
//! The MVP ships **fixed** pipelines only (PHASE_3_PLAN decision C): no graph
//! editor, no custom-workflow upload — that would be arbitrary node execution
//! again. Each function returns a ComfyUI *API-format* prompt graph
//! (`{ "<node id>": { "class_type", "inputs" } }`, links are
//! `["<node id>", <output slot>]`) ready for `POST /prompt`.
//!
//! Two templates: [`checkpoint_txt2img`] (SDXL and any single-file SD
//! checkpoint — core nodes only) and [`flux_txt2img`] (FLUX.1-dev as a GGUF
//! diffusion model + separate dual CLIP + VAE, via the pinned `ComfyUI-GGUF`
//! node). [`Recipe::for_family`] picks between them. A TOML pipeline *registry*
//! (user-editable definitions) stays out of the MVP.

use serde_json::{json, Value};

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

/// Which template a model needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    /// One `.safetensors` file carries model + CLIP + VAE.
    Checkpoint,
    /// FLUX: a GGUF diffusion model plus separate encoders + VAE.
    FluxGguf,
}

impl Recipe {
    /// FLUX models (`family == "flux"`) use [`Recipe::FluxGguf`]; everything
    /// else is a single-file checkpoint.
    pub fn for_family(family: Option<&str>) -> Self {
        match family {
            Some(f) if f.eq_ignore_ascii_case("flux") => Self::FluxGguf,
            _ => Self::Checkpoint,
        }
    }
}

/// The canonical ComfyUI default graph: load a single-file checkpoint, encode
/// both prompts, sample, VAE-decode, save. No custom nodes.
pub fn checkpoint_txt2img(i: &Txt2ImgInputs, checkpoint: &str) -> Value {
    json!({
        "4": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": checkpoint }
        },
        "5": {
            "class_type": "EmptyLatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["4", 1] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.negative, "clip": ["4", 1] }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": i.seed,
                "steps": i.steps,
                "cfg": i.cfg,
                "sampler_name": i.sampler,
                "scheduler": i.scheduler,
                "denoise": 1.0,
                "model": ["4", 0],
                "positive": ["6", 0],
                "negative": ["7", 0],
                "latent_image": ["5", 0]
            }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": i.filename_prefix, "images": ["8", 0] }
        }
    })
}

/// FLUX.1-dev via `ComfyUI-GGUF`: `UnetLoaderGGUF` + `DualCLIPLoaderGGUF`
/// (type `flux`, T5 + CLIP-L) + `VAELoader`, a `FluxGuidance` on the positive
/// conditioning, an SD3-format latent, then the sampler at CFG 1.
pub fn flux_txt2img(i: &Txt2ImgInputs, m: &FluxModels) -> Value {
    // FLUX is guidance-distilled: the sampler runs at CFG 1 and the effect the
    // user reaches for lives in FluxGuidance instead.
    let guidance = i.cfg.clamp(1.0, 10.0);
    json!({
        "12": {
            "class_type": "UnetLoaderGGUF",
            "inputs": { "unet_name": m.unet }
        },
        "11": {
            "class_type": "DualCLIPLoaderGGUF",
            "inputs": { "clip_name1": m.t5, "clip_name2": m.clip_l, "type": "flux" }
        },
        "10": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "5": {
            "class_type": "EmptySD3LatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["11", 0] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.negative, "clip": ["11", 0] }
        },
        "26": {
            "class_type": "FluxGuidance",
            "inputs": { "conditioning": ["6", 0], "guidance": guidance }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": i.seed,
                "steps": i.steps,
                "cfg": 1.0,
                "sampler_name": "euler",
                "scheduler": "simple",
                "denoise": 1.0,
                "model": ["12", 0],
                "positive": ["26", 0],
                "negative": ["7", 0],
                "latent_image": ["5", 0]
            }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["10", 0] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": i.filename_prefix, "images": ["8", 0] }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Txt2ImgInputs<'static> {
        Txt2ImgInputs {
            positive: "a red fox in the snow",
            negative: "blurry, low quality",
            width: 1024,
            height: 1024,
            steps: 25,
            cfg: 7.0,
            sampler: "euler",
            scheduler: "normal",
            seed: 42,
            filename_prefix: "job-abc",
        }
    }

    #[test]
    fn recipe_selects_flux_for_the_family() {
        assert_eq!(Recipe::for_family(Some("flux")), Recipe::FluxGguf);
        assert_eq!(Recipe::for_family(Some("FLUX")), Recipe::FluxGguf);
        assert_eq!(Recipe::for_family(Some("sdxl")), Recipe::Checkpoint);
        assert_eq!(Recipe::for_family(None), Recipe::Checkpoint);
    }

    #[test]
    fn checkpoint_graph_wires_every_node() {
        let g = checkpoint_txt2img(&inputs(), "sd_xl_base_1.0.safetensors");
        assert_eq!(g["4"]["inputs"]["ckpt_name"], "sd_xl_base_1.0.safetensors");
        assert_eq!(g["6"]["inputs"]["clip"], json!(["4", 1]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["3"]["inputs"]["positive"], json!(["6", 0]));
        assert_eq!(g["3"]["inputs"]["negative"], json!(["7", 0]));
        assert_eq!(g["8"]["inputs"]["vae"], json!(["4", 2]));
        assert_eq!(g["9"]["inputs"]["images"], json!(["8", 0]));
    }

    #[test]
    fn checkpoint_graph_substitutes_the_sampling_params() {
        let g = checkpoint_txt2img(&inputs(), "x.safetensors");
        assert_eq!(g["3"]["inputs"]["seed"], 42);
        assert_eq!(g["3"]["inputs"]["steps"], 25);
        assert_eq!(g["3"]["inputs"]["cfg"], 7.0);
        assert_eq!(g["5"]["inputs"]["width"], 1024);
        assert_eq!(g["6"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux_graph_loads_gguf_unet_dual_clip_and_vae() {
        let g = flux_txt2img(
            &inputs(),
            &FluxModels {
                unet: "flux1-dev-Q8_0.gguf",
                t5: "t5xxl_fp8_e4m3fn.safetensors",
                clip_l: "clip_l.safetensors",
                vae: "ae.safetensors",
            },
        );
        assert_eq!(g["12"]["class_type"], "UnetLoaderGGUF");
        assert_eq!(g["12"]["inputs"]["unet_name"], "flux1-dev-Q8_0.gguf");
        assert_eq!(g["11"]["class_type"], "DualCLIPLoaderGGUF");
        assert_eq!(
            g["11"]["inputs"]["clip_name1"],
            "t5xxl_fp8_e4m3fn.safetensors"
        );
        assert_eq!(g["11"]["inputs"]["clip_name2"], "clip_l.safetensors");
        assert_eq!(g["11"]["inputs"]["type"], "flux");
        assert_eq!(g["10"]["inputs"]["vae_name"], "ae.safetensors");
        assert_eq!(g["5"]["class_type"], "EmptySD3LatentImage");
    }

    #[test]
    fn flux_graph_pins_cfg_to_one_and_routes_guidance() {
        let mut i = inputs();
        i.cfg = 3.5;
        let g = flux_txt2img(
            &i,
            &FluxModels {
                unet: "u",
                t5: "t",
                clip_l: "c",
                vae: "v",
            },
        );
        assert_eq!(g["3"]["inputs"]["cfg"], 1.0, "flux sampler runs at CFG 1");
        assert_eq!(g["3"]["inputs"]["scheduler"], "simple");
        assert_eq!(g["26"]["inputs"]["guidance"], 3.5);
        assert_eq!(g["3"]["inputs"]["positive"], json!(["26", 0]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["8"]["inputs"]["vae"], json!(["10", 0]));
    }

    #[test]
    fn flux_guidance_is_clamped() {
        let mut i = inputs();
        i.cfg = 42.0;
        let g = flux_txt2img(
            &i,
            &FluxModels {
                unet: "u",
                t5: "t",
                clip_l: "c",
                vae: "v",
            },
        );
        assert_eq!(g["26"]["inputs"]["guidance"], 10.0);
    }
}
