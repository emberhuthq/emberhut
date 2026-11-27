//! `fcm_devices`: every Android phone's FCM registration token.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "sea-orm's model macro writes async methods with nothing to await for an entity \
              without relations"
)]

use sea_orm::entity::prelude::*;

/// A registered Android phone.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "fcm_devices")]
pub(crate) struct Model {
    /// The registration token, as the phone sent it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) token: String,
    /// The signed-in device that registered it (`devices.id`).
    pub(crate) device_id: i64,
    /// When the phone last registered it.
    pub(crate) registered_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
