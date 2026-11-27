//! The owner's calendar and reminders, from every account on their Mac: the app's Day, Week and
//! Month (`GET /api/calendar`), and the assistant's `calendar`, `reminders`, `add_event` and
//! `add_reminder` tools.
//!
//! [`CalendarProvider`] is the seam: [`HttpMacCalendar`] asks the Mac helper (`GET
//! /calendar?days=&from=&to=` answers a list of events, `GET /reminders` the open reminders,
//! `POST /calendar/add` and `POST /reminders/add` add one), with the helper's token as its
//! bearer; [`NoCalendar`] stands in when no helper is set up, and tests swap in a fake.
//!
//! The app's view asks the Mac once a minute per range at most. Each day the Mac covered is
//! kept (`calendar_days`), so the days behind and a Mac that sleeps still show what was last
//! seen. A helper that only reads from today on (it ignores `from`) is asked only for ranges
//! that reach today; once one answers with an earlier day, it is known to read `from` too
//! (`calendarFrom`). The tools' reads keep the last answer (a setting per read), which they
//! give, with its time, when the Mac doesn't answer.
//!
//! What the person has to work on (deadlines, the days asks must be decided by, today's plan)
//! comes from the tasks, behind [`CalendarItems`]: the server's own by default; [`NoItems`]
//! lists nothing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_api::calendar::{CalendarEvent, CalendarItem, CalendarPage};
use emberhut_api::time::Day;
use emberhut_core::Clock;
use emberhut_core::calendar::{
    self as rules, RangeError, block_key, day_of, day_text, days_between, event_id, event_kind,
    instant, last_day, next_day,
};
use emberhut_db::{CalendarStore, DayEvent, DbError, SettingKey, SettingStore};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::convert::timestamp_to_api;
use crate::http::read_body;
use crate::services::activity::ActivityLog;

/// How long the app's view waits for the Mac.
const VIEW_WAIT: Duration = Duration::from_secs(15);
/// The longest answer read from the Mac helper's calendar (a few months of events).
const ANSWER_LIMIT: usize = 8 << 20;
/// How long a tool waits for the Mac.
const TOOL_WAIT: Duration = Duration::from_secs(20);
/// How long one read of the Mac for a range is reused.
const REUSE: jiff::SignedDuration = jiff::SignedDuration::from_mins(1);
/// The blocks the assistant put on the calendar that are remembered (the newest).
const BLOCKS_KEPT: usize = 300;

/// The blocks the assistant put on the calendar (`add_event`), shown as focus time.
const BLOCKS: SettingKey<Vec<Block>> = SettingKey::new("calendarBlocks");
/// The helper reads `from` (it answered with a day before today once).
const READS_FROM: SettingKey<bool> = SettingKey::new("calendarFrom");
/// The tools' last answer to the reminders read.
const REMINDERS: SettingKey<Cached> = SettingKey::new("reminders");

/// A block the assistant put on the calendar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Block {
    title: String,
    start: String,
}

/// A read kept for when the Mac doesn't answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Cached {
    at: String,
    v: Value,
}

/// Why the calendar's provider did not answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CalendarError {
    /// No provider is set up on this server.
    #[error("no Mac helper is set up on this server")]
    NotSetUp,
    /// The provider didn't answer.
    #[error("{0}")]
    Unreachable(String),
    /// The provider answered with a refusal: its status and the start of what it said.
    #[error("the Mac answered {status}: {text}")]
    Refused {
        /// The answer's status.
        status: u16,
        /// The first 200 characters of its body.
        text: String,
    },
    /// The provider's answer was not what was asked for.
    #[error("{0}")]
    Unreadable(String),
}

