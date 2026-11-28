//! `paired_runners`: the runners paired at run time, one row per runner, by its name.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A paired runner.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "paired_runners")]
pub(crate) struct Model {
    /// The runner's name, the one it reports itself by.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) name: String,
    /// Its API's address, without a final `/`.
    pub(crate) url: String,
    /// Its machine in words, if it gave them.
    pub(crate) words: Option<String>,
    /// Its token, as text: the server presents it to the runner.
    pub(crate) token: String,
    /// The system it said it runs on.
    pub(crate) platform: Option<String>,
    /// Its own version, as it said.
    pub(crate) version: Option<String>,
    /// When it first paired.
    pub(crate) created_at: i64,
    /// When it last paired.
    pub(crate) paired_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
