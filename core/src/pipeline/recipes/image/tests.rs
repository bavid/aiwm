//! Unit tests for the image recipes, moved from `pipeline::tests` when the
//! recipe bodies moved onto the fragment layer. They assert node ids, wiring
//! and LoRA chains; the golden fixtures in `core/tests/pipeline_goldens.rs`
//! assert each whole graph.

use serde_json::json;

use super::*;
use crate::pipeline::{EditInputs, Flux2KleinModels, FluxModels, LoraSpec, Txt2ImgInputs};

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

// --- LoRA splicing ------------------------------------------------------------

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
