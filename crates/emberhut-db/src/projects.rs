//! Projects: what tasks are grouped by. Archiving one parks its open tasks that don't need the
//! person; bringing it back takes back exactly those.

use emberhut_core::task::board::{
    Project, ProjectChange, ProjectKind, ProjectStatus, sort_projects,
};
use emberhut_core::task::{GoalId, PARKED_BY_ARCHIVE, ProjectId, Status};
use jiff::Timestamp;
use jiff::civil::Date;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter,
};

use crate::entity::{issue, project};
use crate::tasks::date;
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// Why a project could not be made or changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectStoreError {
    /// A new project with no name.
    #[error("a new project needs a name")]
    NeedsName,
    /// There is no such project.
    #[error("no project {0}")]
    NoProject(ProjectId),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<DbErr> for ProjectStoreError {
    fn from(error: DbErr) -> Self {
        Self::Db(DbError::Query(error))
    }
}

/// Reads and writes projects.
#[derive(Clone, Debug)]
pub struct ProjectStore {
    conn: DatabaseConnection,
}

impl ProjectStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Every project, in board order: active ones first, then by priority, deadline and name.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn list(&self) -> Result<Vec<Project>, DbError> {
        let mut projects = project::Entity::find()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        sort_projects(&mut projects);
        Ok(projects)
    }

    /// The project `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, id: ProjectId) -> Result<Option<Project>, DbError> {
        project::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Makes a project (`id` none: `change.name` is its name; one of that name already there,
    /// in any case, is the one changed) or changes one. Returns its id.
    ///
    /// # Errors
    ///
    /// [`ProjectStoreError`].
    pub async fn set(
        &self,
        id: Option<ProjectId>,
        change: ProjectChange,
        now: Timestamp,
    ) -> Result<ProjectId, ProjectStoreError> {
        let txn = begin_write(&self.conn).await?;
        let row = if let Some(id) = id.filter(|id| id.get() != 0) {
            project::Entity::find_by_id(id.get())
                .one(&txn)
                .await?
                .ok_or(ProjectStoreError::NoProject(id))?
        } else {
            let name = change
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or(ProjectStoreError::NeedsName)?;
            let same = project::Entity::find()
                .filter(
                    Expr::expr(Func::lower(Expr::col(project::Column::Name)))
                        .eq(name.to_lowercase()),
                )
                .one(&txn)
                .await?;
            if let Some(row) = same {
                row
            } else {
                project::ActiveModel {
                    name: Set(name.to_owned()),
                    goal: Set(None),
                    priority: Set(0),
                    kind: Set(ProjectKind::Deliverable.as_str().to_owned()),
                    deadline: Set(None),
                    hard: Set(false),
                    status: Set(ProjectStatus::Active.as_str().to_owned()),
                    paused_until: Set(None),
                    created: Set(millis(now)),
                    updated: Set(millis(now)),
                    ..Default::default()
                }
                .insert(&txn)
                .await?
            }
        };
        let was = ProjectStatus::parse(&row.status);
        let pid = ProjectId::new(row.id);
        let mut active = row.into_active_model();
        if let Some(name) = change
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            active.name = Set(name.to_owned());
        }
        if let Some(goal) = change.goal {
            active.goal = Set(goal.map(GoalId::get));
        }
        if let Some(priority) = change.priority {
            active.priority = Set(priority);
        }
        if let Some(kind) = change.kind {
            active.kind = Set(kind.as_str().to_owned());
        }
        if let Some(deadline) = change.deadline {
            active.deadline = Set(deadline.map(|d| d.to_string()));
        }
        if let Some(hard) = change.hard {
            active.hard = Set(hard);
        }
        if let Some(status) = change.status {
            active.status = Set(status.as_str().to_owned());
        }
        if change.status.is_some_and(|s| s != ProjectStatus::Paused) {
            active.paused_until = Set(None);
        } else if let Some(until) = change.paused_until {
            active.paused_until = Set(until.map(|d| d.to_string()));
        }
        active.updated = Set(millis(now));
        let saved = active.update(&txn).await?;
        if change.status == Some(ProjectStatus::Archived) && was != Some(ProjectStatus::Archived) {
            park(&txn, pid, saved.hard && saved.deadline.is_some()).await?;
        }
        if change.status.is_some_and(|s| s != ProjectStatus::Archived)
            && was == Some(ProjectStatus::Archived)
        {
            issue::Entity::update_many()
                .col_expr(
                    issue::Column::ParkedUntil,
                    Expr::value(Option::<String>::None),
                )
                .filter(issue::Column::Pid.eq(pid.get()))
                .filter(issue::Column::ParkedUntil.eq(PARKED_BY_ARCHIVE))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(pid)
    }

    /// Brings back every paused project whose day has come (`today` or before); returns them.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn resume_due(&self, today: Date, now: Timestamp) -> Result<Vec<Project>, DbError> {
        let due = project::Entity::find()
            .filter(project::Column::Status.eq(ProjectStatus::Paused.as_str()))
            .filter(project::Column::PausedUntil.is_not_null())
            .filter(project::Column::PausedUntil.lte(today.to_string()))
            .all(&self.conn)
            .await?;
        let mut resumed = Vec::with_capacity(due.len());
        for row in due {
            let mut active = row.into_active_model();
            active.status = Set(ProjectStatus::Active.as_str().to_owned());
            active.paused_until = Set(None);
            active.updated = Set(millis(now));
            resumed.push(from_row(active.update(&self.conn).await?)?);
        }
        Ok(resumed)
    }
}

/// Parks the project's open tasks that don't need the person: not one waiting on them, nor one
/// with a hard deadline (its own, or the project's hard one when it has none).
async fn park(
    conn: &impl ConnectionTrait,
    pid: ProjectId,
    project_hard: bool,
) -> Result<(), DbError> {
    let mut query = issue::Entity::update_many()
        .col_expr(issue::Column::ParkedUntil, Expr::value(PARKED_BY_ARCHIVE))
        .filter(issue::Column::Pid.eq(pid.get()))
        .filter(issue::Column::Status.is_in([Status::Todo.as_str(), Status::Working.as_str()]))
        .filter(issue::Column::ParkedUntil.is_null())
        .filter(
            issue::Column::Hard
                .eq(false)
                .or(issue::Column::Due.is_null()),
        );
    if project_hard {
        query = query.filter(issue::Column::Due.is_not_null());
    }
    query.exec(conn).await?;
    Ok(())
}

fn from_row(row: project::Model) -> Result<Project, DbError> {
    Ok(Project {
        id: ProjectId::new(row.id),
        goal: row.goal.map(GoalId::new),
        kind: ProjectKind::parse(&row.kind).unwrap_or(ProjectKind::Deliverable),
        deadline: date(row.deadline.as_deref()),
        status: ProjectStatus::parse(&row.status).unwrap_or(ProjectStatus::Active),
        paused_until: date(row.paused_until.as_deref()),
        created: instant(row.created)?,
        updated: instant(row.updated)?,
        name: row.name,
        priority: row.priority,
        hard: row.hard,
    })
}
