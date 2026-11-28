//! What a person runs to install a runner and pair it with a one-time code: the server's
//! address the runner is to use, and the lines that carry it and the code
//! (`emberhut-runner install`, and the release's installers through `curl` and `irm`).
//!
//! The address is the one a runner on another machine can reach, best first: the one the app
//! sent (it reached the server there), the server's public URL or the address it listens on
//! when another machine can reach them ([`advertise::runner_routes`]), the address the request
//! came in on over the server's own listener when that isn't loopback. Failing all of them it
//! is the server's own address, and the answer says it is not reachable from elsewhere. A
//! relayed request's address is never used: a runner's token never goes through the relay
//! (ADR 0017).

use std::net::SocketAddr;

use emberhut_api::runner::pairing::RunnerInstall;
use emberhut_identity::Scheme;

use crate::services::advertise;

/// Where the releases' files are, one folder per release (`v<version>/`).
pub const RELEASES: &str = "https://github.com/emberhuthq/emberhut/releases/download";

/// The release's installer for macOS and Linux.
pub const SHELL_INSTALLER: &str = "install-runner.sh";

/// The release's installer for Windows.
pub const POWERSHELL_INSTALLER: &str = "install-runner.ps1";

/// The release this server was built as: the version a release build sets
/// (`EMBERHUT_VERSION`), else the crate's. The installers named are that release's, so the
/// runner installed is the server's version.
pub const RELEASE_VERSION: &str = match option_env!("EMBERHUT_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

/// The server's address a new runner is to use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddress {
    /// The address, without a final `/`.
    pub url: String,
    /// Whether another machine can reach it (not loopback, not `localhost`).
    pub reachable: bool,
}

/// Where the server is, as one request for a code sees it.
#[derive(Clone, Copy, Debug)]
pub struct AddressSources<'a> {
    /// The address the app sent, if it did.
    pub asked: Option<&'a str>,
    /// The configured addresses another machine can reach, best first.
    pub advertised: &'a [String],
    /// The address the request came in on, over the server's own listener.
    pub came_in_on: Option<&'a str>,
    /// The server's own address, its links' base.
    pub own: &'a str,
}

/// The address a new runner is to use, from `sources` (see the module's documentation).
#[must_use]
pub fn pick_server(sources: AddressSources<'_>) -> ServerAddress {
    let address = |url: &str| ServerAddress {
        url: url.trim_end_matches('/').to_owned(),
        reachable: advertise::reachable_host(url),
    };
    if let Some(asked) = sources.asked {
        return address(asked);
    }
    sources
        .advertised
        .iter()
        .map(String::as_str)
        .chain(sources.came_in_on)
        .map(address)
        .find(|candidate| candidate.reachable)
        .unwrap_or_else(|| address(sources.came_in_on.unwrap_or(sources.own)))
}

/// The base URL of `local`, the address a connection came in on, with `scheme`.
#[must_use]
pub fn came_in_on(scheme: Scheme, local: SocketAddr) -> String {
    let local = SocketAddr::new(local.ip().to_canonical(), local.port());
    let scheme = match scheme {
        Scheme::Http => "http",
        Scheme::Https => "https",
    };
    format!("{scheme}://{local}")
}

/// The lines that install a runner of release `version` and pair it with the server at
/// `server` with `code`. The address is quoted where a shell would read it otherwise; a code
/// is letters, digits and a dash.
#[must_use]
pub fn install_lines(server: &str, code: &str, version: &str) -> RunnerInstall {
    let sh_server = sh_word(server);
    let sh_code = sh_word(code);
    let release = format!("{RELEASES}/v{version}");
    RunnerInstall {
        command: format!("emberhut-runner install --server {sh_server} --pair {sh_code}"),
        shell: format!(
            "curl -fsSL {release}/{SHELL_INSTALLER} | sh -s -- --server {sh_server} --pair {sh_code}"
        ),
        powershell: format!(
            "$env:EMBERHUT_SERVER={}; $env:EMBERHUT_PAIR_CODE={}; irm {release}/{POWERSHELL_INSTALLER} | iex",
            powershell_string(server),
            powershell_string(code)
        ),
    }
}

/// Whether every character of `text` means itself in a POSIX shell word and inside a
/// PowerShell string in double quotes.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'.' | b'_' | b'~' | b':' | b'/' | b'@' | b'%' | b'+' | b'=' | b','
                )
        })
}

