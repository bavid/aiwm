//! Model-loading fragments: reproduce the loader node shapes from
//! `pipeline::{checkpoint_txt2img, flux_txt2img, flux2_klein_txt2img,
//! flux2_klein_txt2img_safetensors}`.

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

/// Explicit node ids for the three FLUX-family loaders — callers own id
/// allocation so a fragment reproduces whatever ids an existing recipe uses.
#[derive(Debug, Clone, Copy)]
pub struct FluxGgufIds<'a> {
    pub unet: &'a str,
    pub clip: &'a str,
    pub vae: &'a str,
}

/// FLUX.1 via `ComfyUI-GGUF`: `UnetLoaderGGUF` + `DualCLIPLoaderGGUF`
/// (`type: "flux"`, T5 as `clip_name1` + CLIP-L as `clip_name2`) +
/// `VAELoader`. Exact keys from `flux_txt2img`.
pub fn flux_gguf(
    g: &mut Graph,
    ids: &FluxGgufIds,
    unet_file: &str,
    clip_l_file: &str,
    t5_file: &str,
    vae_file: &str,
) -> Loaded {
    g.node(
        ids.unet,
        "UnetLoaderGGUF",
        json!({ "unet_name": unet_file }),
    );
    g.node(
        ids.clip,
        "DualCLIPLoaderGGUF",
        json!({ "clip_name1": t5_file, "clip_name2": clip_l_file, "type": "flux" }),
    );
    g.node(ids.vae, "VAELoader", json!({ "vae_name": vae_file }));
    Loaded {
        model: OwnedLink::new(ids.unet, 0),
        clip: OwnedLink::new(ids.clip, 0),
        vae: OwnedLink::new(ids.vae, 0),
    }
}

/// FLUX.2 \[klein\] via `ComfyUI-GGUF`: `UnetLoaderGGUF` + a single
/// `CLIPLoader` (`type: "flux2"`) + `VAELoader`. Exact keys from
/// `flux2_klein_txt2img`.
pub fn flux2_klein_gguf(
    g: &mut Graph,
    ids: &FluxGgufIds,
    unet_file: &str,
    clip_file: &str,
    vae_file: &str,
) -> Loaded {
    g.node(
        ids.unet,
        "UnetLoaderGGUF",
        json!({ "unet_name": unet_file }),
    );
    g.node(
        ids.clip,
        "CLIPLoader",
        json!({ "clip_name": clip_file, "type": "flux2" }),
    );
    g.node(ids.vae, "VAELoader", json!({ "vae_name": vae_file }));
    Loaded {
        model: OwnedLink::new(ids.unet, 0),
        clip: OwnedLink::new(ids.clip, 0),
        vae: OwnedLink::new(ids.vae, 0),
    }
}

/// FLUX.2 \[klein\] from a plain `.safetensors` checkpoint: `UNETLoader`
/// (`weight_dtype: "default"`, no custom node) + the same single-encoder
/// `CLIPLoader` (`type: "flux2"`) + `VAELoader`. Exact keys from
/// `flux2_klein_txt2img_safetensors`.
pub fn flux2_klein_safetensors(
    g: &mut Graph,
    ids: &FluxGgufIds,
    model_file: &str,
    clip_file: &str,
    vae_file: &str,
) -> Loaded {
    g.node(
        ids.unet,
        "UNETLoader",
        json!({ "unet_name": model_file, "weight_dtype": "default" }),
    );
    g.node(
        ids.clip,
        "CLIPLoader",
        json!({ "clip_name": clip_file, "type": "flux2" }),
    );
    g.node(ids.vae, "VAELoader", json!({ "vae_name": vae_file }));
    Loaded {
        model: OwnedLink::new(ids.unet, 0),
        clip: OwnedLink::new(ids.clip, 0),
        vae: OwnedLink::new(ids.vae, 0),
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
        let ids = FluxGgufIds {
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
        let ids = FluxGgufIds {
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
    fn flux2_klein_safetensors_loader_uses_unet_loader() {
        let mut g = Graph::default();
        let ids = FluxGgufIds {
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
