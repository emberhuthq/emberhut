//! `call_hints`: the copilot's hints during a call.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A hint.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "call_hints")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The call.
    pub(crate) call_id: String,
    /// Seconds from the call's start it was given at.
    pub(crate) t: Option<f64>,
    /// The hint.
    pub(crate) text: String,
    /// When it was given.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
