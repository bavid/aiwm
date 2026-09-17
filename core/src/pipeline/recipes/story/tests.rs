//! Unit tests for the Story Studio recipes, moved unchanged in intent from
//! `pipeline::tests` when the recipe bodies moved onto the fragment layer.
//! They assert node ids, wiring and LoRA chains; the golden fixtures in
//! `core/tests/pipeline_goldens.rs` assert each whole graph.

use serde_json::json;

use super::*;
use crate::pipeline::{Flux2KleinModels, IpAdapterSpec, LoraSpec, Txt2ImgInputs};

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
