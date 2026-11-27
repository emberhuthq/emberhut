//! `emberhut-relay`: the relay machines dial out to, and the emberhut.com account directory.
//!
//! Configuration comes from flags and `EMBERHUT_RELAY_*` environment variables
//! (`emberhut-relay serve --help`).

use anyhow::Context as _;
use clap::Parser as _;
use emberhut_relay::cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    emberhut_relay::init_logs(&cli.log).context("cannot start logging")?;
    match cli.command {
        Command::Serve(serve) => emberhut_relay::serve(serve, emberhut_relay::stop_requested())
            .await
            .context("the relay stopped")?,
    }
    Ok(())
}
