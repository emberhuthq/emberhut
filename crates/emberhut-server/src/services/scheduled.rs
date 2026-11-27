//! The Scheduled page: everything with a next run, every room's, soonest first, each with how
//! often it runs and the tasks it keeps an eye on.
//!
//! The items come from sources: the timers on this machine (systemd's user timers, read here),
//! and the areas that schedule things themselves (the assistant's watches, check-ins and
//! routines, a calendar's meeting prep) plug in their own [`ScheduleSource`].

use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use emberhut_api::ids::{RoomId, TaskId};
use emberhut_api::scheduled::{ScheduledItem, ScheduledTask};
use emberhut_core::schedule::repeat;
use emberhut_core::task::wire::KEY_PREFIX;
use emberhut_db::{DbError, TaskStore};
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::Value;

use crate::convert::timestamp_to_api;

/// Something that schedules things: it lists them with their next run.
pub trait ScheduleSource: fmt::Debug + Send + Sync {
    /// What it has scheduled now. A source that can't be read has nothing this time.
    fn items(&self) -> BoxFuture<'_, Vec<ScheduledItem>>;
}

/// Lists everything scheduled.
#[derive(Clone, Debug)]
pub struct ScheduledService {
    sources: Vec<Arc<dyn ScheduleSource>>,
    tasks: TaskStore,
}

impl ScheduledService {
    /// A page made of `sources`' items, their tasks looked up in `tasks`.
    #[must_use]
    pub fn new(sources: Vec<Arc<dyn ScheduleSource>>, tasks: TaskStore) -> Self {
        Self { sources, tasks }
    }

    /// Every item, soonest first (those with no next run last), each naming the open tasks it
    /// keeps an eye on. An item's room is its source's (Main for this machine's timers): the
    /// room of the first task's project waits for rooms that carry projects (ADR 0015).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn list(&self) -> Result<Vec<ScheduledItem>, DbError> {
        let mut items = Vec::new();
        for source in &self.sources {
            items.extend(source.items().await);
        }
        let open = self.tasks.open().await?;
        for item in &mut items {
            item.tasks = item
                .tasks
                .iter()
                .filter_map(|wanted| {
                    open.iter()
                        .find(|t| t.id.get() == wanted.id.get())
                        .map(|t| ScheduledTask {
                            id: TaskId::new(t.id.get()),
                            title: t.title.clone(),
                            status: t.status.as_str().into(),
                        })
                })
                .collect();
        }
        items.sort_by(|a, b| {
            let at = |i: &ScheduledItem| {
                i.next_at
                    .as_ref()
                    .map_or_else(|| "9999".to_owned(), |t| t.as_str().to_owned())
            };
            at(a).cmp(&at(b))
        });
        Ok(items)
    }
}

/// The tasks a timer's name or description carries: a task's key, in any case, its dash
/// optional.
#[must_use]
pub fn tasks_in(text: &str) -> Vec<TaskId> {
    static KEYS: LazyLock<Option<Regex>> = LazyLock::new(|| {
        let prefix = KEY_PREFIX.trim_end_matches('-');
        Regex::new(&format!(r"(?i)\b{prefix}-?(\d+)\b")).ok()
    });
    let Some(keys) = KEYS.as_ref() else {
        return Vec::new();
    };
    let mut ids: Vec<TaskId> = keys
        .captures_iter(text)
        .filter_map(|caps| caps[1].parse::<i64>().ok())
        .map(TaskId::new)
        .collect();
    ids.dedup();
    ids
}

/// This machine's own timers: systemd's user timers, with when each runs next and its schedule
/// in words (system ones and desktop ones left out).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemdTimers;

impl SystemdTimers {
    async fn run(args: &[&str]) -> Option<String> {
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new("systemctl")
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn read() -> Vec<ScheduledItem> {
        let Some(listed) = Self::run(&[
            "--user",
            "list-timers",
            "--all",
            "--output=json",
            "--no-pager",
        ])
        .await
        else {
            return Vec::new();
        };
        let timers = parse_timers(&listed);
        if timers.is_empty() {
            return Vec::new();
        }
        let shown = Self::run(&show_args(&timers)).await.unwrap_or_default();
        items_of(&timers, &shown)
    }
}

impl ScheduleSource for SystemdTimers {
    fn items(&self) -> BoxFuture<'_, Vec<ScheduledItem>> {
        Box::pin(Self::read())
    }
}

/// `systemctl show`'s arguments for the timers and the units they start: the options first,
/// then `--`, so no unit's name is read as an option.
fn show_args(timers: &[Timer]) -> Vec<&str> {
    let mut args = vec![
        "--user",
        "show",
        "-p",
        "Id,Description,TimersCalendar,TimersMonotonic",
        "--",
    ];
    args.extend(timers.iter().map(|t| t.unit.as_str()));
    args.extend(
        timers
            .iter()
            .map(|t| t.activates.as_str())
            .filter(|a| !a.is_empty()),
    );
    args
}

