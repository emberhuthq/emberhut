//! Which addresses the helper may listen on and answer, and which servers it may send to over
//! plain http: loopback and the private ranges (`emberhut_core::private_network`), never the open
//! internet.

use std::net::IpAddr;

use emberhut_core::private_network::{
    self, NotPrivate, PrivateRange as Range, ServerUrlError, check_server_url, is_loopback,
};
use url::Url;

/// Why `--listen` or `--server-url` is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetworkError {
    /// `--listen` names an address outside the private ranges.
    #[error("--listen {0}")]
    Listen(#[from] NotPrivate),
    /// `--server-url` is plain http to a public host, or not http at all.
    #[error("--server-url {0}")]
    Server(#[from] ServerUrlError),
}

/// Checks a `--listen` address: a private one (`emberhut_core::private_network::check_listen`).
///
/// # Errors
///
/// [`NetworkError::Listen`] for any other.
pub fn check_listen(ip: IpAddr) -> Result<(), NetworkError> {
    Ok(private_network::check_listen(ip)?)
}

/// Checks a `--server-url` (`emberhut_core::private_network::check_server_url`).
///
/// # Errors
///
/// [`NetworkError::Server`] for one refused.
pub fn check_server(url: &Url) -> Result<(), NetworkError> {
    Ok(check_server_url(url)?)
}

/// The addresses the helper answers: loopback, and the private range of its `--listen` address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Callers {
    range: Option<Range>,
}

impl Callers {
    /// The callers for a helper listening on `listen` too.
    #[must_use]
    pub fn for_listen(listen: Option<IpAddr>) -> Self {
        Self {
            range: listen.and_then(Range::of),
        }
    }

    /// Whether a request from `ip` is answered.
    #[must_use]
    pub fn allow(&self, ip: IpAddr) -> bool {
        is_loopback(ip) || (self.range.is_some() && Range::of(ip) == self.range)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// An address in the carrier-grade NAT range, 100.64/10.
    fn cgnat(last: u8) -> String {
        Ipv4Addr::new(100, 64, 7, last).to_string()
    }

    #[test]
    fn only_private_addresses_are_listened_on() {
        for ok in [
            "10.0.0.5",
            "172.16.0.5",
            "192.168.1.10",
            &cgnat(1),
            "fd12::1",
        ] {
            assert!(check_listen(ip(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "0.0.0.0",
            "::",
            "8.8.8.8",
            "172.32.0.1",
            &Ipv4Addr::new(100, 128, 0, 1).to_string(),
            "127.0.0.1",
        ] {
            assert!(check_listen(ip(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn callers_are_loopback_and_the_listen_address_s_range() {
        let callers = Callers::for_listen(Some(ip(&cgnat(2))));
        assert!(callers.allow(ip("127.0.0.1")));
        assert!(callers.allow(ip("::1")));
        assert!(callers.allow(ip(&cgnat(3))));
        assert!(!callers.allow(ip("192.168.1.11")));
        assert!(!callers.allow(ip("8.8.8.8")));
        let alone = Callers::for_listen(None);
        assert!(alone.allow(ip("127.0.0.1")));
        assert!(!alone.allow(ip("10.0.0.5")));
    }

    #[test]
    fn plain_http_only_to_local_or_private_servers() {
        let check = |url: &str| check_server(&Url::parse(url).unwrap());
        assert!(check("https://emberhut.example").is_ok());
        assert!(check("http://127.0.0.1:8791").is_ok());
        assert!(check(&format!("http://{}:8791", cgnat(4))).is_ok());
        assert!(check("http://server.local:8791").is_ok());
        assert!(check("http://emberhut.example").is_err());
        assert!(check("http://8.8.8.8").is_err());
        assert!(check("ftp://emberhut.example").is_err());
    }
}
