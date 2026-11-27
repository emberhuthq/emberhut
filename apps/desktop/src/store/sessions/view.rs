//! One session as its page holds it: the head from the stream and what the first load knew,
//! the steps so far and the last one seen (the stream starts after it), the words being typed
//! and the thinking being written, the running turn's tokens, and whether the stream is
//! connected. The transcript's first load and each live event are taken here; the page reads
//! the rest off it ([`super::turns`]).

use emberhut_api::ids::{SessionId, ToolCallId};
use emberhut_api::runner::steps::{SessionStep, StepKind};
use emberhut_api::runner::transcript::{SessionHead, SessionLive, StepsQuery, Transcript};
use emberhut_api::time::Timestamp;

/// How many steps the first load asks for: the newest 400 of a session, 1000 of one helper's.
const FIRST_LOAD: u32 = 400;
const FIRST_LOAD_HELPER: u32 = 1000;

/// What a session page shows: a whole session, or one helper agent's steps in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionRef {
    /// The session.
    pub(crate) id: SessionId,
    /// One helper's steps only: the call that started it.
    pub(crate) agent: Option<ToolCallId>,
    /// The helper's name, as its page's title.
    pub(crate) name: Option<String>,
}

impl SessionRef {
    /// A whole session.
    #[must_use]
    pub(crate) fn whole(id: SessionId) -> Self {
        Self {
            id,
            agent: None,
            name: None,
        }
    }

    /// One helper agent's steps in session `id`, started by `call`.
    #[must_use]
    pub(crate) fn helper(id: SessionId, call: ToolCallId, name: impl Into<String>) -> Self {
        Self {
            id,
            agent: Some(call),
            name: Some(name.into()),
        }
    }

    /// Where an agent's transcript opens: its whole session, or its own steps in it.
    #[must_use]
    pub(crate) fn of_agent(viewer: SessionId, parent: Option<ToolCallId>, name: String) -> Self {
        match parent {
            Some(call) => Self::helper(viewer, call, name),
            None => Self::whole(viewer),
        }
    }

    /// Whether this is a task's worker, watched as session `w-<task>` (its helpers' steps come
    /// filtered from the server).
    #[must_use]
    pub(crate) fn is_worker(&self) -> bool {
        self.id.as_str().starts_with("w-")
    }

    /// The first load's query.
    #[must_use]
    pub(crate) fn first_query(&self) -> StepsQuery {
        StepsQuery {
            after: 0,
            limit: Some(if self.agent.is_some() {
                FIRST_LOAD_HELPER
            } else {
                FIRST_LOAD
            }),
            agent: self.agent.clone(),
        }
    }

    /// The live stream's query, after the step `seen`.
    #[must_use]
    pub(crate) fn live_query(&self, seen: u64) -> StepsQuery {
        StepsQuery {
            after: seen,
            limit: None,
            agent: self.agent.clone(),
        }
    }
}

/// A session as its page holds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionView {
    reference: SessionRef,
    /// The head as the stream says it now.
    head: Option<SessionHead>,
    /// The head from the first load: what the server adds (account name, tasks) is there.
    known: Option<SessionHead>,
    steps: Vec<SessionStep>,
    /// The last step from the server (a helper's page drops the others, but has seen them).
    seen: u64,
    /// The model's words in the block being typed now.
    typing: String,
    /// Its thinking being written now (empty: it keeps it to itself), and since when.
    thinking: Option<String>,
    think_since: Option<Timestamp>,
    /// The running turn's output tokens so far.
    tokens: Option<f64>,
    earlier: bool,
    loaded: bool,
    connected: bool,
    /// Why the first load failed, worded for the user.
    failed: Option<String>,
}

impl SessionView {
    /// A page of `reference`, nothing loaded yet.
    #[must_use]
    pub(crate) fn new(reference: SessionRef) -> Self {
        Self {
            reference,
            head: None,
            known: None,
            steps: Vec::new(),
            seen: 0,
            typing: String::new(),
            thinking: None,
            think_since: None,
            tokens: None,
            earlier: false,
            loaded: false,
            connected: false,
            failed: None,
        }
    }

    /// What it shows.
    #[must_use]
    pub(crate) fn reference(&self) -> &SessionRef {
        &self.reference
    }

    /// The head: the stream's, else the first load's.
    #[must_use]
    pub(crate) fn head(&self) -> Option<&SessionHead> {
        self.head.as_ref().or(self.known.as_ref())
    }

    /// The first load's head.
    #[must_use]
    pub(crate) fn known(&self) -> Option<&SessionHead> {
        self.known.as_ref()
    }

