//! Unit tests for the image recipes, moved from `pipeline::tests` when the
//! recipe bodies moved onto the fragment layer. They assert node ids, wiring
//! and LoRA chains; the golden fixtures in `core/tests/pipeline_goldens.rs`
//! assert each whole graph.

use serde_json::json;

use super::*;
use crate::pipeline::{
    latent_upscaled_px, EditInputs, Flux2KleinModels, FluxModels, HiresFix, LoraSpec, Txt2ImgInputs,
};

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
        hires: None,
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

// --- Hi-Res-Fix ---------------------------------------------------------------

/// The knobs every Hi-Res-Fix test below uses: 1.5x bigger, a low denoise so
/// the first pass's composition survives, and about half the first pass's
/// steps.
fn hires() -> HiresFix {
    HiresFix {
        scale_by: 1.5,
        denoise: 0.45,
        steps: 12,
        upscale_method: HiresFix::DEFAULT_METHOD,
    }
}

fn hires_inputs() -> Txt2ImgInputs<'static> {
    Txt2ImgInputs {
        hires: Some(hires()),
        ..inputs()
    }
}

fn flux_models() -> FluxModels<'static> {
    FluxModels {
        unet: "u",
        t5: "t",
        clip_l: "c",
        vae: "v",
    }
}

fn klein_models() -> Flux2KleinModels<'static> {
    Flux2KleinModels {
        unet: "u",
        clip: "c",
        vae: "v",
    }
}

#[test]
fn checkpoint_hires_inserts_latent_upscale_and_a_second_pass_then_decodes_from_it() {
    let g = checkpoint_txt2img(&hires_inputs(), "sd_xl_base_1.0.safetensors", &[]);
    assert_eq!(
        g["40"],
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
        g["41"],
        json!({
            "class_type": "KSampler",
            "inputs": {
                // Same seed as the first pass: the second pass continues that
                // image, it does not roll a new one.
                "seed": 42,
                "steps": 12,
                "cfg": 7.0,
                "sampler_name": "euler",
                "scheduler": "normal",
                "denoise": 0.45,
                "model": ["4", 0],
                "positive": ["6", 0],
                "negative": ["7", 0],
                "latent_image": ["40", 0]
            }
        })
    );
    assert_eq!(
        g["8"]["inputs"]["samples"],
        json!(["41", 0]),
        "the decode hangs off the second pass, not the first"
    );
    assert_eq!(g["3"]["inputs"]["denoise"], 1.0, "first pass is untouched");
}

#[test]
fn flux_hires_second_pass_keeps_cfg_one_and_guidance() {
    let g = flux_txt2img(&hires_inputs(), &flux_models(), &[]);
    assert_eq!(g["40"]["inputs"]["samples"], json!(["3", 0]));
    assert_eq!(g["40"]["inputs"]["scale_by"], 1.5);
    assert_eq!(g["41"]["class_type"], "KSampler");
    assert_eq!(
        g["41"]["inputs"]["cfg"], 1.0,
        "FLUX stays guidance-distilled on the second pass too"
    );
    assert_eq!(
        g["41"]["inputs"]["positive"],
        json!(["26", 0]),
        "the second pass reads the same FluxGuidance conditioning"
    );
    assert_eq!(g["41"]["inputs"]["negative"], json!(["7", 0]));
    assert_eq!(g["41"]["inputs"]["model"], json!(["12", 0]));
    assert_eq!(g["41"]["inputs"]["scheduler"], "simple");
    assert_eq!(g["41"]["inputs"]["sampler_name"], "euler");
    assert_eq!(g["41"]["inputs"]["denoise"], 0.45);
    assert_eq!(g["41"]["inputs"]["steps"], 12);
    assert_eq!(g["41"]["inputs"]["latent_image"], json!(["40", 0]));
    assert_eq!(g["8"]["inputs"]["samples"], json!(["41", 0]));
}

