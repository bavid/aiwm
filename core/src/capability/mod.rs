//! Capability-specific job bodies. The [`orchestrator`](crate::orchestrator)
//! drives the state machine and the scheduler; each capability module owns what
//! happens while a job is `Running`.
//!
//! Phase 2 ships one: [`chat`].

pub mod chat;
