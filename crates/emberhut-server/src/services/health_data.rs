//! The owner's health and screen time, as their phone and Mac send them in: daily Health
//! summaries from the phone (`POST /api/health`, never raw samples) and screen-time totals per
//! day and device from the Mac helper (`POST /api/screentime`), each day kept as it came and
//! replaced when it is sent again. The state shows today's steps and exercise on the phone's
//! calendar day; the assistant's `health` and `screen_time` tools read the days.
//!
//! A real walk closes a walk task: a walking (or hiking, running) workout as long as the task's
//! target, or as many exercise minutes that day; with no walk task open, a done "Walked N min"
//! is logged under the health project. Steps alone never close anything. The tasks are behind
//! [`WalkTasks`]: the server's own by default; [`NoWalkTasks`] has no health project.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use emberhut_api::health_data::Saved;
use emberhut_api::health_today::{HealthToday, WalkTask};
use emberhut_api::ids::{ProjectId, TaskId};
use emberhut_api::time::Timestamp as WireTime;
use emberhut_core::Clock;
use emberhut_core::calendar::{day, day_text};
use emberhut_core::health_data::{
    HealthDay, STEPS_TARGET, WALK_MINUTES, Workout, walk_evidence, walk_target, walk_title, walked,
};
use emberhut_db::{DbError, HealthStore, SettingKey, SettingStore};
use futures_util::future::BoxFuture;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::convert::timestamp_to_api;
use crate::services::activity::ActivityLog;
use crate::services::calendar::{number, text_of};

/// When the phone last sent days.
const HEALTH_AT: SettingKey<WireTime> = SettingKey::new("healthAt");
/// Why the phone last sent no days, until days come again.
const HEALTH_PHONE: SettingKey<NotSent> = SettingKey::new("healthPhone");
/// The phone's time zone.
const HEALTH_TZ: SettingKey<String> = SettingKey::new("healthTz");
/// The daily steps target.
const STEPS: SettingKey<f64> = SettingKey::new("stepsTarget");
/// The days whose walk closed (or logged) a task, and that task.
const WALKS: SettingKey<BTreeMap<String, i64>> = SettingKey::new("healthWalks");
/// When the Mac last sent screen time.
const SCREENTIME_AT: SettingKey<WireTime> = SettingKey::new("screentimeAt");

/// The longest reason the phone gives that is kept.
const WHY_LENGTH: usize = 300;

/// Why the phone sent no days, and when it said so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NotSent {
    why: String,
    at: String,
}

/// The health project, which walk tasks belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthProject {
    /// The project.
    pub id: i64,
    /// The goal it serves.
    pub goal: Option<i64>,
}

/// Why the tasks could not be read or changed.
#[derive(Debug, thiserror::Error)]
#[error("the tasks could not be read or changed")]
pub struct WalkTaskError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

/// The health project's tasks, as the walk rules use them.
pub trait WalkTasks: fmt::Debug + Send + Sync {
    /// The active project named "Health" (any case), if there is one.
    fn project(&self) -> BoxFuture<'_, Result<Option<HealthProject>, WalkTaskError>>;

    /// The project's open tasks (not subtasks), by id: their ids and titles.
    fn open_tasks(
        &self,
        project: HealthProject,
    ) -> BoxFuture<'_, Result<Vec<(i64, String)>, WalkTaskError>>;

    /// Closes `task` with `evidence` as its note, done at `done_at`.
    fn close(
        &self,
        task: i64,
        evidence: String,
        done_at: Timestamp,
    ) -> BoxFuture<'_, Result<(), WalkTaskError>>;

    /// Logs a done task `title` under `project`, with `evidence` as its note, done at
    /// `done_at`: its id.
    fn log(
        &self,
        project: HealthProject,
        title: String,
        evidence: String,
        done_at: Timestamp,
    ) -> BoxFuture<'_, Result<i64, WalkTaskError>>;
}

/// No tasks: there is no health project, so no walk closes anything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWalkTasks;

impl WalkTasks for NoWalkTasks {
    fn project(&self) -> BoxFuture<'_, Result<Option<HealthProject>, WalkTaskError>> {
        Box::pin(async { Ok(None) })
    }

    fn open_tasks(
        &self,
        _project: HealthProject,
    ) -> BoxFuture<'_, Result<Vec<(i64, String)>, WalkTaskError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn close(
        &self,
        _task: i64,
        _evidence: String,
        _done_at: Timestamp,
    ) -> BoxFuture<'_, Result<(), WalkTaskError>> {
        Box::pin(async { Ok(()) })
    }

    fn log(
        &self,
        _project: HealthProject,
        _title: String,
        _evidence: String,
        _done_at: Timestamp,
    ) -> BoxFuture<'_, Result<i64, WalkTaskError>> {
        Box::pin(async { Ok(0) })
    }
}

