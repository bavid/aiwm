//! Process-wide tracing setup: a daily-rotating file in `logs/` plus a stderr
//! mirror, both governed by the same env-filter directive.
//!
//! Call [`init`] exactly once per process (from a binary), not from library
//! code — it installs the global subscriber.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::{CoreError, Result};

/// Validate a `tracing` env-filter directive without installing anything.
pub fn parse_filter(directive: &str) -> Result<EnvFilter> {
    EnvFilter::builder()
        .parse(directive)
        .map_err(|e| CoreError::Config(format!("invalid log_filter {directive:?}: {e}")))
}

/// Install the global subscriber. The returned guard must be held for the life
/// of the process; dropping it flushes buffered file output.
pub fn init(directive: &str, logs_dir: &Path) -> Result<WorkerGuard> {
    // Fail fast on a bad directive before touching global state.
    parse_filter(directive)?;

    let file = tracing_appender::rolling::daily(logs_dir, "aiwm.log");
    let (writer, guard) = tracing_appender::non_blocking(file);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(parse_filter(directive)?);
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(parse_filter(directive)?);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(|e| CoreError::Other(anyhow::anyhow!("tracing already initialised: {e}")))?;

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_normal_directive() {
        assert!(parse_filter("info,aiwm_core=debug").is_ok());
    }

    #[test]
    fn rejects_garbage_directive() {
        let err = parse_filter("not a =valid= filter!!").unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
    }
}
