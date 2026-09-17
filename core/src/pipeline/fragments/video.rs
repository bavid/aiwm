//! Video fragments: the node shapes from `pipeline::{wan_ti2v, ltx_video,
//! rtx_upscale_video}` — model-sampling shift, the two latent-video factories
//! (`WanImageToVideo` / `LTXVImgToVideo`), LTX-Video's `LTXVConditioning`, and
//! ComfyUI's own video load/save nodes.

use serde_json::json;

use crate::pipeline::fragments::conditioning::Cond;
use crate::pipeline::graph::{Dim, Graph, OwnedLink};

/// Every recipe here renders one clip at a time.
const BATCH_SIZE: u32 = 1;

/// The container and codec `SaveVideo` writes. `"auto"` lets ComfyUI pick the
/// codec its ffmpeg build actually has, rather than pinning one that may be
/// missing.
const SAVE_FORMAT: &str = "mp4";
const SAVE_CODEC: &str = "auto";

/// The frame geometry a latent-video factory needs.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Frames. Wan wants `(length - 1) % 4 == 0`; LTX-Video wants `% 8 == 0`
    /// and rounds down internally if it isn't.
    pub length: u32,
}

/// What a latent-video factory node hands the sampler: a conditioning pair
/// (output slots 0 and 1) plus the empty video latent (slot 2).
#[derive(Debug, Clone)]
pub struct VideoLatent {
    pub cond: Cond,
    pub latent: OwnedLink,
}

/// Build the triple a `WanImageToVideo`/`LTXVImgToVideo` node exposes.
fn triple(id: &str) -> VideoLatent {
    VideoLatent {
        cond: Cond {
            positive: OwnedLink::new(id, 0),
            negative: OwnedLink::new(id, 1),
        },
        latent: OwnedLink::new(id, 2),
    }
}

/// `ModelSamplingSD3` — the sampling-shift patch Wan 2.2 applies to its
/// diffusion model before sampling. Exact keys from `wan_ti2v`.
pub fn model_sampling_sd3(g: &mut Graph, id: &str, model: &OwnedLink, shift: f64) -> OwnedLink {
    g.node(
        id,
        "ModelSamplingSD3",
        json!({ "model": model.json(), "shift": shift }),
    );
    OwnedLink::new(id, 0)
}

/// `WanImageToVideo` — Wan 2.2's latent-video factory. It handles text→video
/// and image→video alike: the `start_image` input is simply absent for T2V,
/// which is why it is an `Option` here rather than two node shapes. Exact keys
/// from `wan_ti2v`.
pub fn wan_image_to_video(
    g: &mut Graph,
    id: &str,
    cond: &Cond,
    vae: &OwnedLink,
    f: &Frame,
    start_image: Option<&OwnedLink>,
) -> VideoLatent {
    let mut inputs = json!({
        "positive": cond.positive.json(),
        "negative": cond.negative.json(),
        "vae": vae.json(),
        "width": f.width,
        "height": f.height,
        "length": f.length,
        "batch_size": BATCH_SIZE
    });
    if let Some(frame) = start_image {
        inputs["start_image"] = frame.json();
    }
    g.node(id, "WanImageToVideo", inputs);
    triple(id)
}

/// `LTXVConditioning` — stamps the frame rate onto both halves of the
/// conditioning. Outputs the pair in slots 0 and 1. Exact keys from
/// `ltx_video`.
pub fn ltxv_conditioning(g: &mut Graph, id: &str, cond: &Cond, frame_rate: u32) -> Cond {
    g.node(
        id,
        "LTXVConditioning",
        json!({
            "positive": cond.positive.json(),
            "negative": cond.negative.json(),
            "frame_rate": frame_rate
        }),
    );
    Cond {
        positive: OwnedLink::new(id, 0),
        negative: OwnedLink::new(id, 1),
    }
}

