//! Capability-specific job bodies. The [`orchestrator`](crate::orchestrator)
//! drives the state machine and the scheduler; each capability module owns what
//! happens while a job is `Running`.
//!
//! - [`chat`] — `job_type=chat`, streamed text from llama.cpp (2.4)
//! - [`image`] — `job_type=image`, a fixed SDXL workflow on ComfyUI (3.4)

pub mod chat;
pub mod image;
