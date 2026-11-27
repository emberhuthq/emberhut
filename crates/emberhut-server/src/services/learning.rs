//! Learning from corrections (`emberhut_core::memory::learning`): the corrections log, the
//! standing rules (lessons) the owner approved, the rules on trial, and the chat log the weekly
//! review reads.

use std::sync::Arc;

use emberhut_core::memory::iso;
use emberhut_core::memory::learning::{
    ChatLine, ChatPage, Correction, CorrectionSource, Lesson, NewCorrection, Trial, TrialJudge,
    chat_page, next_lesson_id, noticed, rules_text, sounds_like_correction, trial_of, trial_window,
};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{ConversationStore, CorrectionStore, DbError, SettingKey, SettingStore};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

/// The standing rules: `[{ id, text, at }]`.
const LESSONS: SettingKey<Vec<StoredLesson>> = SettingKey::new("lessons");

/// A standing rule as stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredLesson {
    id: i64,
    text: String,
    at: String,
}

/// A correction to log, as the assistant gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogCorrection {
    /// What the owner said.
    pub said: String,
    /// What went wrong.
    pub wrong: String,
    /// The rule that would have avoided it.
    pub lesson: Option<String>,
    /// The same lesson as one short line to the owner, in the second person, for the app.
    pub lesson_to_owner: Option<String>,
    /// When it happened (an ISO instant); none or unreadable: now.
    pub at: Option<String>,
}

/// Keeps what the assistant learns.
#[derive(Clone, Debug)]
pub struct LearningService {
    corrections: CorrectionStore,
    settings: SettingStore,
    conversation: ConversationStore,
    clock: Arc<dyn Clock>,
}

impl LearningService {
    /// A service over these stores.
    #[must_use]
    pub fn new(
        corrections: CorrectionStore,
        settings: SettingStore,
        conversation: ConversationStore,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            corrections,
            settings,
            conversation,
            clock,
        }
    }

    /// Logs a correction the assistant gives; its id (its lesson's trial rule is `T<id>`).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn log(&self, correction: LogCorrection) -> Result<i64, DbError> {
        let at = correction
            .at
            .as_deref()
            .and_then(|at| at.trim().parse::<Timestamp>().ok())
            .unwrap_or_else(|| self.clock.now());
        self.corrections
            .log(NewCorrection {
                said: correction.said,
                wrong: correction.wrong,
                lesson: correction.lesson,
                lesson_to_owner: correction.lesson_to_owner,
                source: CorrectionSource::Assistant,
                at,
            })
            .await
    }

    /// Logs the owner's message `said` as a candidate correction when it sounds like one,
    /// with the assistant's answer before it; its id, when it was logged.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn notice(
        &self,
        said: &str,
        last_answer: Option<&str>,
    ) -> Result<Option<i64>, DbError> {
        if !sounds_like_correction(said) {
            return Ok(None);
        }
        let id = self
            .corrections
            .log(noticed(said, last_answer, self.clock.now()))
            .await?;
        Ok(Some(id))
    }

    /// The corrections of the last `days` days, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn corrections(&self, days: i64) -> Result<Vec<Correction>, DbError> {
        self.corrections
            .since(self.clock.now() - SignedDuration::from_hours(24 * days))
            .await
    }

    /// The standing rules, by number.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn lessons(&self) -> Result<Vec<Lesson>, DbError> {
        Ok(self
            .stored_lessons()
            .await?
            .into_iter()
            .map(|lesson| Lesson {
                id: lesson.id,
                at: lesson.at.parse().unwrap_or(Timestamp::UNIX_EPOCH),
                text: lesson.text,
            })
            .collect())
    }

    async fn stored_lessons(&self) -> Result<Vec<StoredLesson>, DbError> {
        Ok(self.settings.get(&LESSONS).await?.unwrap_or_default())
    }

    /// Keeps a standing rule: a new one (no `id`), or rule `id` reworded. With `trial`, it is
    /// that trial rule kept: its trial ends as kept, as this rule. Its number.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_lesson(
        &self,
        id: Option<i64>,
        text: &str,
        trial: Option<i64>,
    ) -> Result<i64, DbError> {
        self.keep_rule(id, text, trial.map(|trial| (trial, TrialJudge::Review)))
            .await
    }

    /// Keeps trial rule `id` now, as `by` decided, before the weekly review: its lesson becomes
    /// a new standing rule, as worded. The standing rule's number; none when no such trial runs
    /// (it was kept, dropped, or its days are over).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn keep_trial(&self, id: i64, by: TrialJudge) -> Result<Option<i64>, DbError> {
        let Some(trial) = self
            .trials()
            .await?
            .into_iter()
            .find(|trial| trial.id == id)
        else {
            return Ok(None);
        };
        self.keep_rule(None, &trial.text, Some((id, by)))
            .await
            .map(Some)
    }

    /// Keeps standing rule `id` (none: a new one) as `text`; with `trial`, that trial ends as
    /// kept, as this rule, by whoever kept it. Its number.
    async fn keep_rule(
        &self,
        id: Option<i64>,
        text: &str,
        trial: Option<(i64, TrialJudge)>,
    ) -> Result<i64, DbError> {
        let now = self.clock.now();
        let mut stored = self.stored_lessons().await?;
        let id = id.unwrap_or_else(|| next_lesson_id(stored.iter().map(|lesson| lesson.id)));
        stored.retain(|lesson| lesson.id != id);
        stored.push(StoredLesson {
            id,
            text: text.trim().to_owned(),
            at: iso(now),
        });
        stored.sort_by_key(|lesson| lesson.id);
        self.settings.set(&LESSONS, &stored).await?;
        if let Some((trial, by)) = trial {
            self.corrections.keep_trial(trial, id, by, now).await?;
        }
        Ok(id)
    }

    /// Drops standing rule `id` (nothing when there is none).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn drop_lesson(&self, id: i64) -> Result<(), DbError> {
        let mut stored = self.stored_lessons().await?;
        stored.retain(|lesson| lesson.id != id);
        self.settings.set(&LESSONS, &stored).await
    }

    /// The rules on trial now, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn trials(&self) -> Result<Vec<Trial>, DbError> {
        Ok(self
            .corrections
            .open_trials(trial_window(self.clock.now()))
            .await?
            .iter()
            .filter_map(trial_of)
            .collect())
    }

    /// Takes trial rule `id` out of the prompt, as `by` decided; `false` when no such trial
    /// runs (it was kept, dropped, or its days are over).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn drop_trial(&self, id: i64, by: TrialJudge) -> Result<bool, DbError> {
        let now = self.clock.now();
        self.corrections
            .drop_trial(id, by, now, trial_window(now))
            .await
    }

    /// The standing and trial rules as the assistant's prompt carries them (empty: none).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn rules_text(&self) -> Result<String, DbError> {
        Ok(rules_text(&self.lessons().await?, &self.trials().await?))
    }

    /// A page of the chat with the assistant over the last `days` days, every room, oldest
    /// first, after the instant `after` (the last page's `next`).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn chat_log(&self, days: i64, after: Option<&str>) -> Result<ChatPage, DbError> {
        let since = self.clock.now() - SignedDuration::from_hours(24 * days);
        let lines: Vec<ChatLine> = self
            .conversation
            .said_since(since)
            .await?
            .into_iter()
            .map(|said| ChatLine {
                at: said.at,
                mine: said.mine,
                room: (said.room != RoomId::MAIN).then_some(said.room_name),
                text: said.text,
            })
            .collect();
        Ok(chat_page(&lines, after))
    }
}
