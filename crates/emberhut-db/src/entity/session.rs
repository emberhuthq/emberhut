//! `sessions`: a device's current tokens and the ones they replaced, at most one row per
//! device. Only their hashes.

use sea_orm::entity::prelude::*;

/// A device's current access and refresh tokens.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub(crate) struct Model {
    /// The device this session belongs to.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) device_id: i64,
    /// The hash of its access token.
    #[sea_orm(unique)]
    pub(crate) access_hash: Vec<u8>,
    /// When the access token stops working.
    pub(crate) access_expires_at: i64,
    /// The hash of its refresh token, replaced each time it is used.
    #[sea_orm(unique)]
    pub(crate) refresh_hash: Vec<u8>,
    /// When the current tokens were issued.
    pub(crate) issued_at: i64,
    /// The hash of the access token these replaced; it works until its own expiry.
    #[sea_orm(unique)]
    pub(crate) previous_access_hash: Option<Vec<u8>>,
    /// When that access token stops working.
    pub(crate) previous_access_expires_at: Option<i64>,
    /// The hash of the refresh token these replaced, kept to spot its reuse.
    #[sea_orm(unique)]
    pub(crate) previous_refresh_hash: Option<Vec<u8>>,
    /// When the previous tokens were replaced; a re-issue within the grace keeps it.
    pub(crate) rotated_at: Option<i64>,
    /// The device.
    #[sea_orm(belongs_to, from = "device_id", to = "id")]
    pub(crate) device: HasOne<super::device::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
