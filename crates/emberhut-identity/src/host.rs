//! Hosts in their normal form, as the probe signs them and as a route's host is compared.
//!
//! The normal form of a host (a name or an address, and maybe a port) is:
//!
//! - a DNS name: lowercase, internationalised labels as IDNA A-labels (UTS 46 with the STD3
//!   rules but `_` allowed, hyphens checked and DNS lengths verified), so only letters, digits,
//!   hyphens and underscores (container and `.local` names use them), no hyphen at either end
//!   of a label, labels of 1 to 63 characters, at most 253 in all, and no trailing dot;
//! - or an IPv4 address in dotted decimal, or an IPv6 address in RFC 5952 form inside
//!   brackets, without a zone, and not IPv4-mapped;
//! - with the port dropped when it is the scheme's default (`:443` for `https`, `:80` for
//!   `http`), and kept otherwise.
//!
//! A name that ends in a number must be a dotted-decimal IPv4 address: the other spellings URL
//! parsers accept for addresses (`0x7f.1`, `127.1`, octal) are refused, so an address has one
//! spelling. `docs/api.md` lists every difference from how URLs read hosts.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

/// STD3's deny list less `_`: every ASCII character but letters, digits, `-`, `.` and `_`
/// (controls and the space included, through the first argument).
const HOST_NAME_DENY_LIST: AsciiDenyList =
    AsciiDenyList::new(true, "!\"#$%&'()*+,/:;<=>?@[\\]^`{|}~");

/// A URL scheme a host is reached with, for its default port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// `http`, port 80.
    Http,
    /// `https`, port 443.
    Https,
}

impl Scheme {
    /// The port a URL of this scheme leaves out.
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

/// Why text isn't a host, or isn't one in its normal form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// There is no name.
    #[error("the host is empty")]
    Empty,
    /// The port isn't a number from 1 to 65535.
    #[error("the port isn't a number from 1 to 65535")]
    Port,
    /// An IPv6 address that isn't valid, has a zone id, is IPv4-mapped, or isn't in brackets.
    #[error("not an IPv6 address in brackets, without a zone id and not IPv4-mapped")]
    Ipv6,
    /// A name that isn't a DNS name: a character other than a letter, digit, hyphen or
    /// underscore (after IDNA mapping), a hyphen at a label's end, an empty or over-long label,
    /// a trailing dot, or more than 253 characters.
    #[error("not a DNS name")]
    Name,
    /// A name ending in a number that isn't a dotted-decimal IPv4 address.
    #[error("a name ending in a number must be a dotted-decimal IPv4 address")]
    NumericName,
    /// A valid host, but not written in its normal form.
    #[error("not in normal form; that is {normal}")]
    NotNormal {
        /// The same host in its normal form.
        normal: String,
    },
}

/// A host in its normal form: a name or an address, and a port unless it is the scheme's
/// default.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Host {
    text: String,
    name_len: usize,
}

impl Host {
    /// The normal form of `authority` (`name`, `name:port`, `[v6]:port`...), reached with
    /// `scheme`: the scheme's default port is dropped.
    ///
    /// # Errors
    ///
    /// A [`HostError`] when `authority` isn't a host.
    pub fn normalize(authority: &str, scheme: Scheme) -> Result<Self, HostError> {
        let (name, port) = parse(authority)?;
        Ok(Self::from_parts(
            name,
            port.filter(|&port| port != scheme.default_port()),
        ))
    }

    fn from_parts(name: String, port: Option<u16>) -> Self {
        let name_len = name.len();
        let text = match port {
            Some(port) => format!("{name}:{port}"),
            None => name,
        };
        Self { text, name_len }
    }

    /// The host as text, in its normal form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The name or address, without the port (an IPv6 address keeps its brackets).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.text[..self.name_len]
    }

    /// The port, unless it is the scheme's default.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.text
            .get(self.name_len + 1..)
            .and_then(|port| port.parse().ok())
    }

    /// The address, when the host is an IP address rather than a name.
    #[must_use]
    pub fn address(&self) -> Option<IpAddr> {
        let name = self.name();
        match name.strip_prefix('[').and_then(|v6| v6.strip_suffix(']')) {
            Some(v6) => v6.parse::<Ipv6Addr>().ok().map(IpAddr::V6),
            None => name.parse::<Ipv4Addr>().ok().map(IpAddr::V4),
        }
    }
}

