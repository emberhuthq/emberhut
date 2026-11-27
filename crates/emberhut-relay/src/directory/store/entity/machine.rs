//! `machines`: every linked machine, with its account.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A linked machine.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "machines")]
pub(crate) struct Model {
    /// Its id: 32 characters of lowercase base32.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Its Ed25519 public key, 32 bytes.
    pub(crate) public_key: Vec<u8>,
    /// Its name, as its latest link statement signed it.
    pub(crate) name: String,
    /// Its account's user id.
    pub(crate) account: String,
    /// When it was linked, in milliseconds since the Unix epoch.
    pub(crate) linked_at: i64,
    /// When its uplink was last up, if ever.
    pub(crate) last_seen: Option<i64>,
    /// The gateway box holding its uplink now, if it is connected.
    pub(crate) held_by: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
