//! The options of `cargo xtask bundle-hands`.

use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::{Context as _, anyhow};
use clap::Args;
use emberhut_core::private_network::{check_listen, check_server_url};
use url::Url;

use super::install::Agent;

/// Options of `cargo xtask bundle-hands`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct HandsArgs {
    /// Build with the `dist` profile (smaller, slower to build) instead of `release`.
    #[arg(long)]
    pub(crate) dist: bool,

    /// How many compile jobs cargo may run at once (the build runs under `nice`).
    #[arg(
        long,
        short = 'j',
        value_name = "N",
        default_value_t = 4,
        value_parser = clap::value_parser!(u16).range(1..)
    )]
    pub(crate) jobs: u16,

    /// The code signing identity, as `security find-identity -v -p codesigning` lists it.
    /// Without one the helper is signed ad hoc, and the Mac forgets its Accessibility and
    /// Screen Recording grants at every new build; a Developer ID keeps them.
    #[arg(long, value_name = "IDENTITY", env = "EMBERHUT_SIGN_IDENTITY")]
    pub(crate) sign: Option<String>,

    /// Put the helper in ~/Applications/Emberhut Hands.app and (re)start its launch agent,
    /// com.emberhut.hands. An app already there is replaced only when it is this helper.
    #[arg(long)]
    pub(crate) install: bool,

    /// With --install: one more address the helper listens on (an address on a private
    /// network, for the server on another machine).
    #[arg(long, value_name = "IP", requires = "install")]
    pub(crate) listen: Option<IpAddr>,

    /// With --install: the server recorded calls go to.
    #[arg(long, value_name = "URL", requires_all = ["install", "server_token_file"])]
    pub(crate) server_url: Option<Url>,

    /// With --server-url: the file holding the helper's device token for that server.
    #[arg(long, value_name = "FILE", requires = "server_url")]
    pub(crate) server_token_file: Option<PathBuf>,
}

impl HandsArgs {
    /// What the launch agent sets for the helper (the token file made absolute, as the agent
    /// doesn't start in this directory), checked as the helper checks it at start: `--listen`
    /// a private address, `--server-url` https or plain http to a local or private host.
    ///
    /// # Errors
    ///
    /// For an address or URL the helper would refuse, or a token file whose path can't be
    /// made absolute.
    pub(crate) fn agent(&self) -> anyhow::Result<Agent> {
        if let Some(ip) = self.listen {
            check_listen(ip).map_err(|refused| anyhow!("--listen {refused}"))?;
        }
        if let Some(url) = &self.server_url {
            check_server_url(url).map_err(|refused| anyhow!("--server-url {refused}"))?;
        }
        Ok(Agent {
            listen: self.listen.map(|ip| ip.to_string()),
            server_url: self.server_url.as_ref().map(ToString::to_string),
            server_token_file: self
                .server_token_file
                .as_deref()
                .map(std::path::absolute)
                .transpose()
                .context("making --server-token-file absolute")?,
        })
    }
}

#[cfg(test)]
impl HandsArgs {
    /// A bare `cargo xtask bundle-hands`.
    pub(crate) fn bare() -> Self {
        Self {
            dist: false,
            jobs: 4,
            sign: None,
            install: false,
            listen: None,
            server_url: None,
            server_token_file: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches};

    use super::HandsArgs;
    use crate::cli::{Cli, Task};

    fn parse(args: &[&str]) -> Result<HandsArgs, clap::Error> {
        let command = Cli::command().mut_subcommand("bundle-hands", |sub| {
            sub.mut_arg("sign", |arg| arg.env(None::<&str>))
        });
        let matches =
            command.try_get_matches_from(["cargo xtask", "bundle-hands"].iter().chain(args))?;
        match Cli::from_arg_matches(&matches)?.task {
            Task::BundleHands(hands) => Ok(hands),
            Task::BundleMacos(_) | Task::WindowsInstaller(_) | Task::Release(_) => {
                panic!("bundle-hands")
            }
        }
    }

    #[test]
    fn defaults_are_a_release_build_signed_ad_hoc_and_not_installed() {
        assert_eq!(parse(&[]).unwrap(), HandsArgs::bare());
    }

    #[test]
    fn the_agent_s_settings_need_install() {
        assert!(parse(&["--listen", "10.0.0.5"]).is_err());
        let args = parse(&[
            "--install",
            "--listen",
            "10.0.0.5",
            "--server-url",
            "https://emberhut.example",
            "--server-token-file",
            "/Users/example/.emberhut/hands-device",
        ])
        .unwrap();
        let agent = args.agent().unwrap();
        assert_eq!(agent.listen.as_deref(), Some("10.0.0.5"));
        assert_eq!(
            agent.server_url.as_deref(),
            Some("https://emberhut.example/")
        );
        assert!(agent.server_token_file.unwrap().is_absolute());
    }

    #[test]
    fn the_listen_address_and_the_server_are_checked() {
        assert!(parse(&["--install", "--listen", "not-an-ip"]).is_err());
        assert!(
            parse(&[
                "--install",
                "--server-url",
                "not a url",
                "--server-token-file",
                "t"
            ])
            .is_err()
        );
        let relative = parse(&[
            "--install",
            "--server-url",
            "https://emberhut.example",
            "--server-token-file",
            "hands-device-token",
        ])
        .unwrap();
        let file = relative.agent().unwrap().server_token_file.unwrap();
        assert!(file.is_absolute() && file.ends_with("hands-device-token"));
    }

    #[test]
    fn an_install_the_helper_would_refuse_is_refused_now() {
        let with = |extra: &[&str]| {
            let mut args = vec!["--install"];
            args.extend_from_slice(extra);
            parse(&args).unwrap().agent()
        };
        assert!(with(&["--listen", "0.0.0.0"]).is_err());
        assert!(with(&["--listen", "8.8.8.8"]).is_err());
        assert!(with(&["--listen", "192.168.1.10"]).is_ok());
        let server = |url: &str| with(&["--server-url", url, "--server-token-file", "/t"]);
        assert!(server("http://emberhut.example").is_err());
        assert!(server("ftp://emberhut.example").is_err());
        assert!(server("http://192.168.1.11:8791").is_ok());
        assert!(server("https://emberhut.example").is_ok());
    }

    #[test]
    fn a_server_needs_its_token_file() {
        assert!(parse(&["--install", "--server-url", "https://emberhut.example"]).is_err());
    }
}
