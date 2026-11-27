//! Socket addresses in their normal form, as the probe signs the address that accepted a
//! direct connection.

use std::fmt;
use std::net::{SocketAddr, SocketAddrV6};
use std::str::FromStr;

/// The local socket address that accepted a direct connection, in its normal form:
/// `a.b.c.d:port` or `[v6]:port`, the IPv6 address in RFC 5952 form (lowercase, shortest),
/// with no zone id, and an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) written as the IPv4
/// address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalAddr(SocketAddr);

/// Why text isn't a socket address in its normal form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum LocalAddrError {
    /// Not a socket address at all.
    #[error("not a socket address (a.b.c.d:port or [v6]:port)")]
    Invalid,
    /// A socket address, but not written in its normal form.
    #[error("not in normal form; that is {normal}")]
    NotNormal {
        /// The same address in its normal form.
        normal: String,
    },
}

impl LocalAddr {
    /// The normal form of `addr`: an IPv4-mapped address becomes IPv4, and an IPv6 address
    /// loses its flow label and zone.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        let addr = match addr.ip().to_canonical() {
            std::net::IpAddr::V4(ip) => SocketAddr::from((ip, addr.port())),
            std::net::IpAddr::V6(ip) => SocketAddr::V6(SocketAddrV6::new(ip, addr.port(), 0, 0)),
        };
        Self(addr)
    }

    /// The address.
    #[must_use]
    pub const fn get(self) -> SocketAddr {
        self.0
    }
}

impl From<SocketAddr> for LocalAddr {
    fn from(addr: SocketAddr) -> Self {
        Self::new(addr)
    }
}

impl FromStr for LocalAddr {
    type Err = LocalAddrError;

    /// Reads an address already in its normal form, and refuses any other spelling.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let addr: SocketAddr = text.parse().map_err(|_| LocalAddrError::Invalid)?;
        let local = Self::new(addr);
        let normal = local.to_string();
        if normal == text {
            Ok(local)
        } else {
            Err(LocalAddrError::NotNormal { normal })
        }
    }
}

impl fmt::Display for LocalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The normal form has no zone or flow label, so std's form is the normal one.
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for LocalAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalAddr({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn mapped_addresses_are_ipv4_and_zones_are_dropped() {
        let mapped: SocketAddr = "[::ffff:192.0.2.1]:18990".parse().unwrap();
        assert_eq!(LocalAddr::new(mapped).to_string(), "192.0.2.1:18990");
        let zoned = SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 18990, 7, 3));
        assert_eq!(LocalAddr::new(zoned).to_string(), "[fe80::1]:18990");
    }

    #[test]
    fn parsing_wants_the_normal_form() {
        assert_eq!(
            "[2001:db8::1]:18990"
                .parse::<LocalAddr>()
                .map(|a| a.to_string()),
            Ok("[2001:db8::1]:18990".to_owned())
        );
        assert_eq!(
            "[2001:DB8:0::1]:18990".parse::<LocalAddr>(),
            Err(LocalAddrError::NotNormal {
                normal: "[2001:db8::1]:18990".to_owned()
            })
        );
        assert_eq!(
            "[::ffff:192.0.2.1]:80".parse::<LocalAddr>(),
            Err(LocalAddrError::NotNormal {
                normal: "192.0.2.1:80".to_owned()
            })
        );
        assert_eq!(
            "192.0.2.1".parse::<LocalAddr>(),
            Err(LocalAddrError::Invalid)
        );
        assert_eq!("".parse::<LocalAddr>(), Err(LocalAddrError::Invalid));
    }

    proptest! {
        #[test]
        fn normal_forms_round_trip(v4 in any::<[u8; 4]>(), v6 in any::<[u16; 8]>(), port in any::<u16>()) {
            for addr in [
                SocketAddr::from((Ipv4Addr::from(v4), port)),
                SocketAddr::from((Ipv6Addr::from(v6), port)),
            ] {
                let local = LocalAddr::new(addr);
                prop_assert_eq!(local.to_string().parse::<LocalAddr>(), Ok(local));
            }
        }
    }
}
