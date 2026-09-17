//! The training orchestrator (spec `2026-09-16-training-orchestrator-design.md`):
//! turning a curated dataset into a LoRA by driving the `ai-toolkit` runtime.
//!
//! A **training run** is not a `job`: it is a long-lived, resumable process
//! that can outlive the app (see `training_runs` / [`crate::db::training_runs`])
//! and holds the GPU for minutes to hours, whereas a `job` is one queued,
//! short-lived unit of scheduler work. The run's lifecycle (its own state
//! machine, its own process supervision under `core::runtime::training`) is
//! deliberately separate from the job engine in
//! [`crate::orchestrator::engine`]; the two meet only at the scheduler, where
//! a run reserves the GPU under [`TRAINING_MODEL_ID`] exactly like any other
//! loaded model.
//!
//! [`profile`] is the static registry of trainable model families: which
//! `ai-toolkit` architecture each one maps to, its VRAM strategy, its base
//! weights, and its three preset (fast/balanced/thorough) starting points.
//! [`bases`] is its download-and-verify counterpart: the repo, the exclusion
//! patterns and the pinned per-file sizes and SHA-256 sums of each family's
//! base snapshot.

pub mod bases;
pub mod config;
pub mod process;
pub mod profile;
pub mod progress;
pub mod runner;

use crate::CoreError;

/// The scheduler's synthetic reservation id for an active training run —
/// the training equivalent of `DATASET_VISION_MODEL_ID` in
/// [`crate::orchestrator::engine`]. A run is not a real loaded model, but it
/// occupies the GPU exactly like one, so the scheduler tracks it under this
/// id and blocks image/video jobs behind it.
pub const TRAINING_MODEL_ID: &str = "training-run";

/// A **fault**: something went wrong that the user did not ask for and cannot
/// fix from the UI — a directory that would not create, a config that would
/// not write, a row that vanished from under us. Tagged with the runtime name
/// so the UI can group them, mirroring [`crate::capability::dataset`]'s
/// `dataset_err`. Reaches HTTP as a 500.
pub fn training_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "training".into(),
        message: msg.to_string(),
    }
}

/// A **refusal**: the answer to a request that was never going to work, phrased
/// as a sentence the user can act on — "the trainer is not installed", "the
/// dataset has not been exported yet", "only a running training can be paused".
/// Nothing is broken; the request was.
///
/// The distinction is not cosmetic. It is the difference between a 400 and a
/// 500 at the API boundary (see [`crate::api::http`]'s `ApiError`), which is
/// in turn the difference between the Training tab showing the sentence and
/// the Training tab reporting a bug. Choosing between this and
/// [`training_err`] is therefore part of writing the check, not a mapping some
/// later layer can guess at.
pub fn training_refusal(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(msg.to_string())
}
