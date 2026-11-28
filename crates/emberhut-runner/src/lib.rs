//! The emberhut runner: the daemon on each of the owner's machines (each hut) that runs
//! coding-agent sessions for an emberhut server (ADR 0017, ADR 0033).
//!
//! The server decides what to do; the runner does it where the code, the tools and the logins
//! are. It starts Claude Code sessions through the agent's own command line (stream-json in and
//! out), sends them each turn, picks an account with room and moves a session to another one when
//! an account runs out, samples the machine, and reports every change to the server as it
//! happens. One binary, `emberhut-runner`, with a thin `main.rs` over this library so tests can
//! run a whole runner in-process.

pub mod accounts;
pub mod cli;
pub mod client;
pub mod config;
pub mod engine;
pub mod files;
pub mod http;
pub mod jobs;
pub mod machine;
pub mod pair;
pub mod report;
pub mod runner;
pub mod service;
pub mod sessions;
pub mod store;
pub mod text;
pub mod wire;

/// This build's version: the workspace's, or `EMBERHUT_RUNNER_VERSION` when the build set it, so a
/// release build carries its tag's version.
#[must_use]
pub const fn version() -> &'static str {
    match option_env!("EMBERHUT_RUNNER_VERSION") {
        Some(v) if !v.is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}
