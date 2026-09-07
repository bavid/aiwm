//! Configuration & data-directory resolution. Implemented in WP-1.
//!
//! Sources, in order of precedence: env overrides -> `config.toml` -> defaults.
//! Data dirs (config, db, logs) resolve under `%APPDATA%\AIWorkstationManager\`;
//! the model store defaults to `E:\AI\models` (ADR-012).
