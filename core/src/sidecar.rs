//! Client for the Python sidecar. Implemented in WP-8.
//!
//! Transport: JSON-RPC 2.0 over stdio, sidecar process kept alive inside the
//! Job Object. Phase 1 methods: `handshake { protocol_version }` ->
//! `{ sidecar_version, capabilities }`, and `ping` -> `pong`.
//! `inspect_model_file { path }` is defined but stubbed (`not_implemented`)
//! until Phase 2.
