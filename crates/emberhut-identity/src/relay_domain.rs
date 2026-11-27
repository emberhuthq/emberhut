//! A relay's domain, under which it serves machines, `connect.` and `api.`.

use std::fmt;
use std::str::FromStr;

use crate::host::{Host, HostError, Scheme};
use crate::machine_id::MachineId;

/// A relay's domain: a DNS name in normal form ([`Host`]'s rules, but strict STD3: no `_`,
/// which certificates and the Public Suffix List don't take), without a port, under which the
/// relay serves `<machine id>.<relay domain>`, `connect.` and `api.`.
///
/// It is at most 220 characters, so that `<machine id>.<relay domain>` stays within DNS's 253.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelayDomain(String);

/// Why text isn't a relay domain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum RelayDomainError {
    /// Not a host at all, or not in its normal form.
    #[error("not a relay domain")]
    Host(#[from] HostError),
    /// An address, or a port: a relay domain is a bare DNS name.
    #[error("not a relay domain: it must be a DNS name without a port")]
    NotAName,
    /// Too long for `<machine id>.<relay domain>` to be a DNS name.
    #[error("not a relay domain: over 220 characters")]
    TooLong,
}

impl RelayDomain {
    /// The longest relay domain, in characters: 253 less a machine id and its dot.
    pub const MAX_LEN: usize = 253 - MachineId::LEN - 1;

    /// The normal form of `text` as a relay domain (lowercase, A-labels).
    ///
    /// # Errors
    ///
    /// A [`RelayDomainError`] when it isn't one.
    pub fn normalize(text: &str) -> Result<Self, RelayDomainError> {
        if text.contains(':') {
            return Err(RelayDomainError::NotAName);
        }
        Self::from_host(&Host::normalize(text, Scheme::Https)?)
    }

    fn from_host(host: &Host) -> Result<Self, RelayDomainError> {
        if host.port().is_some() || host.address().is_some() {
            return Err(RelayDomainError::NotAName);
        }
        if host.as_str().contains('_') {
            return Err(RelayDomainError::Host(HostError::Name));
        }
        if host.as_str().len() > Self::MAX_LEN {
            return Err(RelayDomainError::TooLong);
        }
        Ok(Self(host.as_str().to_owned()))
    }

    /// The domain as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The machine's own origin at this relay: `https://<machine id>.<relay domain>`.
    #[must_use]
    pub fn machine_url(&self, machine: &MachineId) -> String {
        format!("https://{machine}.{}", self.0)
    }
}

impl FromStr for RelayDomain {
    type Err = RelayDomainError;

    /// Reads a relay domain already in its normal form.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.contains(':') {
            return Err(RelayDomainError::NotAName);
        }
        Self::from_host(&text.parse()?)
    }
}

impl fmt::Display for RelayDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RelayDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RelayDomain({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn relay_domains_are_bare_dns_names() {
        let domain: RelayDomain = "relay.example".parse().unwrap();
        assert_eq!(domain.as_str(), "relay.example");
        assert_eq!(
            RelayDomain::normalize("Relay.Example").map(|d| d.to_string()),
            Ok("relay.example".to_owned())
        );
        for refused in ["relay.example:443", "192.0.2.1", "[2001:db8::1]"] {
            assert_eq!(
                RelayDomain::normalize(refused),
                Err(RelayDomainError::NotAName),
                "{refused}"
            );
        }
        for refused in [
            "re;lay.example",
            "a,b.example",
            "-bad-.example",
            "relay.example.",
        ] {
            assert!(
                matches!(
                    RelayDomain::normalize(refused),
                    Err(RelayDomainError::Host(_))
                ),
                "{refused}"
            );
        }
        assert!("Relay.example".parse::<RelayDomain>().is_err());
    }

    #[test]
    fn the_longest_domain_leaves_room_for_an_id() {
        // A first label of `first` characters, three of 63, and `example`.
        let domain = |first: usize| {
            let mut labels = vec!["b".repeat(first)];
            labels.extend(vec!["a".repeat(63); 3]);
            labels.push("example".to_owned());
            labels.join(".")
        };
        assert_eq!(domain(20).len(), RelayDomain::MAX_LEN);
        let edge = RelayDomain::normalize(&domain(20)).unwrap();
        assert_eq!(
            RelayDomain::normalize(&domain(21)),
            Err(RelayDomainError::TooLong)
        );
        let id = MachineId::of(&crate::SecretKey::from_bytes(&[1; 32]).public_key());
        let url = edge.machine_url(&id);
        assert_eq!(url.len() - "https://".len(), 253);
    }

    proptest! {
        #[test]
        fn relay_domain_parsing_never_panics(text in "\\PC{0,60}") {
            let _domain = RelayDomain::normalize(&text);
        }
    }
}
