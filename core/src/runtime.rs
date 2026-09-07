//! Runtime adapters and process supervision. Implemented in WP-4.
//!
//! - `RuntimeAdapter` trait: install / start / stop / health / load_model /
//!   unload_model / vram_report / supported_formats / model_link_strategy
//!   (see `docs/ARCHITECTURE.md` §2.2).
//! - `RuntimeSupervisor`: owns child processes inside a Windows Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, runs the health loop, restarts with
//!   backoff.
//! - `FakeRuntimeAdapter`: no real process; configurable latency and simulated
//!   VRAM. Basis for all scheduler/engine tests.
