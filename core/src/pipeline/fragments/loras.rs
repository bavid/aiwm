//! LoRA fragment: a chain of `LoraLoader` nodes spliced between a graph's
//! model/CLIP source and their consumers — the `Graph`-based port of
//! `pipeline::apply_loras`, node-for-node and key-for-key identical to it.

use serde_json::json;

use crate::pipeline::graph::{Graph, NextId, OwnedLink, PipelineError};
use crate::pipeline::LoraSpec;

/// The first node id the chain uses. Clear of every fixed id the recipes
/// allocate (the highest is `78`), so a chain never collides with one.
const LORA_ID_BASE: u32 = 90;

/// Splice `loras` in as a chain of `LoraLoader` nodes between the current
/// model/CLIP source and their consumers, then repoint those consumers at the
/// end of the chain. A no-op when `loras` is empty, so a graph without LoRAs
/// keeps exactly the shape its fragments built.
///
/// Errs only if `model_consumer` or one of `clip_consumers` names a node the
/// graph does not have — a recipe bug, never a user input.
pub fn apply(
    g: &mut Graph,
    loras: &[LoraSpec],
    model_source: &OwnedLink,
    clip_source: &OwnedLink,
    model_consumer: &str,
    clip_consumers: &[&str],
) -> Result<(), PipelineError> {
    if loras.is_empty() {
        return Ok(());
    }
    let mut ids = NextId::new(LORA_ID_BASE);
    let mut model_link = model_source.clone();
    let mut clip_link = clip_source.clone();
    for lora in loras {
        let id = ids.take();
        g.node(
            &id,
            "LoraLoader",
            json!({
                "model": model_link.json(),
                "clip": clip_link.json(),
                "lora_name": lora.file,
                "strength_model": lora.strength,
                "strength_clip": lora.strength,
            }),
        );
        model_link = OwnedLink::new(&id, 0);
        clip_link = OwnedLink::new(&id, 1);
    }
    g.set_input(model_consumer, "model", model_link.json())?;
    for consumer in clip_consumers {
        g.set_input(consumer, "clip", clip_link.json())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    /// A stand-in for the loader + encode + sample shape every recipe has.
    fn graph_with_consumers() -> Graph {
        let mut g = Graph::default();
        g.node("4", "CheckpointLoaderSimple", json!({ "ckpt_name": "x" }));
        g.node(
            "6",
            "CLIPTextEncode",
            json!({ "text": "a", "clip": ["4", 1] }),
        );
        g.node(
            "7",
            "CLIPTextEncode",
            json!({ "text": "b", "clip": ["4", 1] }),
        );
        g.node("3", "KSampler", json!({ "model": ["4", 0] }));
        g
    }

    #[test]
    fn no_loras_leaves_the_graph_untouched() {
        let mut g = graph_with_consumers();
        let before = g.clone().into_value();
        apply(
            &mut g,
            &[],
            &OwnedLink::new("4", 0),
            &OwnedLink::new("4", 1),
            "3",
            &["6", "7"],
        )
        .unwrap();
        assert_eq!(g.into_value(), before);
    }

    #[test]
    fn one_lora_is_spliced_before_the_sampler_and_every_clip_consumer() {
        let mut g = graph_with_consumers();
        apply(
            &mut g,
            &[LoraSpec {
                file: "add-detail-xl.safetensors",
                strength: 0.8,
            }],
            &OwnedLink::new("4", 0),
            &OwnedLink::new("4", 1),
            "3",
            &["6", "7"],
        )
        .unwrap();
        let v = g.into_value();
        assert_eq!(
            v["90"],
            json!({
                "class_type": "LoraLoader",
                "inputs": {
                    "model": ["4", 0],
                    "clip": ["4", 1],
                    "lora_name": "add-detail-xl.safetensors",
                    "strength_model": 0.8,
                    "strength_clip": 0.8
                }
            })
        );
        assert_eq!(v["3"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(v["6"]["inputs"]["clip"], json!(["90", 1]));
        assert_eq!(v["7"]["inputs"]["clip"], json!(["90", 1]));
    }

    #[test]
    fn multiple_loras_chain_in_order_from_id_ninety() {
        let mut g = graph_with_consumers();
        apply(
            &mut g,
            &[
                LoraSpec {
                    file: "a.safetensors",
                    strength: 1.0,
                },
                LoraSpec {
                    file: "b.safetensors",
                    strength: 0.5,
                },
            ],
            &OwnedLink::new("4", 0),
            &OwnedLink::new("4", 1),
            "3",
            &["6"],
        )
        .unwrap();
        let v = g.into_value();
        assert_eq!(v["90"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(v["90"]["inputs"]["clip"], json!(["4", 1]));
        assert_eq!(v["91"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(v["91"]["inputs"]["clip"], json!(["90", 1]));
        assert_eq!(v["91"]["inputs"]["lora_name"], json!("b.safetensors"));
        assert_eq!(v["3"]["inputs"]["model"], json!(["91", 0]));
        assert_eq!(v["6"]["inputs"]["clip"], json!(["91", 1]));
    }

    #[test]
    fn a_missing_consumer_is_an_error_not_a_panic() {
        let mut g = graph_with_consumers();
        let err = apply(
            &mut g,
            &[LoraSpec {
                file: "a.safetensors",
                strength: 1.0,
            }],
            &OwnedLink::new("4", 0),
            &OwnedLink::new("4", 1),
            "nope",
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PipelineError::MissingNode(ref id) if id == "nope"));
    }
}
