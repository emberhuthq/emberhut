//! The relay's configuration: command-line flags, each with an `EMBERHUT_RELAY_*` environment
//! variable (ADR 0007, section 7), parsed into typed values and checked before anything starts.
//!
//! Nothing secret is configured: the directory checks tokens with Clerk's public keys, which it
//! fetches from the JWKS URL.

use std::fmt;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use emberhut_identity::RelayDomain;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::ParseError;

use crate::directory::{ClerkSettings, DirectoryConfig, JwksUrl, KeyTiming, RequestLimits};

/// The `emberhut-relay` command line.
#[derive(Debug, Parser)]
#[command(name = "emberhut-relay", version, about)]
pub struct Cli {
    /// Which log lines to write, as a `tracing` filter (`info`, `emberhut_relay=debug,info`).
    #[arg(
        long,
        env = "EMBERHUT_RELAY_LOG",
        global = true,
        default_value = "info"
    )]
    pub log: LogFilter,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// What `emberhut-relay` can do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve the relay's roles.
    Serve(ServeArgs),
}

/// Which parts of the relay a process runs (ADR 0007, section 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Role {
    /// The gateway and the directory together, in one process.
    All,
    /// The gateway alone: machines' uplinks and devices' sealed connections.
    Gateway,
    /// The account directory alone.
    Directory,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "all",
            Self::Gateway => "gateway",
            Self::Directory => "directory",
        })
    }
}

/// How to serve.
#[derive(Clone, Debug, Args)]
pub struct ServeArgs {
    /// What this process runs. `all` and `gateway` need the gateway, which isn't part of this
    /// build yet; `directory` serves the account directory alone.
    #[arg(long, env = "EMBERHUT_RELAY_ROLE", value_enum, default_value_t = Role::All)]
    pub role: Role,
    /// The relay domain: the one host name the relay serves, which link and unlink statements
    /// name.
    #[arg(long, env = "EMBERHUT_RELAY_DOMAIN")]
    pub domain: RelayDomain,
    /// The address and port to listen on. Plain HTTP for now, meant for loopback behind a
    /// reverse proxy that ends TLS for the relay domain.
    #[arg(long, env = "EMBERHUT_RELAY_LISTEN", default_value = "127.0.0.1:18980")]
    pub listen: SocketAddr,
    /// The account directory's settings.
    #[command(flatten)]
    pub directory: DirectoryArgs,
}

/// The account directory's settings.
#[derive(Clone, Debug, Args)]
pub struct DirectoryArgs {
    /// The directory's SQLite file, made if it isn't there.
    #[arg(long, env = "EMBERHUT_RELAY_DATABASE", default_value = "directory.db")]
    pub database: PathBuf,
    /// How many machines one account may link.
    #[arg(long, env = "EMBERHUT_RELAY_ACCOUNT_CAP", default_value = "5")]
    pub account_cap: NonZeroU32,
    /// Clerk's issuer, exactly as tokens carry it in `iss` (`https://<Clerk frontend API>`).
    #[arg(long, env = "EMBERHUT_RELAY_CLERK_ISSUER")]
    pub clerk_issuer: Option<String>,
    /// Clerk's JWKS URL (`https://<Clerk frontend API>/.well-known/jwks.json`).
    #[arg(long, env = "EMBERHUT_RELAY_CLERK_JWKS_URL")]
    pub clerk_jwks_url: Option<JwksUrl>,
    /// The `aud` of session tokens: the JWT template's audience.
    #[arg(
        long,
        env = "EMBERHUT_RELAY_CLERK_AUDIENCE",
        default_value = "emberhut-directory"
    )]
    pub clerk_audience: String,
    /// The client id of the native OAuth application (the desktop and the CLI).
    #[arg(long, env = "EMBERHUT_RELAY_CLERK_NATIVE_CLIENT_ID")]
    pub clerk_native_client_id: Option<String>,
    /// The client id of the web app's OAuth application.
    #[arg(long, env = "EMBERHUT_RELAY_CLERK_WEB_CLIENT_ID")]
    pub clerk_web_client_id: Option<String>,
    /// The origins a session token's `azp` may name, repeated or comma-separated in the
    /// variable. A session token without `azp` (a native app's) needs none.
    #[arg(
        long = "clerk-allowed-origin",
        env = "EMBERHUT_RELAY_CLERK_ALLOWED_ORIGINS",
        value_name = "ORIGIN",
        value_delimiter = ','
    )]
    pub clerk_allowed_origins: Vec<String>,
    /// The claim that marks an OAuth access token when its header's `typ` isn't `at+jwt`: one
    /// that access tokens carry and ID and session tokens don't.
    #[arg(
        long,
        env = "EMBERHUT_RELAY_CLERK_ACCESS_TOKEN_CLAIM",
        default_value = "client_id"
    )]
    pub clerk_access_token_claim: String,
}