/// Why health or screen time could not be taken or read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HealthError {
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The tasks failed.
    #[error(transparent)]
    Tasks(#[from] WalkTaskError),
}

/// Health and screen time.
#[derive(Clone, Debug)]
pub struct HealthDataService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: HealthStore,
    settings: SettingStore,
    activity: ActivityLog,
    walks: Arc<dyn WalkTasks>,
    clock: Arc<dyn Clock>,
    /// Finds a time zone by name (the phone's).
    zones: fn(&str) -> Option<TimeZone>,
}

/// What the health service is built from.
#[derive(Debug)]
pub struct HealthParts {
    /// The days.
    pub store: HealthStore,
    /// The settings (the phone's zone, the last syncs).
    pub settings: SettingStore,
    /// The activity log.
    pub activity: ActivityLog,
    /// The walk tasks.
    pub walks: Arc<dyn WalkTasks>,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
}

/// The time zone named `name`, from the system's time zone database; `UTC` always.
fn system_zone(name: &str) -> Option<TimeZone> {
    if name == "UTC" {
        return Some(TimeZone::UTC);
    }
    TimeZone::get(name).ok()
}

/// A summary's `date`, when it is a `YYYY-MM-DD` day.
fn date_of(day_json: &Value) -> Option<String> {
    let date = text_of(day_json.get("date"));
    day(&date).map(|_| date)
}

/// A number field, when it is there (as JavaScript's `Number` reads it).
fn figure(value: Option<&Value>) -> Option<f64> {
    value.map(|v| number(Some(v)))
}

/// The parts of a day's summary the walk rules read.
fn health_day(data: &Value) -> HealthDay {
    let workouts = data
        .get("workouts")
        .and_then(Value::as_array)
        .map_or_default(|workouts| {
            workouts
                .iter()
                .map(|w| Workout {
                    kind: match w.get("type") {
                        Some(kind) => text_of(Some(kind)),
                        None => "undefined".to_owned(),
                    },
                    min: figure(w.get("min")),
                    at: w
                        .get("at")
                        .filter(|at| !at.is_null())
                        .map(|at| text_of(Some(at))),
                })
                .collect()
        });
    HealthDay {
        steps: figure(data.get("steps")),
        exercise_min: figure(data.get("exerciseMin")),
        workouts,
    }
}

