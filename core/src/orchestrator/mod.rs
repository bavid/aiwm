//! Job orchestration: the lifecycle state machine ([`state`]) and the engine
//! that drives jobs through it ([`engine`]).

mod engine;
mod state;

pub use engine::{JobEngine, JobOutcome};
pub use state::JobState;
