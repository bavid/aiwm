//! The dataset-prep capability: drive one `job_type=dataset_prep` body — the
//! "bring your own dataset" pipeline turning a folder tree of raw video/
//! images into a curated, captioned dataset in the standard `NNNN.png` +
//! `NNNN.txt` sidecar-file convention most LoRA trainers (kohya-ss/
//! sd-scripts etc.) expect. See docs/TODO.md's "Lokale KI-Trainings-Engine"
//! entry for the full brief — this module is the dataset-prep half only; the
//! training-orchestrator half (actually running a LoRA job) is explicitly
//! out of scope here.
//!
//! Four stages, each in its own submodule:
//! 1. [`ingest`] — walk the root folder; each immediate subfolder is a tag,
//!    files directly in the root are tagged with the root folder's name.
//! 2. [`extract`] — sample video into stills via `ffmpeg`; a plain image
//!    file is already a frame.
//! 3. [`filter`] — judge every frame (dead frame, transition, blur,
//!    near-duplicate, per-clip diversity cap) and record *why* each one was
//!    rejected instead of dropping it.
//! 4. [`caption`] — optional: whichever [`captioner`] the request names
//!    captions every kept frame, and (Florence-2 only) a low-confidence
//!    caption on a video frame is escalated to Qwen2.5-VL with a nearby
//!    frame for temporal context. No captioner means no model is loaded at
//!    all — extraction, filtering and curation never need one.
//!
//! The pipeline itself is split by phase rather than by stage:
//! [`request`] resolves the job's params, [`pipeline`] runs frames mode,
//! [`clip`] runs clip mode, and [`export`] writes the curator's final
//! selection to disk.
//!
//! A run creates one `datasets` row and files every candidate under it in
//! `dataset_frames` for the curation UI to review. In clips mode there is no
//! frame extraction: each source video becomes one row with its duration and
//! a preview still. [`export_dataset`] composes each exported caption from
//! the dataset trigger, the frame's concepts and its own caption (see
//! [`compose`]).
//!
//! [`housekeeping`] measures a dataset's disk use and deletes datasets,
//! frames and discarded frames *with their files* (behind one path guard),
//! and finds near-duplicates across the whole dataset.
//!
//! A cancelled or failed run keeps whatever frames and captions were written
//! before it stopped — a partial dataset is safe to curate and export. Only a
//! dataset that never received a single frame is discarded again (see
//! [`pipeline::run`]).

mod caption;
mod captioner;
mod clip;
mod compose;
mod export;
mod extract;
mod filter;
pub mod housekeeping;
mod ingest;
pub mod location;
mod pipeline;
mod request;
#[cfg(test)]
mod testutil;

pub use caption::{
    DEFAULT_CONTEXT_OFFSET, DEFAULT_ESCALATE, DEFAULT_ESCALATE_EVERY_NTH,
    FLORENCE2_VRAM_FALLBACK_MB, QWEN_VL_VRAM_FALLBACK_MB,
};
pub use captioner::{
    captioner_statuses, captioner_statuses_verified, escalation_status, find_captioner,
    installed_captioner_dir, Captioner, CaptionerStatus, EscalationStatus, CAPTIONERS,
    FLORENCE2_ID, WD_TAGGER_ID,
};
pub use compose::{compose_caption, token_warning, CaptionOrder, CaptionStyle, ConceptPart};
pub use export::{export_dataset, export_dataset_for_job, ExportRequest, ExportSummary};
pub use extract::{DEFAULT_SAMPLE_FPS, MAX_SAMPLE_FPS, MIN_SAMPLE_FPS};
pub use filter::RejectionReason;
pub use filter::{DEFAULT_BLUR_THRESHOLD, DEFAULT_PHASH_MAX_DISTANCE, MAX_PHASH_DISTANCE};
pub use housekeeping::{
    CleanupSummary, DataRoots, DatasetDeleteSummary, DatasetUsage, DedupSummary,
    FramesDeleteSummary, SkippedFile, DEFAULT_DEDUP_THRESHOLD, MAX_DEDUP_THRESHOLD,
};
pub use pipeline::{run, DatasetPrepDone, DatasetPrepOutcome};
pub use request::{DatasetPrepRequest, DEFAULT_MIN_CLIP_SECS};

use crate::CoreError;

/// Every error this capability reports, tagged with the same runtime name so
/// the UI can group them — shared by all submodules.
fn dataset_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "dataset".into(),
        message: msg.to_string(),
    }
}