/// Where the calendar and the reminders come from.
pub trait CalendarProvider: fmt::Debug + Send + Sync {
    /// The events of `from..=to` (a helper that ignores those reads `days` days from today):
    /// the answer as it came (a list of events).
    fn events(
        &self,
        days: i64,
        from: String,
        to: String,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Value, CalendarError>>;

    /// The open reminders, as they came.
    fn reminders(&self) -> BoxFuture<'_, Result<Value, CalendarError>>;

    /// Adds an event (`title`, `start`, `end`, and `calendar`, `location`, `notes` when given):
    /// the answer, whose `ok` says what happened.
    fn add_event(&self, event: Map<String, Value>) -> BoxFuture<'_, Result<Value, CalendarError>>;

    /// Adds a reminder (`title`, and `due`, `list`, `notes` when given): the answer, as for an
    /// event.
    fn add_reminder(
        &self,
        reminder: Map<String, Value>,
    ) -> BoxFuture<'_, Result<Value, CalendarError>>;
}

/// No helper set up: every read and every addition says so.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCalendar;

impl CalendarProvider for NoCalendar {
    fn events(
        &self,
        _days: i64,
        _from: String,
        _to: String,
        _wait: Duration,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async { Err(CalendarError::NotSetUp) })
    }

    fn reminders(&self) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async { Err(CalendarError::NotSetUp) })
    }

    fn add_event(&self, _event: Map<String, Value>) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async { Err(CalendarError::NotSetUp) })
    }

    fn add_reminder(
        &self,
        _reminder: Map<String, Value>,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async { Err(CalendarError::NotSetUp) })
    }
}

/// The calendar through the Mac helper, over HTTP.
pub struct HttpMacCalendar {
    client: reqwest::Client,
    url: reqwest::Url,
    token: String,
}

impl fmt::Debug for HttpMacCalendar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpMacCalendar")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpMacCalendar {
    /// The helper at `url`, sent `token` as its bearer, reached with `client`.
    #[must_use]
    pub fn new(client: reqwest::Client, url: reqwest::Url, token: String) -> Self {
        Self { client, url, token }
    }

    /// `GET`s (no body) or `POST`s `body` to `path`, and reads the JSON answer.
    async fn ask(
        &self,
        path: &str,
        body: Option<Value>,
        wait: Duration,
    ) -> Result<Value, CalendarError> {
        let url = format!("{}{path}", self.url.as_str().trim_end_matches('/'));
        let request = match body {
            None => self.client.get(url),
            Some(body) => self
                .client
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string()),
        };
        let unreachable = |error: reqwest::Error| {
            CalendarError::Unreachable(if error.is_timeout() {
                "the Mac took too long to answer".to_owned()
            } else {
                "the Mac could not be reached".to_owned()
            })
        };
        let response = request
            .bearer_auth(&self.token)
            .timeout(wait)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        let body = read_body(response, ANSWER_LIMIT)
            .await
            .map_err(unreachable)?;
        if body.cut {
            return Err(CalendarError::Unreadable(
                "the Mac's answer is too long".to_owned(),
            ));
        }
        let text = body.text();
        if !status.is_success() {
            return Err(CalendarError::Refused {
                status: status.as_u16(),
                text: text.chars().take(200).collect(),
            });
        }
        serde_json::from_str(&text)
            .map_err(|_| CalendarError::Unreadable("the Mac's answer isn't JSON".to_owned()))
    }
}

impl CalendarProvider for HttpMacCalendar {
    fn events(
        &self,
        days: i64,
        from: String,
        to: String,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async move {
            self.ask(
                &format!("/calendar?days={days}&from={from}&to={to}"),
                None,
                wait,
            )
            .await
        })
    }

    fn reminders(&self) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async move { self.ask("/reminders", None, TOOL_WAIT).await })
    }

    fn add_event(&self, event: Map<String, Value>) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async move {
            self.ask("/calendar/add", Some(Value::Object(event)), TOOL_WAIT)
                .await
        })
    }

    fn add_reminder(
        &self,
        reminder: Map<String, Value>,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        Box::pin(async move {
            self.ask("/reminders/add", Some(Value::Object(reminder)), TOOL_WAIT)
                .await
        })
    }
}

