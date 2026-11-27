//! `agent_spend`: one row per used payment, summed by month against an agent's cap.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A payment an agent made.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_spend")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its agent.
    pub(crate) agent_id: String,
    /// The approval it was made under.
    #[sea_orm(unique)]
    pub(crate) request_id: i64,
    /// Its amount, in cents.
    pub(crate) amount_cents: i64,
    /// Its month, `YYYY-MM`.
    pub(crate) month: String,
    /// When it was made.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
