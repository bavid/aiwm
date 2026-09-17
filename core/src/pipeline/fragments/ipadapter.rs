//! IP-Adapter fragments (`ComfyUI_IPAdapter_plus` plus core ComfyUI's
//! `CLIPVisionLoader`) — the node shapes from
//! `pipeline::checkpoint_ipadapter_txt2img`.
//!
//! These are the *explicit-file* loaders, never `IPAdapterUnifiedLoader`: the
//! unified loader's `preset` resolves to a file by matching hard-coded name
//! patterns (`get_clipvision_file` / `get_ipadapter_file` in the node pack's
//! `utils.py`), which silently breaks the moment a user imports either file
//! under a different name. Passing both bare file names explicitly is one
//! extra node but never depends on a naming convention outside AIWM's control.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

/// `IPAdapterAdvanced`'s fixed knobs. These are the node's own defaults for a
/// plain "steer this render with a reference portrait" application; only
/// `weight` is exposed to callers, because it is the only one Story Studio
/// has a reason to vary.
const WEIGHT_TYPE: &str = "linear";
const COMBINE_EMBEDS: &str = "concat";
const START_AT: f64 = 0.0;
const END_AT: f64 = 1.0;
const EMBEDS_SCALING: &str = "V only";

/// `CLIPVisionLoader` — core ComfyUI's vision encoder loader, by bare file
/// name. Exact keys from `checkpoint_ipadapter_txt2img`.
pub fn clip_vision_loader(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(id, "CLIPVisionLoader", json!({ "clip_name": file }));
    OwnedLink::new(id, 0)
}

/// `IPAdapterModelLoader` — the IP-Adapter weights themselves, by bare file
/// name. Exact keys from `checkpoint_ipadapter_txt2img`.
pub fn model_loader(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(
        id,
        "IPAdapterModelLoader",
        json!({ "ipadapter_file": file }),
    );
    OwnedLink::new(id, 0)
}

/// Everything `IPAdapterAdvanced` needs besides the model it patches.
#[derive(Debug, Clone, Copy)]
pub struct AdvancedInputs<'a> {
    pub ipadapter: &'a OwnedLink,
    pub image: &'a OwnedLink,
    pub clip_vision: &'a OwnedLink,
    /// The node's own default is `1.0`; the pack's README recommends lowering
    /// it (it suggests "at least 0.8") for better prompt adherence, so callers
    /// default there rather than to 1.0.
    pub weight: f64,
}

/// `IPAdapterAdvanced` — patches `model` so the sampler renders in the
/// reference image's likeness. Verified against the real
/// `ComfyUI_IPAdapter_plus` node source (`cubiq/ComfyUI_IPAdapter_plus`,
/// commit `a0f451a`): `apply_ipadapter` accepts a raw `IPADAPTER` model (not
/// just the unified loader's `{clipvision, ipadapter, insightface}` dict) as
/// long as a `clip_vision` is also supplied on its optional input — exactly
/// the shape built here. Exact keys from `checkpoint_ipadapter_txt2img`.
pub fn advanced(g: &mut Graph, id: &str, model: &OwnedLink, a: &AdvancedInputs) -> OwnedLink {
    g.node(
        id,
        "IPAdapterAdvanced",
        json!({
            "model": model.json(),
            "ipadapter": a.ipadapter.json(),
            "image": a.image.json(),
            "clip_vision": a.clip_vision.json(),
            "weight": a.weight,
            "weight_type": WEIGHT_TYPE,
            "combine_embeds": COMBINE_EMBEDS,
            "start_at": START_AT,
            "end_at": END_AT,
            "embeds_scaling": EMBEDS_SCALING
        }),
    );
    OwnedLink::new(id, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn the_two_explicit_loaders_take_bare_file_names() {
        let mut g = Graph::default();
        let vision = clip_vision_loader(&mut g, "40", "CLIP-ViT-H-14.safetensors");
        let model = model_loader(&mut g, "41", "ip-adapter-plus_sdxl_vit-h.safetensors");
        assert_eq!(vision, OwnedLink::new("40", 0));
        assert_eq!(model, OwnedLink::new("41", 0));
        let v = g.into_value();
        assert_eq!(
            v["40"],
            json!({
                "class_type": "CLIPVisionLoader",
                "inputs": { "clip_name": "CLIP-ViT-H-14.safetensors" }
            })
        );
        assert_eq!(
            v["41"],
            json!({
                "class_type": "IPAdapterModelLoader",
                "inputs": { "ipadapter_file": "ip-adapter-plus_sdxl_vit-h.safetensors" }
            })
        );
    }

    #[test]
    fn advanced_patches_the_model_and_pins_the_node_defaults() {
        let mut g = Graph::default();
        let model = OwnedLink::new("4", 0);
        let ipadapter = OwnedLink::new("41", 0);
        let image = OwnedLink::new("42", 0);
        let clip_vision = OwnedLink::new("40", 0);
        let out = advanced(
            &mut g,
            "43",
            &model,
            &AdvancedInputs {
                ipadapter: &ipadapter,
                image: &image,
                clip_vision: &clip_vision,
                weight: 0.8,
            },
        );
        assert_eq!(out, OwnedLink::new("43", 0));
        assert_eq!(
            g.into_value()["43"],
            json!({
                "class_type": "IPAdapterAdvanced",
                "inputs": {
                    "model": ["4", 0],
                    "ipadapter": ["41", 0],
                    "image": ["42", 0],
                    "clip_vision": ["40", 0],
                    "weight": 0.8,
                    "weight_type": "linear",
                    "combine_embeds": "concat",
                    "start_at": 0.0,
                    "end_at": 1.0,
                    "embeds_scaling": "V only"
                }
            })
        );
    }
}
