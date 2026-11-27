//! `GET /api/connect/probe`: the query, the machine's signed answer, and the client's check of
//! it.

use emberhut_identity::{
    Host, LocalAddr, MachineId, Nonce, ProbeKind, ProbeStatement, PublicKey, Reached, ReachedError,
    SecretKey, Signature, SignatureError, Via, WrongKey,
};
use serde::{Deserialize, Serialize};

use super::binding;
use super::routes::{MachineIdentity, Route};

/// `GET /api/connect/probe`'s query. A kind that isn't 1 to 32 characters from `a-z`, `0-9`
/// and `-` doesn't read, and the server answers 400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ProbeQuery {
    /// 32 fresh random bytes, base64url without padding.
    #[cfg_attr(feature = "openapi", schema(value_type = String), param(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub nonce: Nonce,
    /// The kind of route the client is checking: 1 to 32 characters from `a-z`, `0-9` and `-`.
    #[cfg_attr(feature = "openapi", schema(value_type = String), param(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: ProbeKind,
}

/// The probe's answer: the machine's signature over the device's nonce and kind, and over how,
/// where and as what it was reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ProbeAnswerWire", into = "ProbeAnswerWire")]
pub struct ProbeAnswer {
    /// The machine.
    pub machine: MachineIdentity,
    /// How it was reached: `via`, and `local` for a direct connection.
    pub reached: Reached,
    /// The request's host, in its normal form.
    pub host: Host,
    /// The machine's signature over the `emberhut probe v1` statement.
    pub signature: Signature,
}

/// `GET /api/connect/probe`'s answer: the machine's signature over the device's nonce and kind,
/// and over how, where and as what it was reached.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
struct ProbeAnswerWire {
    /// The machine's id: 32 characters of lowercase base32.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    id: MachineId,
    /// The machine's Ed25519 public key, base64url without padding; it must have that id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    public_key: PublicKey,
    /// `direct` or `relayed`.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    via: Via,
    /// The local socket address that accepted a direct connection, in its normal form; empty
    /// when relayed, and only then.
    #[serde(with = "local_text")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    local: Option<LocalAddr>,
    /// The request's host (or, relayed, the `host` of the relay's `Forwarded`), in its normal
    /// form.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    host: Host,
    /// The machine's signature over the `emberhut probe v1` statement, base64url without padding.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    signature: Signature,
}

#[cfg(feature = "openapi")]
super::schema_of_wire!(ProbeAnswer, ProbeAnswerWire);

/// A probe answer whose fields disagree.
#[derive(Debug, thiserror::Error)]
enum ProbeAnswerWireError {
    #[error(transparent)]
    WrongKey(#[from] WrongKey),
    #[error(transparent)]
    Reached(#[from] ReachedError),
}

impl TryFrom<ProbeAnswerWire> for ProbeAnswer {
    type Error = ProbeAnswerWireError;

    fn try_from(wire: ProbeAnswerWire) -> Result<Self, Self::Error> {
        Ok(Self {
            machine: MachineIdentity::new(wire.id, wire.public_key)?,
            reached: Reached::new(wire.via, wire.local)?,
            host: wire.host,
            signature: wire.signature,
        })
    }
}

impl From<ProbeAnswer> for ProbeAnswerWire {
    fn from(answer: ProbeAnswer) -> Self {
        Self {
            id: *answer.machine.id(),
            public_key: *answer.machine.public_key(),
            via: answer.reached.via(),
            local: answer.reached.local(),
            host: answer.host,
            signature: answer.signature,
        }
    }
}

/// Why a probe answer doesn't prove the machine the client meant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// Another machine answered.
    #[error("machine {found} answered, not {expected}")]
    OtherMachine {
        /// The machine the client meant (its pinned key's id, or the pairing link's).
        expected: MachineId,
        /// The machine that answered.
        found: MachineId,
    },
    /// The signature doesn't verify over this probe's nonce and kind.
    #[error("the probe answer's signature doesn't verify")]
    Signature(#[from] SignatureError),
}

impl ProbeAnswer {
    /// The machine's answer to a probe with `nonce` and `kind`, reached as `reached`, for
    /// `host`.
    #[must_use]
    pub fn sign(
        key: &SecretKey,
        nonce: Nonce,
        kind: &ProbeKind,
        reached: Reached,
        host: Host,
    ) -> Self {
        let statement =
            ProbeStatement::new(nonce, key.machine_id(), kind.clone(), reached, host.clone());
        Self {
            machine: MachineIdentity::of(key.public_key()),
            reached,
            host,
            signature: key.sign(&statement),
        }
    }

    /// Checks the answer to the probe the client sent, from the machine it expects: the id is
    /// `expected`, and the signature verifies over this nonce and kind. What the client may
    /// then do is ask whether the answer is bound to the route it probed
    /// ([`VerifiedProbe::binding_matches`]).
    ///
    /// # Errors
    ///
    /// A [`ProbeError`] when another machine answered or the signature doesn't verify.
    pub fn verify(
        &self,
        expected: &MachineId,
        nonce: Nonce,
        kind: &ProbeKind,
    ) -> Result<VerifiedProbe, ProbeError> {
        if self.machine.id() != expected {
            return Err(ProbeError::OtherMachine {
                expected: *expected,
                found: *self.machine.id(),
            });
        }
        let statement = ProbeStatement::new(
            nonce,
            *expected,
            kind.clone(),
            self.reached,
            self.host.clone(),
        );
        self.machine
            .public_key()
            .verify(&statement, &self.signature)?;
        Ok(VerifiedProbe(statement))
    }
}

/// The local forward an `ssh` route runs (`docs/connect.md`):
/// `ssh -L 127.0.0.1:<local port>:127.0.0.1:<machine port> <host>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SshForward {
    /// The free loopback port the client listens on, and sends the probe to.
    pub local_port: u16,
    /// The machine's port, which the forward connects to on the machine's loopback.
    pub machine_port: u16,
}

/// A probe answer from the expected machine, for the probe the client sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProbe(ProbeStatement);

impl VerifiedProbe {
    /// Whether the answer is bound to `route`, the route the client probed: the same kind, and
    ///
    /// - an address route (`loopback`, `lan`, a `vpn` address): reached directly, on the
    ///   route's address and port;
    /// - a name route (`public`, a `vpn` name, `relay`), which must be `https`: the route's
    ///   host, in normal form, reached directly, or relayed for `relay`.
    ///
    /// An `ssh` route's answer comes through its local forward, so it is checked against the
    /// forward with [`VerifiedProbe::forward_matches`], and never matches here. Kinds this
    /// version doesn't know, and URLs that don't read, never match either.
    #[must_use]
    pub fn binding_matches(&self, route: &Route) -> bool {
        binding::matches(&self.0, route)
    }

