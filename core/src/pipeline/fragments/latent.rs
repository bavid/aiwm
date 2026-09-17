//! Latent-image fragments: `EmptyLatentImage` / `EmptySD3LatentImage` /
//! `EmptyFlux2LatentImage` / `EmptyLTXVLatentVideo` / `LatentUpscaleBy`.

use serde_json::json;

use crate::pipeline::graph::{Dim, Graph, OwnedLink};

/// `EmptyLatentImage` — the plain-checkpoint recipe's latent, batch size 1.
/// Exact keys from `checkpoint_txt2img`.
pub fn empty(g: &mut Graph, id: &str, width: u32, height: u32) -> OwnedLink {
    g.node(
        id,
        "EmptyLatentImage",
        json!({ "width": width, "height": height, "batch_size": 1 }),
    );
    OwnedLink::new(id, 0)
}

/// `EmptySD3LatentImage` — FLUX.1's SD3-format latent, batch size 1. Exact
/// keys from `flux_txt2img`.
pub fn empty_sd3(g: &mut Graph, id: &str, width: u32, height: u32) -> OwnedLink {
    g.node(
        id,
        "EmptySD3LatentImage",
        json!({ "width": width, "height": height, "batch_size": 1 }),
    );
    OwnedLink::new(id, 0)
}

/// `EmptyFlux2LatentImage` — FLUX.2 \[klein\]'s latent, batch size 1. Exact
/// keys from `flux2_klein_txt2img`. Takes [`Dim`]s rather than plain numbers
/// because `flux2_klein_edit` sizes its output canvas from a `GetImageSize`
/// node instead of from fixed inputs.
pub fn empty_flux2(
    g: &mut Graph,
    id: &str,
    width: impl Into<Dim>,
    height: impl Into<Dim>,
) -> OwnedLink {
    g.node(
        id,
        "EmptyFlux2LatentImage",
        json!({
            "width": width.into().json(),
            "height": height.into().json(),
            "batch_size": 1
        }),
    );
    OwnedLink::new(id, 0)
}

/// `EmptyLTXVLatentVideo` — LTX-Video's empty *video* latent: a width/height
/// plus a frame count, batch size 1. Exact keys from `ltx_video`.
pub fn empty_ltxv(g: &mut Graph, id: &str, width: u32, height: u32, length: u32) -> OwnedLink {
    g.node(
        id,
        "EmptyLTXVLatentVideo",
        json!({ "width": width, "height": height, "length": length, "batch_size": 1 }),
    );
    OwnedLink::new(id, 0)
}

/// `VAEEncode` — a real image back into a latent, the edit graph's way of
/// starting from the source picture rather than from noise. Exact keys from
/// `flux2_klein_edit`.
pub fn vae_encode(g: &mut Graph, id: &str, pixels: &OwnedLink, vae: &OwnedLink) -> OwnedLink {
    g.node(
        id,
        "VAEEncode",
        json!({ "pixels": pixels.json(), "vae": vae.json() }),
    );
    OwnedLink::new(id, 0)
}

/// `LatentUpscaleBy` — a Hi-Res-Fix second pass's upscale step.
#[allow(dead_code)] // wired up by fragments::hires in the next commit (Plan 3 Task 5)
pub fn upscale_by(
    g: &mut Graph,
    id: &str,
    samples: &OwnedLink,
    method: &str,
    scale_by: f64,
) -> OwnedLink {
    g.node(
        id,
        "LatentUpscaleBy",
        json!({ "samples": samples.json(), "upscale_method": method, "scale_by": scale_by }),
    );
    OwnedLink::new(id, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn empty_latent_variants() {
        let mut g = Graph::default();

        let a = empty(&mut g, "5", 1024, 768);
        assert_eq!(a, OwnedLink::new("5", 0));
        assert_eq!(g.input("5", "width"), Some(&json!(1024)));
        assert_eq!(g.input("5", "height"), Some(&json!(768)));
        assert_eq!(g.input("5", "batch_size"), Some(&json!(1)));

        let b = empty_sd3(&mut g, "5b", 512, 640);
        assert_eq!(b, OwnedLink::new("5b", 0));
        assert_eq!(g.input("5b", "width"), Some(&json!(512)));
        assert_eq!(g.input("5b", "batch_size"), Some(&json!(1)));

        let c = empty_flux2(&mut g, "32", 1024, 1024);
        assert_eq!(c, OwnedLink::new("32", 0));
        assert_eq!(g.input("32", "height"), Some(&json!(1024)));
        assert_eq!(g.input("32", "batch_size"), Some(&json!(1)));
    }

    #[test]
    fn empty_flux2_accepts_link_dimensions() {
        let mut g = Graph::default();
        let width = OwnedLink::new("99", 0);
        empty_flux2(&mut g, "66", &width, OwnedLink::new("99", 1));
        assert_eq!(g.input("66", "width"), Some(&json!(["99", 0])));
        assert_eq!(g.input("66", "height"), Some(&json!(["99", 1])));
    }

    #[test]
    fn empty_ltxv_latent_video_carries_the_frame_count() {
        let mut g = Graph::default();
        let out = empty_ltxv(&mut g, "70", 832, 480, 81);
        assert_eq!(out, OwnedLink::new("70", 0));
        assert_eq!(
            g.into_value()["70"],
            json!({
                "class_type": "EmptyLTXVLatentVideo",
                "inputs": { "width": 832, "height": 480, "length": 81, "batch_size": 1 }
            })
        );
    }

    #[test]
    fn vae_encode_wires_pixels_and_vae() {
        let mut g = Graph::default();
        let pixels = OwnedLink::new("80", 0);
        let vae = OwnedLink::new("72", 0);
        let out = vae_encode(&mut g, "124", &pixels, &vae);
        assert_eq!(out, OwnedLink::new("124", 0));
        assert_eq!(
            g.into_value()["124"],
            json!({
                "class_type": "VAEEncode",
                "inputs": { "pixels": ["80", 0], "vae": ["72", 0] }
            })
        );
    }

    #[test]
    fn latent_upscale_by_node() {
        let mut g = Graph::default();
        let samples = OwnedLink::new("3", 0);
        let up = upscale_by(&mut g, "50", &samples, "nearest-exact", 1.5);
        assert_eq!(up, OwnedLink::new("50", 0));
        assert_eq!(g.input("50", "samples"), Some(&json!(["3", 0])));
        assert_eq!(
            g.input("50", "upscale_method"),
            Some(&json!("nearest-exact"))
        );
        assert_eq!(g.input("50", "scale_by"), Some(&json!(1.5)));
    }
}
