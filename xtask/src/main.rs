//! `cargo xtask`: repository tasks for emberhut, written in Rust so they run wherever cargo does.
//!
//! Each task is one variant of [`cli::Task`] and one module (`bundle-macos` is
//! [`bundle_macos`], `bundle-hands` [`bundle_hands`], `bundle-windows` [`bundle_windows`]); the
//! command runner, workspace paths and progress output are shared.

mod bundle_hands;
mod bundle_macos;
mod bundle_windows;
mod cli;
mod cmd;
mod files;
mod workspace;

use clap::Parser as _;

use crate::cli::{Cli, Task};

fn main() -> anyhow::Result<()> {
    match Cli::parse().task {
        Task::BundleMacos(args) => bundle_macos::run(&args),
        Task::BundleHands(args) => bundle_hands::run(&args),
        Task::WindowsInstaller(args) => bundle_windows::run(&args),
    }
}

/// Prints one progress line for the person running the task.
#[expect(
    clippy::print_stderr,
    reason = "progress lines for the person at the terminal are this tool's output"
)]
fn say(line: &str) {
    eprintln!("xtask: {line}");
}
