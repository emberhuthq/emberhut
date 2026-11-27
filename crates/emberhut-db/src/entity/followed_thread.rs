//! `followed_threads`: the threads the assistant follows, one row per thread id.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A followed thread.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "followed_threads")]
pub(crate) struct Model {
    /// The thread's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) thread: String,
    /// The machine it runs on.
    pub(crate) machine: String,
    /// Why it is followed.
    pub(crate) why: String,
    /// When it was followed.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
