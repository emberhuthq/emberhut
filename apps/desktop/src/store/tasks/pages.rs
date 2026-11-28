//! The task pages fetched for the column. A page is asked for when it opens and again whenever
//! its task changes in the state (its `updated` moves, or its plan's), so its history and its
//! plan keep up, and every few seconds while its agents run (they don't touch the task itself). A page in hand stays shown
//! while it is asked for again, and when asking again fails.

use std::collections::HashMap;
use std::time::Duration;

use emberhut_api::ids::TaskId;
use emberhut_api::task_page::TaskPage;
use emberhut_api::tasks::Issue;
use emberhut_api::time::Timestamp;

/// How often an open page is asked for again while its agents run.
pub(crate) const LIVE_EVERY: Duration = Duration::from_secs(4);

/// Why a page isn't there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    /// The server has no such task.
    NotFound,
    /// Asking failed, worded for the user.
    Failed(String),
}

/// Where a page stands, as the column shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PageStatus<'a> {
    /// Asked for, nothing in hand yet.
    Loading,
    /// In hand.
    Ready(&'a TaskPage),
    /// The server has no such task.
    NotFound,
    /// Asking failed and nothing is in hand.
    Failed(&'a str),
}

/// What the state last said of a task that its page follows: when the task changed, and when
/// its plan did (an agent ticks a step without touching the task).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    updated: Timestamp,
    plan: Option<Timestamp>,
}

impl Seen {
    /// What the state says of `task`.
    #[must_use]
    pub(crate) fn of(task: &Issue) -> Self {
        Self {
            updated: task.updated.clone(),
            plan: task.plan.as_ref().and_then(|plan| plan.updated.clone()),
        }
    }
}

#[derive(Debug, Default)]
struct Entry {
    page: Option<TaskPage>,
    missing: Option<Missing>,
    /// What the state said of the task when it was last asked for.
    asked_with: Option<Seen>,
    /// A request is on its way.
    loading: bool,
}

/// The pages, by task.
#[derive(Debug, Default)]
pub(crate) struct TaskPages {
    entries: HashMap<TaskId, Entry>,
}

