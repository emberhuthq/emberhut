//! An emberhut machine's identity: its Ed25519 key pair, the machine id derived from the public key,
//! and the statements it signs.
//!
//! - [`SecretKey`] is made from the operating system's random source, or read back from its
//!   32 bytes. Its bytes are wiped when it is dropped, and its `Debug` shows only the public
//!   key.
//! - [`MachineId`] is the first 160 bits of a hash of the [`PublicKey`], as 32 lowercase base32
//!   characters: one DNS label, which the relay serves the machine at.
//! - Every signed statement has one byte encoding ([`statement`]), with a prefix of its own:
//!   [`ProbeStatement`], [`RelayAuthStatement`], [`LinkStatement`] and [`UnlinkStatement`].
//! - Every value a statement signs has one spelling, and its type bounds it: [`Host`],
//!   [`LocalAddr`], [`RelayDomain`], [`MachineName`], [`ProbeKind`], [`AccountId`],
//!   [`UnixMillis`], and base64url without padding for keys, nonces and signatures.
//! - [`PairingLink`] reads and writes the pairing link.
//!
//! The formats are specified in `docs/api.md` ("Connect"), and the shared test vectors are in
//! `crates/emberhut-api/tests/fixtures/connect/`. This crate does no I/O: the server keeps the key
//! file, and the callers send and receive what is signed.
//!
//! ```
//! use emberhut_identity::{Host, Nonce, ProbeStatement, Reached, Scheme, SecretKey};
//!
//! let key = SecretKey::from_bytes(&[7; 32]);
//! let local = "192.0.2.1:18990".parse()?;
//! let statement = ProbeStatement::new(
//!     Nonce::from_bytes([1; 32]),
//!     key.machine_id(),
//!     "lan".parse()?,
//!     Reached::Direct(local),
//!     Host::normalize("192.0.2.1:18990", Scheme::Http)?,
//! );
//! let signature = key.sign(&statement);
//! assert!(key.public_key().verify(&statement, &signature).is_ok());
//! assert_eq!(key.machine_id().to_string().len(), 32);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Features
//!
//! - `serde`: `Serialize` and `Deserialize` for the keys, ids, nonces, signatures, names and
//!   normal forms, as their text (and [`UnixMillis`] as a number). Deserializing checks them
//!   as parsing does.

mod account;
mod addr;
mod encoding;
mod host;
mod keys;
mod link;
mod machine_id;
mod machine_name;
mod nonce;
mod pairing_link;
mod probe;
mod random;
mod relay_auth;
mod relay_domain;
#[cfg(feature = "serde")]
mod serde_text;
pub mod statement;
mod time;

pub use account::{AccountId, AccountIdError};
pub use addr::{LocalAddr, LocalAddrError};
pub use encoding::DecodeError;
pub use host::{Host, HostError, Scheme};
pub use keys::{
    KeyError, PUBLIC_KEY_LEN, PublicKey, SECRET_KEY_LEN, SIGNATURE_LEN, SecretKey, Signature,
    SignatureError,
};
pub use link::{LinkStatement, UnlinkStatement};
pub use machine_id::{MachineId, WrongKey};
pub use machine_name::{MachineName, MachineNameError};
pub use nonce::Nonce;
pub use pairing_link::{PairingLink, PairingLinkError, PairingLinkKey};
pub use probe::{ProbeKind, ProbeKindError, ProbeStatement, Reached, ReachedError, Via, ViaError};
pub use random::RandomError;
pub use relay_auth::RelayAuthStatement;
pub use relay_domain::{RelayDomain, RelayDomainError};
pub use statement::Statement;
pub use time::{TimeOutOfRange, UnixMillis};
