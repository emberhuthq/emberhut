//! `push_devices`: every iPhone's APNs device token, and which of Apple's services it is for.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "sea-orm's model macro writes async methods with nothing to await for an entity \
              without relations"
)]

use sea_orm::entity::prelude::*;

/// A registered iPhone.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "push_devices")]
pub(crate) struct Model {
    /// The device token, as the phone sent it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) token: String,
    /// `sandbox` (a development build) or `production`.
    pub(crate) env: String,
    /// The signed-in device that registered it (`devices.id`).
    pub(crate) device_id: i64,
    /// When the phone last registered it.
    pub(crate) registered_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
