//! A task's runs on its page: every turn of work newest first, its STATUS line read into a
//! state, when and how long it ran, and where the task stands from the latest that answered.

use emberhut_api::ids::{MachineId, RunId};
use emberhut_api::task_page::{Moment, Run, TaskPage};
use emberhut_api::tasks::{Artifact, Issue, TaskStatus};
use jiff::ZonedDateTime;

use super::clock;
use crate::store::lookup;

/// What a run's STATUS line says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunState {
    /// Done.
    Done,
    /// Still working.
    Working,
    /// Blocked.
    Blocked,
    /// It needs the user.
    NeedsYou,
}

/// A run's STATUS line: its state and its words (`Needs you: pick the cluster`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunStatus {
    /// The state.
    pub(crate) state: RunState,
    /// Its words, the state's name first.
    pub(crate) text: String,
    rest: Option<String>,
}

impl RunStatus {
    /// Reads `STATUS: done`, `STATUS: needs <the user>: pick the cluster` (any case) out of a
    /// line; none when it has no STATUS.
    #[must_use]
    pub(crate) fn parse(line: &str) -> Option<Self> {
        let lower = line.to_lowercase();
        let at = lower.find("status:")?;
        let after = line[at + "status:".len()..].trim_start();
        let after_lower = after.to_lowercase();
        let needs = format!("needs {}", lookup::PERSON_OWNER);
        let (state, word) = [
            (RunState::Done, "done"),
            (RunState::Working, "working"),
            (RunState::Blocked, "blocked"),
            (RunState::NeedsYou, needs.as_str()),
        ]
        .into_iter()
        .find(|(_, word)| {
            after_lower.starts_with(word)
                && after_lower[word.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric())
        })?;
        let rest = after[word.len()..]
            .trim_start_matches(':')
            .trim()
            .to_owned();
        let label = match state {
            RunState::Done => "Done",
            RunState::Working => "Working",
            RunState::Blocked => "Blocked",
            RunState::NeedsYou => "Needs you",
        };
        let text = if rest.is_empty() {
            label.to_owned()
        } else {
            format!("{label}: {rest}")
        };
        Some(Self {
            state,
            text,
            rest: (!rest.is_empty()).then_some(rest),
        })
    }

    /// What follows the state, when it says more (`pick the cluster`).
    #[must_use]
    pub(crate) fn rest(&self) -> Option<&str> {
        self.rest.as_deref()
    }
}

/// The marker a finished turn leaves in a task's history on an older server.
const STATUS_MARK: &str = " · STATUS:";

/// The task's runs, newest first: the page's, or (an older server sends none) the ones rebuilt
/// from the history lines finished turns left (`… · STATUS: …`).
#[must_use]
pub(crate) fn timeline(page: Option<&TaskPage>) -> Vec<Run> {
    let Some(page) = page else {
        return Vec::new();
    };
    if let Some(runs) = &page.runs {
        return runs.clone();
    }
    let mut rebuilt: Vec<Run> = page
        .history
        .iter()
        .enumerate()
        .filter_map(|(i, moment)| rebuilt_run(i, moment))
        .collect();
    rebuilt.reverse();
    rebuilt
}

fn rebuilt_run(i: usize, moment: &Moment) -> Option<Run> {
    let at = moment.text.find(STATUS_MARK)?;
    Some(Run {
        id: RunId::new(-i64::try_from(i).unwrap_or(i64::MAX)),
        kind: "thread".to_owned(),
        machine: MachineId::new(""),
        thread: String::new(),
        title: None,
        asked: None,
        answer: Some(moment.text[..at].to_owned()),
        status: Some(moment.text[at + " · ".len()..].to_owned()),
        requested: None,
        started: None,
        ended: Some(moment.at.clone()),
        outcome: Some("completed".to_owned()),
    })
}

/// Where a task stands: the latest run's answer, and what is left (its STATUS line's words)
/// unless it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Standing {
    /// The answer.
    pub(crate) line: String,
    /// What is left.
    pub(crate) left: Option<String>,
}

