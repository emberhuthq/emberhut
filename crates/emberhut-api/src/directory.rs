//! The account directory's API, version 1 (`docs/directory.md`), at
//! `https://<relay domain>/v1`.
//!
//! Every route but the machine's own unlink takes an account token as
//! `Authorization: Bearer <token>`. Linking posts the machine's
//! [`SignedLink`](crate::connect::SignedLink) as it is.

use emberhut_identity::{
    MachineId, MachineName, PublicKey, RelayDomain, SecretKey, Signature, SignatureError,
    UnixMillis, UnlinkStatement, WrongKey,
};
use serde::{Deserialize, Serialize};

use crate::connect::MachineIdentity;
use crate::open_enum::open_enum;
use crate::serde_util::is_false;

/// `GET`: the account's machines ([`MachineList`]); `POST`: link one
/// ([`SignedLink`](crate::connect::SignedLink) as the body, a [`LinkedMachine`] back).
pub const MACHINES: &str = "/v1/machines";

/// The only origin answers allow in CORS: the web app's (ADR 0008, section 6).
pub const ALLOWED_ORIGIN: &str = "https://app.emberhut.com";

/// `DELETE`: unlink machine `id` from the token's account (204).
#[must_use]
pub fn machine_path(id: &MachineId) -> String {
    format!("{MACHINES}/{id}")
}

/// `POST`: the machine leaves its account by itself ([`UnlinkRequest`], 204).
#[must_use]
pub fn unlink_path(id: &MachineId) -> String {
    format!("{MACHINES}/{id}/unlink")
}

/// `GET /v1/machines`: the account's machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MachineList {
    /// The machines.
    pub machines: Vec<LinkedMachine>,
}

/// A machine linked to the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LinkedMachineWire", into = "LinkedMachineWire")]
pub struct LinkedMachine {
    /// The machine.
    pub machine: MachineIdentity,
    /// Its name.
    pub name: MachineName,
    /// Its relay route, `wss://<relay domain>/v1/sealed/<machine id>`.
    pub relay_url: String,
    /// Whether its uplink is up now.
    pub connected: bool,
    /// When its uplink last was up, if ever.
    pub last_seen: Option<UnixMillis>,
    /// When it was linked.
    pub linked_at: UnixMillis,
}

/// A machine linked to the account.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
struct LinkedMachineWire {
    /// Its id: 32 characters of lowercase base32.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    id: MachineId,
    /// Its Ed25519 public key, base64url without padding; it must have that id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    public_key: PublicKey,
    /// Its name: 1 to 60 characters, trimmed.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    name: MachineName,
    /// Its relay route, `wss://<relay domain>/v1/sealed/<machine id>`.
    relay_url: String,
    /// Whether its uplink is up now.
    connected: bool,
    /// When its uplink last was up, or `null` if never.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<u64>))]
    last_seen: Option<UnixMillis>,
    /// When it was linked.
    #[cfg_attr(feature = "openapi", schema(value_type = u64))]
    linked_at: UnixMillis,
}

#[cfg(feature = "openapi")]
crate::connect::schema_of_wire!(LinkedMachine, LinkedMachineWire);

impl TryFrom<LinkedMachineWire> for LinkedMachine {
    type Error = WrongKey;

    fn try_from(wire: LinkedMachineWire) -> Result<Self, Self::Error> {
        Ok(Self {
            machine: MachineIdentity::new(wire.id, wire.public_key)?,
            name: wire.name,
            relay_url: wire.relay_url,
            connected: wire.connected,
            last_seen: wire.last_seen,
            linked_at: wire.linked_at,
        })
    }
}

impl From<LinkedMachine> for LinkedMachineWire {
    fn from(machine: LinkedMachine) -> Self {
        Self {
            id: *machine.machine.id(),
            public_key: *machine.machine.public_key(),
            name: machine.name,
            relay_url: machine.relay_url,
            connected: machine.connected,
            last_seen: machine.last_seen,
            linked_at: machine.linked_at,
        }
    }
}

/// `POST /v1/machines/{id}/unlink`: the machine's own statement that it leaves its account,
/// with no account token. The signature covers `emberhut unlink v1` over the id, `relayDomain` and
/// `time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct UnlinkRequest {
    /// The machine's public key; the id in the path must be its id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub public_key: PublicKey,
    /// The relay's domain.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub relay_domain: RelayDomain,
    /// When it was signed.
    #[cfg_attr(feature = "openapi", schema(value_type = u64))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub time: UnixMillis,
    /// The machine's signature over the `emberhut unlink v1` statement.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub signature: Signature,
}

impl UnlinkRequest {
    /// The machine's request to leave its account at `relay_domain`, made at `time`.
    #[must_use]
    pub fn sign(key: &SecretKey, relay_domain: RelayDomain, time: UnixMillis) -> Self {
        let statement = UnlinkStatement::new(key.machine_id(), relay_domain, time);
        Self {
            public_key: key.public_key(),
            relay_domain: statement.relay_domain().clone(),
            time,
            signature: key.sign(&statement),
        }
    }

