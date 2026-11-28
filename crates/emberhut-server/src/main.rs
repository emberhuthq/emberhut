//! `emberhut-server`: serves the emberhut API, prints a pairing code and its link (`emberhut-server pair`,
//! or `pair --json` for a program), a runner's pairing code (`emberhut-server pair-runner`) or the
//! machine's id (`emberhut-server id`), links the machine to an emberhut.com account
//! (`emberhut-server link`, `emberhut-server unlink`).
//!
//! Configuration comes from flags and `EMBERHUT_*` environment variables (`emberhut-server --help`).

use std::io::Write;

use anyhow::Context as _;
use clap::Parser as _;
use emberhut_server::config::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    emberhut_server::telemetry::init(&cli.log).context("cannot start logging")?;
    match cli.into_command() {
        Command::Serve(config) => {
            emberhut_server::serve(&config, emberhut_server::shutdown::requested())
                .await
                .context("the server stopped")?;
        }
        Command::Pair(config) => {
            let made = emberhut_server::make_pairing_code(&config)
                .await
                .with_context(|| format!("no pairing code was made in {}", config.database.url))?;
            if config.json {
                let json = made.to_json(&config.database.url);
                writeln!(std::io::stdout(), "{json}").context("cannot print the pairing code")?;
            } else {
                print_pairing(&made, &config.database.url)
                    .context("cannot print the pairing code")?;
            }
        }
        Command::PairRunner(config) => {
            let made = emberhut_server::make_runner_code(&config)
                .await
                .with_context(|| {
                    format!("no runner pairing code was made in {}", config.database.url)
                })?;
            let mut out = std::io::stdout();
            if config.json {
                writeln!(out, "{}", made.to_json())
            } else {
                print_runner_code(&mut out, &made, &config)
            }
            .context("cannot print the runner pairing code")?;
        }
        Command::Id(config) => {
            let id = emberhut_server::machine_id(&config).context("no machine id")?;
            writeln!(std::io::stdout(), "{id}").context("cannot print the machine id")?;
        }
        Command::Link(config) => emberhut_server::link::link(&config, &mut std::io::stdout())
            .await
            .context("the machine was not linked")?,
        Command::Unlink(config) => emberhut_server::link::unlink(&config, &mut std::io::stdout())
            .await
            .context("the machine was not unlinked")?,
    }
    Ok(())
}
/// Prints a code made on the command line: the code first, so it is shown whatever else
/// fails, then its link and the link's QR code (dark on light, in colour when the output is a
/// terminal), or why there is no link.
fn print_pairing(
    made: &emberhut_server::CommandLineCode,
    database: &emberhut_db::DatabaseUrl,
) -> anyhow::Result<()> {
    use std::io::IsTerminal as _;
    let stdout = std::io::stdout();
    let colors = stdout.is_terminal();
    let mut out = stdout.lock();
    writeln!(
        out,
        "Pairing code: {} (works once, until {}; in {})",
        made.issued.code,
        made.issued.expires_at.strftime("%H:%M:%S UTC"),
        database.path().display()
    )?;
    let link = match &made.link {
        Ok(link) => link,
        Err(error) => {
            writeln!(
                out,
                "No pairing link ({error}): type this machine's address and the code instead."
            )?;
            return Ok(());
        }
    };
    writeln!(out, "Pairing link: {}", link.to_url())?;
    if link.routes().is_empty() {
        writeln!(
            out,
            "The link names no address: set EMBERHUT_PUBLIC_URL, or listen on an address other \
             devices reach (EMBERHUT_LISTEN), for a device to find this machine from it. Typing \
             the address and the code still works."
        )?;
    }
    let qr =
        emberhut_server::terminal_qr(link, colors).context("cannot draw the link's QR code")?;
    writeln!(out, "Scan it in the Emberhut app:\n{qr}")?;
    Ok(())
}

/// Prints a runner code made on the command line: the code, the server's address the runner
/// is to use (with a word when only this computer reaches it), and the lines to run on the
/// runner's machine.
fn print_runner_code(
    out: &mut impl Write,
    made: &emberhut_server::RunnerCodeLines,
    config: &emberhut_server::config::PairRunnerConfig,
) -> std::io::Result<()> {
    writeln!(
        out,
        "Runner pairing code: {} (works once, until {}; in {})",
        made.issued.code,
        made.issued.expires_at.strftime("%H:%M:%S UTC"),
        config.database.url.path().display()
    )?;
    writeln!(out, "Server address for the runner: {}", made.server.url)?;
    if !made.server.reachable {
        writeln!(
            out,
            "Only this computer reaches that address. For a runner on another machine, set \
             EMBERHUT_PUBLIC_URL, listen on an address other machines reach (EMBERHUT_LISTEN), \
             or name one with --server."
        )?;
    }
    writeln!(
        out,
        "\nOn the runner's machine, with emberhut-runner installed:\n  {}",
        made.install.command
    )?;
    writeln!(
        out,
        "Or install it too, on macOS and Linux:\n  {}",
        made.install.shell
    )?;
    writeln!(
        out,
        "On Windows, in PowerShell as an administrator:\n  {}",
        made.install.powershell
    )
}
