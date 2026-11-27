//! Calls ([`CallStore`]): recorded and shared calls, their live transcript's lines and the
//! copilot's hints.

use std::collections::{BTreeMap, BTreeSet};

use emberhut_core::RoomId;
use emberhut_core::calls::{
    Call, CallHint, CallId, CallStatus, HeardLine, LiveLine, NewCall, Track, TrackKind,
};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TryInsertResult,
};
use serde_json::{Value, json};

use crate::DbError;
use crate::entity::{call, call_hint, call_line};
use crate::time::{instant, millis};

/// A call that is being recorded now, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCall {
    /// The call.
    pub id: CallId,
    /// Its room.
    pub room: RoomId,
    /// Its title.
    pub title: String,
}

/// Writes and reads calls.
#[derive(Clone, Debug)]
pub struct CallStore {
    conn: DatabaseConnection,
}

impl CallStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores `new`, started at `at`, with no tracks yet.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when a call with its id exists.
    pub async fn create(&self, new: NewCall, at: Timestamp) -> Result<(), DbError> {
        call::Entity::insert(row(new, at))
            .exec_without_returning(&self.conn)
            .await?;
        Ok(())
    }

    /// Stores `new` unless a call with its id exists; whether it was stored.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn create_if_missing(&self, new: NewCall, at: Timestamp) -> Result<bool, DbError> {
        let inserted = call::Entity::insert(row(new, at))
            .on_conflict_do_nothing_on([call::Column::Id])
            .exec_without_returning(&self.conn)
            .await?;
        Ok(matches!(inserted, TryInsertResult::Inserted(n) if n > 0))
    }

    /// The call `id`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, id: &CallId) -> Result<Option<Call>, DbError> {
        call::Entity::find_by_id(id.as_str())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Removes the call `id` (its lines and hints with it); whether it was there.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete(&self, id: &CallId) -> Result<bool, DbError> {
        let deleted = call::Entity::delete_by_id(id.as_str())
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Sets the call's tracks, and its length when `seconds` is given.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_tracks(
        &self,
        id: &CallId,
        tracks: &[Track],
        seconds: Option<f64>,
    ) -> Result<(), DbError> {
        let mut update = call::Entity::update_many()
            .col_expr(call::Column::Tracks, Expr::value(tracks_json(tracks)))
            .filter(call::Column::Id.eq(id.as_str()));
        if let Some(seconds) = seconds {
            update = update.col_expr(call::Column::Seconds, Expr::value(seconds));
        }
        update.exec(&self.conn).await?;
        Ok(())
    }

    /// Sets the call's one track and the shared recording it was copied from.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_source(
        &self,
        id: &CallId,
        tracks: &[Track],
        source: &str,
    ) -> Result<(), DbError> {
        call::Entity::update_many()
            .col_expr(call::Column::Tracks, Expr::value(tracks_json(tracks)))
            .col_expr(call::Column::Source, Expr::value(source))
            .filter(call::Column::Id.eq(id.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Moves the call to `status`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_status(&self, id: &CallId, status: CallStatus) -> Result<(), DbError> {
        call::Entity::update_many()
            .col_expr(call::Column::Status, Expr::value(status.as_str()))
            .filter(call::Column::Id.eq(id.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Puts every call that was being transcribed back in the queue (after a restart); how
    /// many there were.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn requeue_transcribing(&self) -> Result<u64, DbError> {
        let updated = call::Entity::update_many()
            .col_expr(
                call::Column::Status,
                Expr::value(CallStatus::Queued.as_str()),
            )
            .filter(call::Column::Status.eq(CallStatus::Transcribing.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected)
    }

    /// The call that has waited longest to be transcribed.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn next_queued(&self) -> Result<Option<Call>, DbError> {
        call::Entity::find()
            .filter(call::Column::Status.eq(CallStatus::Queued.as_str()))
            .order_by_asc(call::Column::At)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The call is transcribed, into the file `transcript`, at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_done(
        &self,
        id: &CallId,
        transcript: &str,
        at: Timestamp,
    ) -> Result<(), DbError> {
        call::Entity::update_many()
            .col_expr(call::Column::Status, Expr::value(CallStatus::Done.as_str()))
            .col_expr(call::Column::Transcript, Expr::value(transcript))
            .col_expr(call::Column::DoneAt, Expr::value(millis(at)))
            .filter(call::Column::Id.eq(id.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The call's transcription failed, for `error`, at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_failed(
        &self,
        id: &CallId,
        error: &str,
        at: Timestamp,
    ) -> Result<(), DbError> {
        call::Entity::update_many()
            .col_expr(
                call::Column::Status,
                Expr::value(CallStatus::Failed.as_str()),
            )
            .col_expr(call::Column::Error, Expr::value(error))
            .col_expr(call::Column::DoneAt, Expr::value(millis(at)))
            .filter(call::Column::Id.eq(id.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The newest call being recorded that started after `since`.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn recording_since(&self, since: Timestamp) -> Result<Option<Call>, DbError> {
        call::Entity::find()
            .filter(call::Column::Status.eq(CallStatus::Recording.as_str()))
            .filter(call::Column::At.gt(millis(since)))
            .order_by_desc(call::Column::At)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The newest `limit` calls, newest first.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn latest(&self, limit: u64) -> Result<Vec<Call>, DbError> {
        call::Entity::find()
            .order_by_desc(call::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// The transcribed or failed calls whose audio is still kept and that ended before
    /// `before`.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn audio_older_than(&self, before: Timestamp) -> Result<Vec<Call>, DbError> {
        call::Entity::find()
            .filter(call::Column::AudioGone.eq(false))
            .filter(
                call::Column::Status
                    .is_in([CallStatus::Done.as_str(), CallStatus::Failed.as_str()]),
            )
            .filter(call::Column::DoneAt.lt(millis(before)))
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// The call's audio is deleted.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_audio_gone(&self, id: &CallId) -> Result<(), DbError> {
        call::Entity::update_many()
            .col_expr(call::Column::AudioGone, Expr::value(true))
            .filter(call::Column::Id.eq(id.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Stores a line heard on the call at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when the call doesn't exist.
    pub async fn add_line(
        &self,
        id: &CallId,
        heard: HeardLine,
        at: Timestamp,
    ) -> Result<(), DbError> {
        call_line::Entity::insert(call_line::ActiveModel {
            call_id: Set(id.as_str().to_owned()),
            track: Set(heard.line.track.as_str().to_owned()),
            t: Set(heard.line.t),
            text: Set(heard.line.text),
            lang: Set(heard.lang),
            sure: Set(heard.sure),
            at: Set(millis(at)),
            ..Default::default()
        })
        .exec_without_returning(&self.conn)
        .await?;
        Ok(())
    }

    /// The call's latest `limit` lines (by time, then as stored), oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn last_lines(&self, id: &CallId, limit: u64) -> Result<Vec<LiveLine>, DbError> {
        let mut lines: Vec<LiveLine> = call_line::Entity::find()
            .filter(call_line::Column::CallId.eq(id.as_str()))
            .order_by_desc(call_line::Column::T)
            .order_by_desc(call_line::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .filter_map(line_from_row)
            .collect();
        lines.reverse();
        Ok(lines)
    }

    /// The call's lines from after `t` seconds on, in order.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn lines_after(&self, id: &CallId, t: f64) -> Result<Vec<LiveLine>, DbError> {
        Ok(call_line::Entity::find()
            .filter(call_line::Column::CallId.eq(id.as_str()))
            .filter(call_line::Column::T.gt(t))
            .order_by_asc(call_line::Column::T)
            .order_by_asc(call_line::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .filter_map(line_from_row)
            .collect())
    }

    /// The language most of the call's lines are in, of those the transcriber was sure of.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn call_lang(&self, id: &CallId) -> Result<Option<String>, DbError> {
        let langs: Vec<Option<String>> = call_line::Entity::find()
            .select_only()
            .column(call_line::Column::Lang)
            .filter(call_line::Column::CallId.eq(id.as_str()))
            .filter(call_line::Column::Sure.eq(true))
            .into_tuple()
            .all(&self.conn)
            .await?;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for lang in langs.into_iter().flatten() {
            *counts.entry(lang).or_default() += 1;
        }
        Ok(counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(lang, _)| lang))
    }

    /// The calls being recorded that had a line heard after `since`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn live_since(&self, since: Timestamp) -> Result<Vec<LiveCall>, DbError> {
        let ids: BTreeSet<String> = call_line::Entity::find()
            .select_only()
            .column(call_line::Column::CallId)
            .filter(call_line::Column::At.gt(millis(since)))
            .into_tuple::<String>()
            .all(&self.conn)
            .await?
            .into_iter()
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(call::Entity::find()
            .filter(call::Column::Id.is_in(ids))
            .filter(call::Column::Status.eq(CallStatus::Recording.as_str()))
            .all(&self.conn)
            .await?
            .into_iter()
            .filter_map(|row| {
                Some(LiveCall {
                    id: CallId::parse(&row.id)?,
                    room: RoomId::new(row.room_id),
                    title: row.title,
                })
            })
            .collect())
    }

    /// Stores the copilot's hint at `t` seconds into the call.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when the call doesn't exist.
    pub async fn add_hint(
        &self,
        id: &CallId,
        t: Option<f64>,
        text: &str,
        at: Timestamp,
    ) -> Result<(), DbError> {
        call_hint::Entity::insert(call_hint::ActiveModel {
            call_id: Set(id.as_str().to_owned()),
            t: Set(t),
            text: Set(text.to_owned()),
            at: Set(millis(at)),
            ..Default::default()
        })
        .exec_without_returning(&self.conn)
        .await?;
        Ok(())
    }

    /// The call's hints, in the order they were given.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn hints(&self, id: &CallId) -> Result<Vec<CallHint>, DbError> {
        call_hint::Entity::find()
            .filter(call_hint::Column::CallId.eq(id.as_str()))
            .order_by_asc(call_hint::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(CallHint {
                    t: row.t,
                    text: row.text,
                    at: instant(row.at)?,
                })
            })
            .collect()
    }
}

fn row(new: NewCall, at: Timestamp) -> call::ActiveModel {
    call::ActiveModel {
        id: Set(new.id.as_str().to_owned()),
        title: Set(new.title),
        task: Set(new.task),
        room_id: Set(new.room.get()),
        status: Set(new.status.as_str().to_owned()),
        tracks: Set("[]".to_owned()),
        source: Set(None),
        transcript: Set(None),
        error: Set(None),
        seconds: Set(None),
        at: Set(millis(at)),
        done_at: Set(None),
        audio_gone: Set(false),
    }
}

/// The tracks as stored: `[{ "file": …, "who": … }]`.
fn tracks_json(tracks: &[Track]) -> String {
    Value::Array(
        tracks
            .iter()
            .map(|track| json!({ "file": track.file, "who": track.who }))
            .collect(),
    )
    .to_string()
}

/// The stored tracks; a row that isn't such a list has none.
fn tracks_from(text: &str) -> Vec<Track> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(Track {
                file: item.get("file")?.as_str()?.to_owned(),
                who: item
                    .get("who")
                    .and_then(Value::as_str)
                    .filter(|who| !who.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn from_row(row: call::Model) -> Result<Call, DbError> {
    Ok(Call {
        // A row's id was checked when it was stored; one changed since keeps its text.
        id: CallId::parse(&row.id).unwrap_or_else(|| CallId::stored(row.id.clone())),
        title: row.title,
        task: row.task,
        room: RoomId::new(row.room_id),
        status: CallStatus::from_name(&row.status).unwrap_or(CallStatus::Failed),
        tracks: tracks_from(&row.tracks),
        source: row.source,
        transcript: row.transcript,
        error: row.error,
        seconds: row.seconds,
        at: instant(row.at)?,
        done_at: row.done_at.map(instant).transpose()?,
        audio_gone: row.audio_gone,
    })
}

fn line_from_row(row: call_line::Model) -> Option<LiveLine> {
    Some(LiveLine {
        track: TrackKind::from_name(&row.track)?,
        t: row.t,
        text: row.text,
    })
}