/// `LTXVImgToVideo` — LTX-Video's image→video factory: it produces the
/// conditioning pair *and* the start latent, replacing
/// `EmptyLTXVLatentVideo` outright. `strength` is how strongly the start frame
/// constrains the first frames. Exact keys from `ltx_video`.
pub fn ltxv_img_to_video(
    g: &mut Graph,
    id: &str,
    cond: &Cond,
    vae: &OwnedLink,
    image: &OwnedLink,
    f: &Frame,
    strength: f64,
) -> VideoLatent {
    g.node(
        id,
        "LTXVImgToVideo",
        json!({
            "positive": cond.positive.json(),
            "negative": cond.negative.json(),
            "vae": vae.json(),
            "image": image.json(),
            "width": f.width,
            "height": f.height,
            "length": f.length,
            "batch_size": BATCH_SIZE,
            "strength": strength
        }),
    );
    triple(id)
}

/// `LoadVideo` — a bare file name already staged in ComfyUI's `input/` folder.
/// Exact keys from `rtx_upscale_video`.
pub fn load_video(g: &mut Graph, id: &str, file: &str) -> OwnedLink {
    g.node(id, "LoadVideo", json!({ "file": file }));
    OwnedLink::new(id, 0)
}

/// What `GetVideoComponents` splits a loaded video into: frames (slot 0),
/// audio (slot 1) and fps (slot 2).
#[derive(Debug, Clone)]
pub struct VideoComponents {
    pub images: OwnedLink,
    pub audio: OwnedLink,
    pub fps: OwnedLink,
}

/// `GetVideoComponents` — frames, audio and fps of a loaded video, so a
/// frame-level node can run over the batch and the original audio/fps can ride
/// along into the re-encode. Exact keys from `rtx_upscale_video`.
pub fn get_video_components(g: &mut Graph, id: &str, video: &OwnedLink) -> VideoComponents {
    g.node(id, "GetVideoComponents", json!({ "video": video.json() }));
    VideoComponents {
        images: OwnedLink::new(id, 0),
        audio: OwnedLink::new(id, 1),
        fps: OwnedLink::new(id, 2),
    }
}

/// `CreateVideo` — a frame batch into a video. `audio` is present only when
/// the source had some to carry over (the upscale recipe); `fps` is a literal
/// for a generated clip and a link when it comes from the source video. Exact
/// keys from `wan_ti2v` and `rtx_upscale_video`.
pub fn create_video(
    g: &mut Graph,
    id: &str,
    images: &OwnedLink,
    audio: Option<&OwnedLink>,
    fps: impl Into<Dim>,
) -> OwnedLink {
    let mut inputs = json!({ "images": images.json() });
    if let Some(audio) = audio {
        inputs["audio"] = audio.json();
    }
    inputs["fps"] = fps.into().json();
    g.node(id, "CreateVideo", inputs);
    OwnedLink::new(id, 0)
}

