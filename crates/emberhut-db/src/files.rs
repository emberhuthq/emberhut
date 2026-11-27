//! The chat's files: the stored files, and which messages came with them.

use std::collections::BTreeMap;

use emberhut_core::upload::{FileKind, NewFile, StoredFile};
use emberhut_core::{FileId, MessageId};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder,
};

use crate::DbError;
use crate::entity::{file, message_file};
use crate::time::{instant, millis};

/// Reads and writes the files stored for the chat. (Their bytes are on disk; this is what the
/// server knows of them.)
#[derive(Clone, Debug)]
pub struct FileStore {
    conn: DatabaseConnection,
}

impl FileStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Notes `new` as stored at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when a file with its stored name is noted already.
    pub async fn add(&self, new: NewFile, now: Timestamp) -> Result<StoredFile, DbError> {
        let row = file::ActiveModel {
            stored_name: Set(new.stored_name),
            name: Set(new.name),
            kind: Set(new.kind.into()),
            size: Set(i64::try_from(new.size).unwrap_or(i64::MAX)),
            created_at: Set(millis(now)),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        from_row(row)
    }

    /// The file stored under `stored_name`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn by_stored_name(&self, stored_name: &str) -> Result<Option<StoredFile>, DbError> {
        file::Entity::find()
            .filter(file::Column::StoredName.eq(stored_name))
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The files each of these messages came with, in the order they were attached. A message
    /// without files has no entry.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn of_messages(
        &self,
        messages: &[MessageId],
    ) -> Result<BTreeMap<MessageId, Vec<StoredFile>>, DbError> {
        of_messages(&self.conn, messages).await
    }
}

/// Attaches `files` to `message`, in order.
pub(crate) async fn attach(
    conn: &impl ConnectionTrait,
    message: MessageId,
    files: &[FileId],
) -> Result<(), DbError> {
    if files.is_empty() {
        return Ok(());
    }
    let rows = files
        .iter()
        .enumerate()
        .map(|(position, file)| message_file::ActiveModel {
            message_id: Set(message.get()),
            position: Set(i32::try_from(position).unwrap_or(i32::MAX)),
            file_id: Set(file.get()),
        });
    message_file::Entity::insert_many(rows).exec(conn).await?;
    Ok(())
}

/// [`FileStore::of_messages`] on `conn`.
pub(crate) async fn of_messages(
    conn: &impl ConnectionTrait,
    messages: &[MessageId],
) -> Result<BTreeMap<MessageId, Vec<StoredFile>>, DbError> {
    let mut found = BTreeMap::new();
    if messages.is_empty() {
        return Ok(found);
    }
    let links = message_file::Entity::find()
        .filter(message_file::Column::MessageId.is_in(messages.iter().map(|id| id.get())))
        .order_by_asc(message_file::Column::MessageId)
        .order_by_asc(message_file::Column::Position)
        .all(conn)
        .await?;
    if links.is_empty() {
        return Ok(found);
    }
    let files: BTreeMap<i64, StoredFile> = file::Entity::find()
        .filter(file::Column::Id.is_in(links.iter().map(|link| link.file_id)))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| from_row(row).map(|stored| (stored.id.get(), stored)))
        .collect::<Result<_, _>>()?;
    for link in links {
        if let Some(stored) = files.get(&link.file_id) {
            found
                .entry(MessageId::new(link.message_id))
                .or_insert_with(Vec::new)
                .push(stored.clone());
        }
    }
    Ok(found)
}

fn from_row(row: file::Model) -> Result<StoredFile, DbError> {
    Ok(StoredFile {
        id: FileId::new(row.id),
        stored_name: row.stored_name,
        name: row.name,
        kind: row.kind.into(),
        size: u64::try_from(row.size).unwrap_or(0),
        created_at: instant(row.created_at)?,
    })
}

impl From<FileKind> for file::Kind {
    fn from(kind: FileKind) -> Self {
        match kind {
            FileKind::Image => Self::Image,
            FileKind::Video => Self::Video,
            FileKind::File => Self::File,
        }
    }
}

impl From<file::Kind> for FileKind {
    fn from(kind: file::Kind) -> Self {
        match kind {
            file::Kind::Image => Self::Image,
            file::Kind::Video => Self::Video,
            file::Kind::File => Self::File,
        }
    }
}
