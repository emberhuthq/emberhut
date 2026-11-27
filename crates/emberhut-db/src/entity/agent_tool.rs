//! `agent_tools`: each agent's tools, in order.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One tool of an agent's.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_tools")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its agent.
    pub(crate) agent_id: String,
    /// Its place in the agent's list.
    pub(crate) position: i32,
    /// `server` or `mcp`.
    pub(crate) kind: String,
    /// The server tool's or the MCP server's name.
    pub(crate) name: String,
    /// An MCP server's configuration file.
    pub(crate) config: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