/// `SaveVideo` — the terminal node of every video recipe. Exact keys from
/// `wan_ti2v`.
pub fn save_video(g: &mut Graph, id: &str, video: &OwnedLink, filename_prefix: &str) {
    g.node(
        id,
        "SaveVideo",
        json!({
            "video": video.json(),
            "filename_prefix": filename_prefix,
            "format": SAVE_FORMAT,
            "codec": SAVE_CODEC
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::graph::{Graph, OwnedLink};
    use serde_json::json;

    fn cond() -> Cond {
        Cond {
            positive: OwnedLink::new("6", 0),
            negative: OwnedLink::new("7", 0),
        }
    }

    fn frame() -> Frame {
        Frame {
            width: 832,
            height: 480,
            length: 81,
        }
    }

    #[test]
    fn model_sampling_sd3_patches_the_model_with_a_shift() {
        let mut g = Graph::default();
        let out = model_sampling_sd3(&mut g, "48", &OwnedLink::new("37", 0), 8.0);
        assert_eq!(out, OwnedLink::new("48", 0));
        assert_eq!(
            g.into_value()["48"],
            json!({
                "class_type": "ModelSamplingSD3",
                "inputs": { "model": ["37", 0], "shift": 8.0 }
            })
        );
    }

    #[test]
    fn wan_image_to_video_omits_the_start_image_for_text_to_video() {
        let mut g = Graph::default();
        let out = wan_image_to_video(
            &mut g,
            "55",
            &cond(),
            &OwnedLink::new("39", 0),
            &frame(),
            None,
        );
        assert_eq!(out.cond.positive, OwnedLink::new("55", 0));
        assert_eq!(out.cond.negative, OwnedLink::new("55", 1));
        assert_eq!(out.latent, OwnedLink::new("55", 2));
        assert_eq!(
            g.into_value()["55"],
            json!({
                "class_type": "WanImageToVideo",
                "inputs": {
                    "positive": ["6", 0],
                    "negative": ["7", 0],
                    "vae": ["39", 0],
                    "width": 832,
                    "height": 480,
                    "length": 81,
                    "batch_size": 1
                }
            })
        );
    }

    #[test]
    fn wan_image_to_video_adds_the_start_image_for_image_to_video() {
        let mut g = Graph::default();
        let start = OwnedLink::new("60", 0);
        wan_image_to_video(
            &mut g,
            "55",
            &cond(),
            &OwnedLink::new("39", 0),
            &frame(),
            Some(&start),
        );
        assert_eq!(g.input("55", "start_image"), Some(&json!(["60", 0])));
    }

    #[test]
    fn ltxv_conditioning_stamps_the_frame_rate_and_republishes_the_pair() {
        let mut g = Graph::default();
        let out = ltxv_conditioning(&mut g, "69", &cond(), 24);
        assert_eq!(out.positive, OwnedLink::new("69", 0));
        assert_eq!(out.negative, OwnedLink::new("69", 1));
        assert_eq!(
            g.into_value()["69"],
            json!({
                "class_type": "LTXVConditioning",
                "inputs": { "positive": ["6", 0], "negative": ["7", 0], "frame_rate": 24 }
            })
        );
    }

    #[test]
    fn ltxv_img_to_video_produces_conditioning_and_the_start_latent() {
        let mut g = Graph::default();
        let out = ltxv_img_to_video(
            &mut g,
            "77",
            &cond(),
            &OwnedLink::new("44", 2),
            &OwnedLink::new("78", 0),
            &frame(),
            0.15,
        );
        assert_eq!(out.latent, OwnedLink::new("77", 2));
        assert_eq!(
            g.into_value()["77"],
            json!({
                "class_type": "LTXVImgToVideo",
                "inputs": {
                    "positive": ["6", 0],
                    "negative": ["7", 0],
                    "vae": ["44", 2],
                    "image": ["78", 0],
                    "width": 832,
                    "height": 480,
                    "length": 81,
                    "batch_size": 1,
                    "strength": 0.15
                }
            })
        );
    }

    #[test]
    fn create_and_save_video_for_a_generated_clip() {
        let mut g = Graph::default();
        let clip = create_video(&mut g, "58", &OwnedLink::new("8", 0), None, 24u32);
        assert_eq!(clip, OwnedLink::new("58", 0));
        save_video(&mut g, "59", &clip, "job-vid");
        let v = g.into_value();
        assert_eq!(
            v["58"],
            json!({
                "class_type": "CreateVideo",
                "inputs": { "images": ["8", 0], "fps": 24 }
            })
        );
        assert_eq!(
            v["59"],
            json!({
                "class_type": "SaveVideo",
                "inputs": {
                    "video": ["58", 0],
                    "filename_prefix": "job-vid",
                    "format": "mp4",
                    "codec": "auto"
                }
            })
        );
    }

    #[test]
    fn load_split_and_re_encode_an_existing_video_carries_audio_and_fps() {
        let mut g = Graph::default();
        let source = load_video(&mut g, "1", "job-src.mp4");
        assert_eq!(source, OwnedLink::new("1", 0));
        let parts = get_video_components(&mut g, "2", &source);
        assert_eq!(parts.images, OwnedLink::new("2", 0));
        assert_eq!(parts.audio, OwnedLink::new("2", 1));
        assert_eq!(parts.fps, OwnedLink::new("2", 2));
        create_video(&mut g, "4", &parts.images, Some(&parts.audio), &parts.fps);
        let v = g.into_value();
        assert_eq!(
            v["1"],
            json!({ "class_type": "LoadVideo", "inputs": { "file": "job-src.mp4" } })
        );
        assert_eq!(
            v["4"],
            json!({
                "class_type": "CreateVideo",
                "inputs": { "images": ["2", 0], "audio": ["2", 1], "fps": ["2", 2] }
            })
        );
    }
}
