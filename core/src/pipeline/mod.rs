//! Fixed generation pipelines: a workflow-JSON template plus parameter
//! substitution into known node slots.
//!
//! The MVP ships **fixed** pipelines only (PHASE_3_PLAN decision C): no graph
//! editor, no custom-workflow upload — that would be arbitrary node execution
//! again. Each function returns a ComfyUI *API-format* prompt graph
//! (`{ "<node id>": { "class_type", "inputs" } }`, links are
//! `["<node id>", <output slot>]`) ready for `POST /prompt`.
//!
//! Image: [`checkpoint_txt2img`] (SDXL and any single-file SD checkpoint — core
//! nodes) and [`flux_txt2img`] (FLUX.1-dev GGUF + dual CLIP + VAE, via
//! `ComfyUI-GGUF`); [`Recipe::for_family`] picks. Video: [`wan_ti2v`] (Wan 2.2
//! TI2V — three separate files) and [`ltx_video`] (LTX-Video 0.9.x — a bundled
//! checkpoint + a T5); [`VideoRecipe::for_family`] picks. Both do text→video and
//! image→video (a start frame in ComfyUI's `input/`). A TOML pipeline *registry*
//! stays out of the MVP.

use serde_json::{json, Value};

pub mod fragments;
pub mod graph;

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

/// One LoRA to splice into a graph before sampling, as a `LoraLoader` node.
/// Multiple entries chain in order. A single `strength` drives both the model
/// and CLIP patch — ComfyUI's default UI splits these into two knobs, but one
/// covers the overwhelming majority of real usage and keeps the picker simple.
#[derive(Debug, Clone, Copy)]
pub struct LoraSpec<'a> {
    pub file: &'a str,
    pub strength: f64,
}

/// Splice `loras` in as a chain of `LoraLoader` nodes between the current
/// model/clip source and their consumers — a no-op when `loras` is empty, so
/// every existing graph shape is unchanged. Node ids start at `90`, clear of
/// every fixed id the four builders use (highest is `78`).
fn apply_loras(
    g: &mut Value,
    loras: &[LoraSpec],
    model_source: (&str, u32),
    clip_source: (&str, u32),
    model_consumer: &str,
    clip_consumers: &[&str],
) {
    if loras.is_empty() {
        return;
    }
    let mut model_link = json!([model_source.0, model_source.1]);
    let mut clip_link = json!([clip_source.0, clip_source.1]);
    for (idx, lora) in loras.iter().enumerate() {
        let id = format!("{}", 90 + idx);
        g[id.as_str()] = json!({
            "class_type": "LoraLoader",
            "inputs": {
                "model": model_link,
                "clip": clip_link,
                "lora_name": lora.file,
                "strength_model": lora.strength,
                "strength_clip": lora.strength,
            }
        });
        model_link = json!([id, 0]);
        clip_link = json!([id, 1]);
    }
    g[model_consumer]["inputs"]["model"] = model_link;
    for consumer in clip_consumers {
        g[*consumer]["inputs"]["clip"] = clip_link.clone();
    }
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
}

impl Recipe {
    /// `family == "flux"` → [`Recipe::FluxGguf`]; `"flux2"` → the GGUF or
    /// safetensors Klein recipe depending on `file_name`'s extension (both
    /// share the same companion files, they just load the diffusion model
    /// differently); everything else is a single-file checkpoint.
    pub fn for_family(family: Option<&str>, file_name: &str) -> Self {
        match family {
            Some(f) if f.eq_ignore_ascii_case("flux") => Self::FluxGguf,
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

/// The canonical ComfyUI default graph: load a single-file checkpoint, encode
/// both prompts, sample, VAE-decode, save. No custom nodes.
pub fn checkpoint_txt2img(i: &Txt2ImgInputs, checkpoint: &str, loras: &[LoraSpec]) -> Value {
    let mut g = json!({
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
    });
    apply_loras(&mut g, loras, ("4", 0), ("4", 1), "3", &["6", "7"]);
    g
}

/// FLUX.1-dev via `ComfyUI-GGUF`: `UnetLoaderGGUF` + `DualCLIPLoaderGGUF`
/// (type `flux`, T5 + CLIP-L) + `VAELoader`, a `FluxGuidance` on the positive
/// conditioning, an SD3-format latent, then the sampler at CFG 1.
pub fn flux_txt2img(i: &Txt2ImgInputs, m: &FluxModels, loras: &[LoraSpec]) -> Value {
    // FLUX is guidance-distilled: the sampler runs at CFG 1 and the effect the
    // user reaches for lives in FluxGuidance instead.
    let guidance = i.cfg.clamp(1.0, 10.0);
    let mut g = json!({
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
    });
    apply_loras(&mut g, loras, ("12", 0), ("11", 0), "3", &["6", "7"]);
    g
}

/// FLUX.2 [klein] via `ComfyUI-GGUF`: `UnetLoaderGGUF` + a single `CLIPLoader`
/// (`type: "flux2"`, one Qwen3 encoder — not a T5 + CLIP-L pair) + `VAELoader`.
/// No real negative prompt: FLUX.2's own template zeroes it out
/// (`ConditioningZeroOut`) rather than encoding one. Sampling goes through
/// `CFGGuider` + `SamplerCustomAdvanced` (driven by `Flux2Scheduler`'s sigmas
/// and `RandomNoise`) instead of a plain `KSampler` — genuinely FLUX.2's own
/// graph shape, verified against Comfy-Org's own `image_flux2_klein_text_to_image`
/// workflow template rather than assumed from FLUX.1's.
pub fn flux2_klein_txt2img(i: &Txt2ImgInputs, m: &Flux2KleinModels, loras: &[LoraSpec]) -> Value {
    let mut g = json!({
        "12": {
            "class_type": "UnetLoaderGGUF",
            "inputs": { "unet_name": m.unet }
        },
        "11": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "flux2" }
        },
        "10": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["11", 0] }
        },
        "27": {
            "class_type": "ConditioningZeroOut",
            "inputs": { "conditioning": ["6", 0] }
        },
        "28": {
            "class_type": "KSamplerSelect",
            "inputs": { "sampler_name": i.sampler }
        },
        "29": {
            "class_type": "Flux2Scheduler",
            "inputs": { "steps": i.steps, "width": i.width, "height": i.height }
        },
        "30": {
            "class_type": "RandomNoise",
            "inputs": { "noise_seed": i.seed }
        },
        "31": {
            "class_type": "CFGGuider",
            "inputs": {
                "model": ["12", 0],
                "positive": ["6", 0],
                "negative": ["27", 0],
                "cfg": i.cfg
            }
        },
        "32": {
            "class_type": "EmptyFlux2LatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "3": {
            "class_type": "SamplerCustomAdvanced",
            "inputs": {
                "noise": ["30", 0],
                "guider": ["31", 0],
                "sampler": ["28", 0],
                "sigmas": ["29", 0],
                "latent_image": ["32", 0]
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
    });
    apply_loras(&mut g, loras, ("12", 0), ("11", 0), "31", &["6"]);
    g
}

/// FLUX.2 [klein] from a plain `.safetensors` checkpoint: `UNETLoader` (no
/// custom node) + the same single-encoder `CLIPLoader` (`type: "flux2"`) +
/// `VAELoader`. Same "no real negative prompt" behavior as the GGUF recipe
/// (`ConditioningZeroOut`), but sampled FLUX.1-style: a `FluxGuidance` on the
/// positive conditioning and a plain `KSampler` pinned to CFG 1, rather than
/// `CFGGuider`/`SamplerCustomAdvanced`. Verified against a real exported
/// ComfyUI workflow for `flux-2-klein-9b-fp8.safetensors`.
pub fn flux2_klein_txt2img_safetensors(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    loras: &[LoraSpec],
) -> Value {
    // Same distillation as FLUX.1: the sampler runs at CFG 1 and the knob the
    // user reaches for lives in FluxGuidance instead.
    let guidance = i.cfg.clamp(1.0, 10.0);
    let mut g = json!({
        "12": {
            "class_type": "UNETLoader",
            "inputs": { "unet_name": m.unet, "weight_dtype": "default" }
        },
        "11": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "flux2" }
        },
        "10": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["11", 0] }
        },
        "27": {
            "class_type": "ConditioningZeroOut",
            "inputs": { "conditioning": ["6", 0] }
        },
        "26": {
            "class_type": "FluxGuidance",
            "inputs": { "conditioning": ["6", 0], "guidance": guidance }
        },
        "32": {
            "class_type": "EmptyFlux2LatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": i.seed,
                "steps": i.steps,
                "cfg": 1.0,
                "sampler_name": i.sampler,
                "scheduler": i.scheduler,
                "denoise": 1.0,
                "model": ["12", 0],
                "positive": ["26", 0],
                "negative": ["27", 0],
                "latent_image": ["32", 0]
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
    });
    apply_loras(&mut g, loras, ("12", 0), ("11", 0), "3", &["6"]);
    g
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