/// Why the things to work on could not be read.
#[derive(Debug, thiserror::Error)]
#[error("the tasks could not be read")]
pub struct ItemsError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

/// What the person has to work on in a range of days, each on its day: tasks' deadlines (a
/// project's once), the days asks have to be decided by, and today's plan.
pub trait CalendarItems: fmt::Debug + Send + Sync {
    /// The items of `from..=to`; `today` is the server's today.
    fn items(
        &self,
        from: Date,
        to: Date,
        today: Date,
    ) -> BoxFuture<'_, Result<Vec<CalendarItem>, ItemsError>>;
}

/// No tasks to read: nothing to work on.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoItems;

impl CalendarItems for NoItems {
    fn items(
        &self,
        _from: Date,
        _to: Date,
        _today: Date,
    ) -> BoxFuture<'_, Result<Vec<CalendarItem>, ItemsError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Why the calendar's view could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CalendarViewError {
    /// The range is not one the calendar takes (400, with its words).
    #[error(transparent)]
    Range(#[from] RangeError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The tasks could not be read.
    #[error(transparent)]
    Items(#[from] ItemsError),
}

/// One read of the Mac for a range: when, what it gave, and the note for the app.
#[derive(Clone, Debug)]
struct Look {
    at: Timestamp,
    events: Option<Vec<Value>>,
    note: Option<String>,
}

/// The calendar: the app's view and the assistant's tools.
#[derive(Clone, Debug)]
pub struct CalendarService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    provider: Arc<dyn CalendarProvider>,
    items: Arc<dyn CalendarItems>,
    days: CalendarStore,
    settings: SettingStore,
    activity: ActivityLog,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
    recent: Mutex<HashMap<String, Look>>,
}

/// What the calendar service is built from.
#[derive(Debug)]
pub struct CalendarParts {
    /// Where the events and reminders come from.
    pub provider: Arc<dyn CalendarProvider>,
    /// What the person has to work on.
    pub items: Arc<dyn CalendarItems>,
    /// The days seen.
    pub days: CalendarStore,
    /// The settings (the blocks, the tools' last reads).
    pub settings: SettingStore,
    /// The activity log.
    pub activity: ActivityLog,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The server's time zone: whose days these are.
    pub zone: TimeZone,
}

impl CalendarService {
    /// The service from its parts.
    #[must_use]
    pub fn new(parts: CalendarParts) -> Self {
        let CalendarParts {
            provider,
            items,
            days,
            settings,
            activity,
            clock,
            zone,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                provider,
                items,
                days,
                settings,
                activity,
                clock,
                zone,
                recent: Mutex::default(),
            }),
        }
    }

    /// Today, in the server's zone.
    fn today(&self) -> Date {
        self.inner.zone.to_datetime(self.inner.clock.now()).date()
    }

    /// The app's view of `from..=to`: the events, what there is to work on, and a note when
    /// the Mac didn't answer.
    ///
    /// # Errors
    ///
    /// [`CalendarViewError::Range`] for a range the calendar doesn't take, and the database's
    /// or the tasks' failure.
    pub async fn view(&self, from: &str, to: &str) -> Result<CalendarPage, CalendarViewError> {
        let (from, to) = rules::range(from, to)?;
        let (events, note) = self.events(from, to).await?;
        let items = self.inner.items.items(from, to, self.today()).await?;
        let events: Vec<CalendarEvent> = events.iter().filter_map(event_of).collect();
        Ok(CalendarPage {
            from: Day::new(day_text(from)),
            to: Day::new(day_text(to)),
            events,
            items,
            note,
            at: timestamp_to_api(self.inner.clock.now()),
        })
    }

