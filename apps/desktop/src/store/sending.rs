//! Sending the answers [`Answers`](super::answers::Answers) holds: when their Undo window runs
//! out, when another answer pushes them out, and when the user leaves: the window goes to the
//! background, or the app quits (closing the window quits too). Quitting waits a little for the
//! answers on their way, so they aren't cut off when the process ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::AppContext as _;
use gpui_kit::{App, Context, Entity, Window};

use super::answers::{Answer, Answered, Deed, REFUSAL_SHOWN, UNDO_WINDOW};
use super::{Store, update_or_log};
use emberhut_api::ids::TaskId;

/// How long quitting waits for the answers on their way to reach the server. GPUI's own wait
/// for quit handlers (`SHUTDOWN_TIMEOUT`) is too short for a request over the network.
const QUIT_WAIT: Duration = Duration::from_secs(2);

impl Store {
    /// The user answers a task's ask: it leaves the lists at once and goes to the server once
    /// its Undo window runs out. Whether the answer was taken (not when the task was answered
    /// already).
    pub(crate) fn decide(
        &mut self,
        task: TaskId,
        choice: String,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let answered = self.answers.answer(task, choice);
        self.send_when_due(task, answered, cx)
    }

    /// Sends what `answered` pushed out at once, and `task`'s once its Undo window runs out;
    /// whether `task`'s is held (not when it was answered or acted on already).
    pub(super) fn send_when_due(
        &mut self,
        task: TaskId,
        answered: Answered,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Answered::Held { send_now } = answered else {
            return false;
        };
        if let Some(earlier) = send_now {
            self.send_answer(earlier, cx);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO_WINDOW).await;
            update_or_log(&this, cx, |store, cx| {
                if let Some(answer) = store.answers.due(task) {
                    store.send_answer(answer, cx);
                }
                store.time_refusal(cx);
            });
        })
        .detach();
        true
    }

    /// Undo on the toast `serial`: takes back the held answer (or hides the toast of one
    /// already sent); nothing when another toast has taken its place.
    pub(crate) fn undo(&mut self, serial: u64, cx: &mut Context<'_, Self>) {
        self.answers.undo(serial);
        self.time_refusal(cx);
        cx.notify();
    }

    /// Hide: puts away the toast that says why an answer wasn't sent.
    pub(crate) fn hide_toast(&mut self, cx: &mut Context<'_, Self>) {
        self.answers.hide();
        cx.notify();
    }

    /// Sends the held answer whenever `window` stops being the key window: the app going to the
    /// background, the window being minimized, or a sheet such as About taking the keyboard.
    /// GPUI has no hook for the app alone resigning active, so a sheet ends the Undo window too;
    /// that only sends the answer early, it never loses it.
    pub(crate) fn send_held_when_inactive(store: &Entity<Self>, window: &mut Window, cx: &mut App) {
        store.update(cx, |_, cx| {
            cx.observe_window_activation(window, |store, window, cx| {
                if !window.is_window_active() {
                    store.send_held(cx);
                }
            })
            .detach();
        });
    }

    /// Sends the held answer when the app quits, and waits up to [`QUIT_WAIT`] for every
    /// answer on its way. The hook keeps `store`: closing the window drops the views, and the
    /// store with them, before the quit it leads to.
    pub(crate) fn send_held_on_quit(store: &Entity<Self>, cx: &mut App) {
        let store = store.clone();
        cx.on_app_quit(move |cx| {
            let over = store.update(cx, |store, cx| {
                store.send_held(cx);
                store.in_flight.over()
            });
            // Waited for here, not in the future handed back, which GPUI gives less time.
            if cx
                .foreground_executor()
                .block_with_timeout(QUIT_WAIT, over)
                .is_err()
            {
                log::warn!("quit before every answer reached the server");
            }
            std::future::ready(())
        })
        .detach();
    }

    /// Sends the held answer now, if there is one.
    fn send_held(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(answer) = self.answers.flush() {
            self.send_answer(answer, cx);
        }
        self.time_refusal(cx);
    }

    /// Sends `answer`. The request starts at once on a background thread, so that it goes
    /// even while quitting blocks the UI thread.
    pub(super) fn send_answer(&mut self, answer: Answer, cx: &mut Context<'_, Self>) {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        let request = {
            let answer = answer.clone();
            cx.background_spawn(async move {
                let sent = match &answer.deed {
                    Deed::Choice(choice) => backend.decide(answer.task, choice),
                    Deed::Act(action) => backend.act_on_tasks(action),
                };
                drop(sending);
                (sent, Instant::now())
            })
        };
        cx.spawn(async move |this, cx| {
            let (sent, landed) = request.await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                // The server's own words when it gave any ("Task 12 is done already").
                let outcome = sent.map_err(|error| {
                    error
                        .server_message()
                        .map_or_else(|| error.to_string(), str::to_owned)
                });
                store.answers.sent(&answer, outcome, landed);
                store.time_refusal(cx);
                store.refetch_open_task(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Puts away the refusal toast shown last, if one just showed, after [`REFUSAL_SHOWN`].
    fn time_refusal(&mut self, cx: &mut Context<'_, Self>) {
        let Some(serial) = self.answers.refusal_to_time() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFUSAL_SHOWN).await;
            update_or_log(&this, cx, |store, cx| {
                store.answers.expire(serial);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Entity, TestAppContext};

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::FakeBackend;

    fn store(cx: &mut TestAppContext, backend: FakeBackend) -> Entity<Store> {
        let backend = Arc::new(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::for_tests(backend))
    }

    fn toast_text(store: &Entity<Store>, cx: &mut TestAppContext) -> Option<String> {
        store.read_with(cx, |store, _| {
            store.answers().toast().map(|toast| toast.text.clone())
        })
    }

    #[gpui_kit::test]
    fn a_refusal_says_the_servers_words_and_goes_after_a_while(cx: &mut TestAppContext) {
        let backend = FakeBackend::new([]);
        backend.push_decision(Err(Error::Refused {
            status: Some(409),
            message: Some("Task 4 is done already".into()),
        }));
        let store = store(cx, backend);
        store.update(cx, |store, cx| {
            assert!(store.decide(TaskId::new(4), "Yes".into(), cx));
            assert!(
                !store.decide(TaskId::new(4), "No".into(), cx),
                "answered already"
            );
        });
        cx.executor().advance_clock(UNDO_WINDOW);
        cx.run_until_parked();
        assert_eq!(
            toast_text(&store, cx).as_deref(),
            Some("Answer not sent: Task 4 is done already")
        );
        cx.executor()
            .advance_clock(REFUSAL_SHOWN.saturating_sub(Duration::from_millis(1)));
        cx.run_until_parked();
        assert!(toast_text(&store, cx).is_some(), "still showing");
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(toast_text(&store, cx), None, "gone by itself");
    }

    #[gpui_kit::test]
    fn hide_puts_a_refusal_away(cx: &mut TestAppContext) {
        let backend = FakeBackend::new([]);
        backend.push_decision(Err(Error::Unreachable("connection refused".into())));
        let store = store(cx, backend);
        store.update(cx, |store, cx| {
            store.decide(TaskId::new(4), "Yes".into(), cx);
        });
        cx.executor().advance_clock(UNDO_WINDOW);
        cx.run_until_parked();
        assert_eq!(
            toast_text(&store, cx).as_deref(),
            Some("Answer not sent: can't reach the server"),
            "no words from the server: what went wrong"
        );
        store.update(cx, Store::hide_toast);
        assert_eq!(toast_text(&store, cx), None);
    }
}
