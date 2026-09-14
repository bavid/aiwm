//! Capability-specific job bodies. The [`orchestrator`](crate::orchestrator)
//! drives the state machine and the scheduler; each capability module owns what
//! happens while a job is `Running`.
//!
//! - [`chat`] — `job_type=chat`, streamed text from llama.cpp (2.4)
//! - [`image`] — `job_type=image`, a fixed SDXL/Flux workflow on ComfyUI (3.4/3.6)
//! - [`video`] — `job_type=video`, a fixed Wan 2.2 workflow on ComfyUI (4.1)
//! - [`agent`] — long-running agent sessions; its own subsystem, not a job (5.1c)

pub mod agent;
pub mod chat;
pub mod colibri;
pub mod image;
mod media;
pub mod tts;
pub mod video;
