//! Golden JSON fixtures for every current `pipeline` recipe.
//!
//! This is the safety net for the ComfyUI workflow engine refactor (Plan 3):
//! before `core/src/pipeline/mod.rs` is torn apart into a graph builder plus
//! composable fragments, every recipe's *current* output is pinned here. The
//! refactor is only allowed to proceed once every one of these fixtures still
//! matches byte-for-byte (after normalisation — see below).
//!
//! Run with `AIWM_WRITE_GOLDENS=1` to (re)write the fixtures from whatever the
//! pipeline code currently produces — see `core/tests/fixtures/graphs/README.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use aiwm_core::pipeline::{
    checkpoint_ipadapter_txt2img, checkpoint_txt2img, flux2_klein_edit,
    flux2_klein_reference_txt2img, flux2_klein_reference_txt2img_safetensors, flux2_klein_txt2img,
    flux2_klein_txt2img_safetensors, flux_txt2img, ltx_video, rtx_upscale_image, rtx_upscale_video,
    wan_ti2v, EditInputs, Flux2KleinModels, FluxModels, IpAdapterSpec, LoraSpec, LtxModels,
    Txt2ImgInputs, UpscaleImageInputs, UpscaleResize, UpscaleVideoInputs, VideoInputs, WanModels,
};
use serde_json::Value;

// --- fixed inputs, shared by every fixture --------------------------------------

const SEED: i64 = 4242;
const WIDTH: u32 = 1024;
const HEIGHT: u32 = 1024;
const STEPS: u32 = 24;
const CFG: f64 = 6.5;
const SAMPLER: &str = "dpmpp_2m";
const SCHEDULER: &str = "karras";
const POSITIVE: &str = "a lighthouse at dawn, painterly";
const NEGATIVE: &str = "blurry, watermark";
const FILENAME_PREFIX: &str = "golden";

fn txt2img_inputs() -> Txt2ImgInputs<'static> {
    Txt2ImgInputs {
        positive: POSITIVE,
        negative: NEGATIVE,
        width: WIDTH,
        height: HEIGHT,
        steps: STEPS,
        cfg: CFG,
        sampler: SAMPLER,
        scheduler: SCHEDULER,
        seed: SEED,
        filename_prefix: FILENAME_PREFIX,
    }
}

fn checkpoint_loras() -> Vec<LoraSpec<'static>> {
    vec![
        LoraSpec {
            file: "add-detail-xl.safetensors",
            strength: 0.8,
        },
        LoraSpec {
            file: "film-grain.safetensors",
            strength: 0.5,
        },
    ]
}

fn one_lora(file: &'static str) -> Vec<LoraSpec<'static>> {
    vec![LoraSpec {
        file,
        strength: 0.8,
    }]
}

fn flux_models() -> FluxModels<'static> {
    FluxModels {
        unet: "flux1-dev-Q8_0.gguf",
        t5: "t5xxl_fp8_e4m3fn.safetensors",
        clip_l: "clip_l.safetensors",
        vae: "ae.safetensors",
    }
}

fn flux2_klein_gguf_models() -> Flux2KleinModels<'static> {
    Flux2KleinModels {
        unet: "flux-2-klein-9b-Q4_K_M.gguf",
        clip: "qwen_3_8b_fp8mixed.safetensors",
        vae: "flux2-vae.safetensors",
    }
}

fn flux2_klein_safetensors_models() -> Flux2KleinModels<'static> {
    Flux2KleinModels {
        unet: "flux-2-klein-9b-fp8mixed.safetensors",
        clip: "qwen_3_8b_fp8mixed.safetensors",
        vae: "flux2-vae.safetensors",
    }
}

fn edit_inputs() -> EditInputs<'static> {
    EditInputs {
        instruction: POSITIVE,
        source_image: "golden-source.png",
        steps: STEPS,
        cfg: CFG,
        sampler: SAMPLER,
        seed: SEED,
        filename_prefix: FILENAME_PREFIX,
    }
}

fn ipadapter_spec() -> IpAdapterSpec<'static> {
    IpAdapterSpec {
        clip_vision: "CLIP-ViT-H-14-laion2B-s32B-b79K.safetensors",
        ipadapter_model: "ip-adapter-plus_sdxl_vit-h.safetensors",
        reference_image: "golden-portrait.png",
        weight: 0.8,
    }
}

fn video_inputs(start_image: Option<&'static str>) -> VideoInputs<'static> {
    VideoInputs {
        positive: POSITIVE,
        negative: NEGATIVE,
        width: WIDTH,
        height: HEIGHT,
        length: 81,
        fps: 24,
        steps: STEPS,
        cfg: CFG,
        seed: SEED,
        start_image,
        filename_prefix: FILENAME_PREFIX,
    }
}

fn wan_models() -> WanModels<'static> {
    WanModels {
        unet: "wan2.2_ti2v_5B_fp16.safetensors",
        clip: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
        vae: "wan2.2_vae.safetensors",
    }
}

