//! Areas: the parts of a project that run for a while on their own. A name is unique in its
//! project.

use emberhut_core::task::area::{Area, AreaStatus, sort_areas};
use emberhut_core::task::{AreaId, ProjectId};
use jiff::Timestamp;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};

use crate::entity::{area, project};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// An area to make or change. Left out: unchanged; `Some(None)` clears.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AreaChange {
    /// The area (none: a new one in `pid`).
    pub id: Option<AreaId>,
    /// A new area's project.
    pub pid: Option<ProjectId>,
    /// Its name.
    pub name: Option<String>,
    /// Its status.
    pub status: Option<AreaStatus>,
    /// What done looks like.
    pub goal: Option<Option<String>>,
    /// A short line on what it is.
    pub about: Option<Option<String>>,
}

/// Why an area could not be made or changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AreaStoreError {
    /// A new area with no project, or one that doesn't exist.
    #[error("a new area needs its project (project_id)")]
    NeedsProject,
    /// An area with no name.
    #[error("an area needs a name")]
    NeedsName,
    /// There is no such area.
    #[error("no area {0}")]
    NoArea(AreaId),
    /// Its project has an area of that name.
    #[error("the project has an area named \"{0}\" already")]
    NameTaken(String),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<DbErr> for AreaStoreError {
    fn from(error: DbErr) -> Self {
        Self::Db(DbError::Query(error))
    }
}

/// Reads and writes areas.
#[derive(Clone, Debug)]
pub struct AreaStore {
    conn: DatabaseConnection,
}

impl AreaStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// A project's areas (every project's: none): active first, then paused, then done; by name.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn list(&self, pid: Option<ProjectId>) -> Result<Vec<Area>, DbError> {
        let mut query = area::Entity::find();
        if let Some(pid) = pid {
            query = query.filter(area::Column::Pid.eq(pid.get()));
        }
        let mut areas = query
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        sort_areas(&mut areas);
        Ok(areas)
    }

    /// The area `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, id: AreaId) -> Result<Option<Area>, DbError> {
        area::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// An area by its id ("3") or its name in the project (any case; a done one last).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn find(
        &self,
        id_or_name: &str,
        pid: Option<ProjectId>,
    ) -> Result<Option<Area>, DbError> {
        let text = id_or_name.trim();
        if text.is_empty() {
            return Ok(None);
        }
        if let Ok(id) = text.parse::<i64>() {
            return self.get(AreaId::new(id)).await;
        }
        let mut query = area::Entity::find()
            .filter(Expr::expr(Func::lower(Expr::col(area::Column::Name))).eq(text.to_lowercase()));
        if let Some(pid) = pid {
            query = query.filter(area::Column::Pid.eq(pid.get()));
        }
        let mut found = query
            .order_by_asc(area::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        found.sort_by_key(|a| a.status == AreaStatus::Done);
        Ok(found.into_iter().next())
    }

    /// Makes or changes an area; returns its id. A new one named like one of its project's is
    /// that one.
    ///
    /// # Errors
    ///
    /// [`AreaStoreError`].
    pub async fn set(&self, change: AreaChange, now: Timestamp) -> Result<AreaId, AreaStoreError> {
        let txn = begin_write(&self.conn).await?;
        let name = change.name.as_deref().map(str::trim);
        if name.is_some_and(str::is_empty) {
            return Err(AreaStoreError::NeedsName);
        }
        let same_name = |pid: i64, name: &str| {
            area::Entity::find()
                .filter(area::Column::Pid.eq(pid))
                .filter(
                    Expr::expr(Func::lower(Expr::col(area::Column::Name))).eq(name.to_lowercase()),
                )
        };
        let (row, rename) = match change.id.filter(|id| id.get() != 0) {
            None => {
                let pid = change
                    .pid
                    .filter(|pid| pid.get() != 0)
                    .ok_or(AreaStoreError::NeedsProject)?;
                if project::Entity::find_by_id(pid.get())
                    .one(&txn)
                    .await?
                    .is_none()
                {
                    return Err(AreaStoreError::NeedsProject);
                }
                let name = name.ok_or(AreaStoreError::NeedsName)?;
                match same_name(pid.get(), name).one(&txn).await? {
                    Some(row) => (row, false),
                    None => (
                        area::ActiveModel {
                            pid: Set(pid.get()),
                            name: Set(name.to_owned()),
                            status: Set(AreaStatus::Active.as_str().to_owned()),
                            goal: Set(None),
                            about: Set(None),
                            created: Set(millis(now)),
                            updated: Set(millis(now)),
                            ..Default::default()
                        }
                        .insert(&txn)
                        .await?,
                        false,
                    ),
                }
            }
            Some(id) => {
                let row = area::Entity::find_by_id(id.get())
                    .one(&txn)
                    .await?
                    .ok_or(AreaStoreError::NoArea(id))?;
                if let Some(name) = name
                    && same_name(row.pid, name)
                        .filter(area::Column::Id.ne(id.get()))
                        .one(&txn)
                        .await?
                        .is_some()
                {
                    return Err(AreaStoreError::NameTaken(name.to_owned()));
                }
                (row, true)
            }
        };
        let id = AreaId::new(row.id);
        let mut active = row.into_active_model();
        if rename && let Some(name) = name {
            active.name = Set(name.to_owned());
        }
        if let Some(status) = change.status {
            active.status = Set(status.as_str().to_owned());
        }
        let blank =
            |text: Option<String>| text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        if let Some(goal) = change.goal {
            active.goal = Set(blank(goal));
        }
        if let Some(about) = change.about {
            active.about = Set(blank(about));
        }
        active.updated = Set(millis(now));
        active.update(&txn).await?;
        txn.commit().await?;
        Ok(id)
    }
}

fn from_row(row: area::Model) -> Result<Area, DbError> {
    Ok(Area {
        id: AreaId::new(row.id),
        pid: ProjectId::new(row.pid),
        status: AreaStatus::parse(&row.status).unwrap_or(AreaStatus::Active),
        created: instant(row.created)?,
        updated: instant(row.updated)?,
        name: row.name,
        goal: row.goal,
        about: row.about,
    })
}
