//! Reference-image fragments: the two composite blocks FLUX.2 \[klein\]'s
//! "Kontext-style" character-consistency recipes share — stage a reference
//! picture into a guiding latent, then anchor *both* conditionings to it.
//!
//! These compose the base fragments (`output::load_image` /
//! `output::scale_to_total_pixels`, `latent::vae_encode`,
//! `conditioning::{reference_latent, zero_out}`) rather than emitting nodes
//! themselves; the shapes come from
//! `pipeline::flux2_klein_reference_txt2img{,_safetensors}`, which build the
//! identical seven-node block.

use crate::pipeline::fragments::conditioning::{self, Cond};
use crate::pipeline::fragments::{latent, output};
use crate::pipeline::graph::{Graph, OwnedLink};

/// Node ids for the load → rescale → encode chain.
#[derive(Debug, Clone, Copy)]
pub struct GuidingLatentIds<'a> {
    pub load: &'a str,
    pub scale: &'a str,
    pub encode: &'a str,
}

/// How the reference image is rescaled before it is encoded.
#[derive(Debug, Clone, Copy)]
pub struct Rescale<'a> {
    pub method: &'a str,
    pub megapixels: f64,
    /// `1` = no snapping. Required on an API-submitted graph even though
    /// ComfyUI's node schema gives it a default — see
    /// [`output::scale_to_total_pixels`].
    pub resolution_steps: u32,
}

/// `LoadImage` → `ImageScaleToTotalPixels` → `VAEEncode`: a staged reference
/// picture as the latent a `ReferenceLatent` node can inject into
/// conditioning.
pub fn guiding_latent(
    g: &mut Graph,
    ids: &GuidingLatentIds,
    file: &str,
    vae: &OwnedLink,
    r: &Rescale,
) -> OwnedLink {
    let loaded = output::load_image(g, ids.load, file);
    let scaled = output::scale_to_total_pixels(
        g,
        ids.scale,
        &loaded,
        r.method,
        r.megapixels,
        r.resolution_steps,
    );
    latent::vae_encode(g, ids.encode, &scaled, vae)
}

/// Node ids for the pair of `ReferenceLatent` nodes and the
/// `ConditioningZeroOut` between them.
#[derive(Debug, Clone, Copy)]
pub struct AnchorIds<'a> {
    pub positive: &'a str,
    pub zero_out: &'a str,
    pub negative: &'a str,
}

/// Anchor both halves of the conditioning to `guiding`: the encoded prompt
/// gets a `ReferenceLatent` of its own, and so does its zeroed-out stand-in
/// for a negative prompt. That symmetry is what actually holds the identity —
/// the same technique real FLUX.2/Kontext character-consistency workflows use.
pub fn anchor_both(g: &mut Graph, ids: &AnchorIds, cond: &OwnedLink, guiding: &OwnedLink) -> Cond {
    let positive = conditioning::reference_latent(g, ids.positive, cond, guiding);
    let zeroed = conditioning::zero_out(g, ids.zero_out, cond);
    let negative = conditioning::reference_latent(g, ids.negative, &zeroed, guiding);
    Cond { positive, negative }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn guiding_latent_loads_rescales_and_encodes() {
        let mut g = Graph::default();
        let vae = OwnedLink::new("10", 0);
        let out = guiding_latent(
            &mut g,
            &GuidingLatentIds {
                load: "50",
                scale: "51",
                encode: "52",
            },
            "job-portrait.png",
            &vae,
            &Rescale {
                method: "lanczos",
                megapixels: 1.0,
                resolution_steps: 1,
            },
        );
        assert_eq!(out, OwnedLink::new("52", 0));
        let v = g.into_value();
        assert_eq!(
            v["50"],
            json!({ "class_type": "LoadImage", "inputs": { "image": "job-portrait.png" } })
        );
        assert_eq!(
            v["51"],
            json!({
                "class_type": "ImageScaleToTotalPixels",
                "inputs": {
                    "image": ["50", 0],
                    "upscale_method": "lanczos",
                    "megapixels": 1.0,
                    "resolution_steps": 1
                }
            })
        );
        assert_eq!(
            v["52"],
            json!({
                "class_type": "VAEEncode",
                "inputs": { "pixels": ["51", 0], "vae": ["10", 0] }
            })
        );
    }

    #[test]
    fn anchor_both_injects_the_same_latent_into_real_and_zeroed_conditioning() {
        let mut g = Graph::default();
        let cond = OwnedLink::new("6", 0);
        let guiding = OwnedLink::new("52", 0);
        let out = anchor_both(
            &mut g,
            &AnchorIds {
                positive: "53",
                zero_out: "27",
                negative: "54",
            },
            &cond,
            &guiding,
        );
        assert_eq!(out.positive, OwnedLink::new("53", 0));
        assert_eq!(out.negative, OwnedLink::new("54", 0));
        let v = g.into_value();
        assert_eq!(
            v["53"],
            json!({
                "class_type": "ReferenceLatent",
                "inputs": { "conditioning": ["6", 0], "latent": ["52", 0] }
            })
        );
        assert_eq!(
            v["27"],
            json!({
                "class_type": "ConditioningZeroOut",
                "inputs": { "conditioning": ["6", 0] }
            })
        );
        assert_eq!(
            v["54"],
            json!({
                "class_type": "ReferenceLatent",
                "inputs": { "conditioning": ["27", 0], "latent": ["52", 0] }
            })
        );
    }
}