    /// The events that start after `from` and before `to` (today's and tomorrow's, as the
    /// Mac says now or as last seen), all-day ones left out, each with its `kind`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn starting(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Map<String, Value>>, DbError> {
        let today = self.today();
        let (events, _note) = self.events(today, next_day(today)).await?;
        Ok(events
            .into_iter()
            .filter(|event| !truthy(event.get("allDay")))
            .filter(|event| {
                instant(&text_of(event.get("start")), &self.inner.zone)
                    .is_some_and(|start| start > from && start < to)
            })
            .collect())
    }

    /// The Mac's events for `from..=to`, read at most once a minute per range.
    async fn read_mac(&self, from: Date, to: Date) -> Result<Look, DbError> {
        let now = self.inner.clock.now();
        let key = format!("{from}|{to}");
        if let Some(look) = self
            .inner
            .recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .filter(|look| now.duration_since(look.at) < REUSE)
        {
            return Ok(look.clone());
        }
        let today = self.today();
        let days = (days_between(today, to) + 1).clamp(1, rules::MAX_DAYS);
        let reads_from = self.inner.settings.get_or(&READS_FROM, false).await?;
        let look = if to < today && !reads_from {
            // Before today, from a helper that reads from today on only: what was last seen.
            Look {
                at: now,
                events: None,
                note: None,
            }
        } else {
            match self
                .inner
                .provider
                .events(days, day_text(from), day_text(to), VIEW_WAIT)
                .await
            {
                Ok(Value::Array(events)) => Look {
                    at: now,
                    events: Some(events),
                    note: None,
                },
                Ok(_) => didnt_answer(now, "the Mac's answer isn't a list of events"),
                Err(error) => didnt_answer(now, &error.to_string()),
            }
        };
        self.inner
            .recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, look.clone());
        Ok(look)
    }

    /// The events of `from..=to`: what the Mac says now for the days it covered, what was last
    /// seen for the others; and the note when the Mac didn't answer.
    async fn events(
        &self,
        from: Date,
        to: Date,
    ) -> Result<(Vec<Map<String, Value>>, Option<String>), DbError> {
        let zone = &self.inner.zone;
        let look = self.read_mac(from, to).await?;
        if let Some(events) = look.events {
            self.keep(events, from, to).await?;
        }
        let blocks: HashSet<String> = self
            .inner
            .settings
            .get_or(&BLOCKS, Vec::new())
            .await?
            .into_iter()
            .map(|block| format!("{}|{}", block.title, block.start))
            .collect();
        let mut seen: Vec<Map<String, Value>> = Vec::new();
        for stored in self
            .inner
            .days
            .between(&day_text(from), &day_text(to))
            .await?
        {
            let Ok(Value::Object(mut event)) = serde_json::from_str::<Value>(&stored) else {
                continue;
            };
            let kind = event_kind(
                &field(&event, "title"),
                instant(&text_of(event.get("start")), zone),
                number(event.get("people")),
                |key| blocks.contains(key),
            );
            event.insert("kind".to_owned(), Value::from(kind.as_str()));
            let id = field(&event, "id");
            match seen.iter_mut().find(|kept| field(kept, "id") == id) {
                Some(kept) => *kept = event,
                None => seen.push(event),
            }
        }
        seen.sort_by_key(|event| text_of(event.get("start")));
        Ok((seen, look.note))
    }

    /// Keeps what the Mac said for `from..=to`: each day it covered loses what it had and gets
    /// the events on it (an event over several days on each of them).
    async fn keep(&self, events: Vec<Value>, from: Date, to: Date) -> Result<(), DbError> {
        let zone = &self.inner.zone;
        let today = self.today();
        let fresh: Vec<(Date, Map<String, Value>)> = events
            .into_iter()
            .filter_map(|event| {
                let Value::Object(mut event) = event else {
                    return None;
                };
                if !truthy(event.get("start")) || !truthy(event.get("end")) {
                    return None;
                }
                let day = day_of(&text_of(event.get("start")), zone)?;
                let id = event_id(
                    &field(&event, "calendar"),
                    &field(&event, "title"),
                    &field(&event, "start"),
                    &field(&event, "end"),
                );
                event.insert("id".to_owned(), Value::String(id));
                event.insert("day".to_owned(), Value::String(day_text(day)));
                Some((day, event))
            })
            .collect();
        // A day before today came back: this helper reads `from`, so it covered every day.
        let past = fresh.iter().any(|(day, _)| *day < today);
        if past {
            self.inner.settings.set(&READS_FROM, &true).await?;
        }
        let reads_from = past || self.inner.settings.get_or(&READS_FROM, false).await?;
        let first = if reads_from { from } else { today };
        let covered = |day: Date| day >= first && day <= to;
        let mut days = Vec::new();
        let mut day = from;
        while day <= to {
            if covered(day) {
                days.push(day_text(day));
            }
            if day == to {
                break;
            }
            day = next_day(day);
        }
        let mut rows = Vec::new();
        for (start, event) in &fresh {
            let all_day = truthy(event.get("allDay"));
            let Some(last) = last_day(&text_of(event.get("end")), all_day, zone) else {
                continue;
            };
            let json = Value::Object(event.clone()).to_string();
            let id = field(event, "id");
            let mut day = (*start).max(from);
            while day <= last && day <= to {
                if covered(day) {
                    rows.push(DayEvent {
                        day: day_text(day),
                        id: id.clone(),
                        event: json.clone(),
                    });
                }
                if day == to {
                    break;
                }
                day = next_day(day);
            }
        }
        self.inner
            .days
            .replace(&days, rows, self.inner.clock.now())
            .await?;
        Ok(())
    }

    /// The `calendar` tool: the events from today on, `days` of them, as the Mac gives them;
    /// the last copy (with its time) when the Mac doesn't answer.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the last copy can't be kept or read.
    pub async fn upcoming(&self, days: u32) -> Result<String, DbError> {
        let key = SettingKey::<Cached>::named(format!("calendar:{days}"));
        let today = self.today();
        let to = today
            .checked_add(jiff::Span::new().days(i64::from(days)))
            .unwrap_or(today);
        let read = self
            .inner
            .provider
            .events(i64::from(days), day_text(today), day_text(to), TOOL_WAIT)
            .await;
        self.cached(&key, read).await
    }

    /// The `reminders` tool: the open reminders, or the last copy, as for the calendar.
    ///
    /// # Errors
    ///
    /// As [`Self::upcoming`].
    pub async fn reminders(&self) -> Result<String, DbError> {
        let read = self.inner.provider.reminders().await;
        self.cached(&REMINDERS, read).await
    }

    /// A read kept as the last copy; when it failed, that copy and when it's from.
    async fn cached(
        &self,
        key: &SettingKey<Cached>,
        read: Result<Value, CalendarError>,
    ) -> Result<String, DbError> {
        match read {
            Ok(value) => {
                let at = timestamp_to_api(self.inner.clock.now());
                let text = value.to_string();
                self.inner
                    .settings
                    .set(
                        key,
                        &Cached {
                            at: at.as_str().to_owned(),
                            v: value,
                        },
                    )
                    .await?;
                Ok(text)
            }
            Err(error) => {
                tracing::debug!(%error, "the calendar's provider didn't answer");
                Ok(match self.inner.settings.get(key).await? {
                    Some(copy) => json!({
                        "note": format!("The Mac is asleep; this is from {}", copy.at),
                        "data": copy.v,
                    })
                    .to_string(),
                    None => "The Mac didn't answer and there is no earlier copy.".to_owned(),
                })
            }
        }
    }

    /// The `add_event` tool: adds `event` (only the owner: no invitees) and remembers it as a
    /// block of focus time; what the Mac said, or why it wasn't added.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the block or the log line can't be written.
    pub async fn add_event(&self, event: Map<String, Value>) -> Result<String, DbError> {
        let title = text_of(event.get("title"));
        let start = text_of(event.get("start"));
        let answer = match self.inner.provider.add_event(event).await {
            Ok(answer) => answer,
            Err(error) => return Ok(format!("Not added: {error}")),
        };
        let Some(at) = instant(&start, &self.inner.zone) else {
            return Ok("Not added: Invalid time value".to_owned());
        };
        let mut blocks = self.inner.settings.get_or(&BLOCKS, Vec::new()).await?;
        let key = block_key(&title, at);
        let (title_part, start_part) = key.split_once('|').unwrap_or((&title, ""));
        blocks.push(Block {
            title: title_part.to_owned(),
            start: start_part.to_owned(),
        });
        let skip = blocks.len().saturating_sub(BLOCKS_KEPT);
        blocks.drain(..skip);
        self.inner.settings.set(&BLOCKS, &blocks).await?;
        self.inner.activity.note(format!("Event: {title}")).await?;
        Ok(text_of(answer.get("ok")))
    }

    /// The `add_reminder` tool: adds `reminder`; what the Mac said, or why it wasn't added.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the log line can't be written.
    pub async fn add_reminder(&self, reminder: Map<String, Value>) -> Result<String, DbError> {
        let title = text_of(reminder.get("title"));
        match self.inner.provider.add_reminder(reminder).await {
            Ok(answer) => {
                self.inner
                    .activity
                    .note(format!("Reminder: {title}"))
                    .await?;
                Ok(text_of(answer.get("ok")))
            }
            Err(error) => Ok(format!("Not added: {error}")),
        }
    }
}

