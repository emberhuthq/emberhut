//! The state poll: the open room's state now, then every few seconds with its fingerprint (an
//! unchanged state costs a 304 and nothing to decode), and again at once when something kicks
//! it (a room switch, a message sent, the end of a turn, the window coming forward). How long
//! to wait is decided by [`next_poll`], from where the window is ([`Presence`]): every few
//! seconds in front, now and then behind another app's window, and not at all out of sight.
//! Each poll says whether the room is being looked at, and each answer may mark the room's new
//! answers seen ([`reading`](super::reading)).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::AppContext as _;
use gpui_kit::{AsyncApp, Context, Task, WeakEntity};

use super::presence::Presence;
use super::{RoomId, Store, status, update_or_log};
use crate::backend::{Error, FreshState, StateReply};

/// How often the state is asked for while all is well and the window is in front.
pub(crate) const POLL_EVERY: Duration = Duration::from_secs(3);

/// How often while it fails in front: quiet retries, a second apart.
pub(crate) const RETRY_EVERY: Duration = Duration::from_secs(1);

/// How often while the window is on screen behind another app's: a slow heartbeat, so what it
/// shows (the rooms' unread, what waits on the user) doesn't go stale for long on a screen the
/// user can still glance at, say a second display.
///
/// This goes beyond "stop polling in the background": the live stream doesn't need it (a
/// connection of its own, it kicks the poll at the end of each turn, so answers come at once).
/// It is a middle ground on purpose: the reference client keeps polling every 3 s while it is only
/// inactive, and stops only once the app is in the background; here a window behind costs one
/// request (mostly a 304) every 30 s, and one no one can see costs none.
pub(crate) const BEHIND_EVERY: Duration = Duration::from_secs(30);

/// How long to wait before the next poll, whether the last one `failing` and where the window
/// is; `None` while it is out of sight: no poll until something kicks it (the window coming
/// back does).
#[must_use]
pub(crate) fn next_poll(failing: bool, presence: Presence) -> Option<Duration> {
    match presence {
        Presence::Front if failing => Some(RETRY_EVERY),
        Presence::Front => Some(POLL_EVERY),
        Presence::Behind => Some(BEHIND_EVERY),
        Presence::Hidden => None,
    }
}

/// Wakes a loop (the poll, the machines' refresh) at once.
#[derive(Clone, Debug)]
pub(crate) struct Kicker(smol::channel::Sender<()>);

/// The loop's end of the [`Kicker`].
#[derive(Debug)]
pub(crate) struct Kicked(smol::channel::Receiver<()>);

impl Kicker {
    /// Fetches now rather than at the next tick. Kicks while one is waiting fold into it.
    pub(crate) fn kick(&self) {
        match self.0.try_send(()) {
            Ok(()) | Err(smol::channel::TrySendError::Full(())) => {}
            Err(smol::channel::TrySendError::Closed(())) => {
                log::debug!("the loop has stopped: nothing to wake");
            }
        }
    }
}

impl Kicked {
    /// Waits for a kick (at once if one is waiting, or once no one can kick any more).
    pub(crate) async fn wait(&self) {
        if self.0.recv().await.is_err() {
            log::debug!("no one can kick the loop any more");
        }
    }
}

/// A kicker and its loop's end. One kick is kept at most: a burst of them (a message sent,
/// the end of a turn, a room switch) means one fetch, not one each.
#[must_use]
pub(crate) fn kicker() -> (Kicker, Kicked) {
    let (kick, kicked) = smol::channel::bounded(1);
    (Kicker(kick), Kicked(kicked))
}

/// Starts the poll for the store in `cx`: it runs until the task is dropped (or the store
/// goes), a reply on its way then dropped with it.
pub(crate) fn start(kicked: Kicked, cx: &mut Context<'_, Store>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Some((backend, room, etag, looking, flaky)) =
                update_or_log(&this, cx, |store, cx| {
                    // Time tells too: a window no one used for a while isn't looked at any more.
                    let now = cx.background_executor().now();
                    let looking = store.on_screen(now).is_looking_at(store.room);
                    (
                        Arc::clone(&store.backend),
                        store.room,
                        store.etag.clone(),
                        looking,
                        store.flaky,
                    )
                })
            else {
                return;
            };
            let asked = Instant::now();
            let reply = cx
                .background_spawn(async move {
                    if flaky.drops_at(std::time::SystemTime::now()) {
                        return Err(Error::Unreachable("the flaky test mode dropped it".into()));
                    }
                    backend.state(room, etag.as_ref(), looking)
                })
                .await;
            let Some(failing) =
                update_or_log(&this, cx, |store, cx| store.apply(room, reply, asked, cx))
            else {
                return;
            };
            if wait_turn(&this, &kicked, failing, cx).await.is_none() {
                return;
            }
        }
    })
}

/// Waits for the poll's next turn: its time ([`next_poll`]) or a kick. A window that goes out
/// of sight while it waits isn't polled when the time comes, only once something kicks the poll
/// (the window coming back does). `None` once the store is gone: the poll ends.
async fn wait_turn(
    this: &WeakEntity<Store>,
    kicked: &Kicked,
    failing: bool,
    cx: &mut AsyncApp,
) -> Option<()> {
    let presence = update_or_log(this, cx, |store, _| store.presence())?;
    if let Some(wait) = next_poll(failing, presence) {
        let timer = cx.background_executor().timer(wait);
        let kicked_first = smol::future::or(
            async {
                timer.await;
                false
            },
            async {
                kicked.wait().await;
                true
            },
        )
        .await;
        if kicked_first {
            return Some(());
        }
        let presence = update_or_log(this, cx, |store, _| store.presence())?;
        if next_poll(failing, presence).is_some() {
            return Some(());
        }
    }
    kicked.wait().await;
    Some(())
}

