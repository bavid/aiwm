//! Unit tests for the RTX upscale recipes, moved unchanged in intent from
//! `pipeline::tests` when the recipe bodies moved onto the fragment layer.
//! They assert node ids and the dotted `DynamicCombo` wire keys; the golden
//! fixtures in `core/tests/pipeline_goldens.rs` assert each whole graph.

use serde_json::json;

use super::*;
use crate::pipeline::{UpscaleImageInputs, UpscaleResize, UpscaleVideoInputs};

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
    // `fragments::upscale::set_resize_type`'s doc comment for why it must be
    // dotted.
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
