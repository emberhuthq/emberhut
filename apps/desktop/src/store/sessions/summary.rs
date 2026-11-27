//! What a session's page says of it at the top: its title, its status and since when, where it
//! runs (machine, account, folder, branch), its model and effort, its tasks. From the stream's
//! head, what the first load knew, and the session's row in the state; a helper agent's page
//! takes its status from the agents working now.

use emberhut_api::ids::TaskId;
use emberhut_api::runner::agents::AgentNow;
use emberhut_api::state::State;
use emberhut_api::time::Timestamp;
use emberhut_api::work::RunnerSession;

use super::lists;
use super::view::SessionView;
use super::words::{model_name, short_path, short_title};
use emberhut_api::runner::actions::Effort;

/// A session's page head.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Summary {
    /// Its title.
    pub(crate) title: String,
    /// A helper's page: whose helper it is.
    pub(crate) helper_of: Option<String>,
    /// Its status word.
    pub(crate) status: String,
    /// A turn runs now.
    pub(crate) running: bool,
    /// When the running turn started.
    pub(crate) since: Option<Timestamp>,
    /// Its current step.
    pub(crate) doing: Option<String>,
    /// The user opened it: their box, Stop that only ends the turn, model and effort.
    pub(crate) direct: bool,
    /// Its model's id.
    pub(crate) model: Option<String>,
    /// Its effort's word.
    pub(crate) effort: Option<String>,
    /// Where it runs: `Mac · Personal · ~/work · fix`.
    pub(crate) place: String,
    /// Its tasks.
    pub(crate) tasks: Vec<TaskId>,
    /// How many messages wait behind the running turn.
    pub(crate) queued: u32,
}