impl HealthDataService {
    /// The service from its parts.
    #[must_use]
    pub fn new(parts: HealthParts) -> Self {
        let HealthParts {
            store,
            settings,
            activity,
            walks,
            clock,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                store,
                settings,
                activity,
                walks,
                clock,
                zones: system_zone,
            }),
        }
    }

    /// The phone's time zone (UTC until it says).
    async fn zone(&self) -> Result<TimeZone, DbError> {
        Ok(self
            .inner
            .settings
            .get(&HEALTH_TZ)
            .await?
            .and_then(|name| (self.inner.zones)(&name))
            .unwrap_or(TimeZone::UTC))
    }

    /// The phone's calendar day, `back` days ago.
    async fn phone_day(&self, back: i64) -> Result<Date, DbError> {
        let zone = self.zone().await?;
        let at = self
            .inner
            .clock
            .now()
            .checked_sub(SignedDuration::from_hours(24 * back))
            .unwrap_or(Timestamp::UNIX_EPOCH);
        Ok(zone.to_datetime(at).date())
    }

    /// A day's summary, as stored.
    async fn day(&self, date: &str) -> Result<Option<Value>, DbError> {
        Ok(self
            .inner
            .store
            .health_day(date)
            .await?
            .and_then(|data| serde_json::from_str(&data).ok()))
    }

    /// Takes the phone's upload: each day with a `YYYY-MM-DD` date is kept (in place of what
    /// that day had); `why` (the phone sent no days) is kept until days come; `tz` becomes the
    /// phone's zone. Then a real walk closes its task. How many days were kept.
    ///
    /// # Errors
    ///
    /// [`HealthError`].
    pub async fn take_health(&self, body: &Value) -> Result<Saved, HealthError> {
        let now = self.inner.clock.now();
        let days: Vec<(String, String)> = body
            .get("days")
            .and_then(Value::as_array)
            .map_or_default(|days| {
                days.iter()
                    .filter_map(|d| Some((date_of(d)?, d.to_string())))
                    .collect()
            });
        let saved = u32::try_from(days.len()).unwrap_or(u32::MAX);
        let any = !days.is_empty();
        self.inner.store.put_health(days, now).await?;
        if let Some(Value::String(why)) = body.get("why")
            && !why.trim().is_empty()
        {
            let why: String = why.trim().chars().take(WHY_LENGTH).collect();
            let before = self.inner.settings.get(&HEALTH_PHONE).await?;
            if before.is_none_or(|before| before.why != why) {
                self.inner
                    .activity
                    .note(format!("Health: the phone didn't send its days: {why}"))
                    .await?;
            }
            self.inner
                .settings
                .set(
                    &HEALTH_PHONE,
                    &NotSent {
                        why,
                        at: timestamp_to_api(now).as_str().to_owned(),
                    },
                )
                .await?;
        }
        if any {
            self.inner
                .settings
                .set(&HEALTH_AT, &timestamp_to_api(now))
                .await?;
            self.inner.settings.remove(&HEALTH_PHONE).await?;
        }
        if let Some(Value::String(tz)) = body.get("tz")
            && is_zone_name(tz)
        {
            self.inner.settings.set(&HEALTH_TZ, tz).await?;
        }
        self.walk_tick().await?;
        Ok(Saved { ok: true, saved })
    }

    /// Takes the Mac helper's screen time: each entry with a `YYYY-MM-DD` date and a `device`
    /// is kept (in place of what that day and device had). How many were kept.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn take_screentime(&self, body: &Value) -> Result<Saved, DbError> {
        let now = self.inner.clock.now();
        let days: Vec<(String, String, String)> = body
            .get("days")
            .and_then(Value::as_array)
            .map_or_default(|days| {
                days.iter()
                    .filter_map(|d| {
                        let device = d.get("device")?.as_str()?.to_owned();
                        Some((date_of(d)?, device, d.to_string()))
                    })
                    .collect()
            });
        let saved = u32::try_from(days.len()).unwrap_or(u32::MAX);
        self.inner.store.put_screentime(days, now).await?;
        self.inner
            .settings
            .set(&SCREENTIME_AT, &timestamp_to_api(now))
            .await?;
        Ok(Saved { ok: true, saved })
    }

    /// When the phone last sent days.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn health_at(&self) -> Result<Option<WireTime>, DbError> {
        self.inner.settings.get(&HEALTH_AT).await
    }

    /// When the Mac last sent screen time.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn screentime_at(&self) -> Result<Option<WireTime>, DbError> {
        self.inner.settings.get(&SCREENTIME_AT).await
    }

    /// The health project's walk tasks, and their targets.
    async fn walk_tasks(
        &self,
        project: Option<HealthProject>,
    ) -> Result<Vec<(i64, f64)>, HealthError> {
        let Some(project) = project else {
            return Ok(Vec::new());
        };
        Ok(self
            .inner
            .walks
            .open_tasks(project)
            .await?
            .into_iter()
            .filter_map(|(id, title)| Some((id, walk_target(&title)?)))
            .collect())
    }

    /// Today's health for the state: the phone's day, its numbers, the steps target, and the
    /// walk tasks that get a bar.
    ///
    /// # Errors
    ///
    /// [`HealthError`].
    pub async fn now(&self) -> Result<HealthToday, HealthError> {
        let today = day_text(self.phone_day(0).await?);
        let data = self.day(&today).await?.unwrap_or_else(|| json!({}));
        let walk = walked(&health_day(&data));
        let project = self.inner.walks.project().await?;
        let tasks = self.walk_tasks(project).await?;
        Ok(HealthToday {
            date: today,
            steps: figure(data.get("steps")),
            exercise_min: figure(data.get("exerciseMin")),
            move_min: walk.map(|walk| walk.min),
            steps_target: self.inner.settings.get_or(&STEPS, STEPS_TARGET).await?,
            at: self.health_at().await?,
            project: project.map(|p| ProjectId::from(p.id)),
            tasks: tasks
                .into_iter()
                .map(|(id, target)| WalkTask {
                    id: TaskId::from(id),
                    target,
                })
                .collect(),
        })
    }

    /// After each upload: yesterday's and today's walk (on the phone's days), each once, closes
    /// the first walk task it reaches, or is logged as a done task under the health project.
    async fn walk_tick(&self) -> Result<(), HealthError> {
        let Some(project) = self.inner.walks.project().await? else {
            return Ok(());
        };
        let zone = self.zone().await?;
        let mut done = self.inner.settings.get_or(&WALKS, BTreeMap::new()).await?;
        for back in [1, 0] {
            let date = day_text(self.phone_day(back).await?);
            let stored = self.day(&date).await?.unwrap_or_else(|| json!({}));
            let summary = health_day(&stored);
            let Some(walk) = walked(&summary) else {
                continue;
            };
            if walk.min < WALK_MINUTES || done.get(&date).is_some_and(|task| *task != 0) {
                continue;
            }
            let evidence = walk_evidence(&walk, &summary, &date);
            let done_at = done_at(&date, walk.at.as_deref(), &zone);
            let task = self
                .walk_tasks(Some(project))
                .await?
                .into_iter()
                .find(|(_, target)| *target <= walk.min)
                .map(|(id, _)| id);
            let id = match task {
                Some(id) => {
                    self.inner
                        .walks
                        .close(id, evidence.clone(), done_at)
                        .await?;
                    id
                }
                None => {
                    self.inner
                        .walks
                        .log(project, walk_title(&walk), evidence.clone(), done_at)
                        .await?
                }
            };
            done.insert(date, id);
            self.inner
                .activity
                .note(format!("Health: {evidence} → task {id} done"))
                .await?;
        }
        self.inner.settings.set(&WALKS, &done).await?;
        Ok(())
    }

    /// The `health` tool: the newest `days` days' summaries, newest first, when the phone last
    /// sent days, and why it last sent none.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn health_days(&self, days: u64) -> Result<Option<Value>, DbError> {
        let rows: Vec<Value> = self
            .inner
            .store
            .recent_health(days)
            .await?
            .iter()
            .filter_map(|data| serde_json::from_str(data).ok())
            .collect();
        if rows.is_empty() {
            return Ok(None);
        }
        let last_sync = self.health_at().await?;
        let mut answer = json!({ "lastSync": last_sync, "days": rows });
        if let Some(phone) = self.inner.settings.get(&HEALTH_PHONE).await?
            && let Value::Object(answer) = &mut answer
        {
            answer.insert(
                "notSent".to_owned(),
                json!({ "why": phone.why, "at": phone.at }),
            );
        }
        Ok(Some(answer))
    }

    /// The `screen_time` tool: the last `days` days (UTC), newest first and by device, and
    /// when the Mac last sent them.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn screentime_days(&self, days: i64) -> Result<Option<Value>, DbError> {
        let today = TimeZone::UTC.to_datetime(self.inner.clock.now()).date();
        let from = today
            .checked_sub(jiff::Span::new().days(days))
            .unwrap_or(today);
        let rows: Vec<Value> = self
            .inner
            .store
            .screentime_since(&day_text(from))
            .await?
            .iter()
            .filter_map(|data| serde_json::from_str(data).ok())
            .collect();
        if rows.is_empty() {
            return Ok(None);
        }
        let last_sync = self.screentime_at().await?;
        Ok(Some(json!({ "lastSync": last_sync, "days": rows })))
    }
}

