//! `trusted_requesters`: people whose requests count as the owner's own.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A trusted requester.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "trusted_requesters")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The name a task's request carries.
    pub(crate) requester: String,
    /// The agent it counts for; none: every agent.
    pub(crate) agent_id: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
