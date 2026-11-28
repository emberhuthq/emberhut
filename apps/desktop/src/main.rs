//! Emberhut's desktop app: a GPUI client for an emberhut server's HTTP/SSE API. See the README.
// A release build on Windows is a windowed program, so one started from the Start menu opens no
// console window; started from a terminal, it still writes there ([`windows_console`]).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod app;
mod assets;
#[cfg(feature = "automation")]
mod automation;
mod backend;
mod brand;
mod cli;
mod color;
mod config;
mod device;
mod dictation;
#[cfg(test)]
mod fixtures;
mod format;
mod headless;
mod heic;
mod hut;
mod json_file;
mod links;
mod lock;
mod material;
mod menus;
mod motion;
mod notifications;
mod pdf;
mod prefs;
mod private_files;
mod prose;
mod shell;
mod store;
mod theme;
mod ui;

mod updates;
mod wire;

use anyhow::Context as _;
use clap::Parser as _;

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "automation")]
    automation::mark_start();
    #[cfg(all(windows, not(debug_assertions)))]
    windows_console::attach_to_parent();
    let cli = cli::Cli::parse();
    // RUST_LOG, else warnings and the app's own news; emberhut-client's directives go last, so the
    // HTTP library's unredacted wire trace stays out of the log whatever RUST_LOG says.
    let wanted =
        std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,emberhut_desktop=info".to_owned());
    env_logger::Builder::new()
        .parse_filters(&format!("{wanted},{}", emberhut_client::LOG_DIRECTIVES))
        .try_init()
        .context("couldn't start the log")?;
    app::run(cli)
}

/// A windowed program's way to the terminal it was started from (a Windows release build): the
/// help, the version, `--pair`'s answer and the log are written there.
#[cfg(all(windows, not(debug_assertions)))]
mod windows_console {
    use std::os::windows::io::AsRawHandle as _;

    /// Writes the standard output and error to the console of the program that started this
    /// one, when it has a console and neither is redirected (to a file or a pipe) already;
    /// otherwise nothing changes.
    #[expect(
        unsafe_code,
        reason = "AttachConsole is a system call with no pointers to get right"
    )]
    pub(crate) fn attach_to_parent() {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        if !std::io::stdout().as_raw_handle().is_null()
            || !std::io::stderr().as_raw_handle().is_null()
        {
            return;
        }
        // SAFETY: it takes a process id only; without a parent console it fails and leaves this
        // process as it was.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}
