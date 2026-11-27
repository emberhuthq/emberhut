//! Everything in flight (the state's `working`): each task being worked or waiting on the
//! person, and each coding thread running that no task covers; waiting on the person first,
//! then running, queued, and the rest, the newest first within each. Its state comes from
//! what is happening (the task's live state from its runs), not its stored status.

use emberhut_api as api;
use emberhut_api::tasks::{AskKind, Issue, TaskStatus};
use emberhut_api::work::{Waiting, WaitingDeadline, WorkKind, WorkRow, WorkState};

use super::WorkerService;

/// The order rows come in, by state.
const ORDER: [&str; 8] = [
    "waiting",
    "running",
    "queued",
    "stuck",
    "limit",
    "stalled",
    "scheduled",
    "stopped",
];

/// A line for a phone: plain words, single spaces, 220 characters at most.
fn clean(text: Option<&str>) -> Option<String> {
    let text = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| text.chars().take(220).collect())
}

fn state_word(state: &WorkState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl WorkerService {
    /// The rows of work in flight, from the board's `issues` and the coding threads.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub fn working(
        &self,
        issues: &[Issue],
        threads: &[api::threads::CodingThread],
    ) -> Vec<WorkRow> {
        let mut rows = Vec::new();
        for t in issues {
            let assistant = t.owner == emberhut_core::task::wire::ASSISTANT;
            let queued_work = t.status == TaskStatus::Todo
                && assistant
                && !t.tracked
                && t.body.as_deref().is_some_and(|b| !b.trim().is_empty());
            let in_flight =
                t.status == TaskStatus::Waiting || t.status == TaskStatus::Working || queued_work;
            if !in_flight || t.kind.as_deref() == Some("finding") {
                continue;
            }
            if t.parent.is_some() && t.status == TaskStatus::Working {
                // A parent's working subtasks show inside its row.
                continue;
            }
            let now = t.live.as_ref();
            let live_state = now.map_or_default(|l| l.state.as_str());
            let running_worker = self.is_running(emberhut_core::task::TaskId::new(t.id.get()));
            let state = if t.status == TaskStatus::Waiting || live_state == "waiting" {
                WorkState::Waiting
            } else if live_state == "running" || running_worker {
                WorkState::Running
            } else if live_state == "queued" || queued_work {
                WorkState::Queued
            } else if live_state == "stalled" {
                WorkState::Stalled
            } else if live_state == "scheduled" {
                WorkState::Scheduled
            } else {
                WorkState::Stopped
            };
            let waiting = state == WorkState::Waiting;
            let who = match &state {
                WorkState::Waiting if assistant => "You, then the assistant carries on".to_owned(),
                WorkState::Waiting => "You".to_owned(),
                WorkState::Stalled => "Nothing running".to_owned(),
                WorkState::Scheduled => "The assistant's check-in".to_owned(),
                _ if running_worker => "The assistant's worker on this server".to_owned(),
                _ if now.and_then(|l| l.run.as_ref()).is_some() => {
                    let run = now.and_then(|l| l.run.as_ref());
                    format!(
                        "A session on {}",
                        run.and_then(|r| r.place.clone()).unwrap_or_default()
                    )
                }
                _ if !assistant => "You".to_owned(),
                _ if t.status == TaskStatus::Todo => {
                    "The assistant's worker, starting now".to_owned()
                }
                _ if t.tracked => "Done elsewhere · the assistant follows".to_owned(),
                _ => "You and the assistant, talking it through".to_owned(),
            };
            let progress = match &t.sub {
                Some(sub) => Some(format!("{}/{} done", sub.done, sub.total)),
                None => t.total.filter(|n| *n != 0).map(|total| {
                    format!(
                        "{}/{total} {}",
                        t.progress.unwrap_or(0),
                        t.unit.as_deref().unwrap_or("")
                    )
                    .trim_end()
                    .to_owned()
                }),
            };
            let line = if waiting {
                clean(
                    t.decision
                        .as_ref()
                        .map(|d| d.question.as_str())
                        .or(t.last.as_deref()),
                )
            } else {
                clean(
                    t.last
                        .as_deref()
                        .or_else(|| t.body.as_deref().and_then(|b| b.lines().next())),
                )
            };
            let at = match (&state, now.and_then(|l| l.since.clone())) {
                (WorkState::Running, Some(since)) => Some(since),
                _ => Some(t.updated.clone()),
            };
            let need = t.decision.as_ref().and_then(|d| d.kind.clone()).unwrap_or(
                if t.decision.is_some() {
                    AskKind::Choose
                } else {
                    AskKind::Do
                },
            );
            let deadline = t
                .decide_by
                .clone()
                .into_iter()
                .chain(t.deadline.clone())
                .min();
            rows.push(WorkRow {
                id: api::ids::WorkId::from(format!("task:{}", t.id.get())),
                area: None,
                thumb: None,
                reply: None,
                kind: WorkKind::Task,
                task: Some(t.id),
                machine: None,
                thread: None,
                title: t.title.clone(),
                pid: t.pid,
                who,
                state,
                progress,
                line,
                at,
                eta: t.eta.clone(),
                ask: None,
                last: clean(t.last.as_deref()),
                action: if waiting { t.action.clone() } else { None },
                options: if waiting {
                    t.decision.as_ref().map(|d| d.options.clone())
                } else {
                    None
                },
                draft: waiting
                    && t.decision
                        .as_ref()
                        .is_some_and(|d| d.draft.as_deref().is_some_and(|d| !d.is_empty())),
                waiting: waiting.then(|| Waiting {
                    need,
                    asked: t
                        .decision
                        .as_ref()
                        .and_then(|d| d.asked.clone())
                        .or_else(|| Some(t.updated.clone())),
                    checked: t
                        .decision
                        .as_ref()
                        .and_then(|d| d.checked.clone())
                        .or_else(|| Some(t.updated.clone())),
                    deadline: Some(WaitingDeadline {
                        decide_by: deadline,
                        hard_by: t.hard_deadline,
                    }),
                }),
                paused: false,
            });
        }
        for th in threads.iter().filter(|th| th.parent.is_none()) {
            let state = match th.status.as_str() {
                "running" => WorkState::Running,
                "waiting" => WorkState::Waiting,
                _ => continue,
            };
            let waiting = state == WorkState::Waiting;
            rows.push(WorkRow {
                id: api::ids::WorkId::from(format!("thread:{}", th.id)),
                area: None,
                thumb: None,
                reply: None,
                kind: WorkKind::Thread,
                task: None,
                machine: Some(th.machine.clone()),
                thread: Some(th.id.clone()),
                title: th.title.clone(),
                pid: None,
                who: format!(
                    "A thread on {}",
                    serde_json::to_value(&th.machine)
                        .unwrap_or_default()
                        .as_str()
                        .unwrap_or("its machine")
                ),
                state,
                progress: None,
                line: clean(th.last.as_deref()),
                at: Some(th.updated_at.clone()),
                eta: None,
                ask: clean(th.ask.as_deref()),
                last: clean(th.last.as_deref()),
                action: waiting.then(|| "Answer it in its session".to_owned()),
                options: None,
                draft: false,
                waiting: waiting.then(|| Waiting {
                    need: AskKind::Do,
                    asked: Some(th.updated_at.clone()),
                    checked: Some(th.updated_at.clone()),
                    deadline: None,
                }),
                paused: false,
            });
        }
        rows.sort_by(|a, b| {
            let rank = |r: &WorkRow| {
                ORDER
                    .iter()
                    .position(|s| *s == state_word(&r.state))
                    .unwrap_or(ORDER.len())
            };
            rank(a).cmp(&rank(b)).then_with(|| {
                let at = |r: &WorkRow| r.at.as_ref().map_or_default(|t| t.as_str().to_owned());
                at(b).cmp(&at(a))
            })
        });
        rows
    }
}
