//! The automation hooks, built with the `automation` feature: the app measures itself
//! (`--bench`) and replays steps from a file into its own window (`--drive`), so performance runs
//! and end-to-end tests can drive it without taking the keyboard or mouse. Results are lines on
//! stdout, `BENCH …` and `DRIVE …`.

mod bench;
mod drive;
mod probe;
mod usage;

use std::io::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

use gpui_kit::{AnyWindowHandle, App, Entity};

pub(crate) use bench::BenchPlan;
pub(crate) use probe::ChatProbe;

use crate::cli::Cli;
use crate::ui::window::MainWindow;

/// When the process started, for the launch timings.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Notes the start time; call it first thing in `main`.
pub(crate) fn mark_start() {
    STARTED.get_or_init(Instant::now);
}

/// Milliseconds since [`mark_start`].
fn since_start() -> u128 {
    STARTED
        .get()
        .map_or(0, |started| started.elapsed().as_millis())
}

/// Starts what the command line asked for, once the window is open.
pub(crate) fn start(cli: &Cli, window: AnyWindowHandle, view: Entity<MainWindow>, cx: &mut App) {
    if let Some(plan) = cli.bench.clone() {
        bench::start(plan, view.clone(), cx);
    }
    if let Some(file) = cli.drive.clone() {
        drive::start(file, view, window, cx);
    }
}

/// One line of results on stdout, flushed at once (the harness reads it as it comes).
fn report(prefix: &str, line: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(error) = writeln!(out, "{prefix} {line}").and_then(|()| out.flush()) {
        log::warn!("couldn't write the automation's results: {error}");
    }
}
