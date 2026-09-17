//! Base fragments: reusable graph-building blocks carrying the exact node
//! shapes the image recipes need. [`crate::pipeline::recipes::image`] composes
//! every image recipe from these; the golden fixtures in
//! `core/tests/pipeline_goldens.rs` pin the result byte-for-byte against the
//! output the recipes produced when they were inline `json!` graphs.

pub mod conditioning;
pub mod latent;
pub mod loaders;
pub mod loras;
pub mod output;
pub mod sampling;

pub use conditioning::Cond;
pub use loaders::{FluxGgufIds, Loaded};
pub use output::ImageSize;
pub use sampling::{CustomAdvancedIds, CustomAdvancedParams, SamplerParams};
