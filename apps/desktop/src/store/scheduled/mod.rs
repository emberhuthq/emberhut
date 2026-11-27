//! Everything the assistant has scheduled, for the Scheduled page and the sidebar's count: asked
//! for at launch, every minute while the page is open in the front window, and every five
//! minutes while the app is in sight otherwise (the count). Out of sight the loop sleeps until
//! a kick (the page opening, the app coming back). A watch is stopped and a check-in cancelled
//! from its row, and the list asked for again.
//!
//! What a row says and how the page groups and filters it are pure functions ([`when`],
//! [`filter`]), tested on their own.

pub(crate) mod filter;
pub(crate) mod when;

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::scheduled::ScheduledItem;
use emberhut_api::settings::SettingsAction;
use emberhut_api::time::Timestamp;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use self::filter::ScheduledFilter;
use super::poll::{self, Kicked, Kicker};
use super::presence::Presence;
use super::{Page, Store, update_or_log};
use crate::backend::Error;
use emberhut_api::ids::WatchId;

/// How often the list is asked for while its page is open in the front window.
const ON_PAGE: Duration = Duration::from_secs(60);
/// How often it is asked for otherwise, for the sidebar's count.
const IN_SIGHT: Duration = Duration::from_secs(300);

/// The list, as last fetched.
#[derive(Debug)]
pub(crate) struct ScheduledFeed {
    items: Option<Vec<ScheduledItem>>,
    problem: Option<String>,
    filter: ScheduledFilter,
    /// Wakes the loop; the loop's end of it waits here until the loop starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for ScheduledFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            items: None,
            problem: None,
            filter: ScheduledFilter::default(),
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl ScheduledFeed {
    /// The rows, once a list came.
    #[must_use]
    pub(crate) fn items(&self) -> Option<&[ScheduledItem]> {
        self.items.as_deref()
    }

    /// How many things are scheduled (the sidebar's count): none before a list came.
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        self.items.as_ref().map_or(0, Vec::len)
    }

    /// Why the last fetch failed, if it did.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The page's filters.
    #[must_use]
    pub(crate) fn filter(&self) -> &ScheduledFilter {
        &self.filter
    }

    /// Asks for the list now rather than at the next tick.
    pub(super) fn kick(&self) {
        self.kicker.kick();
    }

    /// Takes a fetch's outcome in; whether anything changed. A failure keeps the last list.
    fn take(&mut self, page: Result<Vec<ScheduledItem>, Error>) -> bool {
        match page {
            Ok(items) => {
                let changed = self.items.as_ref() != Some(&items) || self.problem.is_some();
                (self.items, self.problem) = (Some(items), None);
                changed
            }
            Err(error) => {
                let problem = Some(problem(&error));
                let changed = self.problem != problem;
                self.problem = problem;
                changed
            }
        }
    }
}

/// What a failed fetch says: a server without the list is on older code, one out of reach
/// can't be reached, else the list couldn't be read.
fn problem(error: &Error) -> String {
    match error {
        Error::Refused {
            status: Some(404), ..
        } => "The server is on older code: the full list comes with its next update.".to_owned(),
        Error::Unreachable(_) => format!("Can't reach {}", crate::brand::APP_NAME),
        _ => "Couldn't read the list".to_owned(),
    }
}

/// How long the loop waits before asking again: a minute on the page in front, five minutes
/// in sight, and only for a kick out of sight.
#[must_use]
fn interval(on_page: bool, presence: Presence) -> Option<Duration> {
    match presence {
        Presence::Hidden => None,
        Presence::Front if on_page => Some(ON_PAGE),
        Presence::Front | Presence::Behind => Some(IN_SIGHT),
    }
}

impl Store {
    /// What is scheduled.
    #[must_use]
    pub(crate) fn scheduled(&self) -> &ScheduledFeed {
        &self.scheduled
    }

    /// Sets the Scheduled page's filters.
    pub(crate) fn set_scheduled_filter(
        &mut self,
        filter: ScheduledFilter,
        cx: &mut Context<'_, Self>,
    ) {
        if self.scheduled.filter != filter {
            self.scheduled.filter = filter;
            cx.notify();
        }
    }

    /// Asks for the list again now (the page's refresh, after a change).
    pub(crate) fn reload_scheduled(&self) {
        self.scheduled.kick();
    }

    /// Stops the watch `watch`, then asks for the list again.
    pub(crate) fn stop_watch(&mut self, watch: WatchId, cx: &mut Context<'_, Self>) {
        self.change_scheduled(SettingsAction::StopWatch(watch), "Watch not stopped", cx);
    }

    /// Cancels the check-in set for `at`, then asks for the list again.
    pub(crate) fn cancel_checkin(&mut self, at: Timestamp, cx: &mut Context<'_, Self>) {
        self.change_scheduled(
            SettingsAction::CancelCheckin(at),
            "Check-in not cancelled",
            cx,
        );
    }

