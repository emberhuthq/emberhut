//! Private network addresses: loopback, and the private ranges (RFC 1918, the carrier-grade
//! NAT range of RFC 6598 that private meshes use, and IPv6 unique-local, RFC 4193). Plain http
//! is taken only to these (and to `localhost` and `.local` names): a self-hosted server is
//! often reached that way, without a certificate, and nothing on the open internet is.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// A private range of addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateRange {
    /// 10/8.
    Ten,
    /// 172.16/12.
    OneSevenTwo,
    /// 192.168/16.
    OneNineTwo,
    /// 100.64/10, carrier-grade NAT.
    Cgnat,
    /// `fc00::/7`, IPv6 unique-local.
    UniqueLocal,
}

impl PrivateRange {
    /// The private range `ip` is in (an IPv4 address mapped into IPv6 too).
    #[must_use]
    pub fn of(ip: IpAddr) -> Option<Self> {
        match ip {
            IpAddr::V4(v4) => Self::of_v4(v4),
            IpAddr::V6(v6) => v6
                .to_ipv4_mapped()
                .and_then(Self::of_v4)
                .or_else(|| is_unique_local(v6).then_some(Self::UniqueLocal)),
        }
    }

    fn of_v4(ip: Ipv4Addr) -> Option<Self> {
        let [a, b, ..] = ip.octets();
        match (a, b) {
            (10, _) => Some(Self::Ten),
            (172, 16..=31) => Some(Self::OneSevenTwo),
            (192, 168) => Some(Self::OneNineTwo),
            (100, 64..=127) => Some(Self::Cgnat),
            _ => None,
        }
    }
}

/// Whether `ip` is in `fc00::/7`.
fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

/// Whether `ip` is a loopback address (an IPv4 one mapped into IPv6 too).
#[must_use]
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// Why an address isn't taken to listen on: it isn't private.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{0} is not a private address (10/8, 172.16/12, 192.168/16, 100.64/10 or fc00::/7): \
     nothing listens on the open internet"
)]
pub struct NotPrivate(pub IpAddr);

/// Checks an address to listen on for a server elsewhere: a private one.
///
/// # Errors
///
/// [`NotPrivate`] for any other.
pub fn check_listen(ip: IpAddr) -> Result<(), NotPrivate> {
    match PrivateRange::of(ip) {
        Some(_) => Ok(()),
        None => Err(NotPrivate(ip)),
    }
}

/// Why a server's URL isn't taken.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServerUrlError {
    /// Plain http to a host that isn't local or private.
    #[error(
        "{0} is plain http to a public host: use https, or a loopback, private or .local address"
    )]
    PlainHttp(String),
    /// Neither http nor https.
    #[error("{0} is not an http or https URL")]
    Scheme(String),
}

/// Checks a server's URL: https anywhere, plain http only to a loopback or private address,
/// `localhost` or a `.local` name.
///
/// # Errors
///
/// [`ServerUrlError`] for any other.
pub fn check_server_url(url: &Url) -> Result<(), ServerUrlError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if plain_http_allowed(url) => Ok(()),
        "http" => Err(ServerUrlError::PlainHttp(url.to_string())),
        _ => Err(ServerUrlError::Scheme(url.to_string())),
    }
}

/// Whether plain http may go to `url`'s host: a loopback or private address, `localhost`, or
/// a `.local` name.
fn plain_http_allowed(url: &Url) -> bool {
    let ip = |ip: IpAddr| is_loopback(ip) || PrivateRange::of(ip).is_some();
    match url.host() {
        Some(Host::Ipv4(v4)) => ip(v4.into()),
        Some(Host::Ipv6(v6)) => ip(v6.into()),
        Some(Host::Domain(name)) => {
            let name = name.to_lowercase();
            name == "localhost"
                || name
                    .rsplit_once('.')
                    .is_some_and(|(_, last)| last == "local")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn the_private_ranges_and_their_edges() {
        assert_eq!(PrivateRange::of(v4(10, 1, 2, 3)), Some(PrivateRange::Ten));
        assert_eq!(
            PrivateRange::of(v4(172, 31, 0, 1)),
            Some(PrivateRange::OneSevenTwo)
        );
        assert_eq!(PrivateRange::of(v4(172, 32, 0, 1)), None);
        assert_eq!(
            PrivateRange::of(v4(192, 168, 1, 5)),
            Some(PrivateRange::OneNineTwo)
        );
        assert_eq!(
            PrivateRange::of(v4(100, 64, 0, 1)),
            Some(PrivateRange::Cgnat)
        );
        assert_eq!(PrivateRange::of(v4(100, 128, 0, 1)), None);
        assert_eq!(
            PrivateRange::of("fd12::1".parse().unwrap()),
            Some(PrivateRange::UniqueLocal)
        );
        assert_eq!(PrivateRange::of(v4(8, 8, 8, 8)), None);
        assert_eq!(PrivateRange::of(v4(0, 0, 0, 0)), None);
        assert!(is_loopback(v4(127, 0, 0, 1)));
        assert!(is_loopback("::1".parse().unwrap()));
    }

    #[test]
    fn a_server_s_url_is_https_or_http_to_a_local_host() {
        let check = |url: &str| check_server_url(&Url::parse(url).unwrap());
        assert_eq!(check("https://emberhut.example"), Ok(()));
        assert_eq!(check("http://192.168.1.11:8791"), Ok(()));
        assert!(matches!(
            check("http://emberhut.example"),
            Err(ServerUrlError::PlainHttp(_))
        ));
        assert!(matches!(
            check("ftp://emberhut.example"),
            Err(ServerUrlError::Scheme(_))
        ));
        assert!(check_listen(v4(10, 0, 0, 2)).is_ok());
        assert_eq!(
            check_listen(v4(8, 8, 8, 8)),
            Err(NotPrivate(v4(8, 8, 8, 8)))
        );
    }

    #[test]
    fn plain_http_only_to_local_or_private_hosts() {
        let allowed = |url: &str| plain_http_allowed(&Url::parse(url).unwrap());
        assert!(allowed("http://127.0.0.1:8791"));
        assert!(allowed(&format!("http://{}:8791", v4(100, 64, 7, 4))));
        assert!(allowed("http://server.local:8791"));
        assert!(allowed("http://localhost"));
        assert!(!allowed("http://emberhut.example"));
        assert!(!allowed("http://8.8.8.8"));
    }
}