impl Summary {
    /// Its model and effort: `Opus 5.5 · High`; none when it has neither.
    #[must_use]
    pub(crate) fn model_line(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.model.as_deref().map(|m| model_name(m).to_owned()),
            self.effort
                .as_deref()
                .map(|e| Effort::from(e).name().to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// The helper agent running now that `view` shows, if it shows one.
#[must_use]
pub(crate) fn helper<'a>(view: &SessionView, state: Option<&'a State>) -> Option<&'a AgentNow> {
    let reference = view.reference();
    let agent = reference.agent.as_ref()?;
    state?
        .agents_now
        .iter()
        .find(|a| a.viewer.as_ref() == Some(&reference.id) && a.parent.as_ref() == Some(agent))
}

/// The page head of `view`, with the state's row of its session.
#[must_use]
pub(crate) fn summary(view: &SessionView, state: Option<&State>) -> Summary {
    let reference = view.reference();
    let info: Option<&RunnerSession> = state.and_then(|state| lists::find(state, &reference.id));
    let known = view.known();
    let head = view.head();
    let lead_title = head
        .map(|h| h.title.as_str())
        .or(info.map(|i| i.title.as_str()));
    let title = reference.name.clone().unwrap_or_else(|| {
        lead_title.map_or_else(|| "Session".to_owned(), |t| short_title(t).to_owned())
    });
    let helper_of = reference.agent.is_some().then(|| {
        format!(
            "A helper agent of {}",
            lead_title.map_or("its lead", short_title)
        )
    });
    let (status, running, since, doing) = if reference.agent.is_some() {
        let helper = helper(view, state);
        let running = helper.is_some();
        (
            if running { "running" } else { "completed" }.to_owned(),
            running,
            helper.and_then(|h| h.started.clone()),
            helper.and_then(|h| h.line.clone()),
        )
    } else {
        let status = head
            .map(|h| h.status.clone())
            .or_else(|| info.map(|i| i.status.clone()))
            .unwrap_or_default();
        let running = head.map_or_else(
            || matches!(status.as_str(), "running" | "queued"),
            emberhut_api::runner::transcript::SessionHead::is_running,
        );
        let since = head
            .and_then(|h| h.since.clone())
            .or_else(|| info.and_then(|i| i.since.clone()));
        let doing = head
            .and_then(|h| h.doing.clone())
            .or_else(|| info.and_then(|i| i.doing.clone()));
        (status, running, since, doing)
    };
    let direct = reference.agent.is_none()
        && (head.is_some_and(emberhut_api::runner::transcript::SessionHead::is_direct)
            || info.is_some_and(|i| i.direct));
    let account = super::words::account_name(
        known
            .and_then(|h| h.account_name.as_deref())
            .or_else(|| info.and_then(|i| i.account_name.as_deref())),
        head.and_then(|h| h.account.as_deref()),
        state.map_or(&[], |s| s.accounts.as_slice()),
    );
    let place = [
        known
            .and_then(|h| h.place.clone())
            .or_else(|| info.map(|i| i.place.clone())),
        account,
        head.and_then(|h| h.cwd.as_deref())
            .or_else(|| info.and_then(|i| i.cwd.as_deref()))
            .map(short_path),
        head.and_then(|h| h.branch.clone())
            .or_else(|| info.and_then(|i| i.branch.clone())),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let tasks = info
        .map(|i| i.tasks.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| known.map(|h| h.tasks.clone()))
        .unwrap_or_default();
    Summary {
        title,
        helper_of,
        status,
        running,
        since,
        doing,
        direct,
        model: head
            .and_then(|h| h.model.clone())
            .or_else(|| info.and_then(|i| i.model.clone())),
        effort: head
            .and_then(|h| h.effort.clone())
            .or_else(|| info.and_then(|i| i.effort.clone())),
        place,
        tasks,
        queued: head.and_then(|h| h.queued).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::ids::{SessionId, ToolCallId};
    use emberhut_api::runner::transcript::{SessionLive, Transcript};
    use serde_json::json;

    use super::*;
    use crate::store::sessions::view::SessionRef;

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the tests build the head with json! in place"
    )]
    fn transcript(head: serde_json::Value) -> Transcript {
        serde_json::from_value(json!({ "session": head, "events": [] })).unwrap()
    }

    #[test]
    fn the_head_comes_from_the_stream_the_first_load_and_the_row() {
        let state = crate::fixtures::state(json!({"mySessions": [{
            "id": "r-1", "machine": "runner-b", "title": "Mine", "project": "", "status": "waiting",
            "updatedAt": "2026-11-25T08:00:00Z", "place": "Mac", "doing": null, "account": "a",
            "accountName": "Personal", "ask": null, "last": null, "direct": true, "model": null,
            "tasks": [3], "effort": "xhigh", "cwd": "/Users/example/work"
        }]}));
        let mut view = SessionView::new(SessionRef::whole(SessionId::from("r-1")));
        let empty = summary(&view, None);
        assert_eq!((empty.title.as_str(), empty.running), ("Session", false));
        view.load(Ok(transcript(
            json!({"id": "r-1", "title": format!("{}Mine", crate::wire::ASSISTANT_TITLE_PREFIX), "status": "idle",
            "model": "claude-opus-5-5", "meta": {"direct": true}, "branch": "fix"}),
        )));
        let head = summary(&view, Some(&state));
        assert_eq!(head.title, "Mine");
        assert_eq!(head.status, "idle", "the session's own head before its row");
        assert!(head.direct && !head.running);
        assert_eq!(head.place, "Mac · Personal · ~/work · fix");
        assert_eq!(head.model_line().as_deref(), Some("Opus 5.5 · Extra high"));
        assert_eq!(head.tasks, [TaskId::new(3)]);
        view.take(SessionLive::Head {
            session: serde_json::from_value(
                json!({"id": "r-1", "title": "Mine", "status": "running",
                "running": true, "since": "2026-11-25T08:01:00Z", "queued": 2, "doing": "Reading"}),
            )
            .unwrap(),
        });
        let live = summary(&view, Some(&state));
        assert!(live.running);
        assert_eq!((live.queued, live.doing.as_deref()), (2, Some("Reading")));
    }

    #[test]
    fn a_helpers_page_runs_while_its_agent_works() {
        let state = crate::fixtures::state(json!({"agentsNow": [{
            "id": "agent-1", "task": 3, "key": "EX-3", "lead": false, "viewer": "r-1",
            "parent": "c-1", "line": "Reading", "started": "2026-11-25T08:00:00Z"
        }]}));
        let reference = SessionRef::helper(SessionId::from("r-1"), ToolCallId::from("c-1"), "Docs");
        let mut view = SessionView::new(reference);
        view.load(Ok(transcript(
            json!({"id": "r-1", "title": "The lead", "status": "running"}),
        )));
        let head = summary(&view, Some(&state));
        assert_eq!(head.title, "Docs");
        assert_eq!(
            head.helper_of.as_deref(),
            Some("A helper agent of The lead")
        );
        assert!(head.running && !head.direct);
        assert_eq!(head.doing.as_deref(), Some("Reading"));
        let done = summary(&view, Some(&crate::fixtures::state(json!({}))));
        assert_eq!((done.status.as_str(), done.running), ("completed", false));
    }
}