/// Where `task` stands, from `runs` (newest first); an older server's page has no runs, and
/// its last turn's first line says it best.
#[must_use]
pub(crate) fn standing(task: &Issue, page: Option<&TaskPage>, runs: &[Run]) -> Option<Standing> {
    let last_line = task
        .live
        .as_ref()
        .and_then(|live| live.last.as_ref())
        .and_then(|last| last.line.clone());
    let has_runs = page.is_some_and(|page| page.runs.is_some());
    let answered = runs.iter().find_map(|run| {
        run.answer
            .as_deref()
            .filter(|answer| !answer.trim().is_empty())
            .map(|answer| (run, answer))
    });
    match answered {
        Some((run, answer)) if has_runs || last_line.is_none() => {
            let status = run.status.as_deref().and_then(RunStatus::parse);
            let done = task.status == TaskStatus::Done
                || status.as_ref().is_some_and(|s| s.state == RunState::Done);
            Some(Standing {
                line: answer.to_owned(),
                left: status
                    .filter(|_| !done)
                    .and_then(|status| status.rest().map(str::to_owned)),
            })
        }
        _ => last_line.map(|line| Standing { line, left: None }),
    }
}

/// When a run ran and for how long: `Today 9:12 AM → 9:40 AM · 28m`, `Started Today 9:12 AM ·
/// 5m so far`, `Sent Thu 8:00 AM · queued`.
#[must_use]
pub(crate) fn run_when(run: &Run, now: &ZonedDateTime) -> String {
    let start = run.started.as_ref().and_then(clock::instant);
    let end = run.ended.as_ref().and_then(clock::instant);
    match (start, end) {
        (Some(start), Some(end)) => {
            let zone = now.time_zone().clone();
            let same_day = start.to_zoned_datetime(zone.clone()).date()
                == end.to_zoned_datetime(zone.clone()).date();
            let to = if same_day {
                clock::clock_minutes(&end.to_zoned_datetime(zone))
            } else {
                clock::day_and_time(end, now)
            };
            format!(
                "{} → {to} · {}",
                clock::day_and_time(start, now),
                clock::span(start, end)
            )
        }
        (Some(start), None) => format!(
            "Started {} · {} so far",
            clock::day_and_time(start, now),
            clock::span(start, now.timestamp())
        ),
        (None, Some(end)) => clock::day_and_time(end, now),
        (None, None) => run.requested.as_ref().and_then(clock::instant).map_or_else(
            || "Queued".to_owned(),
            |sent| format!("Sent {} · queued", clock::day_and_time(sent, now)),
        ),
    }
}

/// `Ended 2 hours ago, after 14m`: the last run that ended, for the page's top.
#[must_use]
pub(crate) fn last_run_line(runs: &[Run], now: &ZonedDateTime) -> Option<String> {
    let last = runs.iter().find(|run| run.ended.is_some())?;
    let end = last.ended.as_ref().and_then(clock::instant)?;
    let after = last
        .started
        .as_ref()
        .and_then(clock::instant)
        .map_or_else(String::new, |start| {
            format!(", after {}", clock::span(start, end))
        });
    Some(format!(
        "Ended {}{after}",
        clock::relative(end, now.timestamp())
    ))
}

/// A run's pictures and videos: what the task got while it ran, or around its end (a minute
/// before it started to two after it ended; a run still open runs to `now`).
#[must_use]
pub(crate) fn media<'a>(
    run: &Run,
    artifacts: &'a [Artifact],
    now: &ZonedDateTime,
) -> Vec<&'a Artifact> {
    let end = run
        .ended
        .as_ref()
        .and_then(clock::instant)
        .or_else(|| run.started.as_ref().map(|_| now.timestamp()));
    let Some(end) = end else {
        return Vec::new();
    };
    let start = run
        .started
        .as_ref()
        .and_then(clock::instant)
        .map_or(end.as_second() - 120, jiff::Timestamp::as_second);
    artifacts
        .iter()
        .filter(|artifact| artifact.thumb.is_some())
        .filter(|artifact| {
            clock::instant(&artifact.at).is_some_and(|at| {
                at.as_second() >= start - 60 && at.as_second() <= end.as_second() + 120
            })
        })
        .collect()
}

