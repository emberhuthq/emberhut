//! `prefs`: the person's small account choices, one text value per key.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A pref.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "prefs")]
pub(crate) struct Model {
    /// Its key.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) key: String,
    /// Its value.
    pub(crate) value: String,
}

impl ActiveModelBehavior for ActiveModel {}
