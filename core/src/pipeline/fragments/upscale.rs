//! Upscale fragment: NVIDIA's RTX Video Super Resolution node
//! (`Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`) — the node shape from
//! `pipeline::{rtx_upscale_image, rtx_upscale_video}`.

use serde_json::{json, Value};

use crate::pipeline::graph::{Graph, OwnedLink};
use crate::pipeline::UpscaleResize;

/// `RTXVideoSuperResolution` — sharpens, denoises and resizes an image or a
/// whole frame batch (one image and a video's worth of frames are the same
/// node to it). It does not hallucinate new detail the way a diffusion
/// upscaler would. Exact keys from `rtx_upscale_image`.
pub fn rtx_super_resolution(
    g: &mut Graph,
    id: &str,
    images: &OwnedLink,
    quality: &str,
    resize: UpscaleResize,
) -> OwnedLink {
    let mut inputs = json!({ "images": images.json(), "quality": quality });
    set_resize_type(&mut inputs, resize);
    g.node(id, "RTXVideoSuperResolution", inputs);
    OwnedLink::new(id, 0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn scale_by_lands_under_the_dotted_scale_key() {
        let mut g = Graph::default();
        let out = rtx_super_resolution(
            &mut g,
            "2",
            &OwnedLink::new("1", 0),
            "ULTRA",
            UpscaleResize::ScaleBy(2.0),
        );
        assert_eq!(out, OwnedLink::new("2", 0));
        assert_eq!(
            g.into_value()["2"],
            json!({
                "class_type": "RTXVideoSuperResolution",
                "inputs": {
                    "images": ["1", 0],
                    "quality": "ULTRA",
                    "resize_type": "scale by multiplier",
                    "resize_type.scale": 2.0
                }
            })
        );
    }

    #[test]
    fn target_dimensions_land_under_dotted_width_and_height() {
        let mut g = Graph::default();
        rtx_super_resolution(
            &mut g,
            "3",
            &OwnedLink::new("2", 0),
            "HIGH",
            UpscaleResize::Target {
                width: 1920,
                height: 1080,
            },
        );
        assert_eq!(
            g.into_value()["3"],
            json!({
                "class_type": "RTXVideoSuperResolution",
                "inputs": {
                    "images": ["2", 0],
                    "quality": "HIGH",
                    "resize_type": "target dimensions",
                    "resize_type.width": 1920,
                    "resize_type.height": 1080
                }
            })
        );
    }
}