    /// The statement the signature covers, for the machine named in the path.
    #[must_use]
    pub fn statement(&self, id: MachineId) -> UnlinkStatement {
        UnlinkStatement::new(id, self.relay_domain.clone(), self.time)
    }

    /// Checks the request as the directory at `own_relay_domain` receives it for the machine
    /// named in the path: its relay, that the key has that id, then the signature. Freshness
    /// and replays are the directory's own checks.
    ///
    /// # Errors
    ///
    /// An [`UnlinkError`], whose [`UnlinkError::code`] is the directory's answer.
    pub fn verify(
        &self,
        id: &MachineId,
        own_relay_domain: &RelayDomain,
    ) -> Result<(), UnlinkError> {
        if &self.relay_domain != own_relay_domain {
            return Err(UnlinkError::OtherRelay);
        }
        id.check(&self.public_key)
            .map_err(|_| UnlinkError::OtherMachine)?;
        self.public_key
            .verify(&self.statement(*id), &self.signature)?;
        Ok(())
    }
}

/// Why the directory refuses a machine's own unlink, before its own checks of time and
/// replays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UnlinkError {
    /// It is for another relay than the directory's own.
    #[error("the unlink is for another relay")]
    OtherRelay,
    /// The key isn't the key of the machine named in the path.
    #[error("the key isn't the machine's in the path")]
    OtherMachine,
    /// The signature doesn't verify against `publicKey`.
    #[error("the unlink's signature doesn't verify")]
    Signature(#[from] SignatureError),
}

impl UnlinkError {
    /// The directory's error code for it.
    #[must_use]
    pub fn code(&self) -> DirectoryErrorCode {
        match self {
            Self::OtherRelay | Self::OtherMachine => DirectoryErrorCode::BadRequest,
            Self::Signature(_) => DirectoryErrorCode::BadSignature,
        }
    }
}

open_enum! {
    /// A refusal's `code`. Clients act on it, never on the sentence.
    pub enum DirectoryErrorCode {
        /// 400: a body that isn't JSON, or lacks a field.
        BadRequest = "bad-request",
        /// 400: a statement more than 5 minutes from the clock, or older than the machine's
        /// latest link or unlink.
        Stale = "stale",
        /// 400: a statement already accepted.
        Replayed = "replayed",
        /// 401: no token, or one that fails a check (with `signIn: true`).
        SignIn = "sign-in",
        /// 403: the statement doesn't verify against `publicKey`.
        BadSignature = "bad-signature",
        /// 403: the statement names another account than the token's.
        NotYours = "not-yours",
        /// 404: no such machine in this account (or, unlinking itself, not linked).
        NotFound = "not-found",
        /// 409: the account already has as many machines as it may link.
        AccountFull = "account-full",
        /// 409: the machine is linked to another account.
        LinkedElsewhere = "linked-elsewhere",
        /// 429: too many requests; `Retry-After` says when to try again.
        Limit = "limit",
        /// 503: the account provider's keys can't be had right now; `Retry-After` says when.
        /// Never a reason to sign out.
        Unavailable = "unavailable",
    }
}

/// Every refusal of the directory: `{ "error": "<a sentence to show>", "code": "<code>" }`,
/// with `"signIn": true` on 401.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DirectoryError {
    /// What went wrong, in words meant for a person.
    pub error: String,
    /// What went wrong, for the client to act on.
    pub code: DirectoryErrorCode,
    /// The token is missing or failed a check: sign in to the account again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sign_in: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_name_the_machine() {
        let id = SecretKey::from_bytes(&[2; 32]).machine_id();
        assert_eq!(machine_path(&id), format!("/v1/machines/{id}"));
        assert_eq!(unlink_path(&id), format!("/v1/machines/{id}/unlink"));
    }

    #[test]
    fn an_unlink_verifies_for_its_own_machine_and_relay_only() {
        let key = SecretKey::from_bytes(&[2; 32]);
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let request = UnlinkRequest::sign(
            &key,
            domain.clone(),
            UnixMillis::new(1_795_561_073_000).unwrap(),
        );
        assert_eq!(request.verify(&key.machine_id(), &domain), Ok(()));
        let other = SecretKey::from_bytes(&[3; 32]).machine_id();
        assert_eq!(
            request.verify(&other, &domain),
            Err(UnlinkError::OtherMachine)
        );
        let elsewhere: RelayDomain = "other.example".parse().unwrap();
        assert_eq!(
            request.verify(&key.machine_id(), &elsewhere),
            Err(UnlinkError::OtherRelay)
        );
        assert_eq!(
            UnlinkError::OtherRelay.code(),
            DirectoryErrorCode::BadRequest
        );
    }
}
