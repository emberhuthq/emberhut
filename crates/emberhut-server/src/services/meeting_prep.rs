//! Meeting prep: about an hour before each timed event on the owner's calendar, the assistant
//! gets a task to prepare the owner for it (who it is with, what's open between them, what to
//! get out of it, questions to ask), which a worker picks up like any assistant task with a
//! brief.
//!
//! Every [`TICK`] the server looks at the events starting between [`FROM`] and [`TO`] from now
//! ([`CalendarService::starting`]); each gets its task once (the `prepped` setting keeps the
//! last [`KEPT`] events prepared, by title and start). Focus time is left out. The
//! `meetingPrep` setting (on unless set to `false`) turns it off.

use std::sync::Arc;
use std::time::Duration;

use emberhut_core::Clock;
use emberhut_core::task::{Owner, Who};
use emberhut_db::{DbError, NewTask, SettingKey, SettingStore};
use jiff::SignedDuration;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::services::calendar::{CalendarService, text_of, truthy};
use crate::services::tasks::{TaskError, TaskService};

/// Whether meetings get a prep task; on unless set to `false`.
pub const MEETING_PREP: SettingKey<bool> = SettingKey::new("meetingPrep");
/// The events prepared, `title@start`, the newest last.
const PREPPED: SettingKey<Vec<String>> = SettingKey::new("prepped");
/// The events prepared that are remembered.
const KEPT: usize = 100;
/// How often the calendar is looked at.
pub const TICK: Duration = Duration::from_mins(10);
/// An event is prepared once it starts sooner than this...
const TO: SignedDuration = SignedDuration::from_mins(80);
/// ...and not later than this, so each one is seen by a look of the window.
const FROM: SignedDuration = SignedDuration::from_mins(35);

/// Why a look failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PrepError {
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The task couldn't be made.
    #[error("the prep task couldn't be made")]
    Task(#[from] TaskError),
}

/// Prepares the owner for their meetings.
#[derive(Clone, Debug)]
pub struct MeetingPrep {
    calendar: CalendarService,
    tasks: TaskService,
    settings: SettingStore,
    clock: Arc<dyn Clock>,
}

impl MeetingPrep {
    /// Prep over the calendar, the tasks and the settings, at the time `clock` says.
    #[must_use]
    pub fn new(
        calendar: CalendarService,
        tasks: TaskService,
        settings: SettingStore,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            calendar,
            tasks,
            settings,
            clock,
        }
    }

    /// One look: a prep task for each event starting in the window that has none yet. How
    /// many were made.
    ///
    /// # Errors
    ///
    /// [`PrepError`].
    pub async fn tick(&self) -> Result<usize, PrepError> {
        if !self.settings.get_or(&MEETING_PREP, true).await? {
            return Ok(0);
        }
        let now = self.clock.now();
        let soon = self
            .calendar
            .starting(
                now.checked_add(FROM).unwrap_or(now),
                now.checked_add(TO).unwrap_or(now),
            )
            .await?;
        let mut made = 0;
        for event in soon {
            if event.get("kind").and_then(Value::as_str) == Some("focus") {
                continue;
            }
            let title = text_of(event.get("title"));
            let start = text_of(event.get("start"));
            let key = format!("{title}@{start}");
            let mut prepped = self.settings.get_or(&PREPPED, Vec::new()).await?;
            if prepped.contains(&key) {
                continue;
            }
            prepped.push(key);
            let excess = prepped.len().saturating_sub(KEPT);
            prepped.drain(..excess);
            self.settings.set(&PREPPED, &prepped).await?;
            let at = start
                .parse::<jiff::Timestamp>()
                .map(|start| {
                    self.tasks
                        .zone()
                        .to_datetime(start)
                        .strftime("%H:%M")
                        .to_string()
                })
                .unwrap_or(start);
            self.tasks
                .create(
                    NewTask {
                        title: format!("Prep: {title} at {at}"),
                        body: Some(brief(&event, &title, &at)),
                        owner: Some(Owner::Assistant),
                        priority: Some(1),
                        ..NewTask::default()
                    },
                    Who::Assistant,
                )
                .await?;
            made += 1;
        }
        Ok(made)
    }

    /// Looks every `every` until `stop`; a failed look is logged and the next one tries again.
    pub async fn keep_ticking(self, every: Duration, stop: CancellationToken) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            if let Err(error) = self.tick().await {
                tracing::warn!(%error, "meeting prep: the calendar wasn't looked at");
            }
        }
    }
}

/// The prep task's brief for `event`, titled `title`, at `at` (the server's time).
fn brief(event: &Map<String, Value>, title: &str, at: &str) -> String {
    let location = if truthy(event.get("location")) {
        format!(", {}", text_of(event.get("location")))
    } else {
        String::new()
    };
    let people = if truthy(event.get("people")) {
        format!(", with {} people", text_of(event.get("people")))
    } else {
        String::new()
    };
    let notes = if truthy(event.get("notes")) {
        format!(" Notes: {}", text_of(event.get("notes")))
    } else {
        String::new()
    };
    format!(
        "The owner has \"{title}\" at {at}{location}{people}.{notes} Prepare them in one \
         screen: who it is with and what they are to the owner (facts), what's open between \
         them (recent messages, tasks, threads, promises), what the owner should get out of \
         it, and 2-3 questions to ask. Finish with task_done: the brief itself is the summary."
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_brief_says_where_and_with_how_many() {
        let Value::Object(event) =
            json!({ "location": "Room 2", "people": 3, "notes": "bring the plan" })
        else {
            return;
        };
        let text = brief(&event, "Review", "10:00");
        assert!(text.starts_with(
            "The owner has \"Review\" at 10:00, Room 2, with 3 people. Notes: bring the plan \
             Prepare them"
        ));
        let text = brief(&Map::new(), "Call", "09:30");
        assert!(text.starts_with("The owner has \"Call\" at 09:30. Prepare them"));
    }
}
