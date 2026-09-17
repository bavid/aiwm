//! Output fragment: `VAEDecode` + `SaveImage`, every recipe's terminal pair.

use serde_json::json;

use crate::pipeline::graph::{Graph, OwnedLink};

/// `VAEDecode` then `SaveImage`. Exact keys from `checkpoint_txt2img`.
pub fn decode_and_save(
    g: &mut Graph,
    decode_id: &str,
    save_id: &str,
    samples: &OwnedLink,
    vae: &OwnedLink,
    filename_prefix: &str,
) {
    g.node(
        decode_id,
        "VAEDecode",
        json!({ "samples": samples.json(), "vae": vae.json() }),
    );
    let images = OwnedLink::new(decode_id, 0);
    g.node(
        save_id,
        "SaveImage",
        json!({ "filename_prefix": filename_prefix, "images": images.json() }),
    );
}

#[cfg(test)]
mod tests {
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    #[test]
    fn decode_and_save() {
        let mut g = Graph::default();
        let samples = OwnedLink::new("3", 0);
        let vae = OwnedLink::new("4", 2);
        super::decode_and_save(&mut g, "8", "9", &samples, &vae, "job-abc");
        assert_eq!(
            g.into_value(),
            json!({
                "8": {
                    "class_type": "VAEDecode",
                    "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
                },
                "9": {
                    "class_type": "SaveImage",
                    "inputs": { "filename_prefix": "job-abc", "images": ["8", 0] }
                }
            })
        );
    }
}