impl FromStr for Host {
    type Err = HostError;

    /// Reads a host already in its normal form, and refuses any other spelling. Whether a port
    /// is its scheme's default can't be told here: compare with [`Host::normalize`] of the
    /// route's own host, which drops it.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (name, port) = parse(text)?;
        let host = Self::from_parts(name, port);
        if host.text == text {
            Ok(host)
        } else {
            Err(HostError::NotNormal { normal: host.text })
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Host({})", self.text)
    }
}

/// The normal name and the port (any, the default included) of `authority`.
fn parse(authority: &str) -> Result<(String, Option<u16>), HostError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (inside, after) = rest.split_once(']').ok_or(HostError::Ipv6)?;
        let address: Ipv6Addr = inside.parse().map_err(|_| HostError::Ipv6)?;
        if address.to_ipv4_mapped().is_some() {
            return Err(HostError::Ipv6);
        }
        let port = match after {
            "" => None,
            after => Some(parse_port(after.strip_prefix(':').ok_or(HostError::Port)?)?),
        };
        return Ok((format!("[{address}]"), port));
    }
    if authority.matches(':').count() > 1 {
        return Err(HostError::Ipv6);
    }
    let (name, port) = match authority.split_once(':') {
        Some((name, port)) => (name, Some(parse_port(port)?)),
        None => (authority, None),
    };
    Ok((normal_name(name)?, port))
}

/// A port: 1 to 65535, in ASCII digits.
fn parse_port(text: &str) -> Result<u16, HostError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HostError::Port);
    }
    match text.parse::<u16>() {
        Ok(0) | Err(_) => Err(HostError::Port),
        Ok(port) => Ok(port),
    }
}

/// A name's normal form: a dotted-decimal IPv4 address, or a DNS name's ASCII form.
pub(crate) fn normal_name(name: &str) -> Result<String, HostError> {
    if name.is_empty() {
        return Err(HostError::Empty);
    }
    if let Ok(address) = name.parse::<Ipv4Addr>() {
        return Ok(address.to_string());
    }
    let ascii = Uts46::new()
        .to_ascii(
            name.as_bytes(),
            HOST_NAME_DENY_LIST,
            Hyphens::Check,
            DnsLength::Verify,
        )
        .map_err(|_| HostError::Name)?;
    if !ends_in_a_number(&ascii) {
        return Ok(ascii.into_owned());
    }
    // Full-width digits and dots map to ASCII ones, so an address can arrive this way.
    let address: Ipv4Addr = ascii.parse().map_err(|_| HostError::NumericName)?;
    Ok(address.to_string())
}

