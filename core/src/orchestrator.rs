//! Job state machine and `JobEngine`. Implemented in WP-5.
//!
//! States: queued -> scheduled -> preparing -> running -> post ->
//! completed | failed | blocked | cancelled. Transitions are an explicit table;
//! invalid transitions return `CoreError::InvalidJobTransition`.
//!
//! On startup, jobs left `running`/`preparing` by a crash are marked `failed`
//! (with an event); `queued` jobs stay in the queue.