/// What the task produced that no run shows: its results section's.
#[must_use]
pub(crate) fn unclaimed<'a>(
    artifacts: &'a [Artifact],
    runs: &[Run],
    now: &ZonedDateTime,
) -> Vec<&'a Artifact> {
    let claimed: Vec<_> = runs
        .iter()
        .flat_map(|run| media(run, artifacts, now))
        .map(|artifact| artifact.id)
        .collect();
    artifacts
        .iter()
        .filter(|artifact| !claimed.contains(&artifact.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, task_page};
    use crate::wire::ASSISTANT_OWNER;

    fn now() -> ZonedDateTime {
        "2026-11-25T12:00:00+02:00[+02:00]".parse().unwrap()
    }

    fn run(value: serde_json::Value) -> Run {
        let mut base = json!({"id": 1, "kind": "thread", "machine": "runner", "thread": "t-1",
            "title": null, "asked": null, "answer": null, "status": null, "requested": null,
            "started": null, "ended": null, "outcome": null});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), value) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn a_status_line_reads_as_its_state_and_words() {
        let needs = RunStatus::parse(&format!(
            "STATUS: needs {}: pick the cluster",
            lookup::PERSON_OWNER
        ))
        .unwrap();
        assert_eq!(needs.state, RunState::NeedsYou);
        assert_eq!(needs.text, "Needs you: pick the cluster");
        assert_eq!(needs.rest(), Some("pick the cluster"));
        let done = RunStatus::parse("All good. status: Done").unwrap();
        assert_eq!(
            (done.state, done.text.as_str(), done.rest()),
            (RunState::Done, "Done", None)
        );
        assert_eq!(RunStatus::parse("STATUS: doneish"), None);
        assert_eq!(RunStatus::parse("no status here"), None);
    }

    #[test]
    fn an_older_servers_runs_come_from_its_history() {
        let page = task_page(
            1,
            json!({"history": [
                {"at": "2026-11-24T08:00:00Z", "who": ASSISTANT_OWNER, "text": "Created"},
                {"at": "2026-11-24T09:00:00Z", "who": ASSISTANT_OWNER, "text": "Drafted it · STATUS: working: the rest"},
                {"at": "2026-11-24T10:00:00Z", "who": ASSISTANT_OWNER, "text": "Sent it · STATUS: done"},
            ]}),
        );
        let runs = timeline(Some(&page));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].answer.as_deref(), Some("Sent it"));
        assert_eq!(runs[0].status.as_deref(), Some("STATUS: done"));
        let page = task_page(2, json!({"runs": []}));
        assert!(
            timeline(Some(&page)).is_empty(),
            "a newer server's own runs, none yet"
        );
    }

    #[test]
    fn where_it_stands_comes_from_the_latest_answer() {
        let task = issue(1, "working", json!({}));
        let page = task_page(1, json!({"runs": []}));
        let runs = [
            run(json!({"answer": "  "})),
            run(json!({"answer": "Half of it", "status": "STATUS: working: the other half"})),
        ];
        assert_eq!(
            standing(&task, Some(&page), &runs),
            Some(Standing {
                line: "Half of it".into(),
                left: Some("the other half".into())
            })
        );
        let older = issue(
            2,
            "working",
            json!({"live": {"state": "stalled", "alarm": false,
            "last": {"ended": null, "line": "Its first line"}}}),
        );
        assert_eq!(
            standing(&older, None, &[]).map(|s| s.line),
            Some("Its first line".to_owned())
        );
    }

    #[test]
    fn a_runs_pictures_are_those_made_while_it_ran() {
        let artifact = |id: i64, at: &str, thumb: bool| -> Artifact {
            serde_json::from_value(json!({"id": id, "kind": "image", "caption": null, "at": at,
                "name": format!("a{id}.png"), "url": format!("/api/artifacts/{id}"),
                "thumb": thumb.then(|| format!("/api/artifacts/{id}/thumb"))}))
            .unwrap()
        };
        let artifacts = [
            artifact(1, "2026-11-25T07:20:00Z", true),
            artifact(2, "2026-11-25T07:41:30Z", true),
            artifact(3, "2026-11-25T07:30:00Z", false),
            artifact(4, "2026-11-25T09:00:00Z", true),
        ];
        let ran = run(json!({"started": "2026-11-25T07:12:00Z", "ended": "2026-11-25T07:40:00Z"}));
        let ids = |list: Vec<&Artifact>| list.iter().map(|a| a.id.get()).collect::<Vec<_>>();
        assert_eq!(ids(media(&ran, &artifacts, &now())), [1, 2]);
        assert_eq!(ids(unclaimed(&artifacts, &[ran], &now())), [3, 4]);
    }

    #[test]
    fn a_run_says_when_and_how_long() {
        let now = now();
        let ended =
            run(json!({"started": "2026-11-25T07:12:00Z", "ended": "2026-11-25T07:40:00Z"}));
        assert_eq!(run_when(&ended, &now), "Today 9:12 AM → 9:40 AM · 28m");
        let open = run(json!({"started": "2026-11-25T09:55:00Z"}));
        assert_eq!(run_when(&open, &now), "Started Today 11:55 AM · 5m so far");
        assert_eq!(run_when(&run(json!({})), &now), "Queued");
        assert_eq!(
            last_run_line(&[open, ended], &now).as_deref(),
            Some("Ended 2 hours ago, after 28m")
        );
    }
}
