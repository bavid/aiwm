//! LoRA fragment: a chain of `LoraLoader` nodes spliced between a graph's
//! model/CLIP source and their consumers — the `Graph`-based port of
//! `pipeline::apply_loras`, node-for-node and key-for-key identical to it.

use serde_json::json;

use crate::pipeline::graph::{Graph, NextId, OwnedLink, PipelineError};
use crate::pipeline::LoraSpec;

/// The first node id the chain uses. The chain runs from here to
/// `LORA_ID_BASE + MAX_LORAS - 1` (= 94) and no recipe emits a fixed id in
/// that window — see the node-id map in [`crate::pipeline::recipes::ids`],
/// which reserves it and has the test that pins the reservation.
pub(crate) const LORA_ID_BASE: u32 = 90;

/// How many `LoraLoader` nodes a chain can be long. More than this and the
/// graph (and the render) gets unwieldy for little benefit — a soft ceiling,
/// not a ComfyUI limitation. It lives here rather than in the request parser
/// that enforces it ([`crate::capability::media::parse_loras`], which reads
/// it from here) because it is what makes the reserved id window above
/// finite: cap and range have to move together or the reservation is a lie.
pub(crate) const MAX_LORAS: usize = 5;

/// Splice `loras` in as a chain of `LoraLoader` nodes between the current
/// model/CLIP source and their consumers, then repoint those consumers at the
/// end of the chain. A no-op when `loras` is empty, so a graph without LoRAs
/// keeps exactly the shape its fragments built.
///
/// `model_consumers` is a list because a Hi-Res-Fix second pass is a second
/// consumer of the same model link (see [`super::hires`]); most recipes name
/// exactly one.
///
/// At most [`MAX_LORAS`] entries are spliced — anything past that is dropped
/// rather than written to a node id outside the reserved window. The request
/// parser ([`crate::capability::media::parse_loras`]) already caps the list,
/// so in practice this only holds the line for other callers.
///
/// Errs only if one of `model_consumers` or `clip_consumers` names a node the
/// graph does not have — a recipe bug, never a user input.
pub fn apply(
    g: &mut Graph,
    loras: &[LoraSpec],
    model_source: &OwnedLink,
    clip_source: &OwnedLink,
    model_consumers: &[&str],
    clip_consumers: &[&str],
) -> Result<(), PipelineError> {
    if loras.is_empty() {
        return Ok(());
    }
    let mut ids = NextId::new(LORA_ID_BASE);
    let mut model_link = model_source.clone();
    let mut clip_link = clip_source.clone();
    for lora in loras.iter().take(MAX_LORAS) {
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
    for consumer in model_consumers {
        g.set_input(consumer, "model", model_link.json())?;
    }
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

    /// The doc on [`LORA_ID_BASE`] promises the chain never leaves the
    /// 90–94 window. The request parser caps a job at [`MAX_LORAS`], but this
    /// fragment is also reachable from any other caller, so the cap is
    /// enforced here too rather than trusted: a sixth entry would otherwise
    /// write node "95", straight into no-man's-land.
    #[test]
    fn a_chain_longer_than_max_loras_is_truncated_to_the_reserved_window() {
        let files = [
            "a.safetensors",
            "b.safetensors",
            "c.safetensors",
            "d.safetensors",
            "e.safetensors",
            "f.safetensors",
        ];
        assert_eq!(files.len(), MAX_LORAS + 1);
        let specs: Vec<LoraSpec> = files
            .iter()
            .map(|file| LoraSpec {
                file,
                strength: 1.0,
            })
            .collect();

        let mut g = graph_with_consumers();
        apply(
            &mut g,
            &specs,
            &OwnedLink::new("4", 0),
            &OwnedLink::new("4", 1),
            &["3"],
            &["6", "7"],
        )
        .unwrap();

        let v = g.into_value();
        assert!(
            v.get("95").is_none(),
            "the sixth LoRA must not write outside the reserved 90-94 window"
        );
        assert!(v.get("94").is_some(), "the fifth still writes the last id");
        // The consumers read the END of the truncated chain, not a dangling
        // node id that was never built.
        assert_eq!(v["3"]["inputs"]["model"], json!(["94", 0]));
        assert_eq!(v["6"]["inputs"]["clip"], json!(["94", 1]));
        assert_eq!(v["7"]["inputs"]["clip"], json!(["94", 1]));
        // The dropped entry is the last one, not one from the middle.
        assert_eq!(v["94"]["inputs"]["lora_name"], "e.safetensors");
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
            &["3"],
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
            &["3"],
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
            &["3"],
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
            &["nope"],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PipelineError::MissingNode(ref id) if id == "nope"));
    }
}
