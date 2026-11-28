//! `emberhut-runner`: runs coding-agent sessions on this machine for an emberhut server.
//!
//! Settings come from `config.json` in the runner's home (`EMBERHUT_RUNNER_HOME`) and a few
//! environment variables (`emberhut-runner --help`). Logs go to standard output, which the
//! service appends to the runner's log; `EMBERHUT_RUNNER_LOG` sets their level (`info` by default).

use clap::Parser as _;
use tracing_subscriber::EnvFilter;

fn main() -> std::process::ExitCode {
    let cli = emberhut_runner::cli::Cli::parse();
    let filter =
        EnvFilter::try_from_env("EMBERHUT_RUNNER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    // A Windows service has no console: it writes the runner's log itself.
    let log_file = matches!(
        cli.command,
        emberhut_runner::cli::Command::Run { service: true }
    )
    .then(|| emberhut_runner::cli::config().ok())
    .flatten()
    .and_then(|c| {
        let path = emberhut_runner::service::log_file(&c);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    // Logging set up twice is the only failure here; the first one stands.
    let started = match log_file {
        Some(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(false)
            .with_writer(std::sync::Mutex::new(file))
            .try_init(),
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
            .with_target(false)
            .try_init(),
    };
    started.unwrap_or_default();
    match emberhut_runner::cli::main(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            if !e.0.is_empty() {
                tracing::error!("{e}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
