//! A task's state in words, from what is actually happening on it (its live state) rather
//! than its label: the assistant working (a run open and started), queued, waiting on the user,
//! nothing running, scheduled, parked, on hold, done, archived. With it, where and since when it
//! runs, or what ran last.

use emberhut_api::ids::TaskId;
use emberhut_api::task_live::{LiveState, LiveWatch, TaskLive};
use emberhut_api::tasks::{Issue, TaskStatus};
use jiff::ZonedDateTime;

use super::clock;
use crate::brand::APP_NAME;

/// What is happening on a task, as its row and page show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Doing {
    /// A run of its own is open and started.
    Running,
    /// Sent, behind another turn.
    Queued,
    /// It waits on the user.
    Waiting,
    /// Open, nothing running and nothing asked.
    Stalled,
    /// A watch or a check-in looks at it now and then.
    Scheduled,
    /// Put off until a day.
    Parked,
    /// Held by a parent that waits on the user.
    Held,
    /// Done.
    Done,
    /// Archived.
    Archived,
    /// Not started.
    Todo,
}

/// The colour a state is drawn in (the view maps it onto the palette).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// The accent: it needs the user.
    Accent,
    /// Working.
    Warn,
    /// Queued, or a look on now.
    Teal,
    /// Scheduled.
    Info,
    /// Done.
    Success,
    /// Needs a look.
    Danger,
    /// Nothing special.
    Muted,
}

/// What the state icon draws inside its ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Half filled: running.
    Half,
    /// A quarter: queued.
    Quarter,
    /// A soft fill: waiting on the user.
    Soft,
    /// A pause sign: nothing running.
    Pause,
    /// A clock: scheduled.
    Clock,
    /// An eye: a scheduled look on now.
    Eye,
    /// A tick on a full disc: done.
    Tick,
    /// Just the ring.
    Ring,
}

/// What is happening on `task`: its live state, else (an older server) its status, a working
/// one read as nothing running.
#[must_use]
pub(crate) fn doing(task: &Issue) -> Doing {
    match task.live.as_ref().map(|live| &live.state) {
        Some(LiveState::Running) => Doing::Running,
        Some(LiveState::Queued) => Doing::Queued,
        Some(LiveState::Waiting) => Doing::Waiting,
        Some(LiveState::Stalled) => Doing::Stalled,
        Some(LiveState::Scheduled) => Doing::Scheduled,
        Some(LiveState::Parked) => Doing::Parked,
        Some(LiveState::Held) => Doing::Held,
        Some(LiveState::Done) => Doing::Done,
        Some(LiveState::Canceled) => Doing::Archived,
        Some(_) => Doing::Todo,
        None => match task.status {
            TaskStatus::Working => Doing::Stalled,
            TaskStatus::Waiting => Doing::Waiting,
            TaskStatus::Done => Doing::Done,
            TaskStatus::Canceled => Doing::Archived,
            _ => Doing::Todo,
        },
    }
}

fn alarm(task: &Issue) -> bool {
    task.live.as_ref().is_some_and(|live| live.alarm)
}

fn checking(task: &Issue) -> bool {
    task.live
        .as_ref()
        .is_some_and(|live| live.checking == Some(true))
}

/// The state's name: `Emberhut is working`, `Queued`, `Needs you`, `Nothing running`, ...
#[must_use]
pub(crate) fn name(task: &Issue) -> String {
    match doing(task) {
        Doing::Waiting => "Needs you".to_owned(),
        Doing::Running => format!("{APP_NAME} is working"),
        Doing::Queued => "Queued".to_owned(),
        Doing::Stalled => "Nothing running".to_owned(),
        Doing::Scheduled if checking(task) => "Checking now".to_owned(),
        Doing::Scheduled => "Scheduled".to_owned(),
        Doing::Parked => "Parked".to_owned(),
        Doing::Held => "On hold".to_owned(),
        Doing::Done => "Done".to_owned(),
        Doing::Archived => "Archived".to_owned(),
        Doing::Todo if task.live.is_none() => "To do".to_owned(),
        Doing::Todo => "Not started".to_owned(),
    }
}

