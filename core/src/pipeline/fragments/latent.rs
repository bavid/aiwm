//! Latent-image fragments: `EmptyLatentImage` / `EmptySD3LatentImage` /
//! `EmptyFlux2LatentImage` / `LatentUpscaleBy`.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

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
/// keys from `flux2_klein_txt2img`.
pub fn empty_flux2(g: &mut Graph, id: &str, width: u32, height: u32) -> OwnedLink {
    g.node(
        id,
        "EmptyFlux2LatentImage",
        json!({ "width": width, "height": height, "batch_size": 1 }),
    );
    OwnedLink::new(id, 0)
}

/// `LatentUpscaleBy` — a Hi-Res-Fix second pass's upscale step (a later
/// task); no current recipe uses it, but it belongs to the base fragment
/// layer this task ships.
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
