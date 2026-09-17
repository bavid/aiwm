//! Image-side fragments: loading a source image, rescaling and measuring it,
//! and every recipe's terminal `VAEDecode` + `SaveImage` pair.

use serde_json::json;

use crate::pipeline::graph::{Dim, Graph, OwnedLink};

/// `VAEDecode` — a sampled latent back into pixels. The video recipes decode
/// into `CreateVideo` rather than `SaveImage`, so this stands alone as well as
/// feeding [`decode_and_save`]. Exact keys from `checkpoint_txt2img`.
pub fn vae_decode(g: &mut Graph, id: &str, samples: &OwnedLink, vae: &OwnedLink) -> OwnedLink {
    g.node(
        id,
        "VAEDecode",
        json!({ "samples": samples.json(), "vae": vae.json() }),
    );
    OwnedLink::new(id, 0)
}

/// `SaveImage` — the terminal node of every image recipe (and of the RTX image
/// upscale, which never touches a VAE). Exact keys from `checkpoint_txt2img`.
pub fn save_image(g: &mut Graph, id: &str, images: &OwnedLink, filename_prefix: &str) {
    g.node(
        id,
        "SaveImage",
        json!({ "filename_prefix": filename_prefix, "images": images.json() }),
    );
}

/// `VAEDecode` then `SaveImage`. Exact keys from `checkpoint_txt2img`.
pub fn decode_and_save(
    g: &mut Graph,
    decode_id: &str,
    save_id: &str,
    samples: &OwnedLink,
    vae: &OwnedLink,
    filename_prefix: &str,
) {
    let images = vae_decode(g, decode_id, samples, vae);
    save_image(g, save_id, &images, filename_prefix);
}

/// `LoadImage` — a bare file name already staged in ComfyUI's `input/`
/// folder. Returns the IMAGE output (slot 0); the MASK output (slot 1) goes
/// unused by every current recipe. Exact keys from `flux2_klein_edit`.
pub fn load_image(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(id, "LoadImage", json!({ "image": file }));
    OwnedLink::new(id, 0)
}

/// `ImageScaleToTotalPixels` — rescale an image to a total pixel budget.
///
/// `resolution_steps` has a default (1) in ComfyUI's own node schema, but that
/// default is a widget/editor concept: an API-submitted `/prompt` graph that
/// omits it outright gets rejected with "Required input is missing" (confirmed
/// against ComfyUI's real v0.34.0 source, `comfy_extras/
/// nodes_post_processing.py`). 1 = no snapping. Exact keys from
/// `flux2_klein_edit`.
pub fn scale_to_total_pixels(
    g: &mut Graph,
    id: &str,
    image: &OwnedLink,
    method: &str,
    megapixels: f64,
    resolution_steps: u32,
) -> OwnedLink {
    g.node(
        id,
        "ImageScaleToTotalPixels",
        json!({
            "image": image.json(),
            "upscale_method": method,
            "megapixels": megapixels,
            "resolution_steps": resolution_steps
        }),
    );
    OwnedLink::new(id, 0)
}

/// The width/height a `GetImageSize` node reports, ready to feed any fragment
/// that takes a [`Dim`].
#[derive(Debug, Clone)]
pub struct ImageSize {
    pub width: Dim,
    pub height: Dim,
}

/// `GetImageSize` — width in output slot 0, height in slot 1. Exact keys from
/// `flux2_klein_edit`.
pub fn image_size(g: &mut Graph, id: &str, image: &OwnedLink) -> ImageSize {
    g.node(id, "GetImageSize", json!({ "image": image.json() }));
    ImageSize {
        width: Dim::Link(OwnedLink::new(id, 0)),
        height: Dim::Link(OwnedLink::new(id, 1)),
    }
}

#[cfg(test)]
mod tests {
    use crate::pipeline::graph::{Dim, Graph, OwnedLink};
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

    #[test]
    fn vae_decode_and_save_image_stand_alone() {
        let mut g = Graph::default();
        let images = super::vae_decode(
            &mut g,
            "8",
            &OwnedLink::new("3", 0),
            &OwnedLink::new("4", 2),
        );
        assert_eq!(images, OwnedLink::new("8", 0));
        super::save_image(&mut g, "9", &images, "job-abc");
        let v = g.into_value();
        assert_eq!(
            v["8"],
            json!({
                "class_type": "VAEDecode",
                "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
            })
        );
        assert_eq!(
            v["9"],
            json!({
                "class_type": "SaveImage",
                "inputs": { "filename_prefix": "job-abc", "images": ["8", 0] }
            })
        );
    }

    #[test]
    fn load_scale_and_measure_an_input_image() {
        let mut g = Graph::default();
        let loaded = super::load_image(&mut g, "76", "job-abc.png");
        assert_eq!(loaded, OwnedLink::new("76", 0));

        let scaled = super::scale_to_total_pixels(&mut g, "80", &loaded, "lanczos", 1.0, 1);
        assert_eq!(scaled, OwnedLink::new("80", 0));

        let size = super::image_size(&mut g, "99", &scaled);
        assert_eq!(size.width, Dim::Link(OwnedLink::new("99", 0)));
        assert_eq!(size.height, Dim::Link(OwnedLink::new("99", 1)));

        let v = g.into_value();
        assert_eq!(
            v["76"],
            json!({ "class_type": "LoadImage", "inputs": { "image": "job-abc.png" } })
        );
        assert_eq!(
            v["80"],
            json!({
                "class_type": "ImageScaleToTotalPixels",
                "inputs": {
                    "image": ["76", 0],
                    "upscale_method": "lanczos",
                    "megapixels": 1.0,
                    "resolution_steps": 1
                }
            })
        );
        assert_eq!(
            v["99"],
            json!({ "class_type": "GetImageSize", "inputs": { "image": ["80", 0] } })
        );
    }
}