/// A read of the Mac that failed: nothing new, and the note the app shows.
fn didnt_answer(at: Timestamp, why: &str) -> Look {
    Look {
        at,
        events: None,
        note: Some(format!(
            "Your Mac didn't answer ({why}); showing what was last seen."
        )),
    }
}

/// Whether a JSON value counts as true, as JavaScript reads it.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// A JSON value as text: a string as it is, anything else as JSON, nothing as `null`.
pub(crate) fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "null".to_owned(),
    }
}

/// An event's field as its id is made from: missing is `undefined`, as JavaScript writes it.
fn field(event: &Map<String, Value>, key: &str) -> String {
    match event.get(key) {
        None => "undefined".to_owned(),
        value => text_of(value),
    }
}

/// A JSON value as a number, as JavaScript's `Number` reads it (NaN when it isn't one).
pub(crate) fn number(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Null) => 0.0,
        Some(Value::String(s)) if s.trim().is_empty() => 0.0,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// An event as the contract writes it, from the fields the Mac gave it (one without a start, an
/// end or a day is left out; a field it lacks takes its default).
fn event_of(event: &Map<String, Value>) -> Option<CalendarEvent> {
    let text = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(CalendarEvent {
        id: text("id").unwrap_or_default(),
        title: text("title").unwrap_or_default(),
        start: emberhut_api::time::Timestamp::new(text("start")?),
        end: emberhut_api::time::Timestamp::new(text("end")?),
        all_day: event
            .get("allDay")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        calendar: text("calendar").unwrap_or_default(),
        location: text("location").filter(|l| !l.is_empty()),
        notes: text("notes").filter(|n| !n.is_empty()),
        people: event
            .get("people")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        day: Day::new(text("day")?),
        kind: text("kind").unwrap_or_default().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_javascript_reads_them() {
        assert!(truthy(Some(&json!("x"))) && !truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!(0))) && truthy(Some(&json!({}))));
        assert_eq!(text_of(Some(&json!(true))), "true");
        assert_eq!(text_of(Some(&json!("ok"))), "ok");
        assert_eq!(text_of(None), "null");
        assert!((number(Some(&json!("3"))) - 3.0).abs() < f64::EPSILON);
        assert!(number(Some(&json!("x"))).is_nan());
        assert!(number(None).is_nan());
    }
}
