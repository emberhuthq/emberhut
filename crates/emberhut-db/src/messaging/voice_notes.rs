//! Voice notes ([`VoiceNoteStore`]).

use emberhut_core::messaging::voice::{SeenNote, VoiceNote, VoiceSource, VoiceStatus};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::voice_note;
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// How a note's try ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Transcribed: what was said, and its length when the fetch told.
    Done {
        /// What was said.
        text: String,
        /// Its length, when known now.
        seconds: Option<f64>,
    },
    /// Over the length limit: skipped.
    Long {
        /// Its length, when known now.
        seconds: Option<f64>,
    },
    /// Failed again.
    Failed {
        /// Why.
        error: String,
        /// The rounds failed so far.
        tries: u32,
        /// When to try again; none to give up.
        next_at: Option<Timestamp>,
    },
}

/// Keeps voice notes and what they said.
#[derive(Clone, Debug)]
pub struct VoiceNoteStore {
    conn: DatabaseConnection,
}

impl VoiceNoteStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps a note seen in a message at `now`. One kept already gets the new way to fetch it,
    /// and whatever it lacked (chat, sender, message, length) from this sighting. Returns the
    /// note as kept.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn seen(&self, seen: &SeenNote, now: Timestamp) -> Result<VoiceNote, DbError> {
        let txn = begin_write(&self.conn).await?;
        let kept = voice_note::Entity::find_by_id(seen.id.as_str())
            .one(&txn)
            .await?;
        let row = if let Some(kept) = kept {
            let mut row = kept.clone().into_active_model();
            row.reference = Set(seen.reference.clone());
            row.chat = Set(kept.chat.or_else(|| seen.chat.clone()));
            row.chat_name = Set(kept.chat_name.or_else(|| seen.chat_name.clone()));
            row.sender = Set(kept.sender.or_else(|| seen.sender.clone()));
            row.msg_at = Set(kept.msg_at.or(seen.msg_at.map(millis)));
            row.msg = Set(kept.msg.or_else(|| seen.msg.clone()));
            row.seconds = Set(kept.seconds.or(seen.seconds));
            voice_note::Entity::update(row).exec(&txn).await?
        } else {
            let row = voice_note::ActiveModel {
                id: Set(seen.id.clone()),
                source: Set(seen.source.as_str().to_owned()),
                reference: Set(seen.reference.clone()),
                name: Set(seen.name.clone()),
                seconds: Set(seen.seconds),
                status: Set(VoiceStatus::New.as_str().to_owned()),
                text: Set(None),
                error: Set(None),
                tries: Set(0),
                next_at: Set(None),
                chat: Set(seen.chat.clone()),
                chat_name: Set(seen.chat_name.clone()),
                sender: Set(seen.sender.clone()),
                msg_at: Set(seen.msg_at.map(millis)),
                msg: Set(seen.msg.clone()),
                shown: Set(None),
                at: Set(millis(now)),
                done_at: Set(None),
            };
            voice_note::Entity::insert(row)
                .exec_with_returning(&txn)
                .await?
        };
        txn.commit().await?;
        from_row(row)
    }

    /// The note `id`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn get(&self, id: &str) -> Result<Option<VoiceNote>, DbError> {
        voice_note::Entity::find_by_id(id)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Records how note `id`'s try ended, at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn finish(&self, id: &str, outcome: &Outcome, now: Timestamp) -> Result<(), DbError> {
        let update = voice_note::Entity::update_many().filter(voice_note::Column::Id.eq(id));
        let update = match outcome {
            Outcome::Done { text, seconds } => {
                let update = update
                    .col_expr(
                        voice_note::Column::Status,
                        Expr::value(VoiceStatus::Done.as_str()),
                    )
                    .col_expr(voice_note::Column::Text, Expr::value(text.clone()))
                    .col_expr(
                        voice_note::Column::Error,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(voice_note::Column::DoneAt, Expr::value(millis(now)));
                with_seconds(update, *seconds)
            }
            Outcome::Long { seconds } => {
                let update = update
                    .col_expr(
                        voice_note::Column::Status,
                        Expr::value(VoiceStatus::Long.as_str()),
                    )
                    .col_expr(voice_note::Column::DoneAt, Expr::value(millis(now)));
                with_seconds(update, *seconds)
            }
            Outcome::Failed {
                error,
                tries,
                next_at,
            } => {
                let status = if next_at.is_some() {
                    VoiceStatus::Failed
                } else {
                    VoiceStatus::GaveUp
                };
                update
                    .col_expr(voice_note::Column::Status, Expr::value(status.as_str()))
                    .col_expr(voice_note::Column::Tries, Expr::value(*tries))
                    .col_expr(voice_note::Column::Error, Expr::value(error.clone()))
                    .col_expr(voice_note::Column::NextAt, Expr::value(next_at.map(millis)))
            }
        };
        update.exec(&self.conn).await?;
        Ok(())
    }

    /// Records what note `id`'s message shows for it now.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_shown(&self, id: &str, shown: &str) -> Result<(), DbError> {
        voice_note::Entity::update_many()
            .col_expr(voice_note::Column::Shown, Expr::value(shown))
            .filter(voice_note::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The notes due a try at `now`: failed ones whose time has come, and new ones first seen
    /// before `stale` (left half-done by a restart).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn due(&self, now: Timestamp, stale: Timestamp) -> Result<Vec<String>, DbError> {
        let rows = voice_note::Entity::find()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(voice_note::Column::Status.eq(VoiceStatus::Failed.as_str()))
                            .add(voice_note::Column::NextAt.lte(millis(now))),
                    )
                    .add(
                        Condition::all()
                            .add(voice_note::Column::Status.eq(VoiceStatus::New.as_str()))
                            .add(voice_note::Column::At.lt(millis(stale))),
                    ),
            )
            .all(&self.conn)
            .await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    /// The done WhatsApp and iMessage notes whose message came after `since` and whose text,
    /// chat name or sender holds `words`, newest first (at most `limit`).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn search(
        &self,
        words: &str,
        since: Timestamp,
        limit: usize,
    ) -> Result<Vec<VoiceNote>, DbError> {
        let since = millis(since);
        let rows = voice_note::Entity::find()
            .filter(voice_note::Column::Status.eq(VoiceStatus::Done.as_str()))
            .filter(voice_note::Column::Source.is_in([
                VoiceSource::WhatsApp.as_str(),
                VoiceSource::IMessage.as_str(),
            ]))
            .filter(
                Condition::any()
                    .add(voice_note::Column::MsgAt.gt(since))
                    .add(
                        Condition::all()
                            .add(voice_note::Column::MsgAt.is_null())
                            .add(voice_note::Column::At.gt(since)),
                    ),
            )
            .filter(
                Condition::any()
                    .add(voice_note::Column::Text.contains(words))
                    .add(voice_note::Column::ChatName.contains(words))
                    .add(voice_note::Column::Sender.contains(words)),
            )
            .all(&self.conn)
            .await?;
        let mut notes = rows
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        notes.sort_by_key(|note| std::cmp::Reverse(note.msg_at.unwrap_or(note.at)));
        notes.truncate(limit);
        Ok(notes)
    }

    /// The notes transcribed after `after` (every done one without it), the earliest first, at
    /// most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn done_after(
        &self,
        after: Option<Timestamp>,
        limit: u64,
    ) -> Result<Vec<VoiceNote>, DbError> {
        let mut query = voice_note::Entity::find()
            .filter(voice_note::Column::Status.eq(VoiceStatus::Done.as_str()))
            .filter(voice_note::Column::DoneAt.is_not_null());
        if let Some(after) = after {
            query = query.filter(voice_note::Column::DoneAt.gt(millis(after)));
        }
        query
            .order_by_asc(voice_note::Column::DoneAt)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Slack's failed and given-up notes, due again at `now` with their tries reset (a new
    /// token may fetch them now); how many.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn retry_slack(&self, now: Timestamp) -> Result<u64, DbError> {
        let updated = voice_note::Entity::update_many()
            .col_expr(
                voice_note::Column::Status,
                Expr::value(VoiceStatus::Failed.as_str()),
            )
            .col_expr(voice_note::Column::NextAt, Expr::value(millis(now)))
            .col_expr(voice_note::Column::Tries, Expr::value(0))
            .filter(voice_note::Column::Source.eq(VoiceSource::Slack.as_str()))
            .filter(
                voice_note::Column::Status
                    .is_in([VoiceStatus::Failed.as_str(), VoiceStatus::GaveUp.as_str()]),
            )
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected)
    }
}

fn with_seconds(
    update: sea_orm::UpdateMany<voice_note::Entity>,
    seconds: Option<f64>,
) -> sea_orm::UpdateMany<voice_note::Entity> {
    match seconds {
        Some(seconds) => update.col_expr(voice_note::Column::Seconds, Expr::value(seconds)),
        None => update,
    }
}

fn from_row(row: voice_note::Model) -> Result<VoiceNote, DbError> {
    Ok(VoiceNote {
        source: VoiceSource::from_word(&row.source).unwrap_or(VoiceSource::IMessage),
        status: VoiceStatus::from_word(&row.status),
        tries: u32::try_from(row.tries).unwrap_or(0),
        next_at: row.next_at.map(instant).transpose()?,
        msg_at: row.msg_at.map(instant).transpose()?,
        at: instant(row.at)?,
        done_at: row.done_at.map(instant).transpose()?,
        id: row.id,
        reference: row.reference,
        name: row.name,
        seconds: row.seconds,
        text: row.text,
        error: row.error,
        chat: row.chat,
        chat_name: row.chat_name,
        sender: row.sender,
        msg: row.msg,
        shown: row.shown,
    })
}
