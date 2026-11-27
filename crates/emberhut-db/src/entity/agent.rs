//! `agents`: the sub-agents of tasks' runs.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A sub-agent.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agents")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its task.
    pub(crate) task: i64,
    /// The session that started it.
    pub(crate) run: String,
    /// The CLI's own id for it.
    pub(crate) key: String,
    /// The tool call that started it.
    pub(crate) tool_use: Option<String>,
    /// Where it was seen: `worker` or `runner`.
    pub(crate) source: Option<String>,
    /// `part`, `verify`, `lane` or `other`.
    pub(crate) kind: Option<String>,
    /// Its name.
    pub(crate) name: Option<String>,
    /// Its part.
    pub(crate) part: Option<f64>,
    /// `running`, `queued`, `done`, `failed` or `stopped`.
    pub(crate) state: String,
    /// Its latest line.
    pub(crate) last: Option<String>,
    /// Its tokens so far.
    pub(crate) tokens: Option<i64>,
    /// Its tool calls so far.
    pub(crate) tools: Option<i64>,
    /// The account it runs on.
    pub(crate) account: Option<String>,
    /// The machine it runs on.
    pub(crate) machine: Option<String>,
    /// When it started.
    pub(crate) started: Option<i64>,
    /// When it ended.
    pub(crate) ended: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