#[test]
fn flux2_klein_safetensors_hires() {
    let g = flux2_klein_txt2img_safetensors(&hires_inputs(), &klein_models(), &[]);
    assert_eq!(g["40"]["inputs"]["samples"], json!(["3", 0]));
    assert_eq!(g["41"]["class_type"], "KSampler");
    assert_eq!(g["41"]["inputs"]["cfg"], 1.0);
    assert_eq!(g["41"]["inputs"]["positive"], json!(["26", 0]));
    assert_eq!(
        g["41"]["inputs"]["negative"],
        json!(["27", 0]),
        "still the zeroed-out conditioning, same as the first pass"
    );
    assert_eq!(g["41"]["inputs"]["model"], json!(["12", 0]));
    assert_eq!(g["41"]["inputs"]["denoise"], 0.45);
    assert_eq!(g["41"]["inputs"]["latent_image"], json!(["40", 0]));
    assert_eq!(g["8"]["inputs"]["samples"], json!(["41", 0]));
}

#[test]
fn flux2_klein_gguf_hires_uses_flux2_scheduler_at_the_upscaled_size_and_split_sigmas_denoise() {
    let g = flux2_klein_txt2img(&hires_inputs(), &klein_models(), &[]);
    assert_eq!(
        g["40"],
        json!({
            "class_type": "LatentUpscaleBy",
            "inputs": {
                "samples": ["3", 0],
                "upscale_method": "nearest-exact",
                "scale_by": 1.5
            }
        })
    );
    // Flux2Scheduler has no `denoise` input (ComfyUI v0.34.0,
    // comfy_extras/nodes_flux.py) -- the schedule is cut by
    // SplitSigmasDenoise instead, whose slot 1 is the low-sigma tail. Because
    // of that cut the scheduler is built LONGER than `hires.steps`: 12
    // executed steps at denoise 0.45 need a 12 / 0.45 = 27-step schedule, so
    // that `hires.steps` means the same thing here as it does for KSampler.
    assert_eq!(
        g["42"],
        json!({
            "class_type": "Flux2Scheduler",
            "inputs": { "steps": 27, "width": 1536, "height": 1536 }
        })
    );
    assert_eq!(
        g["43"],
        json!({
            "class_type": "SplitSigmasDenoise",
            "inputs": { "sigmas": ["42", 0], "denoise": 0.45 }
        })
    );
    assert_eq!(
        g["44"],
        json!({
            "class_type": "SamplerCustomAdvanced",
            "inputs": {
                "noise": ["30", 0],
                "guider": ["31", 0],
                "sampler": ["28", 0],
                "sigmas": ["43", 1],
                "latent_image": ["40", 0]
            }
        })
    );
    assert_eq!(g["8"]["inputs"]["samples"], json!(["44", 0]));
    assert_eq!(
        g["29"]["inputs"]["width"], 1024,
        "the first pass's scheduler still runs at the original size"
    );
}

#[test]
fn flux2_klein_gguf_hires_sizes_the_scheduler_from_the_upscaled_latent() {
    let sized = |scale_by: f64| {
        let mut i = hires_inputs();
        i.width = 1000;
        i.height = 1024;
        i.hires = Some(HiresFix {
            scale_by,
            ..hires()
        });
        flux2_klein_txt2img(&i, &klein_models(), &[])
    };

    // LatentUpscaleBy scales the LATENT, so the upscaled image is
    // `round(px / 8 * scale) * 8`: 1000 px = 125 latent units, 125 * 1.25 =
    // 156.25 -> 156 -> 1248 px (not 1250, and not the 1264 a pixel-space
    // formula would give). 1248 is already a multiple of 16, so the
    // scheduler is built for exactly that size. 1024 * 1.25 = 1280 likewise.
    let g = sized(1.25);
    assert_eq!(g["42"]["inputs"]["width"], 1248);
    assert_eq!(g["42"]["inputs"]["height"], 1280);
    assert_eq!(
        g["42"]["inputs"]["width"],
        json!(latent_upscaled_px(1000, 1.25)),
        "same formula the decoded size uses"
    );

    // Where the upscaled size is not a multiple of 16, the schedule rounds
    // UP to the next whole token row -- never fewer tokens than the latent
    // actually has. 125 * 1.35 = 168.75 -> 169 -> 1352 -> 1360;
    // 128 * 1.35 = 172.8 -> 173 -> 1384 -> 1392.
    let g = sized(1.35);
    assert_eq!(g["42"]["inputs"]["width"], 1360);
    assert_eq!(g["42"]["inputs"]["height"], 1392);
}

