//! What the assistant was corrected on, and the trials of the lessons it drew.

use emberhut_core::memory::learning::{Correction, NewCorrection, TrialJudge};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};

use crate::entity::correction;
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// How a trial ended, as stored.
const KEPT: &str = "kept";
/// How a trial ended, as stored.
const DROPPED: &str = "dropped";

/// How a correction's trial stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialEnd {
    /// It runs, or ran out unreviewed.
    Open,
    /// Kept as this standing rule.
    Kept(Option<i64>),
    /// Dropped, by the owner or the review.
    Dropped(Option<String>),
}

/// Reads and writes corrections.
#[derive(Clone, Debug)]
pub struct CorrectionStore {
    conn: DatabaseConnection,
}

impl CorrectionStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Logs `new`; its id.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn log(&self, new: NewCorrection) -> Result<i64, DbError> {
        let row = correction::ActiveModel {
            at: Set(millis(new.at)),
            said: Set(new.said),
            wrong: Set(new.wrong),
            lesson: Set(new.lesson),
            source: Set(new.source.as_str().to_owned()),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        Ok(row.id)
    }

    /// The corrections after `after`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn since(&self, after: Timestamp) -> Result<Vec<Correction>, DbError> {
        correction::Entity::find()
            .filter(correction::Column::At.gt(millis(after)))
            .order_by_asc(correction::Column::At)
            .order_by_asc(correction::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// The corrections after `after` whose lesson's trial was neither kept nor dropped,
    /// oldest first (a blank lesson is no trial: the caller leaves those out).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn open_trials(&self, after: Timestamp) -> Result<Vec<Correction>, DbError> {
        correction::Entity::find()
            .filter(correction::Column::At.gt(millis(after)))
            .filter(correction::Column::Lesson.is_not_null())
            .filter(correction::Column::Trial.is_null())
            .order_by_asc(correction::Column::At)
            .order_by_asc(correction::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Ends the trial of correction `id` as dropped by `by` at `now`, when it still runs (logged
    /// after `after`, with a lesson, not ended); `false` when it doesn't.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn drop_trial(
        &self,
        id: i64,
        by: TrialJudge,
        now: Timestamp,
        after: Timestamp,
    ) -> Result<bool, DbError> {
        self.end_trial(id, DROPPED, by, None, now, Some(after))
            .await
    }

    /// Ends the trial of correction `id` as kept, by the review, as standing rule `rule`.
    /// `false` when there is no such correction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn keep_trial(&self, id: i64, rule: i64, now: Timestamp) -> Result<bool, DbError> {
        self.end_trial(id, KEPT, TrialJudge::Review, Some(rule), now, None)
            .await
    }

    async fn end_trial(
        &self,
        id: i64,
        how: &str,
        by: TrialJudge,
        rule: Option<i64>,
        now: Timestamp,
        open_after: Option<Timestamp>,
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let Some(row) = correction::Entity::find_by_id(id).one(&txn).await? else {
            return Ok(false);
        };
        if let Some(after) = open_after {
            let open = row.trial.is_none()
                && row.at > millis(after)
                && row
                    .lesson
                    .as_deref()
                    .is_some_and(|lesson| !lesson.trim().is_empty());
            if !open {
                return Ok(false);
            }
        }
        let mut change: correction::ActiveModel = row.into();
        change.trial = Set(Some(how.to_owned()));
        change.trial_by = Set(Some(by.as_str().to_owned()));
        change.trial_at = Set(Some(millis(now)));
        if rule.is_some() {
            change.rule = Set(rule);
        }
        change.update(&txn).await?;
        txn.commit().await?;
        Ok(true)
    }

    /// How the trial of correction `id` stands; none when there is no such correction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn trial_end(&self, id: i64) -> Result<Option<TrialEnd>, DbError> {
        Ok(correction::Entity::find_by_id(id)
            .one(&self.conn)
            .await?
            .map(|row| match row.trial.as_deref() {
                Some(KEPT) => TrialEnd::Kept(row.rule),
                Some(DROPPED) => TrialEnd::Dropped(row.trial_by),
                _ => TrialEnd::Open,
            }))
    }
}

fn from_row(row: correction::Model) -> Result<Correction, DbError> {
    Ok(Correction {
        id: row.id,
        at: instant(row.at)?,
        said: row.said,
        wrong: row.wrong,
        lesson: row.lesson,
        source: row.source,
    })
}
