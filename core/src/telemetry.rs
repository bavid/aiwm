//! System telemetry: GPU (NVML) + RAM/CPU (sysinfo), sampled at 1 Hz and
//! published on a `tokio::sync::watch` channel. Implemented in WP-3.
//!
//! Degrades gracefully: with no NVIDIA driver (or in CI) the GPU field becomes
//! `Unavailable { reason }` and the rest keeps working.
