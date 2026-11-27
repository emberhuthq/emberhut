//! The machine and its routes: `GET /api/connect`, and the machine in the pairing answer.

use emberhut_identity::{MachineId, MachineName, ProbeKind, ProbeKindError, PublicKey, WrongKey};
use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;

open_enum! {
    /// What kind of route a URL is, best first. Readers skip kinds they don't know.
    pub enum RouteKind {
        /// The same computer: `http://127.0.0.1:<port>`.
        Loopback = "loopback",
        /// The local network: `http://<local address>:<port>`.
        Lan = "lan",
        /// An address on a VPN interface, or a VPN's HTTPS name.
        Vpn = "vpn",
        /// `https://<a name the person runs>`.
        Public = "public",
        /// `ssh://<host>`, a local forward (desktop only).
        Ssh = "ssh",
        /// `https://<machine id>.<relay domain>`.
        Relay = "relay",
    }
}

impl RouteKind {
    /// The kind as a probe sends it.
    ///
    /// # Errors
    ///
    /// [`ProbeKindError`] for a word from a newer server that isn't a valid probe kind.
    pub fn probe_kind(&self) -> Result<ProbeKind, ProbeKindError> {
        self.as_str().parse()
    }
}

/// One way to reach the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Route {
    /// Its kind.
    pub kind: RouteKind,
    /// Its base URL (`ssh://<host>` for an SSH route).
    pub url: String,
    /// A name for it (a VPN's name), or `null`.
    pub label: Option<String>,
    /// Whether the machine can serve it now. Every configured route is listed, up or not.
    pub up: bool,
}

/// A machine's id and its public key, which has that id: the two always agree.
///
/// On the wire, `{ "id", "publicKey" }`: `machine` in an accepted pairing answer, which the
/// device pins, and the first two fields of the route list, the probe's answer and the
/// directory's machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "MachineIdentityWire", into = "MachineIdentityWire")]
pub struct MachineIdentity {
    id: MachineId,
    public_key: PublicKey,
}

/// A machine's id and its Ed25519 public key, which must have that id.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
struct MachineIdentityWire {
    /// The machine's id: 32 characters of lowercase base32.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    id: MachineId,
    /// The machine's Ed25519 public key, base64url without padding; it must have that id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    public_key: PublicKey,
}

#[cfg(feature = "openapi")]
super::schema_of_wire!(MachineIdentity, MachineIdentityWire);

impl MachineIdentity {
    /// The identity of the machine whose key is `public_key`.
    #[must_use]
    pub fn of(public_key: PublicKey) -> Self {
        Self {
            id: public_key.machine_id(),
            public_key,
        }
    }

    /// An id and a key as they arrived together.
    ///
    /// # Errors
    ///
    /// [`WrongKey`] when the key doesn't have that id.
    pub fn new(id: MachineId, public_key: PublicKey) -> Result<Self, WrongKey> {
        id.check(&public_key)?;
        Ok(Self { id, public_key })
    }

    /// The machine's id.
    #[must_use]
    pub const fn id(&self) -> &MachineId {
        &self.id
    }

    /// The machine's public key.
    #[must_use]
    pub const fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}

impl TryFrom<MachineIdentityWire> for MachineIdentity {
    type Error = WrongKey;

    fn try_from(wire: MachineIdentityWire) -> Result<Self, Self::Error> {
        Self::new(wire.id, wire.public_key)
    }
}

impl From<MachineIdentity> for MachineIdentityWire {
    fn from(identity: MachineIdentity) -> Self {
        Self {
            id: identity.id,
            public_key: identity.public_key,
        }
    }
}

/// `GET /api/connect`: the machine and every route its configuration gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RouteListWire", into = "RouteListWire")]
pub struct RouteList {
    /// The machine.
    pub machine: MachineIdentity,
    /// The machine's name.
    pub name: MachineName,
    /// Whether the machine supports end-to-end encryption (always `false` until then).
    pub sealed: bool,
    /// Every configured route, in the machine's order.
    pub routes: Vec<Route>,
}

/// `GET /api/connect`'s answer: the machine and every route its configuration gives.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
struct RouteListWire {
    /// The machine's id: 32 characters of lowercase base32.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    id: MachineId,
    /// The machine's Ed25519 public key, base64url without padding; it must have that id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    public_key: PublicKey,
    /// The machine's name: 1 to 60 characters, trimmed.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    name: MachineName,
    /// Whether the machine supports end-to-end encryption (always `false` until then).
    #[serde(default)]
    sealed: bool,
    /// Every configured route, in the machine's order.
    routes: Vec<Route>,
}

#[cfg(feature = "openapi")]
super::schema_of_wire!(RouteList, RouteListWire);

impl TryFrom<RouteListWire> for RouteList {
    type Error = WrongKey;

    fn try_from(wire: RouteListWire) -> Result<Self, Self::Error> {
        Ok(Self {
            machine: MachineIdentity::new(wire.id, wire.public_key)?,
            name: wire.name,
            sealed: wire.sealed,
            routes: wire.routes,
        })
    }
}

impl From<RouteList> for RouteListWire {
    fn from(list: RouteList) -> Self {
        Self {
            id: list.machine.id,
            public_key: list.machine.public_key,
            name: list.name,
            sealed: list.sealed,
            routes: list.routes,
        }
    }
}

#[cfg(test)]
mod tests {
    use emberhut_identity::SecretKey;

    use super::*;

    #[test]
    fn an_identity_keeps_its_key_and_id_together() {
        let key = SecretKey::from_bytes(&[5; 32]).public_key();
        let identity = MachineIdentity::of(key);
        assert_eq!(MachineIdentity::new(*identity.id(), key), Ok(identity));
        let other = SecretKey::from_bytes(&[6; 32]).machine_id();
        assert!(MachineIdentity::new(other, key).is_err());
        let json = serde_json::json!({ "id": other.to_string(), "publicKey": key.to_string() });
        assert!(serde_json::from_value::<MachineIdentity>(json).is_err());
    }

    #[test]
    fn route_kinds_are_probe_kinds() {
        assert_eq!(
            RouteKind::Lan.probe_kind().map(|k| k.to_string()),
            Ok("lan".to_owned())
        );
        assert!(RouteKind::from("Not A Kind").probe_kind().is_err());
    }
}