/// `text` as one POSIX shell word: as it is when [`plain`], else in single quotes.
fn sh_word(text: &str) -> String {
    if plain(text) {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// `text` as a PowerShell string: in double quotes when [`plain`], else in single quotes with
/// each single quote (PowerShell counts the typographic ones too) doubled.
fn powershell_string(text: &str) -> String {
    if plain(text) {
        return format!("\"{text}\"");
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(advertised: &[String]) -> AddressSources<'_> {
        AddressSources {
            asked: None,
            advertised,
            came_in_on: None,
            own: "http://127.0.0.1:18990",
        }
    }

    #[test]
    fn the_lines_are_the_readme_s_with_the_address_and_code() {
        let lines = install_lines("https://emberhut.example.com", "ABCD-EFGH", "1.4.0");
        assert_eq!(
            lines.command,
            "emberhut-runner install --server https://emberhut.example.com --pair ABCD-EFGH"
        );
        assert_eq!(
            lines.shell,
            "curl -fsSL https://github.com/emberhuthq/emberhut/releases/download/v1.4.0/install-runner.sh \
             | sh -s -- --server https://emberhut.example.com --pair ABCD-EFGH"
        );
        assert_eq!(
            lines.powershell,
            "$env:EMBERHUT_SERVER=\"https://emberhut.example.com\"; $env:EMBERHUT_PAIR_CODE=\"ABCD-EFGH\"; \
             irm https://github.com/emberhuthq/emberhut/releases/download/v1.4.0/install-runner.ps1 | iex"
        );
    }

    #[test]
    fn an_address_a_shell_would_read_otherwise_is_quoted() {
        let lines = install_lines("http://[2001:db8::1]:18990/a'b$c", "ABCD-EFGH", "1.4.0");
        assert_eq!(
            lines.command,
            r"emberhut-runner install --server 'http://[2001:db8::1]:18990/a'\''b$c' --pair ABCD-EFGH"
        );
        assert!(
            lines
                .powershell
                .starts_with("$env:EMBERHUT_SERVER='http://[2001:db8::1]:18990/a''b$c'; "),
            "{}",
            lines.powershell
        );
        assert_eq!(powershell_string("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    }

    #[test]
    fn the_app_s_address_comes_first() {
        let advertised = ["https://emberhut.example.com".to_owned()];
        let picked = pick_server(AddressSources {
            asked: Some("http://192.0.2.10:18990/"),
            ..sources(&advertised)
        });
        assert_eq!(
            picked,
            ServerAddress {
                url: "http://192.0.2.10:18990".to_owned(),
                reachable: true
            }
        );
        let picked = pick_server(AddressSources {
            asked: Some("http://localhost:18990"),
            ..sources(&advertised)
        });
        assert!(!picked.reachable, "the app's own word, said plainly");
    }

    #[test]
    fn then_the_configured_address_then_the_one_the_request_came_in_on() {
        let advertised = ["https://emberhut.example.com".to_owned()];
        let picked = pick_server(AddressSources {
            came_in_on: Some("http://192.0.2.10:18990"),
            ..sources(&advertised)
        });
        assert_eq!(picked.url, "https://emberhut.example.com");
        let picked = pick_server(AddressSources {
            came_in_on: Some("http://192.0.2.10:18990"),
            ..sources(&[])
        });
        assert_eq!(
            (picked.url.as_str(), picked.reachable),
            ("http://192.0.2.10:18990", true)
        );
    }

    #[test]
    fn a_loopback_address_is_used_but_not_reachable() {
        let picked = pick_server(AddressSources {
            came_in_on: Some("http://127.0.0.1:18990"),
            ..sources(&[])
        });
        assert_eq!(
            (picked.url.as_str(), picked.reachable),
            ("http://127.0.0.1:18990", false)
        );
        let picked = pick_server(sources(&[]));
        assert_eq!(
            (picked.url.as_str(), picked.reachable),
            ("http://127.0.0.1:18990", false)
        );
    }

    #[test]
    fn a_connection_s_address_is_a_base_url() {
        assert_eq!(
            came_in_on(Scheme::Http, "192.0.2.10:18990".parse().unwrap()),
            "http://192.0.2.10:18990"
        );
        assert_eq!(
            came_in_on(Scheme::Https, "[::ffff:192.0.2.10]:443".parse().unwrap()),
            "https://192.0.2.10:443"
        );
        assert_eq!(
            came_in_on(Scheme::Http, "[2001:db8::1]:18990".parse().unwrap()),
            "http://[2001:db8::1]:18990"
        );
    }
}
