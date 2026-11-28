//! The owner's quick notes and the assistant's diary.

use emberhut_core::memory::notes::{Note, NoteAuthor, assistant_uid};
use emberhut_core::memory::wire;
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder,
};

use crate::entity::note;
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// `who` as stored: the wire words ([`wire`]).
fn who_text(who: NoteAuthor) -> &'static str {
    match who {
        NoteAuthor::Owner => wire::OWNER,
        NoteAuthor::Assistant => wire::ASSISTANT,
    }
}

/// A note the owner saves from the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerNote<'a> {
    /// The app's id for it.
    pub uid: &'a str,
    /// What it says (trimmed; not empty).
    pub text: &'a str,
    /// When it was written.
    pub at: Timestamp,
    /// The day it belongs to.
    pub day: &'a str,
}

/// Reads and writes notes.
#[derive(Clone, Debug)]
pub struct NoteStore {
    conn: DatabaseConnection,
}

impl NoteStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The notes of the days `from` to `to` (both included; none: no bound), newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn list(&self, from: Option<&str>, to: Option<&str>) -> Result<Vec<Note>, DbError> {
        let mut query = note::Entity::find();
        if let Some(from) = from {
            query = query.filter(note::Column::Day.gte(from));
        }
        if let Some(to) = to {
            query = query.filter(note::Column::Day.lte(to));
        }
        query
            .order_by_desc(note::Column::At)
            .order_by_desc(note::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// What changes whenever a note is written or deleted: how many there are and when the
    /// last was changed (`3 @ 1764000000000`).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn stamp(&self) -> Result<String, DbError> {
        let count = note::Entity::find().count(&self.conn).await?;
        let last = note::Entity::find()
            .order_by_desc(note::Column::Updated)
            .one(&self.conn)
            .await?
            .map_or(0, |row| row.updated);
        Ok(format!("{count} @ {last}"))
    }

    /// The note `uid`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, uid: &str) -> Result<Option<Note>, DbError> {
        note::Entity::find()
            .filter(note::Column::Uid.eq(uid))
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Saves the owner's note at `now`: a new one, or an edit of theirs with the same id. An
    /// edit keeps its time and day; the same text again (a resend) changes nothing. The note as
    /// it is now (an id the assistant's entry holds is left alone, and is what comes back).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn save_owners(&self, new: &OwnerNote<'_>, now: Timestamp) -> Result<Note, DbError> {
        let txn = begin_write(&self.conn).await?;
        let found = note::Entity::find()
            .filter(note::Column::Uid.eq(new.uid))
            .one(&txn)
            .await?;
        let row = match found {
            None => {
                note::ActiveModel {
                    uid: Set(new.uid.to_owned()),
                    who: Set(who_text(NoteAuthor::Owner).to_owned()),
                    day: Set(new.day.to_owned()),
                    at: Set(millis(new.at)),
                    text: Set(new.text.to_owned()),
                    created: Set(millis(now)),
                    updated: Set(millis(now)),
                    ..Default::default()
                }
                .insert(&txn)
                .await?
            }
            Some(row) if row.who == who_text(NoteAuthor::Owner) && row.text != new.text => {
                let mut change: note::ActiveModel = row.into();
                change.text = Set(new.text.to_owned());
                change.updated = Set(millis(now));
                change.update(&txn).await?
            }
            Some(row) => row,
        };
        txn.commit().await?;
        from_row(row)
    }

    /// Deletes the owner's note `uid`; `false` when there was none of theirs.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete_owners(&self, uid: &str) -> Result<bool, DbError> {
        let deleted = note::Entity::delete_many()
            .filter(note::Column::Uid.eq(uid))
            .filter(note::Column::Who.eq(who_text(NoteAuthor::Owner)))
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Writes the assistant's entry for `day` at `now`, in place of the one it had.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn write_assistants(
        &self,
        day: &str,
        text: &str,
        now: Timestamp,
    ) -> Result<Note, DbError> {
        let uid = assistant_uid(day);
        let txn = begin_write(&self.conn).await?;
        let found = note::Entity::find()
            .filter(note::Column::Uid.eq(uid.as_str()))
            .one(&txn)
            .await?;
        let row = match found {
            Some(row) => {
                let mut change: note::ActiveModel = row.into();
                change.text = Set(text.to_owned());
                change.updated = Set(millis(now));
                change.update(&txn).await?
            }
            None => {
                note::ActiveModel {
                    uid: Set(uid),
                    who: Set(who_text(NoteAuthor::Assistant).to_owned()),
                    day: Set(day.to_owned()),
                    at: Set(millis(now)),
                    text: Set(text.to_owned()),
                    created: Set(millis(now)),
                    updated: Set(millis(now)),
                    ..Default::default()
                }
                .insert(&txn)
                .await?
            }
        };
        txn.commit().await?;
        from_row(row)
    }

    /// The owner's notes that changed after `after` (none: all of them), oldest written first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn owners_changed_after(
        &self,
        after: Option<Timestamp>,
    ) -> Result<Vec<Note>, DbError> {
        let mut query =
            note::Entity::find().filter(note::Column::Who.eq(who_text(NoteAuthor::Owner)));
        if let Some(after) = after {
            query = query.filter(note::Column::Updated.gt(millis(after)));
        }
        query
            .order_by_asc(note::Column::At)
            .order_by_asc(note::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }
}

fn from_row(row: note::Model) -> Result<Note, DbError> {
    Ok(Note {
        who: if row.who == who_text(NoteAuthor::Assistant) {
            NoteAuthor::Assistant
        } else {
            NoteAuthor::Owner
        },
        at: instant(row.at)?,
        created: instant(row.created)?,
        updated: instant(row.updated)?,
        uid: row.uid,
        day: row.day,
        text: row.text,
    })
}