impl Store {
    /// Where the window is for the user now.
    #[must_use]
    pub(crate) fn presence(&self) -> Presence {
        self.window.presence()
    }

    /// The platform showed the window, or stopped showing it (minimised, the app hidden, on
    /// another Space, fully covered). Out of sight nothing is polled; back in sight the poll
    /// goes at once.
    pub(crate) fn set_window_visible(&mut self, visible: bool) {
        if self.window.visible == visible {
            return;
        }
        let before = self.presence();
        self.window.visible = visible;
        self.presence_changed(before);
    }

    /// The window moved from `before` to where it is now. Come forward, what it shows may be
    /// stale: the state is asked for at once, and the machines too once it is in front (their
    /// page refreshes only there).
    pub(super) fn presence_changed(&mut self, before: Presence) {
        let now = self.presence();
        if now.is_forward_of(before) {
            self.kicker.kick();
            if before == Presence::Hidden {
                self.reload_scheduled();
            }
            if now == Presence::Front {
                self.machines.kick();
                self.settings_forward();
                self.working_forward();
                self.approvals_forward();
            }
        }
    }

    /// Takes in a poll's reply for `room`, asked at `asked`; whether it failed.
    pub(super) fn apply(
        &mut self,
        room: RoomId,
        reply: Result<StateReply, Error>,
        asked: Instant,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.metrics.polled();
        if room != self.room {
            return false; // the user switched rooms while it was on its way
        }
        if reply.is_ok() {
            self.set_refused(false, cx);
            if self.failing.passed() {
                cx.notify();
            }
        }
        match reply {
            Ok(StateReply::Unchanged) => {
                self.metrics.unchanged();
                self.let_in();
                // Back after a failure: the problem line goes, though nothing else changed.
                if self.problem.take().is_some() {
                    cx.notify();
                }
                // A mark seen that failed is tried again with each poll.
                self.mark_read(cx);
                false
            }
            Ok(StateReply::Fresh(fresh)) => {
                self.metrics.fresh(asked.elapsed());
                self.let_in();
                self.take_fresh(room, fresh, asked, cx);
                self.mark_read(cx);
                self.setup_state_came(cx);
                false
            }
            Ok(StateReply::SignInRequired) | Err(Error::SignInRequired) => {
                if !self.needs_sign_in {
                    self.signed_out(cx);
                }
                true
            }
            Err(error) => {
                log::debug!("the state poll failed: {error}");
                // A 401 without a sign-in: the server doesn't know this device.
                if matches!(
                    error,
                    Error::Refused {
                        status: Some(401),
                        ..
                    }
                ) {
                    self.set_refused(true, cx);
                }
                let problem = status::problem(&error);
                if self
                    .failing
                    .failed(problem.clone(), status::Outage::of(&error), asked)
                {
                    cx.notify();
                }
                if self.problem.as_ref() != Some(&problem) {
                    self.problem = Some(problem);
                    cx.notify();
                }
                true
            }
        }
    }

    fn take_fresh(
        &mut self,
        room: RoomId,
        fresh: FreshState,
        asked: Instant,
        cx: &mut Context<'_, Self>,
    ) {
        let FreshState { state, etag } = fresh;
        self.etag = etag;
        if self.chats.join(room, &state.chat) {
            self.chat_changed(room, cx);
        }
        if state.live.is_none() {
            self.live = None;
        }
        self.settle_pending(&state.chat);
        self.answers.settle(asked, |task| {
            state
                .working
                .iter()
                .any(|row| row.task == Some(task) && row.needs_you())
        });
        if let Some(brief) = &state.theme {
            self.theme_seen(brief, cx);
        }
        self.state = Some(*state);
        self.settle_put_away(cx);
        self.problem = None;
        self.needs_sign_in = false;
        self.ask_to_sign_in();
        self.tasks_follow_state(cx);
        self.fetch_needs_pictures(cx);
        self.browser_follows_state(cx);
        // A fresh state (a 200, not a 304) changed something here: the chat, the live step, the
        // pending line, the answers or the pictures being fetched. Redrawing is cheap.
        cx.notify();
    }
}

#[cfg(test)]
impl Store {
    /// Takes in `state` for the open room as a fresh poll reply would, for the views' tests.
    pub(crate) fn take_state_for_tests(
        &mut self,
        state: emberhut_api::state::State,
        cx: &mut Context<'_, Self>,
    ) {
        let fresh = FreshState {
            state: Box::new(state),
            etag: None,
        };
        let failed = self.apply(self.room, Ok(StateReply::Fresh(fresh)), Instant::now(), cx);
        assert!(!failed, "a fresh state is taken in");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_front_the_state_is_asked_for_every_few_seconds_and_retried_sooner() {
        assert_eq!(next_poll(false, Presence::Front), Some(POLL_EVERY));
        assert_eq!(next_poll(true, Presence::Front), Some(RETRY_EVERY));
        assert!(RETRY_EVERY < POLL_EVERY);
    }

    #[test]
    fn behind_another_apps_window_it_is_asked_for_now_and_then_even_while_failing() {
        assert_eq!(next_poll(false, Presence::Behind), Some(BEHIND_EVERY));
        assert_eq!(next_poll(true, Presence::Behind), Some(BEHIND_EVERY));
        assert!(BEHIND_EVERY > POLL_EVERY * 5);
    }

    #[test]
    fn out_of_sight_it_isnt_asked_for_at_all() {
        assert_eq!(next_poll(false, Presence::Hidden), None);
        assert_eq!(
            next_poll(true, Presence::Hidden),
            None,
            "no quiet retries either"
        );
    }
}
