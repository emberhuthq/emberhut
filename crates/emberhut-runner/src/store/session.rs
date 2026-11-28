//! `sessions`: one row per session the runner runs.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A session as stored.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    /// Its id: `r-` and 8 lowercase hex digits.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// Its title.
    pub title: Option<String>,
    /// The folder it runs in (its worktree when it has one).
    pub cwd: String,
    /// The project it is for.
    pub project: Option<String>,
    /// The model it runs on (`None`: the runner's default).
    pub model: Option<String>,
    /// Its effort.
    pub effort: Option<String>,
    /// The account it is on.
    pub account: Option<String>,
    /// The agent's own session id (a UUID).
    pub claude: Option<String>,
    /// 1 once the agent has written its transcript.
    pub materialized: i64,
    /// Its status (`queued`, `running`, `completed`, `failed`, `interrupted`, `waiting`, `idle`).
    pub status: String,
    /// Its worktree's folder.
    pub worktree: Option<String>,
    /// Its worktree's branch.
    pub branch: Option<String>,
    /// What it was opened with (`direct`, `task`, `pin`, `optIn`…), as JSON.
    pub meta: Option<String>,
    /// The number of its last turn.
    pub seq: i64,
    /// When it carries on by itself after every account was out.
    pub resume_at: Option<String>,
    /// When it was opened.
    pub created: Option<String>,
    /// When it last changed.
    pub updated: Option<String>,
    /// How many background jobs its agent process waits on.
    pub background: i64,
    /// The folder it was asked to start in (before a worktree).
    pub origin: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
