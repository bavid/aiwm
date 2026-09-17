//! Conditioning fragments: `CLIPTextEncode` pairs, `ConditioningZeroOut`, and
//! `FluxGuidance` — the shapes from `pipeline::{checkpoint_txt2img,
//! flux_txt2img, flux2_klein_txt2img}`.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

/// The positive/negative conditioning pair a sampler consumes.
#[derive(Debug, Clone)]
pub struct Cond {
    pub positive: OwnedLink,
    pub negative: OwnedLink,
}

/// Two `CLIPTextEncode` nodes off the same CLIP link.
pub fn encode_pair(
    g: &mut Graph,
    pos_id: &str,
    neg_id: &str,
    clip: &OwnedLink,
    positive: &str,
    negative: &str,
) -> Cond {
    let positive = encode_single(g, pos_id, clip, positive);
    let negative = encode_single(g, neg_id, clip, negative);
    Cond { positive, negative }
}

/// One `CLIPTextEncode` node.
pub fn encode_single(g: &mut Graph, id: &str, clip: &OwnedLink, text: &str) -> OwnedLink {
    g.node(
        id,
        "CLIPTextEncode",
        json!({ "text": text, "clip": clip.json() }),
    );
    OwnedLink::new(id, 0)
}

/// `ConditioningZeroOut` — FLUX.2 \[klein\]'s stand-in for a real negative
/// prompt.
pub fn zero_out(g: &mut Graph, id: &str, cond: &OwnedLink) -> OwnedLink {
    g.node(
        id,
        "ConditioningZeroOut",
        json!({ "conditioning": cond.json() }),
    );
    OwnedLink::new(id, 0)
}

/// `FluxGuidance` — the CFG-scale stand-in for FLUX.1 and the FLUX.2
/// \[klein\] safetensors recipe (both are guidance-distilled).
pub fn flux_guidance(g: &mut Graph, id: &str, cond: &OwnedLink, guidance: f64) -> OwnedLink {
    g.node(
        id,
        "FluxGuidance",
        json!({ "conditioning": cond.json(), "guidance": guidance }),
    );
    OwnedLink::new(id, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn encode_pair_and_zero_out_and_flux_guidance() {
        let mut g = Graph::default();
        let clip = OwnedLink::new("11", 0);
        let cond = encode_pair(&mut g, "6", "7", &clip, "a red fox", "blurry");
        assert_eq!(cond.positive, OwnedLink::new("6", 0));
        assert_eq!(cond.negative, OwnedLink::new("7", 0));
        assert_eq!(g.input("6", "clip"), Some(&json!(["11", 0])));
        assert_eq!(g.input("6", "text"), Some(&json!("a red fox")));
        assert_eq!(g.input("7", "text"), Some(&json!("blurry")));

        let zeroed = zero_out(&mut g, "27", &cond.positive);
        assert_eq!(zeroed, OwnedLink::new("27", 0));
        assert_eq!(g.input("27", "conditioning"), Some(&json!(["6", 0])));

        let guided = flux_guidance(&mut g, "26", &cond.positive, 3.5);
        assert_eq!(guided, OwnedLink::new("26", 0));
        assert_eq!(g.input("26", "conditioning"), Some(&json!(["6", 0])));
        assert_eq!(g.input("26", "guidance"), Some(&json!(3.5)));
    }
}
