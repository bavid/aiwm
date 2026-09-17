//! Unit tests for the video recipes, moved unchanged in intent from
//! `pipeline::tests` when the recipe bodies moved onto the fragment layer.
//! They assert node ids, wiring and LoRA chains; the golden fixtures in
//! `core/tests/pipeline_goldens.rs` assert each whole graph.

use serde_json::json;

use super::*;
use crate::pipeline::{LoraSpec, LtxModels, VideoInputs, WanModels};

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

fn ltx_models() -> LtxModels<'static> {
    LtxModels {
        checkpoint: "ltx-video-2b-v0.9.5.safetensors",
        t5: "t5xxl_fp8_e4m3fn.safetensors",
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