impl TaskPages {
    /// Where `task`'s page stands.
    #[must_use]
    pub(crate) fn status(&self, task: TaskId) -> PageStatus<'_> {
        let Some(entry) = self.entries.get(&task) else {
            return PageStatus::Loading;
        };
        match (&entry.page, &entry.missing) {
            (Some(page), _) => PageStatus::Ready(page),
            (None, Some(Missing::NotFound)) => PageStatus::NotFound,
            (None, Some(Missing::Failed(reason))) if !entry.loading => PageStatus::Failed(reason),
            (None, _) => PageStatus::Loading,
        }
    }

    /// `task`'s page, if one is in hand.
    #[must_use]
    pub(crate) fn page(&self, task: TaskId) -> Option<&TaskPage> {
        self.entries.get(&task)?.page.as_ref()
    }

    /// Whether to ask for `task`'s page now, `seen` being what the state says of it (none: not
    /// in the state, a task done weeks ago): it was never asked for, it changed since, or
    /// `again` (its agents run, or the user just changed it). None is asked twice at once.
    /// When it says yes, the request is taken as on its way.
    pub(crate) fn begin(&mut self, task: TaskId, seen: Option<&Seen>, again: bool) -> bool {
        let entry = self.entries.entry(task).or_default();
        if entry.loading {
            return false;
        }
        let never = entry.page.is_none() && entry.missing.is_none();
        let changed = seen.is_some() && entry.asked_with.as_ref() != seen;
        if !(never || changed || again) {
            return false;
        }
        entry.loading = true;
        entry.asked_with = seen.cloned();
        true
    }

    /// The answer for `task`'s page came.
    pub(crate) fn finish(&mut self, task: TaskId, answer: Result<TaskPage, Missing>) {
        let entry = self.entries.entry(task).or_default();
        entry.loading = false;
        match answer {
            Ok(page) => {
                entry.page = Some(page);
                entry.missing = None;
            }
            Err(Missing::NotFound) => {
                entry.page = None;
                entry.missing = Some(Missing::NotFound);
            }
            Err(failed) => entry.missing = Some(failed),
        }
    }

    /// Whether `task`'s agents run now, so its page is asked for every few seconds.
    #[must_use]
    pub(crate) fn is_live(&self, task: TaskId) -> bool {
        self.page(task)
            .and_then(|page| page.ultra_run.as_ref())
            .is_some_and(emberhut_api::ultra::UltraRun::is_live)
    }

    /// Forgets every page `keep` says no to (pages off the column's path).
    pub(crate) fn retain(&mut self, keep: impl Fn(TaskId) -> bool) {
        self.entries.retain(|&task, _| keep(task));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, task_page};

    const ONE: TaskId = TaskId::new(1);

    fn at(time: &str) -> Seen {
        Seen::of(&issue(1, "todo", json!({"updated": time})))
    }

    #[test]
    fn a_page_is_asked_for_once_and_again_when_its_task_changes() {
        let mut pages = TaskPages::default();
        assert_eq!(pages.status(ONE), PageStatus::Loading);
        let first = at("2026-11-25T08:00:00Z");
        assert!(pages.begin(ONE, Some(&first), false));
        assert!(!pages.begin(ONE, Some(&first), false), "one at a time");
        pages.finish(ONE, Ok(task_page(1, json!({}))));
        assert!(matches!(pages.status(ONE), PageStatus::Ready(_)));
        assert!(!pages.begin(ONE, Some(&first), false), "nothing changed");
        assert!(pages.begin(ONE, Some(&at("2026-11-25T09:00:00Z")), false));
        pages.finish(ONE, Err(Missing::Failed("offline".into())));
        assert!(
            matches!(pages.status(ONE), PageStatus::Ready(_)),
            "the page in hand stays when asking again fails"
        );
    }

    #[test]
    fn a_page_is_asked_for_again_when_its_plan_moves() {
        let mut pages = TaskPages::default();
        let plan = |updated: &str| {
            Seen::of(&issue(
                1,
                "working",
                json!({"plan": {"done": 1, "total": 3, "updated": updated}}),
            ))
        };
        assert!(pages.begin(ONE, Some(&plan("2026-11-28T08:00:00Z")), false));
        pages.finish(ONE, Ok(task_page(1, json!({}))));
        assert!(!pages.begin(ONE, Some(&plan("2026-11-28T08:00:00Z")), false));
        assert!(
            pages.begin(ONE, Some(&plan("2026-11-28T08:01:00Z")), false),
            "a step ticked, the task itself untouched"
        );
    }

    #[test]
    fn a_task_outside_the_state_is_asked_for_once_and_can_be_missing() {
        let mut pages = TaskPages::default();
        assert!(pages.begin(ONE, None, false));
        pages.finish(ONE, Err(Missing::NotFound));
        assert_eq!(pages.status(ONE), PageStatus::NotFound);
        assert!(!pages.begin(ONE, None, false));
        assert!(pages.begin(ONE, None, true), "asked again on purpose");
    }

    #[test]
    fn a_failure_with_nothing_in_hand_shows_once_the_request_is_back() {
        let mut pages = TaskPages::default();
        pages.begin(ONE, None, false);
        pages.finish(ONE, Err(Missing::Failed("offline".into())));
        assert_eq!(pages.status(ONE), PageStatus::Failed("offline"));
        pages.begin(ONE, None, true);
        assert_eq!(pages.status(ONE), PageStatus::Loading);
    }

    #[test]
    fn a_page_with_running_agents_is_live() {
        let mut pages = TaskPages::default();
        pages.begin(ONE, None, false);
        pages.finish(
            ONE,
            Ok(task_page(
                1,
                json!({"ultraRun": {"on": true, "state": "running", "parallel": 2, "budget": 1.0,
                    "spent": 0.0, "done": 0, "total": 2, "running": 1, "agents": []}}),
            )),
        );
        assert!(pages.is_live(ONE));
        pages.retain(|_| false);
        assert!(!pages.is_live(ONE));
        assert_eq!(pages.status(ONE), PageStatus::Loading);
    }
}
