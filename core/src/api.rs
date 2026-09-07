//! Handlers shared by the Tauri IPC layer and the loopback HTTP/WS API.
//! Implemented in WP-6.
//!
//! Commands (Phase 1): `get_system_telemetry`, `get_settings`,
//! `set_setting { key, value }`, `list_models`, `list_jobs { filter }`,
//! `get_runtime_status`, `get_recent_logs { lines }`.
//! Events: `telemetry` (1 Hz), `job_updated`, `runtime_health`.
//!
//! The HTTP/WS server binds `127.0.0.1` only — no LAN listener (ADR-008).
