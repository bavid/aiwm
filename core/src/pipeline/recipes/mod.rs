//! Concrete recipes, each composed from [`crate::pipeline::fragments`] on top
//! of a [`crate::pipeline::graph::Graph`].
//!
//! The public entry points stay on [`crate::pipeline`] itself (`pipeline::
//! checkpoint_txt2img` and friends) — this module holds the bodies, so a
//! recipe file stays about one model family rather than about every family at
//! once.

pub mod image;
