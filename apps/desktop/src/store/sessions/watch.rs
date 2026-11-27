//! One session followed live, as a GPUI entity its pages share: its first load, then its
//! stream from the last step it has, again 2 s after each end (nothing missed, nothing
//! polled), and the user's message, Stop, Stop this turn and settings. The stream runs on a
//! thread of its own while some page holds the entity; once none does, the thread ends at its
//! next event or ping.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use emberhut_api::chat::ChatFile;
use emberhut_api::runner::actions::{FILES_ONLY_TEXT, SendToSession, SessionSettings};
use emberhut_api::runner::transcript::{SessionLive, Transcript};
use gpui_kit::{AppContext as _, AsyncApp, Context, WeakEntity};

use super::view::{SessionRef, SessionView};
use crate::backend::{Backend, Error};
use crate::store::gate::reason;

/// How long to wait before following the stream again after it ended or failed.
const RECONNECT_AFTER: Duration = Duration::from_secs(2);

/// What the stream's thread tells the entity.
#[derive(Debug)]
pub(crate) enum Sent {
    /// The first load's answer.
    Loaded(Result<Transcript, String>),
    /// The stream opened (its first event came) or closed.
    Connected(bool),
    /// An event.
    Event(SessionLive),
}

/// Something the user asked of the session, for its "Not …" line when it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Send,
    Stop,
    Change,
}

impl Act {
    fn failed(self, why: &str) -> String {
        match self {
            Self::Send => format!("Not sent: {why}"),
            Self::Stop => format!("Not stopped: {why}"),
            Self::Change => format!("Not changed: {why}"),
        }
    }
}

/// A session followed live.
pub(crate) struct SessionWatch {
    view: SessionView,
    backend: Arc<dyn Backend>,
    /// Bumped when the entity goes: its thread stops.
    generation: Arc<AtomicU64>,
    /// A message is on its way.
    sending: bool,
    /// The last action that failed, worded for the user.
    problem: Option<String>,
    /// How many messages went through: a page empties its box when this moves.
    sent: u64,
    /// The words and files of the last message that didn't go, for its box to have them back.
    unsent: Option<(String, Vec<ChatFile>)>,
}

impl SessionWatch {
    /// Follows `reference` over `backend` from now on.
    pub(crate) fn new(
        reference: SessionRef,
        backend: Arc<dyn Backend>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let (events, received) = smol::channel::unbounded::<Sent>();
        {
            let (backend, generation) = (Arc::clone(&backend), Arc::clone(&generation));
            let reference = reference.clone();
            let thread = std::thread::Builder::new()
                .name(format!("session-{}", reference.id))
                .spawn(move || follow(&*backend, &reference, &generation, 0, &events));
            if let Err(error) = thread {
                log::error!("couldn't start a session's stream: {error}");
            }
        }
        cx.spawn(async move |this, cx| {
            while let Ok(sent) = received.recv().await {
                if update(&this, cx, |watch, cx| watch.take(sent, cx)).is_none() {
                    return;
                }
            }
        })
        .detach();
        Self {
            view: SessionView::new(reference),
            backend,
            generation,
            sending: false,
            problem: None,
            sent: 0,
            unsent: None,
        }
    }

    /// The session as it stands.
    #[must_use]
    pub(crate) fn view(&self) -> &SessionView {
        &self.view
    }

    /// Whether a message is on its way.
    #[must_use]
    pub(crate) fn is_sending(&self) -> bool {
        self.sending
    }

    /// The last action that failed, worded for the user (else why the first load failed).
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref().or(self.view.failure())
    }

    /// How many messages went through.
    #[must_use]
    pub(crate) fn sent(&self) -> u64 {
        self.sent
    }

    /// The words and files of the last message that didn't go, taken once.
    pub(crate) fn take_unsent(&mut self) -> Option<(String, Vec<ChatFile>)> {
        self.unsent.take()
    }

    fn take(&mut self, sent: Sent, cx: &mut Context<'_, Self>) {
        let changed = match sent {
            Sent::Loaded(answer) => {
                self.view.load(answer);
                true
            }
            Sent::Connected(connected) => {
                let changed = self.view.is_connected() != connected;
                self.view.set_connected(connected);
                changed
            }
            Sent::Event(event) => self.view.take(event),
        };
        if changed {
            cx.notify();
        }
    }

    /// Sends the user's message (with files stored already); while a turn runs, it waits
    /// behind it. Blank words with no files send nothing, and neither does a message while the
    /// one before is on its way. Whether it took the message.
    pub(crate) fn send(
        &mut self,
        text: &str,
        files: Vec<ChatFile>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let text = text.trim();
        if self.sending || (text.is_empty() && files.is_empty()) {
            return false;
        }
        let message = SendToSession {
            text: if text.is_empty() {
                FILES_ONLY_TEXT.to_owned()
            } else {
                text.to_owned()
            },
            files,
        };
        let words = (text.to_owned(), message.files.clone());
        self.sending = true;
        self.problem = None;
        let (backend, id) = (Arc::clone(&self.backend), self.view.reference().id.clone());
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.send_to_session(&id, &message) })
                .await;
            update(&this, cx, |watch, cx| {
                watch.sending = false;
                match answer {
                    Ok(_) => watch.sent += 1,
                    Err(error) => {
                        watch.problem = Some(Act::Send.failed(&reason(&error)));
                        watch.unsent = Some(words);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        true
    }

    /// Stops the session: the step it's on ends and nothing queued runs (asked first).
    pub(crate) fn stop(&mut self, cx: &mut Context<'_, Self>) {
        self.act(Act::Stop, |backend, id| backend.stop_session(id), cx);
    }

    /// Stops only the running turn; what is queued runs next.
    pub(crate) fn interrupt(&mut self, cx: &mut Context<'_, Self>) {
        self.act(Act::Stop, |backend, id| backend.interrupt_session(id), cx);
    }

    /// Changes the model or effort its next turn opens with.
    pub(crate) fn change(&mut self, settings: SessionSettings, cx: &mut Context<'_, Self>) {
        self.act(
            Act::Change,
            move |backend, id| backend.change_session(id, &settings),
            cx,
        );
    }

    fn act(
        &mut self,
        act: Act,
        call: impl FnOnce(&dyn Backend, &emberhut_api::ids::SessionId) -> Result<(), Error>
        + Send
        + 'static,
        cx: &mut Context<'_, Self>,
    ) {
        let (backend, id) = (Arc::clone(&self.backend), self.view.reference().id.clone());
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { call(&*backend, &id) })
                .await;
            update(&this, cx, |watch, cx| {
                watch.problem = answer.err().map(|error| act.failed(&reason(&error)));
                cx.notify();
            });
        })
        .detach();
    }
}