/// FLUX.2 [klein] 9B's own image-editing graph -- the same model unifies
/// generation and editing, it just needs its own VAE
/// (`full_encoder_small_decoder.safetensors`, not the plain generation one)
/// and a different graph shape: `LoadImage` the source, rescale to ~1
/// megapixel, encode it into a latent, and inject that latent into *both*
/// the positive and the zeroed-out negative conditioning via
/// `ReferenceLatent` -- the model edits from the real image instead of
/// generating from nothing. Output size matches the (rescaled) input image,
/// not a fixed square. Verified against Comfy-Org's own
/// `image_flux2_klein_image_edit_9b_distilled` workflow template.
pub fn flux2_klein_edit(i: &EditInputs, m: &Flux2KleinModels, loras: &[LoraSpec]) -> Value {
    let mut g = json!({
        "70": {
            "class_type": "UNETLoader",
            "inputs": { "unet_name": m.unet, "weight_dtype": "default" }
        },
        "71": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "flux2" }
        },
        "72": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "76": {
            "class_type": "LoadImage",
            "inputs": { "image": i.source_image }
        },
        "80": {
            "class_type": "ImageScaleToTotalPixels",
            // `resolution_steps` has a default (1) in ComfyUI's own node
            // schema, but that default is a widget/editor concept -- an
            // API-submitted `/prompt` graph that omits it outright gets
            // rejected with "Required input is missing" (confirmed against
            // ComfyUI's real v0.34.0 source, comfy_extras/
            // nodes_post_processing.py). 1 = no snapping, i.e. today's exact
            // prior behavior.
            "inputs": {
                "image": ["76", 0],
                "upscale_method": "lanczos",
                "megapixels": 1.0,
                "resolution_steps": 1
            }
        },
        "99": {
            "class_type": "GetImageSize",
            "inputs": { "image": ["80", 0] }
        },
        "124": {
            "class_type": "VAEEncode",
            "inputs": { "pixels": ["80", 0], "vae": ["72", 0] }
        },
        "74": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.instruction, "clip": ["71", 0] }
        },
        "123": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["74", 0], "latent": ["124", 0] }
        },
        "82": {
            "class_type": "ConditioningZeroOut",
            "inputs": { "conditioning": ["74", 0] }
        },
        "125": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["82", 0], "latent": ["124", 0] }
        },
        "61": {
            "class_type": "KSamplerSelect",
            "inputs": { "sampler_name": i.sampler }
        },
        "62": {
            "class_type": "Flux2Scheduler",
            "inputs": { "steps": i.steps, "width": ["99", 0], "height": ["99", 1] }
        },
        "73": {
            "class_type": "RandomNoise",
            "inputs": { "noise_seed": i.seed }
        },
        "63": {
            "class_type": "CFGGuider",
            "inputs": {
                "model": ["70", 0],
                "positive": ["123", 0],
                "negative": ["125", 0],
                "cfg": i.cfg
            }
        },
        "66": {
            "class_type": "EmptyFlux2LatentImage",
            "inputs": { "width": ["99", 0], "height": ["99", 1], "batch_size": 1 }
        },
        "64": {
            "class_type": "SamplerCustomAdvanced",
            "inputs": {
                "noise": ["73", 0],
                "guider": ["63", 0],
                "sampler": ["61", 0],
                "sigmas": ["62", 0],
                "latent_image": ["66", 0]
            }
        },
        "65": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["64", 0], "vae": ["72", 0] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": i.filename_prefix, "images": ["65", 0] }
        }
    });
    apply_loras(&mut g, loras, ("70", 0), ("71", 0), "63", &["74"]);
    g
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

/// SDXL (and any single-file SD checkpoint) txt2img with IP-Adapter character/style
/// conditioning spliced in: the same graph as [`checkpoint_txt2img`], plus a
/// `CLIPVisionLoader` + `IPAdapterModelLoader` + `LoadImage` feeding an
/// `IPAdapterAdvanced` node between the checkpoint (and any LoRAs) and the sampler.
///
/// Uses the *explicit-file* loaders (`CLIPVisionLoader`, `IPAdapterModelLoader`),
/// not `IPAdapterUnifiedLoader` — the unified loader's `preset` argument resolves
/// to a file by matching hard-coded name patterns (`get_clipvision_file` /
/// `get_ipadapter_file` in the node pack's `utils.py`), which would silently break
/// the moment a user imports either file under a different name. Passing both
/// bare file names explicitly (the same convention every other function in this
/// module already uses for `checkpoint`/`vae`/`unet`/…) is one extra node but
/// never depends on a naming convention outside AIWM's control.
///
/// Verified against the real `ComfyUI_IPAdapter_plus` node source
/// (`cubiq/ComfyUI_IPAdapter_plus`, commit `a0f451a`): `IPAdapterAdvanced.
/// apply_ipadapter` accepts a raw `IPADAPTER` model (not just the unified
/// loader's `{clipvision, ipadapter, insightface}` dict) as long as a `clip_vision`
/// is also supplied on its optional input — exactly the shape built here.
pub fn checkpoint_ipadapter_txt2img(
    i: &Txt2ImgInputs,
    checkpoint: &str,
    ip: &IpAdapterSpec,
    loras: &[LoraSpec],
) -> Value {
    let mut g = json!({
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
        "40": {
            "class_type": "CLIPVisionLoader",
            "inputs": { "clip_name": ip.clip_vision }
        },
        "41": {
            "class_type": "IPAdapterModelLoader",
            "inputs": { "ipadapter_file": ip.ipadapter_model }
        },
        "42": {
            "class_type": "LoadImage",
            "inputs": { "image": ip.reference_image }
        },
        "43": {
            "class_type": "IPAdapterAdvanced",
            "inputs": {
                "model": ["4", 0],
                "ipadapter": ["41", 0],
                "image": ["42", 0],
                "clip_vision": ["40", 0],
                "weight": ip.weight,
                "weight_type": "linear",
                "combine_embeds": "concat",
                "start_at": 0.0,
                "end_at": 1.0,
                "embeds_scaling": "V only"
            }
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
                "model": ["43", 0],
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
    });
    // LoRAs patch the checkpoint's model/clip *before* IPAdapter sees it (the
    // common community ordering: checkpoint -> LoRA -> IPAdapter -> sampler),
    // so route the chain into node "43" instead of the sampler directly.
    apply_loras(&mut g, loras, ("4", 0), ("4", 1), "43", &["6", "7"]);
    g
}

/// FLUX.2 \[klein\] GGUF txt2img with a reference portrait steering the render,
/// built on the same mechanism [`flux2_klein_edit`] uses for instruction-based
/// edits: `ReferenceLatent`. Unlike an edit, this is a *fresh* generation from
/// noise at the caller's own `width`/`height` (not the reference's own size) —
/// the reference only anchors identity/style via the guiding latent injected
/// into both the real and the zeroed-out conditioning, the same
/// "Kontext-style" technique real FLUX.2/Kontext character-consistency
/// workflows use.
///
/// Only meaningful for FLUX.2 \[klein\], which is edit-trained (that's what
/// makes `ReferenceLatent` actually do something -- see its own doc string,
/// "sets the guiding latent for an edit model"). Plain FLUX.1-dev was never
/// trained to read `reference_latents` conditioning, so this recipe is not
/// offered for it (see `docs/TODO.md`'s Story Studio Phase 2 entry for the
/// FLUX.1 gap and what would close it — Flux Redux).
///
/// GGUF checkpoints only — see [`flux2_klein_reference_txt2img_safetensors`]
/// for a plain `.safetensors` FLUX.2 [klein] file (a real distinction, not a
/// pedantic one: `UnetLoaderGGUF` only ever lists `.gguf` files to ComfyUI, so
/// handing it a `.safetensors` name fails at `/prompt` submission — hit live
/// during this slice's real end-to-end smoke test, not a hypothetical).
pub fn flux2_klein_reference_txt2img(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    reference_image: &str,
    loras: &[LoraSpec],
) -> Value {
    let mut g = json!({
        "12": {
            "class_type": "UnetLoaderGGUF",
            "inputs": { "unet_name": m.unet }
        },
        "11": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "flux2" }
        },
        "10": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "50": {
            "class_type": "LoadImage",
            "inputs": { "image": reference_image }
        },
        "51": {
            "class_type": "ImageScaleToTotalPixels",
            // Same "required despite having a default" quirk as the edit
            // graph -- an API-submitted `/prompt` rejects a missing
            // `resolution_steps` outright. 1 = no snapping.
            "inputs": {
                "image": ["50", 0],
                "upscale_method": "lanczos",
                "megapixels": 1.0,
                "resolution_steps": 1
            }
        },
        "52": {
            "class_type": "VAEEncode",
            "inputs": { "pixels": ["51", 0], "vae": ["10", 0] }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["11", 0] }
        },
        "53": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["6", 0], "latent": ["52", 0] }
        },
        "27": {
            "class_type": "ConditioningZeroOut",
            "inputs": { "conditioning": ["6", 0] }
        },
        "54": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["27", 0], "latent": ["52", 0] }
        },
        "28": {
            "class_type": "KSamplerSelect",
            "inputs": { "sampler_name": i.sampler }
        },
        "29": {
            "class_type": "Flux2Scheduler",
            "inputs": { "steps": i.steps, "width": i.width, "height": i.height }
        },
        "30": {
            "class_type": "RandomNoise",
            "inputs": { "noise_seed": i.seed }
        },
        "31": {
            "class_type": "CFGGuider",
            "inputs": {
                "model": ["12", 0],
                "positive": ["53", 0],
                "negative": ["54", 0],
                "cfg": i.cfg
            }
        },
        "32": {
            "class_type": "EmptyFlux2LatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "3": {
            "class_type": "SamplerCustomAdvanced",
            "inputs": {
                "noise": ["30", 0],
                "guider": ["31", 0],
                "sampler": ["28", 0],
                "sigmas": ["29", 0],
                "latent_image": ["32", 0]
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
    });
    apply_loras(&mut g, loras, ("12", 0), ("11", 0), "31", &["6"]);
    g
}

