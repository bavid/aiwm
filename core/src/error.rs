//! Core error type. Library code returns [`CoreError`]; binary edges use `anyhow`.

use std::result::Result as StdResult;

/// Convenience alias for `Result<T, CoreError>`.
pub type Result<T> = StdResult<T, CoreError>;

/// All failure modes the core can surface. Variants are added as modules land.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Db(String),

    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("telemetry unavailable: {0}")]
    TelemetryUnavailable(String),

    #[error("runtime error [{runtime}]: {message}")]
    Runtime { runtime: String, message: String },

    #[error("invalid job transition: {from} -> {to}")]
    InvalidJobTransition { from: String, to: String },

    #[error("scheduler blocked: {0}")]
    SchedulerBlocked(String),

    #[error("sidecar error: {0}")]
    Sidecar(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
