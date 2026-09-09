//! Fixed image-generation pipelines: a workflow-JSON template plus parameter
//! substitution into known node slots.
//!
//! The MVP ships **fixed** pipelines only (PHASE_3_PLAN decision C): no graph
//! editor, no custom-workflow upload — that would be arbitrary node execution
//! again. Each function here returns a ComfyUI *API-format* prompt graph
//! (`{ "<node id>": { "class_type", "inputs" } }`, links are
//! `["<node id>", <output slot>]`) ready for `POST /prompt`.
//!
//! One template today (`sdxl_txt2img`). Flux lands in 3.6; a TOML pipeline
//! registry follows once there are two templates carrying real duplication.

use serde_json::{json, Value};

/// Everything a text-to-image workflow needs, already resolved (no `Auto`, no
/// negative seeds). Borrowed — the caller owns the strings.
#[derive(Debug, Clone, Copy)]
pub struct Txt2ImgInputs<'a> {
    /// Checkpoint file name as ComfyUI sees it in its `checkpoints` folder
    /// (the bare file name — `extra_model_paths.yaml` points at our store).
    pub checkpoint: &'a str,
    pub positive: &'a str,
    pub negative: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f64,
    pub sampler: &'a str,
    pub scheduler: &'a str,
    pub seed: i64,
    /// `SaveImage` prefix — we pass the job id so the output is easy to find.
    pub filename_prefix: &'a str,
}

/// SDXL (and any single-file SD checkpoint) text-to-image: the canonical
/// ComfyUI default graph — load checkpoint, encode both prompts, sample, VAE
/// decode, save. Needs no custom nodes.
pub fn sdxl_txt2img(i: &Txt2ImgInputs) -> Value {
    json!({
        "4": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": i.checkpoint }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Txt2ImgInputs<'static> {
        Txt2ImgInputs {
            checkpoint: "sd_xl_base_1.0.safetensors",
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
    fn sdxl_graph_wires_every_node() {
        let g = sdxl_txt2img(&inputs());

        // Checkpoint feeds model/clip/vae; sampler feeds the decode; decode feeds save.
        assert_eq!(g["4"]["inputs"]["ckpt_name"], "sd_xl_base_1.0.safetensors");
        assert_eq!(g["6"]["inputs"]["clip"], json!(["4", 1]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["3"]["inputs"]["positive"], json!(["6", 0]));
        assert_eq!(g["3"]["inputs"]["negative"], json!(["7", 0]));
        assert_eq!(g["8"]["inputs"]["samples"], json!(["3", 0]));
        assert_eq!(g["8"]["inputs"]["vae"], json!(["4", 2]));
        assert_eq!(g["9"]["inputs"]["images"], json!(["8", 0]));
    }

    #[test]
    fn sdxl_graph_substitutes_the_sampling_params() {
        let g = sdxl_txt2img(&inputs());
        assert_eq!(g["3"]["inputs"]["seed"], 42);
        assert_eq!(g["3"]["inputs"]["steps"], 25);
        assert_eq!(g["3"]["inputs"]["cfg"], 7.0);
        assert_eq!(g["3"]["inputs"]["sampler_name"], "euler");
        assert_eq!(g["5"]["inputs"]["width"], 1024);
        assert_eq!(g["6"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(g["7"]["inputs"]["text"], "blurry, low quality");
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }
}