/// Same as [`flux2_klein_reference_txt2img`] but for a plain `.safetensors`
/// FLUX.2 [klein] checkpoint (`UNETLoader`, no `ComfyUI-GGUF` custom node) —
/// [`Recipe::Flux2KleinSafetensors`]'s own graph shape (`FluxGuidance` + a
/// plain `KSampler` at CFG 1, like [`flux2_klein_txt2img_safetensors`]), not
/// the GGUF recipe's `CFGGuider`/`SamplerCustomAdvanced` chain. A real bug
/// hit live (Story Studio Phase 2 smoke test): reusing the GGUF-only
/// function for a safetensors checkpoint fails at `/prompt` submission with
/// `unet_name: '<file>' not in ['<the-gguf-file>']` — `UnetLoaderGGUF` only
/// ever lists `.gguf` files, so a `.safetensors` FLUX.2 [klein] file (like
/// the fp8 mixed-precision one) is invisible to it. This function exists
/// specifically so [`Recipe::for_family`]'s file-extension split has a
/// correct home on both sides.
pub fn flux2_klein_reference_txt2img_safetensors(
    i: &Txt2ImgInputs,
    m: &Flux2KleinModels,
    reference_image: &str,
    loras: &[LoraSpec],
) -> Value {
    let guidance = i.cfg.clamp(1.0, 10.0);
    let mut g = json!({
        "12": {
            "class_type": "UNETLoader",
            "inputs": { "unet_name": m.unet, "weight_dtype": "default" }
        },
        "11": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "flux2" }
        },
        "10": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "50": {
            "class_type": "LoadImage",
            "inputs": { "image": reference_image }
        },
        "51": {
            "class_type": "ImageScaleToTotalPixels",
            "inputs": {
                "image": ["50", 0],
                "upscale_method": "lanczos",
                "megapixels": 1.0,
                "resolution_steps": 1
            }
        },
        "52": {
            "class_type": "VAEEncode",
            "inputs": { "pixels": ["51", 0], "vae": ["10", 0] }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["11", 0] }
        },
        "53": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["6", 0], "latent": ["52", 0] }
        },
        "26": {
            "class_type": "FluxGuidance",
            "inputs": { "conditioning": ["53", 0], "guidance": guidance }
        },
        "27": {
            "class_type": "ConditioningZeroOut",
            "inputs": { "conditioning": ["6", 0] }
        },
        "54": {
            "class_type": "ReferenceLatent",
            "inputs": { "conditioning": ["27", 0], "latent": ["52", 0] }
        },
        "32": {
            "class_type": "EmptyFlux2LatentImage",
            "inputs": { "width": i.width, "height": i.height, "batch_size": 1 }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": i.seed,
                "steps": i.steps,
                "cfg": 1.0,
                "sampler_name": i.sampler,
                "scheduler": i.scheduler,
                "denoise": 1.0,
                "model": ["12", 0],
                "positive": ["26", 0],
                "negative": ["54", 0],
                "latent_image": ["32", 0]
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
    });
    apply_loras(&mut g, loras, ("12", 0), ("11", 0), "3", &["6"]);
    g
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

/// Wan 2.2 TI2V text/image-to-video. Core ComfyUI video nodes only:
/// `UNETLoader` + `CLIPLoader type=wan` + `VAELoader` → `ModelSamplingSD3`
/// (shift) → `WanImageToVideo` (the latent factory; `start_image` optional) →
/// `KSampler` → `VAEDecode` → `CreateVideo` → `SaveVideo` (mp4/h264).
pub fn wan_ti2v(i: &VideoInputs, m: &WanModels, loras: &[LoraSpec]) -> Value {
    // Wan 5B recommends a sampling shift around 8.
    const WAN_SHIFT: f64 = 8.0;

    let mut g = json!({
        "37": {
            "class_type": "UNETLoader",
            "inputs": { "unet_name": m.unet, "weight_dtype": "default" }
        },
        "38": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.clip, "type": "wan" }
        },
        "39": {
            "class_type": "VAELoader",
            "inputs": { "vae_name": m.vae }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["38", 0] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.negative, "clip": ["38", 0] }
        },
        "48": {
            "class_type": "ModelSamplingSD3",
            "inputs": { "model": ["37", 0], "shift": WAN_SHIFT }
        },
        "55": {
            "class_type": "WanImageToVideo",
            "inputs": {
                "positive": ["6", 0],
                "negative": ["7", 0],
                "vae": ["39", 0],
                "width": i.width,
                "height": i.height,
                "length": i.length,
                "batch_size": 1
            }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": i.seed,
                "steps": i.steps,
                "cfg": i.cfg,
                "sampler_name": "uni_pc",
                "scheduler": "simple",
                "denoise": 1.0,
                "model": ["48", 0],
                "positive": ["55", 0],
                "negative": ["55", 1],
                "latent_image": ["55", 2]
            }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["39", 0] }
        },
        "58": {
            "class_type": "CreateVideo",
            "inputs": { "images": ["8", 0], "fps": i.fps }
        },
        "59": {
            "class_type": "SaveVideo",
            "inputs": {
                "video": ["58", 0],
                "filename_prefix": i.filename_prefix,
                "format": "mp4",
                "codec": "auto"
            }
        }
    });
    apply_loras(&mut g, loras, ("37", 0), ("38", 0), "48", &["6", "7"]);

    if let Some(frame) = i.start_image {
        g["60"] = json!({
            "class_type": "LoadImage",
            "inputs": { "image": frame }
        });
        g["55"]["inputs"]["start_image"] = json!(["60", 0]);
    }
    g
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

