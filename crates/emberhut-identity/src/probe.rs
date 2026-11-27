//! The probe: a machine proves, for one request, who it is and how and where it was reached.

use std::fmt;
use std::str::FromStr;

use crate::addr::LocalAddr;
use crate::host::Host;
use crate::machine_id::MachineId;
use crate::nonce::Nonce;
use crate::statement::{Encoder, Statement, prefix, sealed};

/// The route kind a probe checks, as the device sent it: 1 to 32 characters from `a-z`, `0-9`
/// and `-`. A server answers 400 to a probe whose kind isn't one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProbeKind(String);

/// Text that isn't a probe kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("a route kind is 1 to 32 characters from a-z, 0-9 and -")]
pub struct ProbeKindError;

impl ProbeKind {
    /// The longest kind, in characters.
    pub const MAX_LEN: usize = 32;

    /// The kind as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProbeKind {
    type Err = ProbeKindError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid = (1..=Self::MAX_LEN).contains(&text.len())
            && text
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(ProbeKindError)
        }
    }
}

impl fmt::Display for ProbeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a request reached the machine, as the probe's `via` says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    /// `direct`: over a TCP connection to the machine.
    Direct,
    /// `relayed`: through the relay.
    Relayed,
}

/// Text that isn't `direct` or `relayed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("via is direct or relayed")]
pub struct ViaError;

impl Via {
    /// The wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Relayed => "relayed",
        }
    }
}

impl FromStr for Via {
    type Err = ViaError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "direct" => Ok(Self::Direct),
            "relayed" => Ok(Self::Relayed),
            _ => Err(ViaError),
        }
    }
}

impl fmt::Display for Via {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a probe reached the machine, and for a direct connection, where: a direct answer always
/// has its local address, and a relayed one never has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reached {
    /// Over a TCP connection the machine accepted on this local socket address.
    Direct(LocalAddr),
    /// Through the relay.
    Relayed,
}

/// A `via` and a `local` that disagree: direct without an address, or relayed with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("a direct probe answer names its local address, and a relayed one doesn't")]
pub struct ReachedError;

impl Reached {
    /// The pair as the wire carries it: `via`, and `local` (none when relayed).
    ///
    /// # Errors
    ///
    /// [`ReachedError`] when they disagree.
    pub const fn new(via: Via, local: Option<LocalAddr>) -> Result<Self, ReachedError> {
        match (via, local) {
            (Via::Direct, Some(local)) => Ok(Self::Direct(local)),
            (Via::Relayed, None) => Ok(Self::Relayed),
            _ => Err(ReachedError),
        }
    }

    /// The statement's `via`.
    #[must_use]
    pub const fn via(&self) -> Via {
        match self {
            Self::Direct(_) => Via::Direct,
            Self::Relayed => Via::Relayed,
        }
    }

    /// The statement's `local`: the address of a direct connection, none when relayed.
    #[must_use]
    pub const fn local(&self) -> Option<LocalAddr> {
        match self {
            Self::Direct(local) => Some(*local),
            Self::Relayed => None,
        }
    }
}

/// `emberhut probe v1`: nonce, machine id, kind, via, local, host.
///
/// The machine signs the device's nonce and route kind together with how it was reached
/// ([`Reached`]) and the host it was asked for, so the answer is bound to that route: an answer
/// forwarded from another route carries that route's binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProbeStatement {
    nonce: Nonce,
    machine: MachineId,
    kind: ProbeKind,
    reached: Reached,
    host: Host,
}

impl ProbeStatement {
    /// The statement a machine signs for a probe.
    #[must_use]
    pub const fn new(
        nonce: Nonce,
        machine: MachineId,
        kind: ProbeKind,
        reached: Reached,
        host: Host,
    ) -> Self {
        Self {
            nonce,
            machine,
            kind,
            reached,
            host,
        }
    }

    /// The device's nonce.
    #[must_use]
    pub const fn nonce(&self) -> &Nonce {
        &self.nonce
    }

    /// The machine's id.
    #[must_use]
    pub const fn machine(&self) -> &MachineId {
        &self.machine
    }