/// The state's colour.
#[must_use]
pub(crate) fn tone(task: &Issue) -> Tone {
    match doing(task) {
        Doing::Waiting => Tone::Accent,
        Doing::Running => Tone::Warn,
        Doing::Queued => Tone::Teal,
        Doing::Done => Tone::Success,
        Doing::Stalled if alarm(task) => Tone::Danger,
        Doing::Scheduled if checking(task) => Tone::Teal,
        Doing::Scheduled => Tone::Info,
        _ => Tone::Muted,
    }
}

/// What the state icon draws.
#[must_use]
pub(crate) fn mark(task: &Issue) -> Mark {
    match doing(task) {
        Doing::Running => Mark::Half,
        Doing::Queued => Mark::Quarter,
        Doing::Waiting => Mark::Soft,
        Doing::Stalled => Mark::Pause,
        Doing::Scheduled if checking(task) => Mark::Eye,
        Doing::Scheduled => Mark::Clock,
        Doing::Done => Mark::Tick,
        _ => Mark::Ring,
    }
}

/// One line on where and since when it runs, or what ran last: `Emberhut's worker on the Mac ·
/// started 5 minutes ago`, `Until 3 Dec`, `Waits on T-4, which needs you`; none when there
/// is nothing to say. `key_of` names another task.
#[must_use]
pub(crate) fn live_line(
    task: &Issue,
    now: &ZonedDateTime,
    key_of: &dyn Fn(TaskId) -> String,
) -> Option<String> {
    let live = task.live.as_ref()?;
    let ago = |at: Option<&emberhut_api::time::Timestamp>| {
        at.and_then(clock::instant)
            .map(|at| clock::relative(at, now.timestamp()))
    };
    match &live.state {
        LiveState::Running if live.runner.is_some() => live.runner.as_deref().map(runner_line),
        LiveState::Scheduled => schedule_line(live, now),
        LiveState::Parked => live.until.as_ref().and_then(clock::instant).map(|until| {
            let day = until.to_zoned_datetime(now.time_zone().clone());
            format!("Until {}", day.strftime("%-d %b"))
        }),
        LiveState::Held => live
            .by
            .map(|by| format!("Waits on {}, which needs you", key_of(by))),
        LiveState::Running | LiveState::Queued => {
            let run = live.run.as_ref()?;
            let kind = if run.machine.as_str().starts_with("runner") {
                "session"
            } else {
                "thread"
            };
            let what = if run.worker {
                format!("{APP_NAME}'s worker")
            } else {
                run.title
                    .as_ref()
                    .map_or_else(|| format!("A {kind}"), |title| format!("{title} {kind}"))
            };
            let on = run
                .place
                .clone()
                .unwrap_or_else(|| run.machine.as_str().to_owned());
            let when = if live.state == LiveState::Running {
                ago(run.started.as_ref()).map(|ago| format!("started {ago}"))
            } else {
                Some("waiting behind another turn".to_owned())
            };
            let guessed = (live.inferred == Some(true)).then(|| "task guessed".to_owned());
            Some(
                [Some(format!("{what} on {on}")), when, guessed]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
        }
        LiveState::Stalled => {
            let last = live.last.as_ref();
            let ended = ago(last.and_then(|last| last.ended.as_ref())).map_or_else(
                || "no turn yet".to_owned(),
                |ago| format!("last turn ended {ago}"),
            );
            Some(match last.and_then(|last| last.line.as_deref()) {
                Some(said) => format!("{ended}: {said}"),
                None => ended,
            })
        }
        _ => None,
    }
}

/// What does the work when it isn't a turn: the browser, a watch, a unit of its own.
fn runner_line(runner: &str) -> String {
    if runner == "browser" {
        format!("{APP_NAME}'s browser is on it")
    } else if let Some(watch) = runner.strip_prefix("watch:") {
        format!("{APP_NAME}'s watch #{watch} is on it")
    } else {
        let what = runner.split_once(':').map_or(runner, |(_, name)| name);
        format!("Waiting on {what}")
    }
}

/// A scheduled task's next look, said the way people say it: `Next check around 12:20 AM ·
/// every 4 h. Last: still no reply (8:50 PM)`. A check-in has no interval and no note.
#[must_use]
pub(crate) fn schedule_line(live: &TaskLive, now: &ZonedDateTime) -> Option<String> {
    let next = live
        .watch
        .as_ref()
        .map(|watch| &watch.next_at)
        .or(live.checkin.as_ref().map(|checkin| &checkin.at))
        .and_then(clock::instant)?;
    let every = live.watch.as_ref().map(|watch| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a watch's minutes are far inside i64's range"
        )]
        let minutes = watch.every.round() as i64;
        format!("every {}", clock::interval(minutes))
    });
    let first = if live.checking == Some(true) {
        "Checking now".to_owned()
    } else {
        format!("Next check {}", clock::around(next, now))
    };
    let first = match every {
        Some(every) => format!("{first} · {every}"),
        None => first,
    };
    let Some((note, at)) = live.watch.as_ref().and_then(last_note) else {
        return Some(first);
    };
    let when = at
        .and_then(clock::instant)
        .map_or_else(String::new, |at| format!(" ({})", clock::said(at, now)));
    Some(format!("{first}. Last: {note}{when}"))
}

