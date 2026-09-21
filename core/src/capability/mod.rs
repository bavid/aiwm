//! Capability-specific job bodies. The [`orchestrator`](crate::orchestrator)
//! drives the state machine and the scheduler; each capability module owns what
//! happens while a job is `Running`.
//!
//! - [`chat`] — `job_type=chat`, streamed text from llama.cpp (2.4)
//! - [`image`] — `job_type=image`, a fixed SDXL/Flux workflow on ComfyUI (3.4/3.6)
//! - [`video`] — `job_type=video`, a fixed Wan 2.2 workflow on ComfyUI (4.1)
//! - [`upscale`] — `job_type=upscale`, NVIDIA RTX Video Super Resolution on an
//!   already-finished image/video job's output, also on ComfyUI (7.x)
//! - [`agent`] — long-running agent sessions; its own subsystem, not a job (5.1c)
//! - [`dataset`] — `job_type=dataset_prep`, the "bring your own dataset" prep
//!   pipeline (ingest → ffmpeg extraction → blur/duplicate filtering →
//!   Florence-2/Qwen2.5-VL captioning) that turns a folder tree of video/
//!   images into a curated, captioned LoRA-trainer-ready dataset

pub mod agent;
pub mod audio_clean;
pub mod chat;
pub mod colibri;
pub mod dataset;
pub mod image;
pub mod image_defaults;
mod media;
pub mod tts;
pub mod upscale;
pub mod video;
