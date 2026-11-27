//! The assistant's own schedule for the Scheduled page: its watches (with the tasks each keeps
//! an eye on), the check-ins it set (with the task a check-in names) and the routines.

use emberhut_api as api;
use emberhut_api::scheduled::{ScheduledItem, ScheduledTask};
use emberhut_core::RoomId;
use emberhut_core::schedule::{every, next_routine, routine_name};
use emberhut_core::wake::{hour_minute, weekday_of};
use emberhut_db::TaskStore;
use futures_util::future::BoxFuture;

use crate::convert::timestamp_to_api;
use crate::services::routines::RoutineService;
use crate::services::scheduled::{ScheduleSource, tasks_in};
use crate::services::wake::WakeService;

/// The watches, check-ins and routines.
#[derive(Clone, Debug)]
pub struct AssistantSchedule {
    wake: WakeService,
    routines: RoutineService,
    tasks: TaskStore,
}

impl AssistantSchedule {
    /// What `wake` and `routines` have scheduled; a watch's tasks are the open ones in `tasks`
    /// it runs.
    #[must_use]
    pub fn new(wake: WakeService, routines: RoutineService, tasks: TaskStore) -> Self {
        Self {
            wake,
            routines,
            tasks,
        }
    }

    /// An item with nothing but its id, kind, title and room.
    fn item(id: String, kind: &str, title: String, room: api::ids::RoomId) -> ScheduledItem {
        ScheduledItem {
            id,
            kind: kind.into(),
            title,
            next_at: None,
            repeat: None,
            room,
            note: None,
            note_at: None,
            tasks: Vec::new(),
            watch: None,
            checkin: None,
        }
    }

    /// Every item; a part that can't be read is left out this time.
    async fn read(&self) -> Vec<ScheduledItem> {
        let mut items = self.watches().await;
        items.extend(self.checkins().await);
        items.extend(self.routines().await);
        items
    }

    /// The watches still running, each with the open tasks it runs.
    async fn watches(&self) -> Vec<ScheduledItem> {
        let watches = match self.wake.watches().await {
            Ok(watches) => watches,
            Err(error) => {
                tracing::warn!(%error, "the watches can't be read");
                return Vec::new();
            }
        };
        let open = match self.tasks.open().await {
            Ok(open) => open,
            Err(error) => {
                tracing::warn!(%error, "the open tasks can't be read for the Scheduled page");
                Vec::new()
            }
        };
        let now = timestamp_to_api(self.routines.now());
        watches
            .into_iter()
            .filter(|w| w.until.as_str() > now.as_str())
            .map(|watch| {
                let runner = format!("watch:{}", watch.id);
                let mut item = Self::item(
                    runner.clone(),
                    "watch",
                    watch.title.clone().unwrap_or_else(|| watch.what.clone()),
                    watch.room.unwrap_or_else(main),
                );
                item.tasks = open
                    .iter()
                    .filter(|t| t.runner.as_deref() == Some(runner.as_str()))
                    .map(|t| task(t.id.get()))
                    .collect();
                item.next_at = Some(watch.next_at);
                item.repeat = Some(minutes(watch.every));
                item.note = Some(watch.note).filter(|note| !note.is_empty());
                item.note_at = watch.note_at;
                item.watch = Some(emberhut_api::ids::WatchId::new(watch.id));
                item
            })
            .collect()
    }

    /// The check-ins, each with the task its why names.
    async fn checkins(&self) -> Vec<ScheduledItem> {
        let checkins = match self.wake.checkins().await {
            Ok(checkins) => checkins,
            Err(error) => {
                tracing::warn!(%error, "the check-ins can't be read");
                return Vec::new();
            }
        };
        checkins
            .into_iter()
            .map(|checkin| {
                let at = checkin.at.as_str().to_owned();
                let mut item = Self::item(
                    format!("checkin:{at}"),
                    "checkin",
                    checkin.why.clone(),
                    checkin.room.unwrap_or_else(main),
                );
                item.next_at = Some(checkin.at);
                item.tasks = tasks_in(&checkin.why)
                    .into_iter()
                    .take(1)
                    .map(|id| task(id.get()))
                    .collect();
                item.checkin = Some(emberhut_api::time::Timestamp::from(at));
                item
            })
            .collect()
    }

    /// The routines, in Main at their time on the server's clock.
    async fn routines(&self) -> Vec<ScheduledItem> {
        let routines = match self.routines.routines().await {
            Ok(routines) => routines,
            Err(error) => {
                tracing::warn!(%error, "the routines can't be read");
                return Vec::new();
            }
        };
        let zone = self.routines.zone().clone();
        let here = zone.to_datetime(self.routines.now());
        let mut items = Vec::new();
        for routine in routines {
            let Some(at) = hour_minute(&routine.at) else {
                continue;
            };
            let day = routine.day.and_then(weekday_of);
            let next = next_routine(at, day, here);
            let mut item = Self::item(
                format!(
                    "routine:{}:{}",
                    routine.day.map_or_else(String::new, |d| d.to_string()),
                    routine.at
                ),
                "routine",
                routine_name(&routine.what),
                main(),
            );
            item.next_at = zone
                .to_ambiguous_timestamp(next)
                .compatible()
                .ok()
                .map(timestamp_to_api);
            item.repeat = Some(match day {
                None => format!("daily {}", routine.at),
                Some(day) => format!("weekly, {} {}", short_day(day), routine.at),
            });
            item.note = Some(routine.what.chars().take(240).collect());
            items.push(item);
        }
        items
    }
}

/// Main, as the page names rooms.
fn main() -> api::ids::RoomId {
    api::ids::RoomId::new(RoomId::MAIN.get())
}

/// A task the item keeps an eye on, by id (the page fills in the rest).
fn task(id: i64) -> ScheduledTask {
    ScheduledTask {
        id: api::ids::TaskId::new(id),
        title: String::new(),
        status: "".into(),
    }
}

/// `every` minutes in words, rounded to whole minutes.
fn minutes(every_minutes: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a watch's period in minutes: positive and far inside u64's range"
    )]
    let whole = every_minutes.max(0.0).round() as u64;
    every(whole)
}

/// A weekday's short English name.
fn short_day(day: jiff::civil::Weekday) -> &'static str {
    use jiff::civil::Weekday;
    match day {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

impl ScheduleSource for AssistantSchedule {
    fn items(&self) -> BoxFuture<'_, Vec<ScheduledItem>> {
        Box::pin(self.read())
    }
}
