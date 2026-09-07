//! VRAM-aware scheduling. Implemented in WP-5 (skeleton) and Phase 2 (full).
//!
//! `Scheduler::plan(job) -> Decision` where
//! `Decision = RunNow | LoadThenRun | EvictThenLoad(victim) | Blocked(reason)`.
//!
//! `HybridScheduler` (ADR-003): one resident slot, `pin`/`unpin` for active
//! agent sessions, conservative VRAM budget
//! (`free = total - driver_overhead(~1GB) - Σ(loaded) - headroom`). A pinned
//! model is never auto-evicted; on conflict the job goes `Blocked` and the user
//! decides. Scenario matrix is table-tested (see `docs/PHASE_1_PLAN.md` §5.3).
//!
//! Model selection accepts `Explicit(id)` or `Auto { capability, constraints }`,
//! and a job may spawn child jobs — the hooks for Phase 2 rule-based AUTO and
//! Phase 6 best-of-N. Nothing more than the interface in Phase 1.
