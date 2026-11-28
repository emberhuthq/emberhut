//! Moving a session to another task, on a server that keeps plans: the tasks it can move to
//! (open ones the server takes, not one it is on already, those whose key or title has every
//! word typed, the latest first) and the move itself, after which the state and the task pages
//! shown are asked for again. A refusal reads as the server's reason (no such session or task,
//! a task that is done or archived).

use std::sync::Arc;

use emberhut_api::ids::{SessionId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::{Issue, TaskStatus};
use gpui_kit::{AppContext as _, Context, Task};

use super::reason;
use crate::store::sessions::words::short_title;
use crate::store::{Store, update_or_log};

/// How many tasks the picker shows at most.
pub(crate) const SHOWN: usize = 40;

/// A session to move: the title the picker names it by, and the tasks it is on now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Moving {
    /// The session.
    pub(crate) session: SessionId,
    /// Its title, short.
    pub(crate) title: String,
    /// The tasks it works on now: the picker leaves them out.
    pub(crate) from: Vec<TaskId>,
}

impl Moving {
    /// Session `session` titled `title`, on `from`.
    #[must_use]
    pub(crate) fn new(session: SessionId, title: &str, from: Vec<TaskId>) -> Self {
        Self {
            session,
            title: short_title(title).to_owned(),
            from,
        }
    }
}

/// Whether a session can be moved to a task of the picker's: the server answers it only while
/// it keeps plans.
#[must_use]
pub(crate) fn can_move(state: Option<&State>) -> bool {
    state.is_some_and(crate::store::lookup::keeps_plans)
}

/// The tasks in `state` that `moving` can move to, `gone` ones (put away here, the server not
/// told yet) left out: open ones it isn't on, whose key or title has every word of `query`,
/// the latest changed first, [`SHOWN`] at most.
#[must_use]
pub(crate) fn targets<'a>(
    state: &'a State,
    moving: &Moving,
    query: &str,
    gone: impl Fn(TaskId) -> bool,
) -> Vec<&'a Issue> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let hits = |task: &Issue| {
        let (key, title) = (task.key.to_lowercase(), task.title.to_lowercase());
        words
            .iter()
            .all(|word| key.contains(word.as_str()) || title.contains(word.as_str()))
    };
    let mut found: Vec<&Issue> = state
        .issues
        .iter()
        .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled))
        .filter(|task| !moving.from.contains(&task.id) && !gone(task.id) && hits(task))
        .collect();
    found.sort_by(|a, b| b.updated.as_str().cmp(a.updated.as_str()));
    found.dedup_by_key(|task| task.id);
    found.truncate(SHOWN);
    found
}

/// What a move that didn't happen says: the server's reason.
#[must_use]
pub(crate) fn failed(reason: &str) -> String {
    format!("Not moved: {reason}")
}

impl Store {
    /// The tasks the picker offers for `moving` with `query` typed.
    #[must_use]
    pub(crate) fn move_targets(&self, moving: &Moving, query: &str) -> Vec<&Issue> {
        self.state
            .as_ref()
            .map_or_default(|state| targets(state, moving, query, |task| self.is_gone(task)))
    }

    /// Moves `session` to `task`: once the server answers, the state and the task pages shown
    /// are asked for again. The task it is on, or what the move's failure says.
    pub(crate) fn move_session(
        &mut self,
        session: SessionId,
        task: TaskId,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<TaskId, String>> {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        let request = cx.background_spawn(async move {
            let moved = backend.move_session(&session, task);
            drop(sending);
            moved
        });
        cx.spawn(async move |this, cx| {
            let moved = request.await;
            if let Err(error) = &moved {
                log::info!("a session wasn't moved: {error}");
            }
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                store.refetch_open_task(cx);
                cx.notify();
            });
            moved.map_err(|error| failed(&reason(&error)))
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state};

    fn tasks() -> State {
        state(json!({"issues": [
            issue(1, "todo", json!({"title": "Bake the bread", "updated": "2026-11-20T00:00:00Z"})),
            issue(2, "working", json!({"title": "Fix the oven", "updated": "2026-11-28T00:00:00Z"})),
            issue(3, "done", json!({"title": "Buy flour"})),
            issue(4, "canceled", json!({"title": "Old idea"})),
            issue(5, "waiting", json!({"title": "Bread prices", "updated": "2026-11-27T00:00:00Z"})),
            issue(12, "todo", json!({"title": "Shop sign", "updated": "2026-11-01T00:00:00Z"})),
        ]}))
    }

    fn ids(found: &[&Issue]) -> Vec<i64> {
        found.iter().map(|task| task.id.get()).collect()
    }

    fn moving(from: &[i64]) -> Moving {
        Moving::new(
            SessionId::from("r-1"),
            "A session",
            from.iter().copied().map(TaskId::new).collect(),
        )
    }

    #[test]
    fn open_tasks_it_isnt_on_the_latest_first() {
        let state = tasks();
        let found = targets(&state, &moving(&[1]), "", |_| false);
        assert_eq!(
            ids(&found),
            [2, 5, 12],
            "not done, archived, or the one it is on"
        );
        let found = targets(&state, &moving(&[]), "", |task| task == TaskId::new(2));
        assert_eq!(ids(&found), [5, 1, 12], "one put away here stays out");
    }

    #[test]
    fn every_word_typed_is_in_its_key_or_title() {
        let state = tasks();
        let found = |query: &str| ids(&targets(&state, &moving(&[]), query, |_| false));
        assert_eq!(found("bread"), [5, 1]);
        assert_eq!(found("BREAD bake"), [1], "any case, every word");
        assert_eq!(found("t-12"), [12], "by its key");
        assert_eq!(found("12 sign"), [12]);
        assert_eq!(
            found("flour"),
            Vec::<i64>::new(),
            "a done task isn't offered"
        );
    }

    #[test]
    fn only_a_server_that_keeps_plans_takes_a_move() {
        assert_eq!(moving(&[]).title, "A session");
        assert_eq!(
            Moving::new(
                SessionId::from("r-7"),
                "Emberhut · Fix the oven",
                Vec::new()
            )
            .title,
            "Fix the oven",
            "named short"
        );
        assert!(!can_move(None));
        assert!(!can_move(Some(&tasks())));
        let plans = state(json!({"ui": {"hideProjects": true, "hideGoals": true,
                                        "taskPlans": true}}));
        assert!(can_move(Some(&plans)));
        assert_eq!(failed("no such task"), "Not moved: no such task");
    }
}
