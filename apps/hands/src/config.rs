//! The helper's options: where it listens, its token file, the server recordings go to, and the
//! extension's folder. Every one can come from the environment (the launch agent sets them).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use clap::Parser;
use url::Url;

use crate::network::{NetworkError, check_listen, check_server};

/// The port the helper listens on: next to the reference helper's, which keeps running.
pub const PORT: u16 = 8794;

/// Emberhut Hands: the assistant's hands on this Mac.
#[derive(Clone, Debug, Parser)]
#[command(name = "emberhut-hands", version, about, long_about = None)]
pub struct Config {
    /// The port, on 127.0.0.1 and on --listen's address.
    #[arg(long, env = "EMBERHUT_HANDS_PORT", default_value_t = PORT)]
    pub port: u16,

    /// One more address to listen on, for the server on another machine: a private address
    /// (10/8, 172.16/12, 192.168/16, 100.64/10, `fc00::/7`), and only callers in its range (and
    /// this Mac) are answered; the bearer token is checked on every request too.
    #[arg(long, env = "EMBERHUT_HANDS_LISTEN", value_name = "IP")]
    pub listen: Option<IpAddr>,

    /// The file holding the bearer token the server sends; made with a new token when missing.
    /// Default: `~/Library/Application Support/Emberhut/Hands/token`.
    #[arg(long, env = "EMBERHUT_HANDS_TOKEN_FILE", value_name = "FILE")]
    pub token_file: Option<PathBuf>,

    /// The server the recorded calls go to (`https://…`, or `http://` to a loopback, private or
    /// `.local` address).
    #[arg(
        long,
        env = "EMBERHUT_HANDS_SERVER_URL",
        value_name = "URL",
        requires = "server_token_file"
    )]
    pub server_url: Option<Url>,

    /// The file holding the device token the helper signs in to the server with.
    #[arg(
        long,
        env = "EMBERHUT_HANDS_SERVER_TOKEN_FILE",
        value_name = "FILE",
        requires = "server_url"
    )]
    pub server_token_file: Option<PathBuf>,

    /// The browser extension's folder, loaded unpacked in Chrome.
    /// Default: `~/Library/Application Support/Emberhut/Extension`.
    #[arg(long, env = "EMBERHUT_HANDS_EXTENSION_DIR", value_name = "DIR")]
    pub extension_dir: Option<PathBuf>,
}

impl Config {
    /// Checks `--listen` and `--server-url` (see [`crate::network`]).
    ///
    /// # Errors
    ///
    /// [`NetworkError`] for a public listen address, or plain http to a public server.
    pub fn check(&self) -> Result<(), NetworkError> {
        if let Some(ip) = self.listen {
            check_listen(ip)?;
        }
        if let Some(url) = &self.server_url {
            check_server(url)?;
        }
        Ok(())
    }

    /// The addresses to listen on: 127.0.0.1, and --listen's.
    #[must_use]
    pub fn addresses(&self) -> Vec<SocketAddr> {
        let mut addresses = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)];
        if let Some(ip) = self
            .listen
            .filter(|ip| *ip != IpAddr::V4(Ipv4Addr::LOCALHOST))
        {
            addresses.push(SocketAddr::new(ip, self.port));
        }
        addresses
    }
}

/// The helper's places under the owner's Application Support folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// `~/Library/Application Support/Emberhut`.
    pub emberhut: PathBuf,
}

impl Places {
    /// The places under `home`.
    #[must_use]
    pub fn under(home: &std::path::Path) -> Self {
        Self {
            emberhut: home
                .join("Library")
                .join("Application Support")
                .join("Emberhut"),
        }
    }

    /// The helper's own folder.
    #[must_use]
    pub fn hands(&self) -> PathBuf {
        self.emberhut.join("Hands")
    }

    /// The default token file.
    #[must_use]
    pub fn token(&self) -> PathBuf {
        self.hands().join("token")
    }

    /// Recorded calls waiting to be sent.
    #[must_use]
    pub fn calls(&self) -> PathBuf {
        self.hands().join("calls")
    }

    /// The default extension folder.
    #[must_use]
    pub fn extension(&self) -> PathBuf {
        self.emberhut.join("Extension")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_is_well_formed() {
        Config::command().debug_assert();
    }

    #[test]
    fn it_listens_on_localhost_and_the_extra_address() {
        let config = Config::try_parse_from(["emberhut-hands", "--listen", "10.0.0.5"]).unwrap();
        assert_eq!(config.port, PORT);
        let addresses: Vec<String> = config.addresses().iter().map(ToString::to_string).collect();
        assert_eq!(addresses, ["127.0.0.1:8794", "10.0.0.5:8794"]);
        let alone = Config::try_parse_from(["emberhut-hands", "--listen", "127.0.0.1"]).unwrap();
        assert_eq!(alone.addresses().len(), 1);
        assert!(config.check().is_ok());
        let open = Config::try_parse_from(["emberhut-hands", "--listen", "0.0.0.0"]).unwrap();
        assert!(open.check().is_err());
    }

    #[test]
    fn the_server_url_and_its_token_file_come_together() {
        assert!(
            Config::try_parse_from(["emberhut-hands", "--server-url", "https://emberhut.example"])
                .is_err()
        );
        assert!(
            Config::try_parse_from([
                "emberhut-hands",
                "--server-url",
                "https://emberhut.example",
                "--server-token-file",
                "/tmp/t"
            ])
            .is_ok()
        );
    }

    #[test]
    fn places_are_under_application_support() {
        let places = Places::under(Path::new("/Users/example"));
        assert_eq!(
            places.token(),
            Path::new("/Users/example/Library/Application Support/Emberhut/Hands/token")
        );
        assert_eq!(
            places.extension(),
            Path::new("/Users/example/Library/Application Support/Emberhut/Extension")
        );
        assert_eq!(
            places.calls(),
            Path::new("/Users/example/Library/Application Support/Emberhut/Hands/calls")
        );
    }
}
