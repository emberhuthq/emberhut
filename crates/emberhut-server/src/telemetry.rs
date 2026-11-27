//! Logs: `tracing` events and spans written to standard error, one line each, coloured only
//! on a terminal.
//!
//! Libraries that log through the `log` crate are captured too.

use std::io::IsTerminal as _;

use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::{SubscriberInitExt as _, TryInitError};

use crate::config::LogFilter;

/// Starts writing log lines that pass `filter`.
///
/// # Errors
///
/// [`TryInitError`] when a global subscriber is already set.
pub fn init(filter: &LogFilter) -> Result<(), TryInitError> {
    tracing_subscriber::registry()
        .with(filter.to_env_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal()),
        )
        .try_init()
}