/// Whether `name` looks like a time zone's name (`Area/City`).
fn is_zone_name(name: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '-'))
    };
    let mut parts = name.split('/');
    matches!((parts.next(), parts.next()), (Some(area), Some(city)) if word(area) && word(city.split('/').next().unwrap_or_default()))
}

/// When a day's walk happened: its start (`07:40`) that day, else noon, in `zone`.
fn done_at(date: &str, at: Option<&str>, zone: &TimeZone) -> Timestamp {
    let day = day(date).unwrap_or_default();
    let time = at
        .and_then(|at| format!("{at}:00").parse::<Time>().ok())
        .unwrap_or_else(|| Time::constant(12, 0, 0, 0));
    zone.to_timestamp(DateTime::from_parts(day, time))
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_reads_its_steps_exercise_and_workouts() {
        let day = health_day(&json!({
            "date": "2026-10-12", "steps": 9120, "exerciseMin": "41",
            "workouts": [{ "type": "Walking", "min": 22, "at": "07:40" }, { "min": 5 }]
        }));
        assert_eq!(day.steps, Some(9120.0));
        assert_eq!(day.exercise_min, Some(41.0));
        assert_eq!(day.workouts[0].kind, "Walking");
        assert_eq!(day.workouts[1].kind, "undefined");
    }

    #[test]
    fn zone_names_and_walk_times() {
        assert!(is_zone_name("Europe/Paris"));
        assert!(is_zone_name("America/Argentina/Salta"));
        assert!(!is_zone_name("UTC"));
        assert!(!is_zone_name("/Paris"));
        let utc = TimeZone::UTC;
        assert_eq!(
            done_at("2026-10-12", Some("07:40"), &utc).to_string(),
            "2026-10-12T07:40:00Z"
        );
        assert_eq!(
            done_at("2026-10-12", None, &utc).to_string(),
            "2026-10-12T12:00:00Z"
        );
    }
}
