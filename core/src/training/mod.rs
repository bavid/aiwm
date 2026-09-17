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

pub mod config;
pub mod profile;

use crate::CoreError;

/// The scheduler's synthetic reservation id for an active training run —
/// the training equivalent of `DATASET_VISION_MODEL_ID` in
/// [`crate::orchestrator::engine`]. A run is not a real loaded model, but it
/// occupies the GPU exactly like one, so the scheduler tracks it under this
/// id and blocks image/video jobs behind it.
pub const TRAINING_MODEL_ID: &str = "training-run";

/// Every error this subsystem reports, tagged with the same runtime name so
/// the UI can group them — mirrors
/// [`crate::capability::dataset`]'s `dataset_err`.
pub fn training_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "training".into(),
        message: msg.to_string(),
    }
}
