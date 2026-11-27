//! The emberhut relay (ADR 0007, section 3, and ADR 0008), and the emberhut.com account directory
//! (ADR 0007, section 5): one binary, `emberhut-relay`, with a thin `main.rs` over this library so
//! tests can build what it serves in-process.
//!
//! - [`directory`] lists an account's machines and tells the relay which machines may connect
//!   (`docs/directory.md`). It checks Clerk's tokens behind
//!   [`AccountVerifier`](directory::AccountVerifier) and keeps its own SQLite file.
//! - The gateway (machines' uplinks and devices' sealed connections, `docs/relay.md`) is not
//!   part of this build yet. `emberhut-relay serve --role directory` serves the directory alone;
//!   the gateway will reach it through [`GatewayDirectory`](directory::GatewayDirectory).
//!
//! For now the relay serves plain HTTP on its listen address, meant for loopback behind a
//! reverse proxy that ends TLS for the relay domain. Ending TLS itself comes with the gateway.

use std::future::Future;
use std::io::IsTerminal as _;
use std::sync::Arc;

use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::{SubscriberInitExt as _, TryInitError};

pub mod cli;
pub mod directory;

use cli::{LogFilter, Role, ServeArgs, SettingError};
use directory::{ClerkVerifier, Directory, SystemClock};

/// Why the relay stopped, or never started.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServeError {
    /// The role asked for needs the gateway, which this build doesn't have yet.
    #[error(
        "--role {0} needs the relay's gateway, which isn't part of this build yet; \
         run `emberhut-relay serve --role directory` to serve the account directory alone"
    )]
    GatewayNotBuilt(Role),
    /// A setting is missing or doesn't fit the others.
    #[error(transparent)]
    Setting(#[from] SettingError),
    /// Clerk's tokens can't be checked with these settings.
    #[error("the account directory can't check Clerk's tokens")]
    Verifier(#[source] directory::ClerkSetupError),
    /// The directory's database couldn't be opened.
    #[error("the account directory's database couldn't be opened")]
    Database(#[source] directory::StoreError),
    /// The listen address couldn't be taken.
    #[error("cannot listen on {address}")]
    Bind {
        /// The address.
        address: std::net::SocketAddr,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
    /// Serving failed.
    #[error("serving failed")]
    Serve(#[source] std::io::Error),
}

/// Serves what `args` asks for until `shutdown` resolves, then lets open requests finish.
///
/// # Errors
///
/// A [`ServeError`] when the relay can't start, or serving fails.
pub async fn serve(
    args: ServeArgs,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    match args.role {
        Role::Directory => {}
        role @ (Role::All | Role::Gateway) => return Err(ServeError::GatewayNotBuilt(role)),
    }
    let listen = args.listen;
    let (config, clerk) = args.directory_settings()?;
    let clock = Arc::new(SystemClock);
    let verifier = ClerkVerifier::new(clerk, clock.clone()).map_err(ServeError::Verifier)?;
    let directory = Directory::open(config, verifier, clock)
        .await
        .map_err(ServeError::Database)?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|source| ServeError::Bind {
            address: listen,
            source,
        })?;
    if !listen.ip().is_loopback() {
        tracing::warn!(
            %listen,
            "serving plain HTTP on an address that isn't loopback: put a TLS proxy in front"
        );
    }
    tracing::info!(%listen, "serving the account directory");
    axum::serve(listener, directory.router())
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(ServeError::Serve)
}

/// Starts writing log lines that pass `filter` to standard error, one line each, coloured only
/// on a terminal. Libraries that log through the `log` crate are captured too.
///
/// # Errors
///
/// [`TryInitError`] when a global subscriber is already set.
pub fn init_logs(filter: &LogFilter) -> Result<(), TryInitError> {
    tracing_subscriber::registry()
        .with(filter.to_env_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal()),
        )
        .try_init()
}

/// Resolves when the process is asked to stop: on Ctrl-C, or on `SIGTERM` (what service
/// managers and `docker stop` send).
pub async fn stop_requested() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "cannot watch for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                terminate.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot watch for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("Ctrl-C: shutting down"),
        () = terminate => tracing::info!("SIGTERM: shutting down"),
    }
}