fn ltx_models() -> LtxModels<'static> {
    LtxModels {
        checkpoint: "ltx-video-2b-v0.9.5.safetensors",
        t5: "t5xxl_fp8_e4m3fn.safetensors",
    }
}

// --- normalisation + diff helpers ------------------------------------------------

/// Recursively sort object keys (arrays keep their order — link pairs like
/// `["4", 0]` are position-sensitive) so fixture comparison isn't sensitive to
/// `serde_json`'s incidental map ordering.
fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> =
                map.iter().map(|(k, v)| (k.clone(), normalize(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        other => other.clone(),
    }
}

/// Find the first point where `actual` and `expected` diverge, reported as a
/// JSON-pointer-shaped path (`/12/inputs/model`), so a mismatch is readable
/// without diffing the whole (potentially large) graph by eye.
fn first_difference(actual: &Value, expected: &Value) -> Option<String> {
    first_difference_at(actual, expected, String::new())
}

fn first_difference_at(actual: &Value, expected: &Value, pointer: String) -> Option<String> {
    match (actual, expected) {
        (Value::Object(am), Value::Object(em)) => {
            let mut keys: Vec<&String> = am.keys().chain(em.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let next = format!("{pointer}/{key}");
                match (am.get(key), em.get(key)) {
                    (Some(a), Some(e)) => {
                        if let Some(diff) = first_difference_at(a, e, next) {
                            return Some(diff);
                        }
                    }
                    (Some(a), None) => {
                        return Some(format!(
                            "{next}: present in actual ({a}) but missing in expected"
                        ))
                    }
                    (None, Some(e)) => {
                        return Some(format!(
                            "{next}: missing in actual but present in expected ({e})"
                        ))
                    }
                    (None, None) => unreachable!("key came from one of the two maps"),
                }
            }
            None
        }
        (Value::Array(aa), Value::Array(ea)) => {
            if aa.len() != ea.len() {
                return Some(format!(
                    "{pointer}: array length differs (actual {}, expected {})",
                    aa.len(),
                    ea.len()
                ));
            }
            aa.iter()
                .zip(ea.iter())
                .enumerate()
                .find_map(|(idx, (a, e))| first_difference_at(a, e, format!("{pointer}/{idx}")))
        }
        _ => {
            if actual == expected {
                None
            } else {
                Some(format!("{pointer}: actual {actual} != expected {expected}"))
            }
        }
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/graphs")
        .join(format!("{name}.json"))
}

/// Compare `actual` against the fixture named `name`, or (with
/// `AIWM_WRITE_GOLDENS=1`) write `actual` as the new fixture.
fn check_golden(name: &str, actual: &Value) {
    let path = fixture_path(name);
    let normalized_actual = normalize(actual);

    if env::var("AIWM_WRITE_GOLDENS").as_deref() == Ok("1") {
        let mut pretty = serde_json::to_string_pretty(&normalized_actual)
            .expect("golden graphs serialize to JSON");
        pretty.push('\n');
        fs::write(&path, pretty)
            .unwrap_or_else(|e| panic!("failed to write fixture {}: {e}", path.display()));
        return;
    }

    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read fixture {}: {e}\n\
             (run `AIWM_WRITE_GOLDENS=1 cargo test -p aiwm-core --test pipeline_goldens` to create it)",
            path.display()
        )
    });
    let expected: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse fixture {} as JSON: {e}", path.display()));
    let normalized_expected = normalize(&expected);

    if let Some(diff) = first_difference(&normalized_actual, &normalized_expected) {
        panic!(
            "golden mismatch for `{name}` against {path}\n\
             first difference at JSON pointer: {diff}\n\
             if this is an intentional behaviour change, re-run with \
             AIWM_WRITE_GOLDENS=1 and commit the updated fixture alongside the change.",
            path = path.display()
        );
    }
}

// --- fixtures ---------------------------------------------------------------------

#[test]
fn checkpoint_txt2img_golden() {
    let g = checkpoint_txt2img(
        &txt2img_inputs(),
        "sd_xl_base_1.0.safetensors",
        &checkpoint_loras(),
    );
    check_golden("checkpoint_txt2img", &g);
}

/// The LoRA-free shape of the same recipe: a chain that is *absent* is a
/// different graph from one that is present, and only the with-LoRAs fixture
/// above pinned it before.
#[test]
fn checkpoint_txt2img_no_loras_golden() {
    let g = checkpoint_txt2img(&txt2img_inputs(), "sd_xl_base_1.0.safetensors", &[]);
    check_golden("checkpoint_txt2img_no_loras", &g);
}