/// A setting the directory needs is missing or empty.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SettingError {
    /// The directory checks Clerk's tokens, so it needs Clerk's issuer and keys.
    #[error("the account directory needs {0}")]
    Missing(&'static str),
    /// A setting is empty.
    #[error("{0} can't be empty")]
    Empty(&'static str),
}

impl ServeArgs {
    /// The directory's settings, and Clerk's.
    ///
    /// # Errors
    ///
    /// A [`SettingError`] when Clerk's issuer or JWKS URL is missing, or a value is empty.
    pub fn directory_settings(self) -> Result<(DirectoryConfig, ClerkSettings), SettingError> {
        let args = self.directory;
        let issuer = args
            .clerk_issuer
            .ok_or(SettingError::Missing("EMBERHUT_RELAY_CLERK_ISSUER"))?;
        let jwks_url = args
            .clerk_jwks_url
            .ok_or(SettingError::Missing("EMBERHUT_RELAY_CLERK_JWKS_URL"))?;
        let not_empty = |value: String, name: &'static str| {
            if value.trim().is_empty() {
                Err(SettingError::Empty(name))
            } else {
                Ok(value)
            }
        };
        let issuer = not_empty(issuer, "EMBERHUT_RELAY_CLERK_ISSUER")?;
        let audience = not_empty(args.clerk_audience, "EMBERHUT_RELAY_CLERK_AUDIENCE")?;
        let access_token_claim = not_empty(
            args.clerk_access_token_claim,
            "EMBERHUT_RELAY_CLERK_ACCESS_TOKEN_CLAIM",
        )?;
        let client_ids = [args.clerk_native_client_id, args.clerk_web_client_id]
            .into_iter()
            .flatten()
            .filter(|id| !id.trim().is_empty())
            .collect();
        let allowed_origins = args
            .clerk_allowed_origins
            .into_iter()
            .map(|origin| origin.trim().to_owned())
            .filter(|origin| !origin.is_empty())
            .collect();
        let config = DirectoryConfig {
            relay_domain: self.domain,
            database: args.database,
            account_cap: args.account_cap,
            limits: RequestLimits::default(),
        };
        let clerk = ClerkSettings {
            issuer,
            jwks_url,
            audience,
            client_ids,
            allowed_origins,
            access_token_claim,
            key_timing: KeyTiming::default(),
        };
        Ok((config, clerk))
    }
}

/// A `tracing` filter, checked when it is read.
#[derive(Clone, Debug)]
pub struct LogFilter(String);

impl LogFilter {
    /// The filter, ready for the subscriber.
    #[must_use]
    pub fn to_env_filter(&self) -> EnvFilter {
        // The text was parsed strictly when this value was made, so nothing is lost here.
        EnvFilter::builder().parse_lossy(&self.0)
    }
}

impl FromStr for LogFilter {
    type Err = InvalidLogFilter;

    fn from_str(filter: &str) -> Result<Self, Self::Err> {
        EnvFilter::try_new(filter).map_err(InvalidLogFilter)?;
        Ok(Self(filter.to_owned()))
    }
}

/// The text is not a log filter.
#[derive(Debug, thiserror::Error)]
#[error("not a log filter: {0}")]
pub struct InvalidLogFilter(ParseError);

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    fn serve(args: &[&str]) -> ServeArgs {
        let mut all = vec!["emberhut-relay", "serve"];
        all.extend_from_slice(args);
        let Command::Serve(serve) = Cli::try_parse_from(all).unwrap().command;
        serve
    }

    #[test]
    fn the_command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_directory_role_takes_its_settings_and_defaults() {
        let args = serve(&[
            "--role",
            "directory",
            "--domain",
            "relay.example",
            "--clerk-issuer",
            "https://clerk.example",
            "--clerk-jwks-url",
            "https://clerk.example/.well-known/jwks.json",
            "--clerk-native-client-id",
            "native",
            "--clerk-allowed-origin",
            "https://app.example, https://www.example",
        ]);
        assert_eq!(args.role, Role::Directory);
        assert_eq!(args.listen, "127.0.0.1:18980".parse().unwrap());
        let (config, clerk) = args.directory_settings().unwrap();
        assert_eq!(config.account_cap.get(), 5);
        assert_eq!(config.relay_domain.as_str(), "relay.example");
        assert_eq!(clerk.audience, "emberhut-directory");
        assert_eq!(clerk.access_token_claim, "client_id");
        assert_eq!(clerk.client_ids, ["native"]);
        assert_eq!(
            clerk.allowed_origins,
            ["https://app.example", "https://www.example"]
        );
    }

    #[test]
    fn settings_that_mix_up_kinds_of_token_stop_the_start() {
        use std::sync::Arc;

        use crate::directory::{ClerkSetupError, ClerkVerifier, SystemClock};

        let base = [
            "--role",
            "directory",
            "--domain",
            "relay.example",
            "--clerk-issuer",
            "https://clerk.example",
            "--clerk-jwks-url",
            "https://clerk.example/.well-known/jwks.json",
            "--clerk-native-client-id",
            "native",
        ];
        let verifier = |extra: &[&str]| {
            let mut args = base.to_vec();
            args.extend_from_slice(extra);
            let (_, clerk) = serve(&args).directory_settings().unwrap();
            ClerkVerifier::new(clerk, Arc::new(SystemClock))
        };
        assert!(verifier(&[]).is_ok());
        assert!(matches!(
            verifier(&["--clerk-audience", "native"]),
            Err(ClerkSetupError::AudienceIsAClient)
        ));
        assert!(matches!(
            verifier(&["--clerk-access-token-claim", "azp"]),
            Err(ClerkSetupError::SharedClaim(_))
        ));
    }

    #[test]
    fn clerk_settings_are_required_for_the_directory() {
        let args = serve(&["--role", "directory", "--domain", "relay.example"]);
        assert!(matches!(
            args.directory_settings(),
            Err(SettingError::Missing("EMBERHUT_RELAY_CLERK_ISSUER"))
        ));
    }

    #[test]
    fn the_role_defaults_to_all_and_bad_values_are_refused() {
        assert_eq!(serve(&["--domain", "relay.example"]).role, Role::All);
        for bad in [
            ["--domain", "relay.example:443"].as_slice(),
            &["--domain", "relay.example", "--account-cap", "0"],
            &[
                "--domain",
                "relay.example",
                "--clerk-jwks-url",
                "http://clerk.example/jwks",
            ],
        ] {
            let mut all = vec!["emberhut-relay", "serve"];
            all.extend_from_slice(bad);
            assert!(Cli::try_parse_from(all).is_err(), "{bad:?}");
        }
    }
}
