//! Model-loading fragments: reproduce the loader node shapes from every
//! recipe: `pipeline::{checkpoint_txt2img, flux_txt2img,
//! flux2_klein_txt2img, flux2_klein_txt2img_safetensors, wan_ti2v,
//! ltx_video}`.
//!
//! The single-node fragments (`unet_loader`, `clip_loader`, `vae_loader`)
//! are the primitives; the per-family functions below wire two or three of
//! them into the [`Loaded`] triple a recipe actually consumes.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

/// The model/clip/vae links a loader fragment produces, in whatever output
/// slots each loader class uses.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub model: OwnedLink,
    pub clip: OwnedLink,
    pub vae: OwnedLink,
}

/// `UNETLoader`'s weight dtype: every recipe loads at the file's own
/// precision rather than forcing a cast.
const UNET_WEIGHT_DTYPE: &str = "default";

/// `CLIPLoader`'s device: LTX-Video's own ComfyUI template ships this key
/// explicitly, so its graph carries it while the others do not.
const CLIP_DEVICE_DEFAULT: &str = "default";

/// `CLIPLoader`'s `type` for FLUX.2 \[klein\]'s single Qwen3 encoder.
const CLIP_TYPE_FLUX2: &str = "flux2";

/// `CLIPLoader`'s `type` for the Wan 2.2 umt5 encoder.
const CLIP_TYPE_WAN: &str = "wan";

/// `CLIPLoader`'s `type` for LTX-Video's T5 encoder.
const CLIP_TYPE_LTXV: &str = "ltxv";

/// `UnetLoaderGGUF` (`ComfyUI-GGUF`) — a `.gguf` diffusion model. Exact keys
/// from `flux_txt2img`.
pub fn unet_loader_gguf(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(id, "UnetLoaderGGUF", json!({ "unet_name": file }));
    OwnedLink::new(id, 0)
}

/// `UNETLoader` — core ComfyUI's plain `.safetensors` diffusion-model loader.
/// Exact keys from `flux2_klein_txt2img_safetensors` and `wan_ti2v`.
pub fn unet_loader(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(
        id,
        "UNETLoader",
        json!({ "unet_name": file, "weight_dtype": UNET_WEIGHT_DTYPE }),
    );
    OwnedLink::new(id, 0)
}

/// `CLIPLoader` — a single text encoder of the given `type`. Exact keys from
/// `flux2_klein_txt2img` and `wan_ti2v`.
pub fn clip_loader(g: &mut Graph, id: &str, file: &str, clip_type: &str) -> OwnedLink {
    g.node(
        id,
        "CLIPLoader",
        json!({ "clip_name": file, "type": clip_type }),
    );
    OwnedLink::new(id, 0)
}

/// `CLIPLoader` with an explicit `device` — the shape LTX-Video's own template
/// uses. Exact keys from `ltx_video`.
pub fn clip_loader_on_device(
    g: &mut Graph,
    id: &str,
    file: &str,
    clip_type: &str,
    device: &str,
) -> OwnedLink {
    g.node(
        id,
        "CLIPLoader",
        json!({ "clip_name": file, "type": clip_type, "device": device }),
    );
    OwnedLink::new(id, 0)
}

/// `VAELoader` — a standalone VAE file. Exact keys from `flux_txt2img`.
pub fn vae_loader(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(id, "VAELoader", json!({ "vae_name": file }));
    OwnedLink::new(id, 0)
}

/// `CheckpointLoaderSimple`: one `.safetensors` file, model/clip/vae in
/// output slots 0/1/2. Exact keys from `checkpoint_txt2img`.
pub fn checkpoint(g: &mut Graph, id: &str, file: &str) -> Loaded {
    g.node(id, "CheckpointLoaderSimple", json!({ "ckpt_name": file }));
    Loaded {
        model: OwnedLink::new(id, 0),
        clip: OwnedLink::new(id, 1),
        vae: OwnedLink::new(id, 2),
    }
}

/// Explicit node ids for a family whose model, text encoder and VAE load from
/// three separate files — callers own id allocation so a fragment reproduces
/// whatever ids an existing recipe uses.
#[derive(Debug, Clone, Copy)]
pub struct SplitModelIds<'a> {
    pub unet: &'a str,
    pub clip: &'a str,
    pub vae: &'a str,
}

