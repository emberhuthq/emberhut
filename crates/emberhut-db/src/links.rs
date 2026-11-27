//! Links: the work that moves a task (a thread, a ticket, a chat), so the news about that work
//! names the task.

use emberhut_core::task::TaskId;
use sea_orm::sea_query::{Expr, ExprTrait, Func, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::DbError;
use crate::entity::link;

/// A task's link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The task.
    pub task: TaskId,
    /// `thread`, `ticket` or `chat`.
    pub kind: String,
    /// The thread's id, the ticket or the chat.
    pub reference: String,
    /// The machine a thread runs on.
    pub machine: Option<String>,
}

/// Reads and writes links.
#[derive(Clone, Debug)]
pub struct LinkStore {
    conn: DatabaseConnection,
}

impl LinkStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Links `reference` (a ticket's in capitals) to the task, replacing a link it had to it.
    ///
    /// # Errors
    ///
    /// [`DbError`], also when the task doesn't exist.
    pub async fn link(
        &self,
        task: TaskId,
        kind: &str,
        reference: &str,
        machine: Option<&str>,
    ) -> Result<(), DbError> {
        let reference = if kind == "ticket" {
            reference.to_uppercase()
        } else {
            reference.to_owned()
        };
        link::Entity::insert(link::ActiveModel {
            issue: Set(task.get()),
            kind: Set(kind.to_owned()),
            reference: Set(reference),
            machine: Set(machine.map(str::to_owned)),
        })
        .on_conflict(
            OnConflict::columns([
                link::Column::Issue,
                link::Column::Kind,
                link::Column::Reference,
            ])
            .update_column(link::Column::Machine)
            .to_owned(),
        )
        .exec(&self.conn)
        .await?;
        Ok(())
    }

    /// Takes `reference` off the task, in any case; how many links went.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn unlink(&self, task: TaskId, reference: &str) -> Result<u64, DbError> {
        Ok(link::Entity::delete_many()
            .filter(link::Column::Issue.eq(task.get()))
            .filter(
                Expr::expr(Func::lower(Expr::col(link::Column::Reference)))
                    .eq(reference.to_lowercase()),
            )
            .exec(&self.conn)
            .await?
            .rows_affected)
    }

    /// The task's links, by kind and reference.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn of(&self, task: TaskId) -> Result<Vec<Link>, DbError> {
        Ok(link::Entity::find()
            .filter(link::Column::Issue.eq(task.get()))
            .order_by_asc(link::Column::Kind)
            .order_by_asc(link::Column::Reference)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect())
    }

    /// The tasks linked to `reference` of `kind`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tasks_of(&self, kind: &str, reference: &str) -> Result<Vec<TaskId>, DbError> {
        Ok(link::Entity::find()
            .filter(link::Column::Kind.eq(kind))
            .filter(link::Column::Reference.eq(reference))
            .order_by_asc(link::Column::Issue)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| TaskId::new(row.issue))
            .collect())
    }
}

fn from_row(row: link::Model) -> Link {
    Link {
        task: TaskId::new(row.issue),
        kind: row.kind,
        reference: row.reference,
        machine: row.machine,
    }
}