impl Drop for SessionWatch {
    /// No page holds it any more: its thread stops at its next event or ping.
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// Runs `update` on the watch if it is still held.
fn update<R>(
    this: &WeakEntity<SessionWatch>,
    cx: &mut AsyncApp,
    update: impl FnOnce(&mut SessionWatch, &mut Context<'_, SessionWatch>) -> R,
) -> Option<R> {
    this.update(cx, update)
        .inspect_err(|error| log::debug!("the session's page is gone: {error}"))
        .ok()
}

/// The stream's thread: the first load, then the stream from the last step seen, again after
/// each end, while `mine` is the current generation and the entity takes what it sends.
pub(crate) fn follow(
    backend: &dyn Backend,
    reference: &SessionRef,
    generation: &AtomicU64,
    mine: u64,
    events: &smol::channel::Sender<Sent>,
) {
    let current = || generation.load(Ordering::Relaxed) == mine;
    let loaded = backend
        .session_transcript(&reference.id, &reference.first_query())
        .map_err(|error| reason(&error));
    let mut seen = loaded
        .as_ref()
        .ok()
        .and_then(|t| t.events.last())
        .map_or(0, |step| step.seq);
    if events.send_blocking(Sent::Loaded(loaded)).is_err() {
        return;
    }
    while current() {
        let mut open = true;
        let mut first = true;
        let outcome = backend.follow_session(
            &reference.id,
            &reference.live_query(seen),
            &|| !current(),
            &mut |event| {
                if let SessionLive::Step { step } = &event {
                    seen = seen.max(step.seq);
                }
                if std::mem::take(&mut first) {
                    open &= events.send_blocking(Sent::Connected(true)).is_ok();
                }
                open &= events.send_blocking(Sent::Event(event)).is_ok();
            },
        );
        if let Err(error) = outcome {
            log::debug!("session {}'s stream broke off: {error}", reference.id);
        }
        if !current() || !open || events.send_blocking(Sent::Connected(false)).is_err() {
            return;
        }
        std::thread::sleep(RECONNECT_AFTER);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use emberhut_api::ids::SessionId;
    use emberhut_api::runner::steps::{SessionStep, StepKind};

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    fn transcript() -> Transcript {
        serde_json::from_value(serde_json::json!({
            "session": {"id": "r-1", "title": "A session", "status": "running"},
            "events": [{"seq": 4, "kind": "text", "text": "Hi"}],
            "earlier": false
        }))
        .unwrap()
    }

    #[test]
    fn the_thread_loads_then_follows_from_the_last_step_and_stops_when_dropped() {
        let backend = Arc::new(FakeBackend::new([]));
        backend.sessions.put_transcript(transcript());
        backend.sessions.push_stream(Ok(vec![SessionLive::Step {
            step: SessionStep::new(5, StepKind::Text),
        }]));
        let generation = Arc::new(AtomicU64::new(1));
        let (events, received) = smol::channel::unbounded();
        let thread = {
            let (backend, generation) = (Arc::clone(&backend), Arc::clone(&generation));
            std::thread::spawn(move || {
                let reference = SessionRef::whole(SessionId::from("r-1"));
                follow(&*backend, &reference, &generation, 1, &events);
            })
        };
        assert!(matches!(
            received.recv_blocking().unwrap(),
            Sent::Loaded(Ok(_))
        ));
        assert!(matches!(
            received.recv_blocking().unwrap(),
            Sent::Connected(true)
        ));
        assert!(matches!(received.recv_blocking().unwrap(), Sent::Event(_)));
        generation.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(thread.is_finished(), "the thread ended");
        let calls = backend.calls();
        assert!(calls.contains(&Call::Transcript(
            SessionId::from("r-1"),
            "after=0&limit=400".into()
        )));
        assert!(calls.contains(&Call::FollowSession(
            SessionId::from("r-1"),
            "after=4".into()
        )));
    }

    #[test]
    fn a_stream_that_fails_is_followed_again() {
        let backend = Arc::new(FakeBackend::new([]));
        backend
            .sessions
            .push_stream(Err(Error::Unreachable("down".into())));
        let generation = Arc::new(AtomicU64::new(1));
        let (events, received) = smol::channel::unbounded();
        let thread = {
            let (backend, generation) = (Arc::clone(&backend), Arc::clone(&generation));
            std::thread::spawn(move || {
                let reference = SessionRef::whole(SessionId::from("r-2"));
                follow(&*backend, &reference, &generation, 1, &events);
            })
        };
        assert!(matches!(
            received.recv_blocking().unwrap(),
            Sent::Loaded(Err(_))
        ));
        assert!(matches!(
            received.recv_blocking().unwrap(),
            Sent::Connected(false)
        ));
        drop(received);
        generation.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(4);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(thread.is_finished());
    }
}