/// FLUX.1 via `ComfyUI-GGUF`: `UnetLoaderGGUF` + `DualCLIPLoaderGGUF`
/// (`type: "flux"`, T5 as `clip_name1` + CLIP-L as `clip_name2`) +
/// `VAELoader`. Exact keys from `flux_txt2img`.
pub fn flux_gguf(
    g: &mut Graph,
    ids: &SplitModelIds,
    unet_file: &str,
    clip_l_file: &str,
    t5_file: &str,
    vae_file: &str,
) -> Loaded {
    let model = unet_loader_gguf(g, ids.unet, unet_file);
    g.node(
        ids.clip,
        "DualCLIPLoaderGGUF",
        json!({ "clip_name1": t5_file, "clip_name2": clip_l_file, "type": "flux" }),
    );
    let vae = vae_loader(g, ids.vae, vae_file);
    Loaded {
        model,
        clip: OwnedLink::new(ids.clip, 0),
        vae,
    }
}

/// FLUX.2 \[klein\] via `ComfyUI-GGUF`: `UnetLoaderGGUF` + a single
/// `CLIPLoader` (`type: "flux2"`) + `VAELoader`. Exact keys from
/// `flux2_klein_txt2img`.
pub fn flux2_klein_gguf(
    g: &mut Graph,
    ids: &SplitModelIds,
    unet_file: &str,
    clip_file: &str,
    vae_file: &str,
) -> Loaded {
    Loaded {
        model: unet_loader_gguf(g, ids.unet, unet_file),
        clip: clip_loader(g, ids.clip, clip_file, CLIP_TYPE_FLUX2),
        vae: vae_loader(g, ids.vae, vae_file),
    }
}

/// FLUX.2 \[klein\] from a plain `.safetensors` checkpoint: `UNETLoader`
/// (`weight_dtype: "default"`, no custom node) + the same single-encoder
/// `CLIPLoader` (`type: "flux2"`) + `VAELoader`. Exact keys from
/// `flux2_klein_txt2img_safetensors`.
pub fn flux2_klein_safetensors(
    g: &mut Graph,
    ids: &SplitModelIds,
    model_file: &str,
    clip_file: &str,
    vae_file: &str,
) -> Loaded {
    Loaded {
        model: unet_loader(g, ids.unet, model_file),
        clip: clip_loader(g, ids.clip, clip_file, CLIP_TYPE_FLUX2),
        vae: vae_loader(g, ids.vae, vae_file),
    }
}

/// Wan 2.2 TI2V: `UNETLoader` + `CLIPLoader` (`type: "wan"`, the umt5
/// encoder) + `VAELoader`. Exact keys from `wan_ti2v`.
pub fn wan(
    g: &mut Graph,
    ids: &SplitModelIds,
    unet_file: &str,
    clip_file: &str,
    vae_file: &str,
) -> Loaded {
    Loaded {
        model: unet_loader(g, ids.unet, unet_file),
        clip: clip_loader(g, ids.clip, clip_file, CLIP_TYPE_WAN),
        vae: vae_loader(g, ids.vae, vae_file),
    }
}

