//! Linking a machine to an account: `POST /api/connect/link-statement`, and the signed link
//! the directory's `POST /v1/machines` takes.

use emberhut_identity::{
    AccountId, LinkStatement, MachineId, MachineName, PublicKey, RelayDomain, SecretKey, Signature,
    SignatureError, UnixMillis,
};
use serde::{Deserialize, Serialize};

use crate::directory::DirectoryErrorCode;

/// `POST /api/connect/link-statement`: which account the machine should sign a link statement
/// for, at which relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct LinkStatementRequest {
    /// The account's user id: 1 to 255 characters of visible ASCII.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub account: AccountId,
    /// The relay's domain, in its normal form.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub relay_domain: RelayDomain,
}

/// A machine's signed request to be linked to an account: the answer of
/// `POST /api/connect/link-statement`, and, as it is, the body of the directory's
/// `POST /v1/machines`.
///
/// The signature covers `emberhut link v1` over `account`, `relayDomain`, `name` and `time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct SignedLink {
    /// The machine's Ed25519 public key, base64url without padding.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub public_key: PublicKey,
    /// The machine's name: 1 to 60 characters, trimmed. Signed.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub name: MachineName,
    /// The account's user id.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub account: AccountId,
    /// The relay's domain.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub relay_domain: RelayDomain,
    /// When it was signed, in milliseconds since the Unix epoch.
    #[cfg_attr(feature = "openapi", schema(value_type = u64))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub time: UnixMillis,
    /// The machine's signature over the `emberhut link v1` statement, base64url without padding.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub signature: Signature,
}

/// Why the directory refuses a signed link, before its own checks of time, replays and the
/// account's machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    /// It is for another relay than the directory's own.
    #[error("the link is for another relay")]
    OtherRelay,
    /// It names another account than the token's.
    #[error("the link names another account")]
    NotYours,
    /// The signature doesn't verify against `publicKey`.
    #[error("the link's signature doesn't verify")]
    Signature(#[from] SignatureError),
}

impl LinkError {
    /// The directory's error code for it.
    #[must_use]
    pub fn code(&self) -> DirectoryErrorCode {
        match self {
            Self::OtherRelay => DirectoryErrorCode::BadRequest,
            Self::NotYours => DirectoryErrorCode::NotYours,
            Self::Signature(_) => DirectoryErrorCode::BadSignature,
        }
    }
}

impl SignedLink {
    /// The machine's link statement for `account` at `relay_domain`, under `name`, made at
    /// `time`.
    #[must_use]
    pub fn sign(
        key: &SecretKey,
        name: MachineName,
        account: AccountId,
        relay_domain: RelayDomain,
        time: UnixMillis,
    ) -> Self {
        let statement = LinkStatement::new(account, relay_domain, name, time);
        Self {
            public_key: key.public_key(),
            name: statement.name().clone(),
            account: statement.account().clone(),
            relay_domain: statement.relay_domain().clone(),
            time,
            signature: key.sign(&statement),
        }
    }

    /// The statement the signature covers.
    #[must_use]
    pub fn statement(&self) -> LinkStatement {
        LinkStatement::new(
            self.account.clone(),
            self.relay_domain.clone(),
            self.name.clone(),
            self.time,
        )
    }

    /// Checks the link as the directory at `own_relay_domain` receives it with a token for
    /// `account`: its relay, its account, then its signature. Returns the id of the machine
    /// that signed. Freshness and replays are the directory's own checks.
    ///
    /// # Errors
    ///
    /// A [`LinkError`], whose [`LinkError::code`] is the directory's answer.
    pub fn verify(
        &self,
        account: &AccountId,
        own_relay_domain: &RelayDomain,
    ) -> Result<MachineId, LinkError> {
        if &self.relay_domain != own_relay_domain {
            return Err(LinkError::OtherRelay);
        }
        if &self.account != account {
            return Err(LinkError::NotYours);
        }
        self.public_key.verify(&self.statement(), &self.signature)?;
        Ok(self.public_key.machine_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_verifies_for_its_own_account_and_relay_only() {
        let key = SecretKey::from_bytes(&[5; 32]);
        let account = AccountId::new("user_example").unwrap();
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let link = SignedLink::sign(
            &key,
            MachineName::cut("Main computer").unwrap(),
            account.clone(),
            domain.clone(),
            UnixMillis::new(1_795_561_073_000).unwrap(),
        );
        assert_eq!(link.verify(&account, &domain), Ok(key.machine_id()));
        let other = AccountId::new("user_other").unwrap();
        assert_eq!(link.verify(&other, &domain), Err(LinkError::NotYours));
        assert_eq!(LinkError::NotYours.code(), DirectoryErrorCode::NotYours);
        let elsewhere: RelayDomain = "other.example".parse().unwrap();
        assert_eq!(
            link.verify(&account, &elsewhere),
            Err(LinkError::OtherRelay)
        );
        let mut renamed = link;
        renamed.name = MachineName::cut("Another name").unwrap();
        assert_eq!(
            renamed.verify(&account, &domain),
            Err(LinkError::Signature(SignatureError)),
            "the name is signed"
        );
    }
}
