//! `family_devices`: a family member's phone's push address.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A family member's phone.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "family_devices")]
pub(crate) struct Model {
    /// Its APNs device token.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) token: String,
    /// `production` or `sandbox`.
    pub(crate) env: String,
    /// Whose it is.
    pub(crate) person: String,
    /// When it was registered.
    pub(crate) registered_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
