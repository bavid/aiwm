//! Node/id-agnostic graph builder used by [`crate::pipeline::fragments`].
//!
//! `Graph` wraps a `serde_json::Map` of ComfyUI API-format nodes
//! (`{"<node id>": {"class_type", "inputs"}}`). `OwnedLink` is the owned
//! equivalent of a `["<node id>", <slot>]` link pair — owned so fragment
//! functions can hand links back to callers without borrowing from the graph
//! (no lifetimes to thread through the fragment API).

use serde_json::{json, Value};

/// An owned `["<node id>", <slot>]` link. Owned (not borrowed from the
/// graph) so fragment functions can return links to callers freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedLink {
    pub node: String,
    pub slot: u32,
}

impl OwnedLink {
    pub fn new(node: &str, slot: u32) -> Self {
        Self {
            node: node.to_string(),
            slot,
        }
    }

    /// The ComfyUI API-format link JSON: `["<node id>", <slot>]`.
    pub fn json(&self) -> Value {
        json!([self.node, self.slot])
    }
}

/// Failure modes for [`Graph::set_input`] and [`Graph::from_value`].
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("graph has no node {0:?}")]
    MissingNode(String),
    #[error("graph is not an object of nodes")]
    NotAnObject,
}

/// A ComfyUI API-format prompt graph under construction: explicit node ids
/// to `{"class_type", "inputs"}` node bodies.
#[derive(Debug, Default, Clone)]
pub struct Graph {
    nodes: serde_json::Map<String, Value>,
}

impl Graph {
    /// Insert or overwrite the node at `id` with `{"class_type", "inputs"}`.
    pub fn node(&mut self, id: &str, class_type: &str, inputs: Value) {
        self.nodes.insert(
            id.to_string(),
            json!({ "class_type": class_type, "inputs": inputs }),
        );
    }

    /// Set one input field on an existing node. Errs if `id` has no node.
    pub fn set_input(&mut self, id: &str, key: &str, value: Value) -> Result<(), PipelineError> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| PipelineError::MissingNode(id.to_string()))?;
        node["inputs"][key] = value;
        Ok(())
    }

    /// Read one input field of a node, if the node and field both exist.
    pub fn input(&self, id: &str, key: &str) -> Option<&Value> {
        self.nodes.get(id)?.get("inputs")?.get(key)
    }

    /// Whether a node with this id has been added.
    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    /// Consume the graph into the ComfyUI API-format prompt JSON.
    pub fn into_value(self) -> Value {
        Value::Object(self.nodes)
    }

    /// Rebuild a `Graph` from an existing API-format prompt JSON value — for
    /// tests, and for a later task's gradual port of the inline recipes.
    pub fn from_value(v: Value) -> Result<Self, PipelineError> {
        match v {
            Value::Object(map) => Ok(Self { nodes: map }),
            _ => Err(PipelineError::NotAnObject),
        }
    }
}

/// Allocates sequential string node ids ("90", "91", …) starting at `base`,
/// clear of every fixed id an existing recipe uses.
#[derive(Debug, Clone, Copy)]
pub struct NextId(u32);

impl NextId {
    pub fn new(base: u32) -> Self {
        Self(base)
    }

    /// Return the next id and advance past it.
    pub fn take(&mut self) -> String {
        let id = self.0;
        self.0 += 1;
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_ids_are_explicit_and_links_render_as_pairs() {
        let mut g = Graph::default();
        g.node(
            "4",
            "CheckpointLoaderSimple",
            json!({ "ckpt_name": "x.safetensors" }),
        );
        let link = OwnedLink::new("4", 0);
        assert_eq!(link.json(), json!(["4", 0]));
        assert!(g.contains("4"));
        assert_eq!(g.input("4", "ckpt_name"), Some(&json!("x.safetensors")));
    }

    #[test]
    fn set_input_on_a_missing_node_is_an_error() {
        let mut g = Graph::default();
        let err = g.set_input("99", "foo", json!(1)).unwrap_err();
        assert!(matches!(err, PipelineError::MissingNode(ref id) if id == "99"));
    }

    #[test]
    fn next_id_allocates_from_a_base() {
        let mut ids = NextId::new(90);
        assert_eq!(ids.take(), "90");
        assert_eq!(ids.take(), "91");
        assert_eq!(ids.take(), "92");
    }

    #[test]
    fn from_value_round_trips() {
        let original = json!({
            "4": {
                "class_type": "CheckpointLoaderSimple",
                "inputs": { "ckpt_name": "x.safetensors" }
            }
        });
        let g = Graph::from_value(original.clone()).unwrap();
        assert_eq!(g.into_value(), original);
    }
}