/// LTX-Video 0.9.x text/image-to-video. All core ComfyUI nodes (no custom pack):
/// `CheckpointLoaderSimple` (model + VAE) + `CLIPLoader type="ltxv"` →
/// `CLIPTextEncode` ×2 → `LTXVConditioning` → `EmptyLTXVLatentVideo`
/// (or `LTXVImgToVideo` when a start frame is given) → `LTXVScheduler` sigmas →
/// `SamplerCustom` (`KSamplerSelect euler`) → `VAEDecode` → `CreateVideo` →
/// `SaveVideo`.
pub fn ltx_video(i: &VideoInputs, m: &LtxModels, loras: &[LoraSpec]) -> Value {
    // LTXVScheduler defaults from ComfyUI's own LTXV template.
    const MAX_SHIFT: f64 = 2.05;
    const BASE_SHIFT: f64 = 0.95;
    const TERMINAL: f64 = 0.1;
    // LTXVImgToVideo: how strongly the start frame constrains the first frames.
    const IMG_STRENGTH: f64 = 0.15;

    let mut g = json!({
        "44": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": m.checkpoint }
        },
        "38": {
            "class_type": "CLIPLoader",
            "inputs": { "clip_name": m.t5, "type": "ltxv", "device": "default" }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.positive, "clip": ["38", 0] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": i.negative, "clip": ["38", 0] }
        },
        "69": {
            "class_type": "LTXVConditioning",
            "inputs": { "positive": ["6", 0], "negative": ["7", 0], "frame_rate": i.fps }
        },
        "70": {
            "class_type": "EmptyLTXVLatentVideo",
            "inputs": { "width": i.width, "height": i.height, "length": i.length, "batch_size": 1 }
        },
        "71": {
            "class_type": "LTXVScheduler",
            "inputs": {
                "steps": i.steps,
                "max_shift": MAX_SHIFT,
                "base_shift": BASE_SHIFT,
                "stretch": true,
                "terminal": TERMINAL,
                "latent": ["70", 0]
            }
        },
        "73": {
            "class_type": "KSamplerSelect",
            "inputs": { "sampler_name": "euler" }
        },
        "72": {
            "class_type": "SamplerCustom",
            "inputs": {
                "add_noise": true,
                "noise_seed": i.seed,
                "cfg": i.cfg,
                "model": ["44", 0],
                "positive": ["69", 0],
                "negative": ["69", 1],
                "sampler": ["73", 0],
                "sigmas": ["71", 0],
                "latent_image": ["70", 0]
            }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["72", 0], "vae": ["44", 2] }
        },
        "58": {
            "class_type": "CreateVideo",
            "inputs": { "images": ["8", 0], "fps": i.fps }
        },
        "59": {
            "class_type": "SaveVideo",
            "inputs": {
                "video": ["58", 0],
                "filename_prefix": i.filename_prefix,
                "format": "mp4",
                "codec": "auto"
            }
        }
    });
    apply_loras(&mut g, loras, ("44", 0), ("38", 0), "72", &["6", "7"]);

    if let Some(frame) = i.start_image {
        // I2V: LTXVImgToVideo produces the conditioning + the start latent,
        // replacing EmptyLTXVLatentVideo.
        g.as_object_mut().and_then(|o| o.remove("70"));
        g["78"] = json!({ "class_type": "LoadImage", "inputs": { "image": frame } });
        g["77"] = json!({
            "class_type": "LTXVImgToVideo",
            "inputs": {
                "positive": ["6", 0],
                "negative": ["7", 0],
                "vae": ["44", 2],
                "image": ["78", 0],
                "width": i.width,
                "height": i.height,
                "length": i.length,
                "batch_size": 1,
                "strength": IMG_STRENGTH
            }
        });
        g["69"]["inputs"]["positive"] = json!(["77", 0]);
        g["69"]["inputs"]["negative"] = json!(["77", 1]);
        g["71"]["inputs"]["latent"] = json!(["77", 2]);
        g["72"]["inputs"]["latent_image"] = json!(["77", 2]);
    }
    g
}

/// How to resize when running NVIDIA's RTX Video Super Resolution node's
/// `resize_type` `DynamicCombo` input — matches its two branches verbatim,
/// key strings included (`UpscaleType` in `Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`'s
/// own source).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpscaleResize {
    /// Multiply both dimensions by this factor (the node clamps to 1.0-4.0).
    ScaleBy(f64),
    /// Resize to these exact pixel dimensions (the node clamps 64-8192, step 8).
    Target { width: u32, height: u32 },
}

/// Write `resize` onto an `RTXVideoSuperResolution` node's `inputs` object, in
/// the wire shape its `DynamicCombo` schema actually needs.
///
/// **Verification note**: derived (not exercised against a live `/prompt`
/// submission — no real ComfyUI instance was available while building this,
/// Phase 7 upscale slice) from ComfyUI's actual `comfy_api/latest/_io.py`
/// (`DynamicCombo::_expand_schema_for_dynamic`, `parse_class_inputs`) and
/// `execution.py` (`get_input_data`) at the pinned tag (v0.34.0), cross-checked
/// against the RTX node's own shipped `execute()` body and
/// `example_workflows/*.json`. **Independently confirmed** against that same
/// source directly: `_io.py`'s `create_input_dict_v1` builds each nested
/// input's wire key as `prefixed_id = f"{inp.id}.{nested_inp.id}"`, with an
/// explicit comment that this matches "the frontend naming convention (e.g.,
/// `should_texture.enable_pbr`)" — i.e. the dotted form below is what
/// ComfyUI's own schema resolver actually expects, not a guess that happened
/// to compile. Still worth a real end-to-end run once ComfyUI + the node are
/// actually installed, since a source read can't catch every integration
/// quirk (this project's own norm — see `docs/TODO.md`'s Flux/LTX entries).
///
/// The combo's own selector goes under its bare id (`resize_type`); the
/// selected branch's nested widget(s) go under `<id>.<nested id>` —
/// ComfyUI's schema resolver only recognizes the dotted form for a
/// `DynamicCombo`'s nested inputs (a bare `scale`/`width`/`height` doesn't
/// match any key in the resolved schema, so `get_input_data` silently drops
/// it and the node would run whatever it defaults `scale` to instead).
fn set_resize_type(inputs: &mut Value, resize: UpscaleResize) {
    match resize {
        UpscaleResize::ScaleBy(scale) => {
            inputs["resize_type"] = json!("scale by multiplier");
            inputs["resize_type.scale"] = json!(scale);
        }
        UpscaleResize::Target { width, height } => {
            inputs["resize_type"] = json!("target dimensions");
            inputs["resize_type.width"] = json!(width);
            inputs["resize_type.height"] = json!(height);
        }
    }
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

/// An image upscale via NVIDIA's RTX Video Super Resolution custom node
/// (`Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`, installed alongside `ComfyUI-GGUF`):
/// `LoadImage` → `RTXVideoSuperResolution` → `SaveImage`. This sharpens,
/// denoises and resizes an already-rendered image — it does not hallucinate
/// new detail the way a diffusion upscaler would.
pub fn rtx_upscale_image(i: &UpscaleImageInputs) -> Value {
    let mut g = json!({
        "1": {
            "class_type": "LoadImage",
            "inputs": { "image": i.source_image }
        },
        "2": {
            "class_type": "RTXVideoSuperResolution",
            "inputs": {
                "images": ["1", 0],
                "quality": i.quality
            }
        },
        "3": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": i.filename_prefix, "images": ["2", 0] }
        }
    });
    set_resize_type(&mut g["2"]["inputs"], i.resize);
    g
}

/// Inputs for a video upscale via NVIDIA's RTX Video Super Resolution node.
#[derive(Debug, Clone, Copy)]
pub struct UpscaleVideoInputs<'a> {
    pub source_video: &'a str,
    pub resize: UpscaleResize,
    pub quality: &'a str,
    pub filename_prefix: &'a str,
}