    /// Whether the answer is bound to the `ssh` route whose local forward is `forward`: an
    /// `ssh` probe, reached directly on a loopback address at the port the forward targets,
    /// for the host `127.0.0.1:<the forward's local port>`.
    #[must_use]
    pub fn forward_matches(&self, forward: SshForward) -> bool {
        binding::forward_matches(&self.0, forward)
    }

    /// What the machine signed: how and as what it was reached, for showing why a route
    /// doesn't answer ("answered as 127.0.0.1:18990").
    #[must_use]
    pub const fn statement(&self) -> &ProbeStatement {
        &self.0
    }
}

/// `local` on the wire: the address's text, or `""` when relayed.
mod local_text {
    use emberhut_identity::LocalAddr;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(
        clippy::ref_option,
        reason = "serde's `with` passes the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(
        local: &Option<LocalAddr>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match local {
            Some(local) => serializer.collect_str(local),
            None => serializer.serialize_str(""),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<LocalAddr>, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        if text.is_empty() {
            return Ok(None);
        }
        text.parse().map(Some).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_identity::RelayDomain;

    use super::*;

    fn key() -> SecretKey {
        SecretKey::from_bytes(&[5; 32])
    }

    #[test]
    fn a_signed_probe_answer_verifies_for_its_own_probe_and_machine_only() {
        let nonce = Nonce::from_bytes([1; 32]);
        let relay: ProbeKind = "relay".parse().unwrap();
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let url = domain.machine_url(&key().machine_id());
        let host: Host = url["https://".len()..].parse().unwrap();
        let answer = ProbeAnswer::sign(&key(), nonce, &relay, Reached::Relayed, host);
        let id = key().machine_id();
        let verified = answer.verify(&id, nonce, &relay).unwrap();
        let route = Route {
            kind: super::super::RouteKind::Relay,
            url,
            label: None,
            up: true,
        };
        assert!(verified.binding_matches(&route));
        assert_eq!(
            answer.verify(&id, Nonce::from_bytes([2; 32]), &relay),
            Err(ProbeError::Signature(SignatureError))
        );
        assert_eq!(
            answer.verify(&id, nonce, &"public".parse().unwrap()),
            Err(ProbeError::Signature(SignatureError))
        );
        let other = SecretKey::from_bytes(&[6; 32]).machine_id();
        assert_eq!(
            answer.verify(&other, nonce, &relay),
            Err(ProbeError::OtherMachine {
                expected: other,
                found: id
            })
        );
    }
}
