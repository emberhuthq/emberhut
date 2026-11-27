//! The commands that run without a window: `--pair`, which pairs this device with the server
//! from a terminal or a script (a machine whose screen isn't at hand), and `--forget-session`.
//! Both act on the server the app talks to first (never the fallback), with the session kept
//! exactly where the app keeps it, so the app opens signed in (or signed out) afterwards.

use std::io::{BufRead as _, Write as _};

use anyhow::{Context as _, bail};

use crate::backend;
use crate::cli::Cli;
use crate::config::{self, AppFiles, SERVER_ENV};
use crate::device;

/// Whether the command line asks for one of them.
#[must_use]
pub(crate) fn wanted(cli: &Cli) -> bool {
    cli.pair.is_some() || cli.forget_session
}

/// Runs the one asked for, saying on the standard output what it did.
///
/// # Errors
///
/// The app has no files on this system, the code couldn't be read, or the server refused the
/// code, couldn't be reached, or doesn't take the session.
pub(crate) fn run(cli: &Cli) -> anyhow::Result<()> {
    let files = AppFiles::locate()
        .context("this system has no home directory: nowhere to keep a session")?;
    let from_env = std::env::var(SERVER_ENV).ok();
    let server = config::servers(from_env.as_deref(), &files.read_config()).primary;
    let url = server.url.clone();
    if cli.forget_session {
        backend::forget_here(server, &files, &device::name())
            .with_context(|| format!("couldn't forget the session with {url}"))?;
        return say(&format!("Forgot this device's session with {url}."));
    }
    let Some(code) = &cli.pair else {
        return Ok(());
    };
    let code = read_code(code)?;
    let paired = backend::pair_here(server, &files, &device::name(), &code)
        .with_context(|| format!("couldn't pair with {url}"))?;
    if !paired.signed_in {
        bail!("{url} took the code but doesn't take this device's session as signed in");
    }
    say(&match paired.device {
        Some(device) => format!("Paired with {url}: signed in as device {device}."),
        None => format!("Paired with {url}: signed in."),
    })
}

/// Writes `line` to the standard output.
fn say(line: &str) -> anyhow::Result<()> {
    writeln!(std::io::stdout().lock(), "{line}").context("couldn't write to the standard output")
}

/// The code given, or the first line of the standard input for `-`.
fn read_code(given: &str) -> anyhow::Result<String> {
    if given != "-" {
        return Ok(given.to_owned());
    }
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("couldn't read the pairing code from the standard input")?;
    let code = line.trim();
    if code.is_empty() {
        bail!("no pairing code on the standard input");
    }
    Ok(code.to_owned())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[test]
    fn pairing_or_forgetting_runs_without_a_window() {
        let parse = |args: &[&str]| Cli::try_parse_from([&["emberhut"], args].concat());
        assert!(!wanted(&parse(&[]).unwrap()));
        let pair = parse(&["--pair", "ABCD-EFGH"]).unwrap();
        assert!(wanted(&pair));
        assert_eq!(pair.pair.as_deref(), Some("ABCD-EFGH"));
        assert!(wanted(&parse(&["--forget-session"]).unwrap()));
        assert!(
            parse(&["--pair", "ABCD-EFGH", "--forget-session"]).is_err(),
            "one or the other"
        );
        assert_eq!(read_code("ABCD-EFGH").unwrap(), "ABCD-EFGH");
    }
}
