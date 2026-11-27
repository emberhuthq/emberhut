//! `pairing_codes`: one-time codes that let a new device sign in. Only their hashes.

use sea_orm::entity::prelude::*;

/// A pairing code that was made.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pairing_codes")]
pub(crate) struct Model {
    /// The code's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The hash of the code, as typed without its dash.
    #[sea_orm(indexed)]
    pub(crate) code_hash: Vec<u8>,
    /// When it was made.
    pub(crate) created_at: i64,
    /// When it stops working.
    pub(crate) expires_at: i64,
    /// When it was used; `None` while it has not been.
    pub(crate) used_at: Option<i64>,
    /// The device that used it.
    pub(crate) device_id: Option<i64>,
    /// That device.
    #[sea_orm(belongs_to, from = "device_id", to = "id")]
    pub(crate) device: HasOne<super::device::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