/// A video upscale via the same node, sandwiched into ComfyUI's own video
/// load/save nodes rather than inventing a new video pipeline: `LoadVideo` →
/// `GetVideoComponents` (the source's frames, audio and fps) →
/// `RTXVideoSuperResolution` (runs on the whole frame batch — one image, or a
/// video's worth of frames, is the same node either way) → `CreateVideo`
/// (re-encodes with the *original* audio/fps) → `SaveVideo`. Matches the
/// node's own shipped `example_workflows/rtx_video_upscale.json` shape.
pub fn rtx_upscale_video(i: &UpscaleVideoInputs) -> Value {
    let mut g = json!({
        "1": {
            "class_type": "LoadVideo",
            "inputs": { "file": i.source_video }
        },
        "2": {
            "class_type": "GetVideoComponents",
            "inputs": { "video": ["1", 0] }
        },
        "3": {
            "class_type": "RTXVideoSuperResolution",
            "inputs": {
                "images": ["2", 0],
                "quality": i.quality
            }
        },
        "4": {
            "class_type": "CreateVideo",
            "inputs": { "images": ["3", 0], "audio": ["2", 1], "fps": ["2", 2] }
        },
        "5": {
            "class_type": "SaveVideo",
            "inputs": {
                "video": ["4", 0],
                "filename_prefix": i.filename_prefix,
                "format": "mp4",
                "codec": "auto"
            }
        }
    });
    set_resize_type(&mut g["3"]["inputs"], i.resize);
    g
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
    fn checkpoint_graph_wires_every_node() {
        let g = checkpoint_txt2img(&inputs(), "sd_xl_base_1.0.safetensors", &[]);
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
        let g = checkpoint_txt2img(&inputs(), "x.safetensors", &[]);
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
            &[],
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
            &[],
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
            &[],
        );
        assert_eq!(g["26"]["inputs"]["guidance"], 10.0);
    }

    #[test]
    fn flux2_klein_graph_wires_the_single_encoder_and_zeroed_negative() {
        let g = flux2_klein_txt2img(
            &inputs(),
            &Flux2KleinModels {
                unet: "flux-2-klein-9b-Q4_K_M.gguf",
                clip: "qwen_3_8b_fp8mixed.safetensors",
                vae: "flux2-vae.safetensors",
            },
            &[],
        );
        assert_eq!(g["12"]["class_type"], "UnetLoaderGGUF");
        assert_eq!(
            g["12"]["inputs"]["unet_name"],
            "flux-2-klein-9b-Q4_K_M.gguf"
        );
        assert_eq!(g["11"]["class_type"], "CLIPLoader");
        assert_eq!(
            g["11"]["inputs"]["clip_name"],
            "qwen_3_8b_fp8mixed.safetensors"
        );
        assert_eq!(g["11"]["inputs"]["type"], "flux2");
        assert_eq!(g["10"]["inputs"]["vae_name"], "flux2-vae.safetensors");
        assert_eq!(g["6"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(g["6"]["inputs"]["clip"], json!(["11", 0]));
        // No real negative prompt -- FLUX.2's own template zeroes it out.
        assert_eq!(g["27"]["class_type"], "ConditioningZeroOut");
        assert_eq!(g["27"]["inputs"]["conditioning"], json!(["6", 0]));
        assert_eq!(g["31"]["class_type"], "CFGGuider");
        assert_eq!(g["31"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["31"]["inputs"]["positive"], json!(["6", 0]));
        assert_eq!(g["31"]["inputs"]["negative"], json!(["27", 0]));
        assert_eq!(g["31"]["inputs"]["cfg"], 7.0);
        assert_eq!(g["3"]["class_type"], "SamplerCustomAdvanced");
        assert_eq!(g["3"]["inputs"]["guider"], json!(["31", 0]));
        assert_eq!(g["3"]["inputs"]["sigmas"], json!(["29", 0]));
        assert_eq!(g["29"]["class_type"], "Flux2Scheduler");
        assert_eq!(g["29"]["inputs"]["steps"], 25);
        assert_eq!(g["32"]["class_type"], "EmptyFlux2LatentImage");
        assert_eq!(g["32"]["inputs"]["width"], 1024);
        assert_eq!(g["8"]["inputs"]["vae"], json!(["10", 0]));
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux2_klein_graph_splices_a_lora_before_the_guider_and_encode() {
        let g = flux2_klein_txt2img(
            &inputs(),
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[LoraSpec {
                file: "flux2-realistic-detail.safetensors",
                strength: 0.8,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["11", 0]));
        assert_eq!(
            g["31"]["inputs"]["model"],
            json!(["90", 0]),
            "CFGGuider reads the lora'd model"
        );
        assert_eq!(
            g["6"]["inputs"]["clip"],
            json!(["90", 1]),
            "CLIPTextEncode reads the lora'd clip"
        );
    }

    #[test]
    fn flux2_klein_safetensors_graph_uses_a_plain_ksampler_and_flux_guidance() {
        let g = flux2_klein_txt2img_safetensors(
            &inputs(),
            &Flux2KleinModels {
                unet: "flux-2-klein-9b-fp8mixed.safetensors",
                clip: "qwen_3_8b_fp8mixed.safetensors",
                vae: "flux2-vae.safetensors",
            },
            &[],
        );
        assert_eq!(g["12"]["class_type"], "UNETLoader");
        assert_eq!(
            g["12"]["inputs"]["unet_name"],
            "flux-2-klein-9b-fp8mixed.safetensors"
        );
        assert_eq!(g["11"]["class_type"], "CLIPLoader");
        assert_eq!(g["11"]["inputs"]["type"], "flux2");
        assert_eq!(g["10"]["inputs"]["vae_name"], "flux2-vae.safetensors");
        // No real negative prompt, same as the GGUF recipe.
        assert_eq!(g["27"]["class_type"], "ConditioningZeroOut");
        assert_eq!(g["27"]["inputs"]["conditioning"], json!(["6", 0]));
        assert_eq!(g["26"]["class_type"], "FluxGuidance");
        assert_eq!(g["26"]["inputs"]["guidance"], 7.0);
        assert_eq!(g["3"]["class_type"], "KSampler");
        assert_eq!(g["3"]["inputs"]["cfg"], 1.0, "runs at CFG 1, like FLUX.1");
        assert_eq!(g["3"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["3"]["inputs"]["positive"], json!(["26", 0]));
        assert_eq!(g["3"]["inputs"]["negative"], json!(["27", 0]));
        assert_eq!(g["3"]["inputs"]["latent_image"], json!(["32", 0]));
        assert_eq!(g["32"]["class_type"], "EmptyFlux2LatentImage");
        assert_eq!(g["8"]["inputs"]["vae"], json!(["10", 0]));
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux2_klein_safetensors_guidance_is_clamped() {
        let mut i = inputs();
        i.cfg = 42.0;
        let g = flux2_klein_txt2img_safetensors(
            &i,
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[],
        );
        assert_eq!(g["26"]["inputs"]["guidance"], 10.0);
    }

    #[test]
    fn flux2_klein_safetensors_graph_splices_a_lora_before_the_sampler_and_encode() {
        let g = flux2_klein_txt2img_safetensors(
            &inputs(),
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[LoraSpec {
                file: "flux2-realistic-detail.safetensors",
                strength: 0.8,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["11", 0]));
        assert_eq!(
            g["3"]["inputs"]["model"],
            json!(["90", 0]),
            "KSampler reads the lora'd model"
        );
        assert_eq!(
            g["6"]["inputs"]["clip"],
            json!(["90", 1]),
            "CLIPTextEncode reads the lora'd clip"
        );
    }

    fn edit_inputs() -> EditInputs<'static> {
        EditInputs {
            instruction: "make the hair blonde",
            source_image: "job-abc.png",
            steps: 8,
            cfg: 1.5,
            sampler: "euler",
            seed: 42,
            filename_prefix: "job-abc",
        }
    }

    #[test]
    fn flux2_klein_edit_graph_wires_the_source_image_into_both_conditionings() {
        let g = flux2_klein_edit(
            &edit_inputs(),
            &Flux2KleinModels {
                unet: "flux-2-klein-9b-fp8.safetensors",
                clip: "qwen_3_8b_fp8mixed.safetensors",
                vae: "full_encoder_small_decoder.safetensors",
            },
            &[],
        );
        assert_eq!(g["70"]["class_type"], "UNETLoader");
        assert_eq!(g["71"]["class_type"], "CLIPLoader");
        assert_eq!(
            g["72"]["inputs"]["vae_name"],
            "full_encoder_small_decoder.safetensors"
        );
        assert_eq!(g["76"]["class_type"], "LoadImage");
        assert_eq!(g["76"]["inputs"]["image"], "job-abc.png");
        // The source image is rescaled, then both the output canvas and the
        // sampler's sigma schedule are sized from it -- not a fixed square.
        assert_eq!(g["80"]["inputs"]["image"], json!(["76", 0]));
        // Required by ComfyUI's real node schema even though it has a
        // default -- omitting it entirely made every edit job fail with
        // "Required input is missing" (a real bug hit live, not a guess).
        assert_eq!(g["80"]["inputs"]["resolution_steps"], 1);
        assert_eq!(g["99"]["inputs"]["image"], json!(["80", 0]));
        assert_eq!(g["66"]["inputs"]["width"], json!(["99", 0]));
        assert_eq!(g["66"]["inputs"]["height"], json!(["99", 1]));
        assert_eq!(g["62"]["inputs"]["width"], json!(["99", 0]));
        // The rescaled image is encoded once, then referenced by *both* the
        // real (positive) and the zeroed (negative) conditioning.
        assert_eq!(g["124"]["inputs"]["pixels"], json!(["80", 0]));
        assert_eq!(g["74"]["inputs"]["text"], "make the hair blonde");
        assert_eq!(g["123"]["inputs"]["conditioning"], json!(["74", 0]));
        assert_eq!(g["123"]["inputs"]["latent"], json!(["124", 0]));
        assert_eq!(g["82"]["inputs"]["conditioning"], json!(["74", 0]));
        assert_eq!(g["125"]["inputs"]["conditioning"], json!(["82", 0]));
        assert_eq!(g["125"]["inputs"]["latent"], json!(["124", 0]));
        assert_eq!(g["63"]["inputs"]["positive"], json!(["123", 0]));
        assert_eq!(g["63"]["inputs"]["negative"], json!(["125", 0]));
        assert_eq!(g["63"]["inputs"]["cfg"], 1.5);
        assert_eq!(g["62"]["inputs"]["steps"], 8);
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux2_klein_edit_graph_splices_a_lora_before_the_guider_and_encode() {
        let g = flux2_klein_edit(
            &edit_inputs(),
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[LoraSpec {
                file: "some-lora.safetensors",
                strength: 0.8,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["70", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["71", 0]));
        assert_eq!(
            g["63"]["inputs"]["model"],
            json!(["90", 0]),
            "CFGGuider reads the lora'd model"
        );
        assert_eq!(
            g["74"]["inputs"]["clip"],
            json!(["90", 1]),
            "CLIPTextEncode reads the lora'd clip"
        );
    }

    fn ipadapter_spec() -> IpAdapterSpec<'static> {
        IpAdapterSpec {
            clip_vision: "CLIP-ViT-H-14-laion2B-s32B-b79K.safetensors",
            ipadapter_model: "ip-adapter-plus_sdxl_vit-h.safetensors",
            reference_image: "job-portrait.png",
            weight: 0.8,
        }
    }

    #[test]
    fn checkpoint_ipadapter_graph_wires_the_clip_vision_and_ipadapter_chain() {
        let g = checkpoint_ipadapter_txt2img(
            &inputs(),
            "sd_xl_base_1.0.safetensors",
            &ipadapter_spec(),
            &[],
        );
        assert_eq!(g["4"]["inputs"]["ckpt_name"], "sd_xl_base_1.0.safetensors");
        assert_eq!(g["40"]["class_type"], "CLIPVisionLoader");
        assert_eq!(
            g["40"]["inputs"]["clip_name"],
            "CLIP-ViT-H-14-laion2B-s32B-b79K.safetensors"
        );
        assert_eq!(g["41"]["class_type"], "IPAdapterModelLoader");
        assert_eq!(
            g["41"]["inputs"]["ipadapter_file"],
            "ip-adapter-plus_sdxl_vit-h.safetensors"
        );
        assert_eq!(g["42"]["class_type"], "LoadImage");
        assert_eq!(g["42"]["inputs"]["image"], "job-portrait.png");
        assert_eq!(g["43"]["class_type"], "IPAdapterAdvanced");
        assert_eq!(g["43"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["43"]["inputs"]["ipadapter"], json!(["41", 0]));
        assert_eq!(g["43"]["inputs"]["image"], json!(["42", 0]));
        assert_eq!(g["43"]["inputs"]["clip_vision"], json!(["40", 0]));
        assert_eq!(g["43"]["inputs"]["weight"], 0.8);
        assert_eq!(g["43"]["inputs"]["weight_type"], "linear");
        // The sampler reads the IPAdapter-patched model, not the bare checkpoint.
        assert_eq!(g["3"]["inputs"]["model"], json!(["43", 0]));
        assert_eq!(g["3"]["inputs"]["positive"], json!(["6", 0]));
        assert_eq!(g["8"]["inputs"]["vae"], json!(["4", 2]));
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn checkpoint_ipadapter_graph_splices_loras_before_the_ipadapter_node() {
        let g = checkpoint_ipadapter_txt2img(
            &inputs(),
            "x.safetensors",
            &ipadapter_spec(),
            &[LoraSpec {
                file: "style.safetensors",
                strength: 0.6,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["4", 1]));
        assert_eq!(
            g["43"]["inputs"]["model"],
            json!(["90", 0]),
            "IPAdapterAdvanced reads the lora'd model"
        );
        assert_eq!(
            g["6"]["inputs"]["clip"],
            json!(["90", 1]),
            "CLIPTextEncode reads the lora'd clip"
        );
        assert_eq!(
            g["3"]["inputs"]["model"],
            json!(["43", 0]),
            "the sampler still reads the IPAdapter output, on top of the LoRA"
        );
    }

    #[test]
    fn flux2_klein_reference_graph_injects_the_portrait_into_both_conditionings() {
        let g = flux2_klein_reference_txt2img(
            &inputs(),
            &Flux2KleinModels {
                unet: "flux-2-klein-9b-fp8mixed.safetensors",
                clip: "qwen_3_8b_fp8mixed.safetensors",
                vae: "flux2-vae.safetensors",
            },
            "job-portrait.png",
            &[],
        );
        assert_eq!(g["50"]["class_type"], "LoadImage");
        assert_eq!(g["50"]["inputs"]["image"], "job-portrait.png");
        assert_eq!(g["51"]["inputs"]["image"], json!(["50", 0]));
        assert_eq!(g["51"]["inputs"]["resolution_steps"], 1);
        assert_eq!(g["52"]["class_type"], "VAEEncode");
        assert_eq!(g["52"]["inputs"]["pixels"], json!(["51", 0]));
        // The new scene's own dimensions drive the canvas, not the portrait's.
        assert_eq!(g["32"]["inputs"]["width"], 1024);
        assert_eq!(g["29"]["inputs"]["width"], 1024);
        // The reference latent anchors *both* the real and the zeroed-out
        // conditioning, same pattern as the edit graph.
        assert_eq!(g["53"]["class_type"], "ReferenceLatent");
        assert_eq!(g["53"]["inputs"]["conditioning"], json!(["6", 0]));
        assert_eq!(g["53"]["inputs"]["latent"], json!(["52", 0]));
        assert_eq!(g["54"]["class_type"], "ReferenceLatent");
        assert_eq!(g["54"]["inputs"]["conditioning"], json!(["27", 0]));
        assert_eq!(g["54"]["inputs"]["latent"], json!(["52", 0]));
        assert_eq!(g["31"]["inputs"]["positive"], json!(["53", 0]));
        assert_eq!(g["31"]["inputs"]["negative"], json!(["54", 0]));
        assert_eq!(g["6"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux2_klein_reference_graph_splices_a_lora_before_the_guider_and_encode() {
        let g = flux2_klein_reference_txt2img(
            &inputs(),
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            "job-portrait.png",
            &[LoraSpec {
                file: "style.safetensors",
                strength: 0.5,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["11", 0]));
        assert_eq!(g["31"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
    }

    #[test]
    fn flux2_klein_reference_safetensors_graph_uses_unet_loader_and_a_plain_ksampler() {
        // The real bug this test guards: a plain .safetensors FLUX.2 [klein]
        // file (e.g. flux-2-klein-9b-fp8mixed.safetensors) must never be
        // routed through UnetLoaderGGUF -- that node only ever lists .gguf
        // files to ComfyUI, so submitting it fails at /prompt with
        // "unet_name: '<file>' not in ['<some .gguf file>']" (hit live during
        // this slice's real end-to-end smoke test).
        let g = flux2_klein_reference_txt2img_safetensors(
            &inputs(),
            &Flux2KleinModels {
                unet: "flux-2-klein-9b-fp8mixed.safetensors",
                clip: "qwen_3_8b_fp8mixed.safetensors",
                vae: "flux2-vae.safetensors",
            },
            "job-portrait.png",
            &[],
        );
        assert_eq!(g["12"]["class_type"], "UNETLoader");
        assert_eq!(
            g["12"]["inputs"]["unet_name"],
            "flux-2-klein-9b-fp8mixed.safetensors"
        );
        assert_eq!(g["50"]["class_type"], "LoadImage");
        assert_eq!(g["50"]["inputs"]["image"], "job-portrait.png");
        assert_eq!(g["52"]["class_type"], "VAEEncode");
        assert_eq!(g["53"]["class_type"], "ReferenceLatent");
        assert_eq!(g["53"]["inputs"]["conditioning"], json!(["6", 0]));
        assert_eq!(g["53"]["inputs"]["latent"], json!(["52", 0]));
        assert_eq!(g["26"]["class_type"], "FluxGuidance");
        assert_eq!(g["26"]["inputs"]["conditioning"], json!(["53", 0]));
        assert_eq!(g["54"]["class_type"], "ReferenceLatent");
        assert_eq!(g["54"]["inputs"]["conditioning"], json!(["27", 0]));
        assert_eq!(g["54"]["inputs"]["latent"], json!(["52", 0]));
        assert_eq!(g["3"]["class_type"], "KSampler");
        assert_eq!(g["3"]["inputs"]["cfg"], 1.0, "runs at CFG 1, like FLUX.1");
        assert_eq!(g["3"]["inputs"]["positive"], json!(["26", 0]));
        assert_eq!(g["3"]["inputs"]["negative"], json!(["54", 0]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["32"]["inputs"]["width"], 1024);
        assert_eq!(g["9"]["inputs"]["filename_prefix"], "job-abc");
    }

    #[test]
    fn flux2_klein_reference_safetensors_graph_splices_a_lora_before_the_sampler_and_encode() {
        let g = flux2_klein_reference_txt2img_safetensors(
            &inputs(),
            &Flux2KleinModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            "job-portrait.png",
            &[LoraSpec {
                file: "style.safetensors",
                strength: 0.5,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["11", 0]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
    }

    fn video_inputs() -> VideoInputs<'static> {
        VideoInputs {
            positive: "a boat on a calm sea",
            negative: "blurry",
            width: 832,
            height: 480,
            length: 81,
            fps: 24,
            steps: 30,
            cfg: 5.0,
            seed: 7,
            start_image: None,
            filename_prefix: "job-vid",
        }
    }

    #[test]
    fn wan_graph_wires_the_video_chain_for_text_to_video() {
        let g = wan_ti2v(
            &video_inputs(),
            &WanModels {
                unet: "wan2.2_ti2v_5B_fp16.safetensors",
                clip: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                vae: "wan2.2_vae.safetensors",
            },
            &[],
        );
        assert_eq!(g["37"]["class_type"], "UNETLoader");
        assert_eq!(
            g["37"]["inputs"]["unet_name"],
            "wan2.2_ti2v_5B_fp16.safetensors"
        );
        assert_eq!(g["38"]["inputs"]["type"], "wan");
        assert_eq!(g["48"]["class_type"], "ModelSamplingSD3");
        assert_eq!(g["55"]["class_type"], "WanImageToVideo");
        assert_eq!(g["55"]["inputs"]["length"], 81);
        assert!(
            g["55"]["inputs"].get("start_image").is_none(),
            "T2V: no start frame"
        );
        // sampler ← model-sampling; sampler ← the WanImageToVideo latent + conds.
        assert_eq!(g["3"]["inputs"]["model"], json!(["48", 0]));
        assert_eq!(g["3"]["inputs"]["positive"], json!(["55", 0]));
        assert_eq!(g["3"]["inputs"]["latent_image"], json!(["55", 2]));
        assert_eq!(g["3"]["inputs"]["seed"], 7);
        assert_eq!(g["58"]["class_type"], "CreateVideo");
        assert_eq!(g["58"]["inputs"]["fps"], 24);
        assert_eq!(g["59"]["class_type"], "SaveVideo");
        assert_eq!(g["59"]["inputs"]["format"], "mp4");
        assert_eq!(g["59"]["inputs"]["filename_prefix"], "job-vid");
    }

    #[test]
    fn wan_graph_adds_a_load_image_for_image_to_video() {
        let mut i = video_inputs();
        i.start_image = Some("job-src.png");
        let g = wan_ti2v(
            &i,
            &WanModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[],
        );
        assert_eq!(g["60"]["class_type"], "LoadImage");
        assert_eq!(g["60"]["inputs"]["image"], "job-src.png");
        assert_eq!(g["55"]["inputs"]["start_image"], json!(["60", 0]));
    }

    #[test]
    fn video_recipe_selects_ltx_for_the_family() {
        assert_eq!(VideoRecipe::for_family(Some("ltx")), VideoRecipe::Ltx);
        assert_eq!(VideoRecipe::for_family(Some("LTX")), VideoRecipe::Ltx);
        assert_eq!(VideoRecipe::for_family(Some("wan")), VideoRecipe::Wan);
        assert_eq!(VideoRecipe::for_family(None), VideoRecipe::Wan);
    }

    fn ltx_models() -> LtxModels<'static> {
        LtxModels {
            checkpoint: "ltx-video-2b-v0.9.5.safetensors",
            t5: "t5xxl_fp8_e4m3fn.safetensors",
        }
    }

    #[test]
    fn ltx_graph_wires_the_scheduler_driven_sampler_for_text_to_video() {
        let g = ltx_video(&video_inputs(), &ltx_models(), &[]);
        assert_eq!(g["44"]["class_type"], "CheckpointLoaderSimple");
        assert_eq!(
            g["44"]["inputs"]["ckpt_name"],
            "ltx-video-2b-v0.9.5.safetensors"
        );
        assert_eq!(g["38"]["inputs"]["type"], "ltxv");
        assert_eq!(g["69"]["class_type"], "LTXVConditioning");
        assert_eq!(g["69"]["inputs"]["frame_rate"], 24);
        assert_eq!(g["70"]["class_type"], "EmptyLTXVLatentVideo");
        assert_eq!(g["70"]["inputs"]["length"], 81);
        // SamplerCustom is driven by the LTXVScheduler sigmas + a picked sampler.
        assert_eq!(g["72"]["class_type"], "SamplerCustom");
        assert_eq!(g["72"]["inputs"]["noise_seed"], 7);
        assert_eq!(g["72"]["inputs"]["cfg"], 5.0);
        assert_eq!(g["72"]["inputs"]["model"], json!(["44", 0]));
        assert_eq!(g["72"]["inputs"]["sigmas"], json!(["71", 0]));
        assert_eq!(g["72"]["inputs"]["sampler"], json!(["73", 0]));
        assert_eq!(g["72"]["inputs"]["latent_image"], json!(["70", 0]));
        assert_eq!(g["71"]["inputs"]["latent"], json!(["70", 0]));
        assert_eq!(g["8"]["inputs"]["vae"], json!(["44", 2]));
        assert_eq!(g["58"]["class_type"], "CreateVideo");
        assert_eq!(g["59"]["inputs"]["filename_prefix"], "job-vid");
        assert!(g.get("77").is_none(), "T2V: no LTXVImgToVideo");
    }

    #[test]
    fn ltx_graph_swaps_in_img_to_video_for_a_start_frame() {
        let mut i = video_inputs();
        i.start_image = Some("job-src.png");
        let g = ltx_video(&i, &ltx_models(), &[]);
        assert!(g.get("70").is_none(), "EmptyLTXVLatentVideo is replaced");
        assert_eq!(g["78"]["class_type"], "LoadImage");
        assert_eq!(g["78"]["inputs"]["image"], "job-src.png");
        assert_eq!(g["77"]["class_type"], "LTXVImgToVideo");
        assert_eq!(g["77"]["inputs"]["image"], json!(["78", 0]));
        assert_eq!(g["77"]["inputs"]["vae"], json!(["44", 2]));
        // conditioning + the start latent now come from LTXVImgToVideo
        assert_eq!(g["69"]["inputs"]["positive"], json!(["77", 0]));
        assert_eq!(g["71"]["inputs"]["latent"], json!(["77", 2]));
        assert_eq!(g["72"]["inputs"]["latent_image"], json!(["77", 2]));
    }

    // --- LoRA splicing --------------------------------------------------------

    #[test]
    fn checkpoint_graph_splices_a_lora_before_the_sampler_and_clip() {
        let g = checkpoint_txt2img(
            &inputs(),
            "sd_xl_base_1.0.safetensors",
            &[LoraSpec {
                file: "add-detail-xl.safetensors",
                strength: 0.8,
            }],
        );
        assert_eq!(g["90"]["class_type"], "LoraLoader");
        assert_eq!(g["90"]["inputs"]["lora_name"], "add-detail-xl.safetensors");
        assert_eq!(g["90"]["inputs"]["strength_model"], 0.8);
        assert_eq!(g["90"]["inputs"]["strength_clip"], 0.8);
        assert_eq!(g["90"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["4", 1]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
        assert_eq!(g["7"]["inputs"]["clip"], json!(["90", 1]));
    }

    #[test]
    fn checkpoint_graph_chains_multiple_loras_in_order() {
        let g = checkpoint_txt2img(
            &inputs(),
            "x.safetensors",
            &[
                LoraSpec {
                    file: "a.safetensors",
                    strength: 1.0,
                },
                LoraSpec {
                    file: "b.safetensors",
                    strength: 0.5,
                },
            ],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["91"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["91"]["inputs"]["clip"], json!(["90", 1]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["91", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["91", 1]));
    }

    #[test]
    fn checkpoint_graph_with_no_loras_is_unchanged() {
        let g = checkpoint_txt2img(&inputs(), "x.safetensors", &[]);
        assert!(g.get("90").is_none());
        assert_eq!(g["3"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["4", 1]));
    }

    #[test]
    fn flux_graph_splices_a_lora_between_the_gguf_loaders_and_the_sampler() {
        let g = flux_txt2img(
            &inputs(),
            &FluxModels {
                unet: "u",
                t5: "t",
                clip_l: "c",
                vae: "v",
            },
            &[LoraSpec {
                file: "flux-style.safetensors",
                strength: 0.7,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["11", 0]));
        assert_eq!(g["3"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
        assert_eq!(g["7"]["inputs"]["clip"], json!(["90", 1]));
    }

    #[test]
    fn wan_graph_splices_a_lora_before_model_sampling() {
        let g = wan_ti2v(
            &video_inputs(),
            &WanModels {
                unet: "u",
                clip: "c",
                vae: "v",
            },
            &[LoraSpec {
                file: "wan-motion.safetensors",
                strength: 1.0,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["37", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["38", 0]));
        assert_eq!(
            g["48"]["inputs"]["model"],
            json!(["90", 0]),
            "ModelSamplingSD3 reads the lora'd model"
        );
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
    }

    #[test]
    fn ltx_graph_splices_a_lora_before_the_sampler() {
        let g = ltx_video(
            &video_inputs(),
            &ltx_models(),
            &[LoraSpec {
                file: "ltx-style.safetensors",
                strength: 0.9,
            }],
        );
        assert_eq!(g["90"]["inputs"]["model"], json!(["44", 0]));
        assert_eq!(g["90"]["inputs"]["clip"], json!(["38", 0]));
        assert_eq!(g["72"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(g["6"]["inputs"]["clip"], json!(["90", 1]));
    }

    // --- RTX Video Super Resolution (upscale) ---------------------------------

    fn upscale_image_inputs(resize: UpscaleResize) -> UpscaleImageInputs<'static> {
        UpscaleImageInputs {
            source_image: "job-src.png",
            resize,
            quality: "ULTRA",
            filename_prefix: "job-up",
        }
    }

    fn upscale_video_inputs(resize: UpscaleResize) -> UpscaleVideoInputs<'static> {
        UpscaleVideoInputs {
            source_video: "job-src.mp4",
            resize,
            quality: "ULTRA",
            filename_prefix: "job-up",
        }
    }

    #[test]
    fn rtx_upscale_image_wires_load_rtx_save_and_scale_by_as_dotted_dynamic_combo_keys() {
        let g = rtx_upscale_image(&upscale_image_inputs(UpscaleResize::ScaleBy(2.0)));
        assert_eq!(g["1"]["class_type"], "LoadImage");
        assert_eq!(g["1"]["inputs"]["image"], "job-src.png");
        assert_eq!(g["2"]["class_type"], "RTXVideoSuperResolution");
        assert_eq!(g["2"]["inputs"]["images"], json!(["1", 0]));
        // The DynamicCombo's own selector, plus its scale-by branch's one
        // nested widget under the dotted `resize_type.scale` key -- see
        // `set_resize_type`'s doc comment for why it must be dotted.
        assert_eq!(g["2"]["inputs"]["resize_type"], "scale by multiplier");
        assert_eq!(g["2"]["inputs"]["resize_type.scale"], 2.0);
        assert!(g["2"]["inputs"].get("resize_type.width").is_none());
        assert_eq!(g["2"]["inputs"]["quality"], "ULTRA");
        assert_eq!(g["3"]["class_type"], "SaveImage");
        assert_eq!(g["3"]["inputs"]["images"], json!(["2", 0]));
        assert_eq!(g["3"]["inputs"]["filename_prefix"], "job-up");
    }

    #[test]
    fn rtx_upscale_image_wires_target_dimensions_as_dotted_width_height() {
        let g = rtx_upscale_image(&upscale_image_inputs(UpscaleResize::Target {
            width: 1920,
            height: 1080,
        }));
        assert_eq!(g["2"]["inputs"]["resize_type"], "target dimensions");
        assert_eq!(g["2"]["inputs"]["resize_type.width"], 1920);
        assert_eq!(g["2"]["inputs"]["resize_type.height"], 1080);
        assert!(g["2"]["inputs"].get("resize_type.scale").is_none());
    }

    #[test]
    fn rtx_upscale_video_wires_load_video_get_components_rtx_and_create_save_video() {
        let g = rtx_upscale_video(&upscale_video_inputs(UpscaleResize::ScaleBy(1.5)));
        assert_eq!(g["1"]["class_type"], "LoadVideo");
        assert_eq!(g["1"]["inputs"]["file"], "job-src.mp4");
        assert_eq!(g["2"]["class_type"], "GetVideoComponents");
        assert_eq!(g["2"]["inputs"]["video"], json!(["1", 0]));
        assert_eq!(g["3"]["class_type"], "RTXVideoSuperResolution");
        // The frame batch comes from GetVideoComponents' `images` output (0).
        assert_eq!(g["3"]["inputs"]["images"], json!(["2", 0]));
        assert_eq!(g["3"]["inputs"]["resize_type"], "scale by multiplier");
        assert_eq!(g["3"]["inputs"]["resize_type.scale"], 1.5);
        assert_eq!(g["4"]["class_type"], "CreateVideo");
        assert_eq!(g["4"]["inputs"]["images"], json!(["3", 0]));
        // The original audio (output 1) and fps (output 2) ride along --
        // upscaling only touches the frames.
        assert_eq!(g["4"]["inputs"]["audio"], json!(["2", 1]));
        assert_eq!(g["4"]["inputs"]["fps"], json!(["2", 2]));
        assert_eq!(g["5"]["class_type"], "SaveVideo");
        assert_eq!(g["5"]["inputs"]["video"], json!(["4", 0]));
        assert_eq!(g["5"]["inputs"]["filename_prefix"], "job-up");
    }
}
