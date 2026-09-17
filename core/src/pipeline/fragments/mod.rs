//! Base fragments: reusable graph-building blocks carrying the exact node
//! shapes the recipes need. [`crate::pipeline::recipes`] composes every recipe
//! from these; the golden fixtures in `core/tests/pipeline_goldens.rs` pin the
//! result byte-for-byte against the output the recipes produced when they were
//! inline `json!` graphs.

pub mod conditioning;
pub mod hires;
pub mod ipadapter;
pub mod latent;
pub mod loaders;
pub mod loras;
pub mod output;
pub mod reference;
pub mod sampling;
pub mod upscale;
pub mod video;

// No flat re-exports: the recipes name every fragment type through its own
// module (`sampling::SamplerParams`, `loaders::SplitModelIds`, …), which keeps
// a call site's fragment layer obvious at a glance.