/// A timer as `list-timers` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Timer {
    unit: String,
    activates: String,
    next: Option<i64>,
}

fn parse_timers(listed: &str) -> Vec<Timer> {
    let skip = ["snap.", "launchpadlib", "systemd-", "org.", "gnome-"];
    serde_json::from_str::<Vec<Value>>(listed)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| {
            let unit = row.get("unit")?.as_str()?.to_owned();
            Some(Timer {
                activates: row
                    .get("activates")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                next: row.get("next").and_then(Value::as_i64).filter(|n| *n > 0),
                unit,
            })
        })
        .filter(|t| !t.unit.is_empty() && !skip.iter().any(|s| t.unit.starts_with(s)))
        .collect()
}

/// `show`'s blocks, by unit: the first value of each property (a timer can list one twice).
fn properties(shown: &str) -> Vec<(String, Vec<(String, String)>)> {
    shown
        .split("\n\n")
        .filter_map(|block| {
            let mut props: Vec<(String, String)> = Vec::new();
            for line in block.lines() {
                if let Some((key, value)) = line.split_once('=')
                    && !props.iter().any(|(k, _)| k == key)
                {
                    props.push((key.to_owned(), value.to_owned()));
                }
            }
            let id = props.iter().find(|(k, _)| k == "Id")?.1.clone();
            Some((id, props))
        })
        .collect()
}

fn items_of(timers: &[Timer], shown: &str) -> Vec<ScheduledItem> {
    let props = properties(shown);
    let get = |unit: &str, key: &str| -> Option<String> {
        props
            .iter()
            .find(|(id, _)| id == unit)
            .and_then(|(_, p)| p.iter().find(|(k, _)| k == key))
            .map(|(_, v)| v.clone())
    };
    timers
        .iter()
        .map(|t| {
            let description = get(&t.activates, "Description")
                .or_else(|| get(&t.unit, "Description"))
                .unwrap_or_default();
            let name = t.unit.strip_suffix(".timer").unwrap_or(&t.unit).to_owned();
            let next = t
                .next
                .and_then(|micros| jiff::Timestamp::from_microsecond(micros).ok())
                .map(timestamp_to_api);
            ScheduledItem {
                id: format!("unit:{name}"),
                kind: "unit".into(),
                title: if description.is_empty() {
                    name.clone()
                } else {
                    description.clone()
                },
                next_at: next,
                repeat: repeat(
                    get(&t.unit, "TimersCalendar").as_deref(),
                    get(&t.unit, "TimersMonotonic").as_deref(),
                ),
                room: RoomId::MAIN,
                note: Some(name.clone()),
                note_at: None,
                tasks: tasks_in(&format!("{name} {description}"))
                    .into_iter()
                    .map(|id| ScheduledTask {
                        id,
                        title: String::new(),
                        status: "".into(),
                    })
                    .collect(),
                watch: None,
                checkin: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_are_read_with_their_schedule_and_tasks() {
        let listed = r#"[
            {"unit":"t7-backup.timer","activates":"t7-backup.service","next":1795643873000000},
            {"unit":"systemd-tmpfiles-clean.timer","activates":"x.service","next":1},
            {"unit":"quiet.timer","activates":"","next":0}
        ]"#;
        let timers = parse_timers(listed);
        assert_eq!(timers.len(), 2);
        let shown = "Id=t7-backup.timer\nTimersCalendar={ OnCalendar=*-*-* 08:00:00 ; next_elapse=x }\n\n\
                     Id=t7-backup.service\nDescription=Back up the notes\n\n\
                     Id=quiet.timer\nTimersMonotonic={ OnUnitActiveUSec=5min ; next_elapse=x }\nTimersMonotonic={ OnBootUSec=1min }";
        let items = items_of(&timers, shown);
        assert_eq!(items[0].id, "unit:t7-backup");
        assert_eq!(items[0].title, "Back up the notes");
        assert_eq!(items[0].repeat.as_deref(), Some("daily 08:00"));
        assert_eq!(items[0].tasks[0].id, TaskId::new(7));
        assert_eq!(
            items[0].next_at.as_ref().unwrap().as_str(),
            "2026-11-25T21:57:53.000Z"
        );
        assert_eq!(items[1].repeat.as_deref(), Some("every 5 min"));
        assert!(items[1].next_at.is_none());
        assert_eq!(
            show_args(&timers),
            [
                "--user",
                "show",
                "-p",
                "Id,Description,TimersCalendar,TimersMonotonic",
                "--",
                "t7-backup.timer",
                "quiet.timer",
                "t7-backup.service"
            ],
            "the unit names come after --"
        );
        let prefix = KEY_PREFIX;
        let lower = prefix.trim_end_matches('-').to_lowercase();
        assert_eq!(
            tasks_in(&format!("{prefix}7 and {lower}8 not {lower}")),
            [TaskId::new(7), TaskId::new(8)]
        );
    }
}
