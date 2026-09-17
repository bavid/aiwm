//! Base fragments: reusable graph-building blocks carrying the exact node
//! shapes the recipes need. [`crate::pipeline::recipes`] composes every recipe
//! from these; the golden fixtures in `core/tests/pipeline_goldens.rs` pin the
//! result byte-for-byte against the output the recipes produced when they were
//! inline `json!` graphs.

pub mod conditioning;
pub mod ipadapter;
pub mod latent;
pub mod loaders;
pub mod loras;
pub mod output;
pub mod reference;
pub mod sampling;
pub mod upscale;
pub mod video;

pub use conditioning::Cond;
pub use loaders::{Loaded, SplitModelIds};
pub use output::ImageSize;
pub use sampling::{CustomAdvancedIds, CustomAdvancedParams, CustomLinks, SamplerParams};
pub use video::{Frame, VideoComponents, VideoLatent};
