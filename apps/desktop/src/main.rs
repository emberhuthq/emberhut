//! Emberhut's desktop app: a GPUI client for an emberhut server's HTTP/SSE API. See the README.

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
