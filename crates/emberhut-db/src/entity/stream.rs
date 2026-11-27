//! `streams`: how far each worker's stream has been read.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A stream's place.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "streams")]
pub(crate) struct Model {
    /// The stream's file.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) file: String,
    /// The bytes read so far.
    pub(crate) offset: i64,
    /// Its task.
    pub(crate) task: Option<i64>,
    /// What its lead was doing at the last line read.
    pub(crate) doing: Option<String>,
    /// The lead's tokens so far.
    pub(crate) tokens: Option<f64>,
}

impl ActiveModelBehavior for ActiveModel {}