    /// The route kind the device is checking, as it sent it.
    #[must_use]
    pub const fn kind(&self) -> &ProbeKind {
        &self.kind
    }

    /// How the machine was reached.
    #[must_use]
    pub const fn reached(&self) -> &Reached {
        &self.reached
    }

    /// The host the request asked for, in its normal form.
    #[must_use]
    pub const fn host(&self) -> &Host {
        &self.host
    }
}

impl sealed::Sealed for ProbeStatement {}

impl Statement for ProbeStatement {
    const PREFIX: &'static str = prefix::PROBE;

    fn to_bytes(&self) -> Vec<u8> {
        let local = self
            .reached
            .local()
            .map_or_default(|local| local.to_string());
        Encoder::new(Self::PREFIX)
            .bytes(self.nonce.as_bytes())
            .text(&self.machine.to_string())
            .text(self.kind.as_str())
            .text(self.reached.via().as_str())
            .text(&local)
            .text(self.host.as_str())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scheme, SecretKey};

    fn statement(reached: Reached, host: &str) -> ProbeStatement {
        ProbeStatement::new(
            Nonce::from_bytes([9; 32]),
            SecretKey::from_bytes(&[1; 32]).machine_id(),
            "lan".parse().unwrap(),
            reached,
            Host::normalize(host, Scheme::Http).unwrap(),
        )
    }

    #[test]
    fn a_relayed_probe_has_an_empty_local_field() {
        let relayed = statement(Reached::Relayed, "example.com").to_bytes();
        let tail: &[u8] = b"\0\0\0\x07relayed\0\0\0\0\0\0\0\x0bexample.com";
        assert!(relayed.ends_with(tail), "{relayed:?}");
    }

    #[test]
    fn the_binding_is_signed() {
        let key = SecretKey::from_bytes(&[1; 32]);
        let local: LocalAddr = "192.0.2.1:18990".parse().unwrap();
        let direct = statement(Reached::Direct(local), "192.0.2.1:18990");
        let signature = key.sign(&direct);
        assert_eq!(key.public_key().verify(&direct, &signature), Ok(()));
        let elsewhere = statement(Reached::Relayed, "192.0.2.1:18990");
        assert!(key.public_key().verify(&elsewhere, &signature).is_err());
        assert_eq!(direct.reached().via(), Via::Direct);
        assert_eq!(direct.reached().local(), Some(local));
        assert_eq!(direct.kind().as_str(), "lan");
        assert_eq!(direct.host().as_str(), "192.0.2.1:18990");
        assert_eq!(direct.nonce(), &Nonce::from_bytes([9; 32]));
        assert_eq!(direct.machine(), &key.machine_id());
    }

    #[test]
    fn via_and_local_must_agree() {
        let local: LocalAddr = "192.0.2.1:18990".parse().unwrap();
        assert_eq!(
            Reached::new(Via::Direct, Some(local)),
            Ok(Reached::Direct(local))
        );
        assert_eq!(Reached::new(Via::Relayed, None), Ok(Reached::Relayed));
        assert_eq!(Reached::new(Via::Direct, None), Err(ReachedError));
        assert_eq!(Reached::new(Via::Relayed, Some(local)), Err(ReachedError));
        assert_eq!("relayed".parse::<Via>(), Ok(Via::Relayed));
        assert_eq!("Direct".parse::<Via>(), Err(ViaError));
        assert_eq!(Via::Direct.to_string(), "direct");
    }

    #[test]
    fn kinds_are_short_lowercase_words() {
        for kind in ["lan", "relay", "a-later-kind", "x9", &"k".repeat(32)] {
            assert_eq!(
                kind.parse::<ProbeKind>().map(|k| k.to_string()),
                Ok(kind.to_owned())
            );
        }
        for kind in ["", "Lan", "a b", "lan\n", "ü", &"k".repeat(33)] {
            assert_eq!(kind.parse::<ProbeKind>(), Err(ProbeKindError), "{kind:?}");
        }
    }
}