    /// The steps so far, oldest first.
    #[must_use]
    pub(crate) fn steps(&self) -> &[SessionStep] {
        &self.steps
    }

    /// The last step seen: the stream follows on after it.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn seen(&self) -> u64 {
        self.seen
    }

    /// The words being typed now.
    #[must_use]
    pub(crate) fn typing(&self) -> &str {
        &self.typing
    }

    /// The thinking being written now, and since when.
    #[must_use]
    pub(crate) fn thinking(&self) -> Option<(&str, Option<&Timestamp>)> {
        self.thinking
            .as_deref()
            .map(|text| (text, self.think_since.as_ref()))
    }

    /// The running turn's tokens: the stream's count, else the head's.
    #[must_use]
    pub(crate) fn tokens(&self) -> Option<f64> {
        self.tokens
            .or_else(|| self.head.as_ref().and_then(|head| head.tokens))
    }

    /// Older steps than these exist.
    #[must_use]
    pub(crate) fn has_earlier(&self) -> bool {
        self.earlier
    }

    /// The first load is done (or failed).
    #[must_use]
    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The live stream is connected now.
    #[must_use]
    pub(crate) fn is_connected(&self) -> bool {
        self.connected
    }

    /// Why the first load failed.
    #[must_use]
    pub(crate) fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// A step as this page keeps it: a helper's page of a runner session keeps only that
    /// helper's steps, as its own (a worker's come filtered already).
    fn mine(&self, mut step: SessionStep) -> Option<SessionStep> {
        let Some(agent) = &self.reference.agent else {
            return Some(step);
        };
        if self.reference.is_worker() {
            return Some(step);
        }
        if step.parent.as_ref() != Some(agent) {
            return None;
        }
        step.parent = None;
        Some(step)
    }

    /// The first load's answer: its head and steps (a page that already has steps keeps them).
    pub(crate) fn load(&mut self, answer: Result<Transcript, String>) {
        self.loaded = true;
        match answer {
            Ok(transcript) if self.steps.is_empty() => {
                self.failed = None;
                self.seen = transcript.events.last().map_or(0, |step| step.seq);
                self.earlier = transcript.earlier;
                self.steps = transcript
                    .events
                    .into_iter()
                    .filter_map(|step| self.mine(step))
                    .collect();
                self.known = Some(transcript.session);
            }
            Ok(transcript) => self.known = Some(transcript.session),
            Err(reason) => self.failed = Some(reason),
        }
    }

    /// The stream (re)connected, or dropped.
    pub(crate) fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    /// One event of the live stream; whether anything changed.
    pub(crate) fn take(&mut self, event: SessionLive) -> bool {
        let whole = self.reference.agent.is_none();
        match event {
            SessionLive::Step { step } => {
                if step.seq <= self.seen {
                    return false;
                }
                self.seen = step.seq;
                let Some(step) = self.mine(step) else {
                    return false;
                };
                if step.parent.is_none() {
                    match step.kind {
                        StepKind::Text => self.typing.clear(),
                        StepKind::Thinking => {
                            self.thinking = None;
                            self.think_since = None;
                        }
                        _ => {}
                    }
                }
                self.steps.push(step);
                true
            }
            SessionLive::Delta { text, start, .. } if whole => {
                if start == Some(true) {
                    self.typing = text;
                } else {
                    self.typing.push_str(&text);
                }
                true
            }
            SessionLive::Think {
                text, start, since, ..
            } if whole => {
                if start == Some(true) {
                    self.thinking = Some(text);
                    self.think_since = since;
                } else {
                    self.thinking.get_or_insert_default().push_str(&text);
                }
                true
            }
            SessionLive::Tokens { tokens, .. } => {
                self.tokens = Some(tokens);
                true
            }
            SessionLive::Head { session } => {
                if session.is_running() {
                    if let Some(tokens) = session.tokens {
                        self.tokens = Some(self.tokens.map_or(tokens, |had| had.max(tokens)));
                    }
                } else {
                    self.typing.clear();
                    self.thinking = None;
                    self.think_since = None;
                    self.tokens = None;
                }
                self.head = Some(session);
                true
            }
            SessionLive::Delta { .. } | SessionLive::Think { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::runner::steps::StepKind;

    use super::*;

    fn step(seq: u64, kind: StepKind, parent: Option<&str>) -> SessionStep {
        let mut step = SessionStep::new(seq, kind);
        step.parent = parent.map(ToolCallId::from);
        step.text = Some(format!("s{seq}"));
        step
    }

    fn head(status: &str, tokens: Option<f64>) -> SessionHead {
        serde_json::from_value(serde_json::json!({
            "id": "r-1", "title": "A session", "status": status, "tokens": tokens
        }))
        .unwrap()
    }

    fn transcript(steps: Vec<SessionStep>) -> Transcript {
        Transcript {
            session: head("running", None),
            events: steps,
            earlier: true,
        }
    }

    #[test]
    fn the_first_load_sets_the_steps_and_where_the_stream_starts() {
        let mut view = SessionView::new(SessionRef::whole(SessionId::from("r-1")));
        assert!(!view.is_loaded() && view.head().is_none());
        view.load(Ok(transcript(vec![
            step(3, StepKind::User, None),
            step(4, StepKind::Text, None),
        ])));
        assert!(view.is_loaded() && view.has_earlier());
        assert_eq!(view.seen(), 4);
        assert_eq!(
            view.reference().live_query(view.seen()).to_query(),
            "after=4"
        );
        assert_eq!(view.head().unwrap().title, "A session");
        view.load(Err("can't reach the server".into()));
        assert_eq!(view.failure(), Some("can't reach the server"));
        assert_eq!(view.steps().len(), 2, "what it had stays");
    }

    #[test]
    fn typed_words_and_thinking_show_until_their_step_lands() {
        let mut view = SessionView::new(SessionRef::whole(SessionId::from("r-1")));
        view.take(SessionLive::Delta {
            turn: None,
            text: "Hel".into(),
            start: Some(true),
        });
        view.take(SessionLive::Delta {
            turn: None,
            text: "lo".into(),
            start: None,
        });
        assert_eq!(view.typing(), "Hello");
        view.take(SessionLive::Think {
            turn: None,
            text: String::new(),
            start: Some(true),
            since: Some(Timestamp::new("2026-11-25T08:00:00Z")),
        });
        view.take(SessionLive::Think {
            turn: None,
            text: "Hmm".into(),
            start: None,
            since: None,
        });
        assert_eq!(view.thinking().map(|(text, _)| text), Some("Hmm"));
        assert!(view.take(SessionLive::Step {
            step: step(9, StepKind::Thinking, None)
        }));
        assert!(view.thinking().is_none());
        view.take(SessionLive::Step {
            step: step(10, StepKind::Text, None),
        });
        assert_eq!(view.typing(), "");
        assert!(
            !view.take(SessionLive::Step {
                step: step(10, StepKind::Text, None)
            }),
            "a step seen already is dropped"
        );
        assert_eq!(view.steps().len(), 2);
    }

    #[test]
    fn a_head_that_stopped_running_clears_the_live_words() {
        let mut view = SessionView::new(SessionRef::whole(SessionId::from("r-1")));
        view.take(SessionLive::Tokens {
            turn: None,
            tokens: 900.,
        });
        view.take(SessionLive::Head {
            session: head("running", Some(500.)),
        });
        assert_eq!(view.tokens(), Some(900.), "the larger count stays");
        view.take(SessionLive::Delta {
            turn: None,
            text: "x".into(),
            start: Some(true),
        });
        view.take(SessionLive::Head {
            session: head("completed", None),
        });
        assert_eq!((view.typing(), view.tokens()), ("", None));
        view.set_connected(true);
        assert!(view.is_connected());
    }

    #[test]
    fn a_helpers_page_keeps_its_own_steps_as_its_own() {
        let reference = SessionRef::helper(SessionId::from("r-1"), ToolCallId::from("c-1"), "Docs");
        assert_eq!(
            reference.first_query().to_query(),
            "after=0&limit=1000&agent=c-1"
        );
        let mut view = SessionView::new(reference);
        view.load(Ok(transcript(vec![
            step(1, StepKind::Tool, None),
            step(2, StepKind::Text, Some("c-1")),
            step(3, StepKind::Text, Some("c-2")),
        ])));
        assert_eq!(view.steps().len(), 1);
        assert_eq!(view.steps()[0].parent, None);
        assert_eq!(view.seen(), 3);
        assert!(
            !view.take(SessionLive::Delta {
                turn: None,
                text: "x".into(),
                start: Some(true)
            }),
            "the lead's typing isn't the helper's"
        );
        let worker = SessionRef::helper(SessionId::from("w-5"), ToolCallId::from("c-1"), "Part 1");
        let mut view = SessionView::new(worker);
        view.load(Ok(transcript(vec![step(1, StepKind::Text, None)])));
        assert_eq!(view.steps().len(), 1, "a worker's come filtered already");
    }
}