#[test]
fn hires_none_leaves_the_graph_byte_identical() {
    // The byte-level proof is the golden fixtures, recorded before Hi-Res-Fix
    // existed and unchanged by it. This pins the structural claim in-crate:
    // with `hires: None` no recipe emits anything in the reserved 40-44
    // window, and every decode still reads the first sampler.
    let i = inputs();
    let graphs = [
        checkpoint_txt2img(&i, "sd_xl_base_1.0.safetensors", &[]),
        flux_txt2img(&i, &flux_models(), &[]),
        flux2_klein_txt2img(&i, &klein_models(), &[]),
        flux2_klein_txt2img_safetensors(&i, &klein_models(), &[]),
    ];
    for g in graphs {
        for id in ["40", "41", "42", "43", "44"] {
            assert!(
                g.get(id).is_none(),
                "no hires node {id} without a HiresFix, found {}",
                g[id]
            );
        }
        assert_eq!(g["8"]["inputs"]["samples"], json!(["3", 0]));
    }
}

#[test]
fn loras_apply_to_both_passes() {
    let loras = [
        LoraSpec {
            file: "a.safetensors",
            strength: 1.0,
        },
        LoraSpec {
            file: "b.safetensors",
            strength: 0.5,
        },
    ];
    // KSampler family: the chain's model link is what the first pass reads,
    // and the second pass is handed that same link.
    let g = checkpoint_txt2img(&hires_inputs(), "x.safetensors", &loras);
    assert_eq!(g["3"]["inputs"]["model"], json!(["91", 0]));
    assert_eq!(
        g["41"]["inputs"]["model"],
        json!(["91", 0]),
        "the second pass reads the end of the LoRA chain, not the raw loader"
    );

    // FLUX.1 GGUF: same KSampler family, different loader -- the chain starts
    // at the UnetLoaderGGUF/CLIPLoader pair rather than a checkpoint.
    let g = flux_txt2img(&hires_inputs(), &flux_models(), &loras);
    assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
    assert_eq!(g["3"]["inputs"]["model"], json!(["91", 0]));
    assert_eq!(
        g["41"]["inputs"]["model"],
        json!(["91", 0]),
        "the second pass reads the end of the LoRA chain, not the raw loader"
    );

    // FLUX.2 [klein] .safetensors: the UNETLoader variant, also KSampler.
    let g = flux2_klein_txt2img_safetensors(&hires_inputs(), &klein_models(), &loras);
    assert_eq!(g["90"]["inputs"]["model"], json!(["12", 0]));
    assert_eq!(g["3"]["inputs"]["model"], json!(["91", 0]));
    assert_eq!(
        g["41"]["inputs"]["model"],
        json!(["91", 0]),
        "the second pass reads the end of the LoRA chain, not the raw loader"
    );

    // klein GGUF: the second pass reuses the first chain's CFGGuider, which is
    // itself repointed at the LoRA chain -- so the LoRAs reach it for free.
    let g = flux2_klein_txt2img(&hires_inputs(), &klein_models(), &loras);
    assert_eq!(g["31"]["inputs"]["model"], json!(["91", 0]));
    assert_eq!(g["44"]["inputs"]["guider"], json!(["31", 0]));
}