    /// Sends `action`; a refusal says `failure` over the composer. The list is asked for again
    /// either way.
    fn change_scheduled(
        &mut self,
        action: SettingsAction,
        failure: &'static str,
        cx: &mut Context<'_, Self>,
    ) {
        self.hide_action_problem(cx);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_spawn(async move { backend.settings(action) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = sent {
                    store.action_failed(failure, &error, cx);
                }
                store.scheduled.kick();
            });
        })
        .detach();
    }

    /// Starts the list's loop (once).
    pub(crate) fn start_scheduled(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.scheduled.kicked.take() {
            run(kicked, cx);
        }
    }

    /// How long the loop waits now, or `None` for only a kick.
    fn scheduled_interval(&self) -> Option<Duration> {
        interval(self.shows_page(Page::Scheduled), self.presence())
    }
}

/// Runs the list's loop for the store in `cx`: it fetches at once (so the sidebar's count is
/// there at launch) and whenever kicked, then each [`interval`] while one is due and the app is
/// in sight.
fn run(kicked: Kicked, cx: &mut Context<'_, Store>) {
    cx.spawn(async move |this, cx| {
        // At launch, and after a kick, the list is asked for wherever the app is.
        let mut asked = true;
        loop {
            let Some(due) = update_or_log(&this, cx, |store, _| {
                (asked || store.presence() != Presence::Hidden).then(|| Arc::clone(&store.backend))
            }) else {
                return;
            };
            if let Some(backend) = due {
                let page = cx
                    .background_spawn(async move { backend.scheduled() })
                    .await;
                update_or_log(&this, cx, |store, cx| {
                    if store.scheduled.take(page.map(|page| page.items)) {
                        cx.notify();
                    }
                });
            }
            let Some(wait) = update_or_log(&this, cx, |store, _| store.scheduled_interval()) else {
                return;
            };
            asked = if let Some(wait) = wait {
                let timer = cx.background_executor().timer(wait);
                smol::future::or(
                    async {
                        timer.await;
                        false
                    },
                    async {
                        kicked.wait().await;
                        true
                    },
                )
                .await
            } else {
                kicked.wait().await;
                true
            };
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::scheduled::ScheduledPage;
    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    fn page() -> ScheduledPage {
        serde_json::from_value(json!({"items": [{
            "id": "watch:3", "kind": "watch", "title": "A reply", "nextAt": "2026-11-24T12:00:00.000Z",
            "repeat": "every 4 h", "room": 1, "note": null, "noteAt": null, "tasks": [],
            "watch": 3, "checkin": null
        }]}))
        .unwrap()
    }

    #[test]
    fn a_failure_says_why_and_keeps_the_list() {
        let mut feed = ScheduledFeed::default();
        assert!(feed.take(Ok(page().items)));
        assert!(!feed.take(Ok(page().items)), "the same list again");
        assert!(feed.take(Err(Error::Refused {
            status: Some(404),
            message: None,
        })));
        assert_eq!(feed.count(), 1, "the last list stays");
        assert!(feed.problem().unwrap().contains("older code"));
        assert!(feed.take(Err(Error::Unreachable("down".into()))));
        assert!(feed.problem().unwrap().starts_with("Can't reach"));
    }

    #[test]
    fn the_list_is_asked_for_each_minute_on_its_page_and_less_in_sight() {
        assert_eq!(interval(true, Presence::Front), Some(ON_PAGE));
        assert_eq!(interval(false, Presence::Front), Some(IN_SIGHT));
        assert_eq!(interval(true, Presence::Behind), Some(IN_SIGHT));
        assert_eq!(interval(true, Presence::Hidden), None);
    }

    #[gpui_kit::test]
    fn the_list_comes_at_launch_and_a_watch_stops_from_its_row(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend.days.push_scheduled(Ok(page()));
        let store = cx.new(|_| Store::for_tests(backend.clone()));
        store.update(cx, Store::start_scheduled);
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert_eq!(store.scheduled().count(), 1);
            assert_eq!(store.scheduled().problem(), None);
        });

        store.update(cx, |store, cx| store.stop_watch(WatchId::new(3), cx));
        cx.run_until_parked();
        let calls = backend.calls();
        assert!(calls.contains(&Call::Settings(SettingsAction::StopWatch(WatchId::new(3)))));
        assert!(
            calls
                .iter()
                .skip_while(|call| !matches!(call, Call::Settings(_)))
                .any(|call| *call == Call::Scheduled),
            "asked for again after the change"
        );
        store.read_with(cx, |store, _| {
            assert_eq!(store.scheduled().count(), 0, "the watch is gone");
        });
    }
}