/// Whether the last label is a number to a URL parser: decimal digits, or `0x` and hexadecimal
/// digits.
fn ends_in_a_number(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    if !last.is_empty() && last.bytes().all(|byte| byte.is_ascii_digit()) {
        return true;
    }
    last.strip_prefix("0x")
        .is_some_and(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn normal(authority: &str, scheme: Scheme) -> Result<String, HostError> {
        Host::normalize(authority, scheme).map(|host| host.as_str().to_owned())
    }

    #[test]
    fn names_are_lowercase_a_labels() {
        assert_eq!(
            normal("Example.COM", Scheme::Https).as_deref(),
            Ok("example.com")
        );
        assert_eq!(
            normal("Bücher.example:8443", Scheme::Https).as_deref(),
            Ok("xn--bcher-kva.example:8443")
        );
    }

    #[test]
    fn default_ports_are_dropped_and_others_kept() {
        assert_eq!(
            normal("example.com:443", Scheme::Https).as_deref(),
            Ok("example.com")
        );
        assert_eq!(
            normal("example.com:80", Scheme::Https).as_deref(),
            Ok("example.com:80")
        );
        assert_eq!(
            normal("example.com:080", Scheme::Http).as_deref(),
            Ok("example.com")
        );
        let host = Host::normalize("[2001:DB8::1]:18990", Scheme::Http).unwrap();
        assert_eq!(host.as_str(), "[2001:db8::1]:18990");
        assert_eq!(host.name(), "[2001:db8::1]");
        assert_eq!(host.port(), Some(18990));
        assert_eq!(host.address(), Some("2001:db8::1".parse().unwrap()));
        let bare = Host::normalize("example.com", Scheme::Http).unwrap();
        assert_eq!(
            (bare.name(), bare.port(), bare.address()),
            ("example.com", None, None)
        );
    }

    #[test]
    fn addresses_have_one_spelling() {
        assert_eq!(
            normal("[2001:0db8:0:0::1]", Scheme::Https).as_deref(),
            Ok("[2001:db8::1]")
        );
        assert_eq!(
            normal("１９２.０.２.１", Scheme::Http).as_deref(),
            Ok("192.0.2.1")
        );
        for refused in ["0x7f.1", "127.1", "192.0.2.010", "1.2.3.4.5", "example.0x"] {
            assert_eq!(
                normal(refused, Scheme::Http),
                Err(HostError::NumericName),
                "{refused}"
            );
        }
        for mapped in ["[::ffff:192.0.2.1]", "[::ffff:c000:201]"] {
            assert_eq!(
                normal(mapped, Scheme::Http),
                Err(HostError::Ipv6),
                "{mapped}"
            );
        }
    }

    #[test]
    fn names_are_dns_names() {
        assert_eq!(
            normal("My_Box.local:18990", Scheme::Http).as_deref(),
            Ok("my_box.local:18990")
        );
        for refused in [
            "a*b.example",
            "re;lay.example",
            "-bad-.example",
            "ab--cd.example",
            "example.com.",
            "a..b",
            "ex%61mple.com",
            "user@example.com",
        ] {
            assert_eq!(
                normal(refused, Scheme::Https),
                Err(HostError::Name),
                "{refused}"
            );
        }
        let label = "a".repeat(63);
        assert!(normal(&format!("{label}.example"), Scheme::Https).is_ok());
        assert_eq!(
            normal(&format!("a{label}.example"), Scheme::Https),
            Err(HostError::Name)
        );
        let long = [label.as_str(); 4].join(".");
        assert_eq!(long.len(), 255);
        assert_eq!(normal(&long, Scheme::Https), Err(HostError::Name));
        assert!(normal(&long[2..], Scheme::Https).is_ok(), "253 characters");
    }

    #[test]
    fn bad_hosts_are_refused() {
        assert_eq!(normal("", Scheme::Http), Err(HostError::Empty));
        assert_eq!(normal(":80", Scheme::Http), Err(HostError::Empty));
        assert_eq!(normal("example.com:", Scheme::Http), Err(HostError::Port));
        assert_eq!(normal("example.com:0", Scheme::Http), Err(HostError::Port));
        assert_eq!(
            normal("example.com:65536", Scheme::Http),
            Err(HostError::Port)
        );
        assert_eq!(
            normal("example.com:+80", Scheme::Http),
            Err(HostError::Port)
        );
        assert_eq!(normal("::1", Scheme::Http), Err(HostError::Ipv6));
        assert_eq!(
            normal("[fe80::1%25en0]", Scheme::Http),
            Err(HostError::Ipv6)
        );
        assert_eq!(normal("[::1]80", Scheme::Http), Err(HostError::Port));
    }

    #[test]
    fn parsing_wants_the_normal_form() {
        assert_eq!(
            "Example.com".parse::<Host>(),
            Err(HostError::NotNormal {
                normal: "example.com".to_owned()
            })
        );
        assert!(
            "example.com:443".parse::<Host>().is_ok(),
            "the scheme isn't known here"
        );
        assert_eq!(
            "[::1]:018990".parse::<Host>(),
            Err(HostError::NotNormal {
                normal: "[::1]:18990".to_owned()
            })
        );
    }

    proptest! {
        #[test]
        fn normal_forms_are_fixed_points(authority in "[a-zA-Z0-9.:\\[\\]-]{0,40}") {
            if let Ok(host) = Host::normalize(&authority, Scheme::Https) {
                prop_assert_eq!(host.as_str().parse::<Host>(), Ok(host.clone()));
                prop_assert_eq!(Host::normalize(host.as_str(), Scheme::Https), Ok(host));
            }
        }

        #[test]
        fn host_parsing_never_panics(text in "\\PC{0,60}") {
            let _host = Host::normalize(&text, Scheme::Http);
            let _parsed = text.parse::<Host>();
        }
    }
}
