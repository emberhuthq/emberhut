//! `approval_policies`: who approves each kind of each agent's actions.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One agent's approver for one kind.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "approval_policies")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its agent.
    pub(crate) agent_id: String,
    /// The kind of action.
    pub(crate) kind: String,
    /// `owner`, `none`, or an approver's id.
    pub(crate) approver: String,
}

impl ActiveModelBehavior for ActiveModel {}