fn last_note(watch: &LiveWatch) -> Option<(String, Option<&emberhut_api::time::Timestamp>)> {
    let note = last_saw(watch.note.as_deref()?);
    (!note.is_empty()).then_some((note, watch.note_at.as_ref()))
}

/// What a watch last saw, its first sentence, without the time stamped before it
/// (`21 Oct ~20:50: still no reply. More` reads `still no reply`).
#[must_use]
pub(crate) fn last_saw(note: &str) -> String {
    let note = strip_stamp(note.trim());
    let end = note
        .char_indices()
        .find(|&(i, c)| {
            matches!(c, '.' | '!' | '?')
                && note[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map_or(note.len(), |(i, _)| i);
    note[..end].trim().trim_end_matches('.').to_owned()
}

/// `note` without a leading time stamp: up to its first `:` after a clock time (`20:50`),
/// within the first few dozen characters.
fn strip_stamp(note: &str) -> &str {
    let head: String = note.chars().take(40).collect();
    let bytes = head.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        let clock_at = |j: usize| {
            bytes[j].is_ascii_digit()
                && bytes.get(j + 1) == Some(&b':')
                && bytes.get(j + 2).is_some_and(u8::is_ascii_digit)
                && bytes.get(j + 3).is_some_and(u8::is_ascii_digit)
        };
        if clock_at(i)
            && let Some(colon) = head[i + 4..].find(':')
        {
            let cut = i + 4 + colon + 1;
            if head[..cut].contains('.') {
                return note;
            }
            return note[cut..].trim_start();
        }
    }
    note
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;

    fn now() -> ZonedDateTime {
        "2026-11-25T10:00:00+02:00[+02:00]".parse().unwrap()
    }

    fn key(id: TaskId) -> String {
        format!("T-{id}")
    }

    fn live(state: &serde_json::Value) -> Issue {
        issue(1, "working", json!({"live": state}))
    }

    #[test]
    fn the_state_comes_from_the_live_state_not_the_label() {
        let running = live(&json!({"state": "running", "alarm": false}));
        assert_eq!(name(&running), format!("{APP_NAME} is working"));
        assert_eq!((tone(&running), mark(&running)), (Tone::Warn, Mark::Half));
        let stalled = live(&json!({"state": "stalled", "alarm": true}));
        assert_eq!(name(&stalled), "Nothing running");
        assert_eq!(
            (tone(&stalled), mark(&stalled)),
            (Tone::Danger, Mark::Pause)
        );
        let checking = live(&json!({"state": "scheduled", "alarm": false, "checking": true}));
        assert_eq!(name(&checking), "Checking now");
        assert_eq!(mark(&checking), Mark::Eye);
        let held = live(&json!({"state": "held", "alarm": false}));
        assert_eq!(name(&held), "On hold");
        assert_eq!(
            name(&issue(2, "working", json!({}))),
            "Nothing running",
            "an older server"
        );
        assert_eq!(name(&issue(3, "todo", json!({}))), "To do");
        assert_eq!(name(&issue(4, "canceled", json!({}))), "Archived");
        let todo = issue(
            5,
            "todo",
            json!({"live": {"state": "todo", "alarm": false}}),
        );
        assert_eq!(name(&todo), "Not started");
    }

    #[test]
    fn a_run_says_who_runs_it_where_and_since_when() {
        let task = live(&json!({"state": "running", "alarm": false, "run": {
            "machine": "runner", "place": "the Mac", "thread": "w-1", "title": null,
            "worker": true, "requested": null, "started": "2026-11-25T07:55:00Z"}}));
        assert_eq!(
            live_line(&task, &now(), &key).as_deref(),
            Some(format!("{APP_NAME}'s worker on the Mac · started 5 minutes ago").as_str())
        );
        let queued = live(
            &json!({"state": "queued", "alarm": false, "inferred": true, "run": {
            "machine": "studio", "thread": "t-1", "title": "Example", "worker": false,
            "requested": null, "started": null}}),
        );
        assert_eq!(
            live_line(&queued, &now(), &key).as_deref(),
            Some("Example thread on studio · waiting behind another turn · task guessed")
        );
    }

    #[test]
    fn other_states_say_what_they_wait_on() {
        let browser = live(&json!({"state": "running", "alarm": false, "runner": "browser"}));
        assert_eq!(
            live_line(&browser, &now(), &key),
            Some(format!("{APP_NAME}'s browser is on it"))
        );
        let unit = live(&json!({"state": "running", "alarm": false, "runner": "unit:backup"}));
        assert_eq!(
            live_line(&unit, &now(), &key).as_deref(),
            Some("Waiting on backup")
        );
        let parked =
            live(&json!({"state": "parked", "alarm": false, "until": "2026-12-03T00:00:00Z"}));
        assert_eq!(
            live_line(&parked, &now(), &key).as_deref(),
            Some("Until 3 Dec")
        );
        let held = live(&json!({"state": "held", "alarm": false, "by": 7}));
        assert_eq!(
            live_line(&held, &now(), &key).as_deref(),
            Some("Waits on T-7, which needs you")
        );
        let stalled = live(&json!({"state": "stalled", "alarm": false,
            "last": {"ended": "2026-11-25T06:00:00Z", "line": "Drafted it"}}));
        assert_eq!(
            live_line(&stalled, &now(), &key).as_deref(),
            Some("last turn ended 2 hours ago: Drafted it")
        );
    }

    #[test]
    fn a_schedule_says_the_next_look_and_the_last_note() {
        let task = live(&json!({"state": "scheduled", "alarm": false, "watch": {
            "id": 3, "title": "Example reply", "nextAt": "2026-11-25T09:21:00Z", "every": 240.0,
            "note": "25 Nov ~08:50: still no reply. Asked again.", "noteAt": "2026-11-25T06:50:00Z"}}));
        assert_eq!(
            live_line(&task, &now(), &key).as_deref(),
            Some("Next check around 11:20 AM · every 4\u{a0}h. Last: still no reply (8:50 AM)")
        );
    }

    #[test]
    fn the_last_note_loses_its_stamp_and_keeps_its_first_sentence() {
        assert_eq!(
            last_saw("21 Oct ~20:50 local: still no reply. Then more"),
            "still no reply"
        );
        assert_eq!(last_saw("Nothing new yet"), "Nothing new yet");
        assert_eq!(last_saw("v1.2 shipped. Fine"), "v1.2 shipped");
    }
}
