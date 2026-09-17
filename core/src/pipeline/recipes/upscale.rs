//! The RTX Video Super Resolution upscale recipes, composed from
//! [`crate::pipeline::fragments`].
//!
//! Neither touches a model, a VAE or a LoRA: they resample an existing image
//! or video, so there is no `finish`/LoRA step here — the graph goes straight
//! out. Both are re-exported from [`crate::pipeline`] under their original
//! names and signatures, and produce exactly the graph their inline `json!`
//! predecessors did — the golden fixtures in `core/tests/pipeline_goldens.rs`
//! pin that byte-for-byte.

use serde_json::Value;

use crate::pipeline::fragments::{output, upscale, video};
use crate::pipeline::graph::Graph;
use crate::pipeline::{UpscaleImageInputs, UpscaleVideoInputs};

/// An image upscale via NVIDIA's RTX Video Super Resolution custom node
/// (`Comfy-Org/Nvidia_RTX_Nodes_ComfyUI`, installed alongside `ComfyUI-GGUF`):
/// `LoadImage` → `RTXVideoSuperResolution` → `SaveImage`. This sharpens,
/// denoises and resizes an already-rendered image — it does not hallucinate
/// new detail the way a diffusion upscaler would.
pub fn rtx_upscale_image(i: &UpscaleImageInputs) -> Value {
    let mut g = Graph::default();
    let source = output::load_image(&mut g, "1", i.source_image);
    let upscaled = upscale::rtx_super_resolution(&mut g, "2", &source, i.quality, i.resize);
    output::save_image(&mut g, "3", &upscaled, i.filename_prefix);
    g.into_value()
}

/// A video upscale via the same node, sandwiched into ComfyUI's own video
/// load/save nodes rather than inventing a new video pipeline: `LoadVideo` →
/// `GetVideoComponents` (the source's frames, audio and fps) →
/// `RTXVideoSuperResolution` (runs on the whole frame batch — one image, or a
/// video's worth of frames, is the same node either way) → `CreateVideo`
/// (re-encodes with the *original* audio/fps) → `SaveVideo`. Matches the
/// node's own shipped `example_workflows/rtx_video_upscale.json` shape.
pub fn rtx_upscale_video(i: &UpscaleVideoInputs) -> Value {
    let mut g = Graph::default();
    let source = video::load_video(&mut g, "1", i.source_video);
    let parts = video::get_video_components(&mut g, "2", &source);
    let upscaled = upscale::rtx_super_resolution(&mut g, "3", &parts.images, i.quality, i.resize);
    // The original audio (slot 1) and fps (slot 2) ride along — upscaling only
    // touches the frames.
    let encoded = video::create_video(&mut g, "4", &upscaled, Some(&parts.audio), &parts.fps);
    video::save_video(&mut g, "5", &encoded, i.filename_prefix);
    g.into_value()
}

#[cfg(test)]
mod tests;