/// LTX-Video 0.9.x: one `CheckpointLoaderSimple` carrying the diffusion model
/// (slot 0) **and** the VAE (slot 2), plus a separate `CLIPLoader`
/// (`type: "ltxv"`) for the T5 — so the CLIP link comes from that second
/// node, not from the checkpoint's own slot 1. Exact keys from `ltx_video`.
pub fn ltx(
    g: &mut Graph,
    checkpoint_id: &str,
    clip_id: &str,
    checkpoint_file: &str,
    t5_file: &str,
) -> Loaded {
    let bundled = checkpoint(g, checkpoint_id, checkpoint_file);
    let clip = clip_loader_on_device(g, clip_id, t5_file, CLIP_TYPE_LTXV, CLIP_DEVICE_DEFAULT);
    Loaded {
        model: bundled.model,
        clip,
        vae: bundled.vae,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn checkpoint_loader_emits_the_node_and_three_links() {
        let mut g = Graph::default();
        let loaded = checkpoint(&mut g, "4", "sd_xl_base_1.0.safetensors");
        assert_eq!(loaded.model, OwnedLink::new("4", 0));
        assert_eq!(loaded.clip, OwnedLink::new("4", 1));
        assert_eq!(loaded.vae, OwnedLink::new("4", 2));
        assert_eq!(
            g.input("4", "ckpt_name"),
            Some(&json!("sd_xl_base_1.0.safetensors"))
        );
    }

    #[test]
    fn flux_gguf_loader_wires_dual_clip_with_t5_first() {
        let mut g = Graph::default();
        let ids = SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        };
        let loaded = flux_gguf(
            &mut g,
            &ids,
            "flux1-dev-Q8_0.gguf",
            "clip_l.safetensors",
            "t5xxl_fp8_e4m3fn.safetensors",
            "ae.safetensors",
        );
        assert_eq!(loaded.model, OwnedLink::new("12", 0));
        assert_eq!(loaded.clip, OwnedLink::new("11", 0));
        assert_eq!(loaded.vae, OwnedLink::new("10", 0));
        assert_eq!(
            g.input("11", "clip_name1"),
            Some(&json!("t5xxl_fp8_e4m3fn.safetensors"))
        );
        assert_eq!(
            g.input("11", "clip_name2"),
            Some(&json!("clip_l.safetensors"))
        );
        assert_eq!(g.input("11", "type"), Some(&json!("flux")));
        assert_eq!(g.input("10", "vae_name"), Some(&json!("ae.safetensors")));
    }

    #[test]
    fn flux2_klein_gguf_loader_uses_single_clip_loader() {
        let mut g = Graph::default();
        let ids = SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        };
        let loaded = flux2_klein_gguf(
            &mut g,
            &ids,
            "flux-2-klein-9b-Q4_K_M.gguf",
            "qwen3.safetensors",
            "flux2_vae.safetensors",
        );
        assert_eq!(loaded.model, OwnedLink::new("12", 0));
        assert_eq!(loaded.clip, OwnedLink::new("11", 0));
        assert_eq!(loaded.vae, OwnedLink::new("10", 0));
        assert_eq!(
            g.input("11", "clip_name"),
            Some(&json!("qwen3.safetensors"))
        );
        assert_eq!(g.input("11", "type"), Some(&json!("flux2")));
    }

    #[test]
    fn wan_loader_wires_unet_umt5_clip_and_vae() {
        let mut g = Graph::default();
        let ids = SplitModelIds {
            unet: "37",
            clip: "38",
            vae: "39",
        };
        let loaded = wan(
            &mut g,
            &ids,
            "wan2.2_ti2v_5B_fp16.safetensors",
            "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
            "wan2.2_vae.safetensors",
        );
        assert_eq!(loaded.model, OwnedLink::new("37", 0));
        assert_eq!(loaded.clip, OwnedLink::new("38", 0));
        assert_eq!(loaded.vae, OwnedLink::new("39", 0));
        assert_eq!(g.input("37", "weight_dtype"), Some(&json!("default")));
        assert_eq!(g.input("38", "type"), Some(&json!("wan")));
        assert_eq!(
            g.input("39", "vae_name"),
            Some(&json!("wan2.2_vae.safetensors"))
        );
    }

    #[test]
    fn ltx_loader_takes_its_vae_from_the_checkpoint_and_its_clip_from_the_t5() {
        let mut g = Graph::default();
        let loaded = ltx(
            &mut g,
            "44",
            "38",
            "ltx-video-2b-v0.9.5.safetensors",
            "t5xxl_fp8_e4m3fn.safetensors",
        );
        assert_eq!(loaded.model, OwnedLink::new("44", 0));
        assert_eq!(loaded.vae, OwnedLink::new("44", 2));
        assert_eq!(loaded.clip, OwnedLink::new("38", 0));
        assert_eq!(g.input("38", "type"), Some(&json!("ltxv")));
        assert_eq!(g.input("38", "device"), Some(&json!("default")));
    }

    #[test]
    fn flux2_klein_safetensors_loader_uses_unet_loader() {
        let mut g = Graph::default();
        let ids = SplitModelIds {
            unet: "12",
            clip: "11",
            vae: "10",
        };
        let loaded = flux2_klein_safetensors(
            &mut g,
            &ids,
            "flux-2-klein-9b-fp8.safetensors",
            "qwen3.safetensors",
            "flux2_vae.safetensors",
        );
        assert_eq!(loaded.model, OwnedLink::new("12", 0));
        assert_eq!(
            g.input("12", "unet_name"),
            Some(&json!("flux-2-klein-9b-fp8.safetensors"))
        );
        assert_eq!(g.input("12", "weight_dtype"), Some(&json!("default")));
    }
}
