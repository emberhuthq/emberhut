//! Artifacts: what a task produced, for the person's eyes: files on this machine (pictures,
//! videos, PDFs, documents, notes, reports), links, and X posts with the replies suggested for
//! them (`data`). A file is kept by its path and served only through its id.

use std::collections::BTreeMap;

use emberhut_core::task::{ArtifactId, TaskId};
use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder,
};

use crate::DbError;
use crate::entity::artifact;
use crate::time::{instant, millis};

/// Something a task produced, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredArtifact {
    /// Its id.
    pub id: ArtifactId,
    /// Its task.
    pub task: TaskId,
    /// The file's path, or the link (an X reply's: its post's address).
    pub reference: String,
    /// `image`, `video`, `pdf`, `md`, `html`, `doc`, `link` or `x_reply`.
    pub kind: String,
    /// A line about it.
    pub caption: Option<String>,
    /// When it was attached.
    pub at: Timestamp,
    /// An X reply's post and replies, as JSON.
    pub data: Option<String>,
}

/// Reads and writes artifacts.
#[derive(Clone, Debug)]
pub struct ArtifactStore {
    conn: DatabaseConnection,
}

impl ArtifactStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Attaches `reference` (of `kind`) to the task, once; a caption given replaces the one it
    /// had. Returns the artifact.
    ///
    /// # Errors
    ///
    /// [`DbError`], also when the task doesn't exist.
    pub async fn attach(
        &self,
        task: TaskId,
        reference: &str,
        kind: &str,
        caption: Option<&str>,
        now: Timestamp,
    ) -> Result<StoredArtifact, DbError> {
        let row = artifact::ActiveModel {
            issue: Set(task.get()),
            reference: Set(reference.to_owned()),
            kind: Set(kind.to_owned()),
            caption: Set(caption.map(str::to_owned)),
            at: Set(millis(now)),
            data: Set(None),
            ..Default::default()
        };
        artifact::Entity::insert(row)
            .on_conflict_do_nothing_on([artifact::Column::Issue, artifact::Column::Reference])
            .exec(&self.conn)
            .await?;
        let found = self.find(task, reference).await?.ok_or(DbError::Query(
            sea_orm::DbErr::RecordNotFound("the artifact just attached".to_owned()),
        ))?;
        if let Some(caption) = caption.filter(|c| !c.is_empty())
            && found.caption.as_deref() != Some(caption)
        {
            let mut active = artifact::Entity::find_by_id(found.id.get())
                .one(&self.conn)
                .await?
                .ok_or(DbError::Query(sea_orm::DbErr::RecordNotFound(
                    "artifact".to_owned(),
                )))?
                .into_active_model();
            active.caption = Set(Some(caption.to_owned()));
            return from_row(active.update(&self.conn).await?);
        }
        Ok(found)
    }

    /// Keeps an X post's suggestions on the task, under the post's address: sent again, they
    /// are replaced in place (the caller keeps what was posted).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn put_reply(
        &self,
        task: TaskId,
        post_url: &str,
        data: &str,
        now: Timestamp,
    ) -> Result<StoredArtifact, DbError> {
        let row = artifact::ActiveModel {
            issue: Set(task.get()),
            reference: Set(post_url.to_owned()),
            kind: Set(X_REPLY.to_owned()),
            caption: Set(None),
            at: Set(millis(now)),
            data: Set(Some(data.to_owned())),
            ..Default::default()
        };
        artifact::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([artifact::Column::Issue, artifact::Column::Reference])
                    .update_column(artifact::Column::Data)
                    .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        self.find(task, post_url)
            .await?
            .ok_or(DbError::Query(sea_orm::DbErr::RecordNotFound(
                "the reply just kept".to_owned(),
            )))
    }

    /// Replaces an artifact's data.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_data(&self, id: ArtifactId, data: &str) -> Result<bool, DbError> {
        let Some(row) = artifact::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
        else {
            return Ok(false);
        };
        let mut active = row.into_active_model();
        active.data = Set(Some(data.to_owned()));
        active.update(&self.conn).await?;
        Ok(true)
    }

    /// The artifact `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, id: ArtifactId) -> Result<Option<StoredArtifact>, DbError> {
        artifact::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The task's artifact for `reference`, if it has one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn find(
        &self,
        task: TaskId,
        reference: &str,
    ) -> Result<Option<StoredArtifact>, DbError> {
        artifact::Entity::find()
            .filter(artifact::Column::Issue.eq(task.get()))
            .filter(artifact::Column::Reference.eq(reference))
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The artifacts of each of `tasks`, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn of(
        &self,
        tasks: &[TaskId],
    ) -> Result<BTreeMap<TaskId, Vec<StoredArtifact>>, DbError> {
        let mut by_task: BTreeMap<TaskId, Vec<StoredArtifact>> = BTreeMap::new();
        if tasks.is_empty() {
            return Ok(by_task);
        }
        for row in artifact::Entity::find()
            .filter(artifact::Column::Issue.is_in(tasks.iter().map(|t| t.get())))
            .order_by_desc(artifact::Column::At)
            .order_by_desc(artifact::Column::Id)
            .all(&self.conn)
            .await?
        {
            let stored = from_row(row)?;
            by_task.entry(stored.task).or_default().push(stored);
        }
        Ok(by_task)
    }
}

/// The kind of an X post's suggestions.
pub const X_REPLY: &str = "x_reply";

fn from_row(row: artifact::Model) -> Result<StoredArtifact, DbError> {
    Ok(StoredArtifact {
        id: ArtifactId::new(row.id),
        task: TaskId::new(row.issue),
        at: instant(row.at)?,
        reference: row.reference,
        kind: row.kind,
        caption: row.caption,
        data: row.data,
    })
}