#[test]
fn flux_txt2img_golden() {
    let g = flux_txt2img(
        &txt2img_inputs(),
        &flux_models(),
        &one_lora("flux-realistic-detail.safetensors"),
    );
    check_golden("flux_txt2img", &g);
}

#[test]
fn flux2_klein_txt2img_golden() {
    let g = flux2_klein_txt2img(
        &txt2img_inputs(),
        &flux2_klein_gguf_models(),
        &one_lora("flux2-realistic-detail.safetensors"),
    );
    check_golden("flux2_klein_txt2img", &g);
}

#[test]
fn flux2_klein_txt2img_safetensors_golden() {
    let g = flux2_klein_txt2img_safetensors(
        &txt2img_inputs(),
        &flux2_klein_safetensors_models(),
        &one_lora("flux2-realistic-detail.safetensors"),
    );
    check_golden("flux2_klein_txt2img_safetensors", &g);
}

#[test]
fn flux2_klein_edit_golden() {
    let g = flux2_klein_edit(
        &edit_inputs(),
        &flux2_klein_safetensors_models(),
        &one_lora("flux2-realistic-detail.safetensors"),
    );
    check_golden("flux2_klein_edit", &g);
}

/// The edit graph without a LoRA chain — the most branch-heavy recipe, and
/// the one where a mis-spliced chain would be hardest to spot by eye.
#[test]
fn flux2_klein_edit_no_loras_golden() {
    let g = flux2_klein_edit(&edit_inputs(), &flux2_klein_safetensors_models(), &[]);
    check_golden("flux2_klein_edit_no_loras", &g);
}

#[test]
fn checkpoint_ipadapter_txt2img_golden() {
    let g = checkpoint_ipadapter_txt2img(
        &txt2img_inputs(),
        "sd_xl_base_1.0.safetensors",
        &ipadapter_spec(),
        &one_lora("add-detail-xl.safetensors"),
    );
    check_golden("checkpoint_ipadapter_txt2img", &g);
}

#[test]
fn flux2_klein_reference_txt2img_golden() {
    let g = flux2_klein_reference_txt2img(
        &txt2img_inputs(),
        &flux2_klein_gguf_models(),
        "golden-reference.png",
        &one_lora("flux2-realistic-detail.safetensors"),
    );
    check_golden("flux2_klein_reference_txt2img", &g);
}

#[test]
fn flux2_klein_reference_txt2img_safetensors_golden() {
    let g = flux2_klein_reference_txt2img_safetensors(
        &txt2img_inputs(),
        &flux2_klein_safetensors_models(),
        "golden-reference.png",
        &one_lora("flux2-realistic-detail.safetensors"),
    );
    check_golden("flux2_klein_reference_txt2img_safetensors", &g);
}

#[test]
fn wan_ti2v_text_to_video_golden() {
    let g = wan_ti2v(
        &video_inputs(None),
        &wan_models(),
        &one_lora("wan-detail-enhancer.safetensors"),
    );
    check_golden("wan_ti2v_t2v", &g);
}

#[test]
fn wan_ti2v_image_to_video_golden() {
    let g = wan_ti2v(
        &video_inputs(Some("golden-start.png")),
        &wan_models(),
        &one_lora("wan-detail-enhancer.safetensors"),
    );
    check_golden("wan_ti2v_i2v", &g);
}

#[test]
fn ltx_video_text_to_video_golden() {
    let g = ltx_video(
        &video_inputs(None),
        &ltx_models(),
        &one_lora("ltx-detail-enhancer.safetensors"),
    );
    check_golden("ltx_video_t2v", &g);
}

#[test]
fn ltx_video_image_to_video_golden() {
    let g = ltx_video(
        &video_inputs(Some("golden-start.png")),
        &ltx_models(),
        &one_lora("ltx-detail-enhancer.safetensors"),
    );
    check_golden("ltx_video_i2v", &g);
}

#[test]
fn rtx_upscale_image_golden() {
    let g = rtx_upscale_image(&UpscaleImageInputs {
        source_image: "golden-source.png",
        resize: UpscaleResize::ScaleBy(2.0),
        quality: "ULTRA",
        filename_prefix: FILENAME_PREFIX,
    });
    check_golden("rtx_upscale_image", &g);
}

#[test]
fn rtx_upscale_video_golden() {
    let g = rtx_upscale_video(&UpscaleVideoInputs {
        source_video: "golden-source.mp4",
        resize: UpscaleResize::Target {
            width: 1920,
            height: 1080,
        },
        quality: "ULTRA",
        filename_prefix: FILENAME_PREFIX,
    });
    check_golden("rtx_upscale_video", &g);
}
