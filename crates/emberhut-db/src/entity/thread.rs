//! `threads`: the coding threads the server knows about, one row per thread and machine, as
//! last seen.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]
#![expect(
    clippy::struct_field_names,
    reason = "`model` is the column of the thread's model, not a name for the row"
)]

use sea_orm::entity::prelude::*;

/// A coding thread.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "threads")]
pub(crate) struct Model {
    /// The machine it runs on.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// Its id on that machine.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Its title.
    pub(crate) title: String,
    /// Its project, by name.
    pub(crate) project: String,
    /// Its status's word.
    pub(crate) status: String,
    /// When it last changed.
    pub(crate) updated_at: i64,
    /// The thread that started it, for a helper agent's.
    pub(crate) parent: Option<String>,
    /// What it was first asked.
    pub(crate) ask: Option<String>,
    /// What it was asked last.
    pub(crate) last_ask: Option<String>,
    /// Its latest answer.
    pub(crate) last: Option<String>,
    /// Its latest answer's status line.
    pub(crate) status_line: Option<String>,
    /// What it is doing now.
    pub(crate) doing: Option<String>,
    /// The account it runs on.
    pub(crate) account: Option<String>,
    /// The folder it works in.
    pub(crate) cwd: Option<String>,
    /// The branch it works on.
    pub(crate) branch: Option<String>,
    /// Since when its current turn runs.
    pub(crate) since: Option<i64>,
    /// The person drives it themselves.
    pub(crate) direct: bool,
    /// Its model.
    pub(crate) model: Option<String>,
    /// Its reasoning effort.
    pub(crate) effort: Option<String>,
    /// A question it waits on.
    pub(crate) question: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
