//! The machines page's data: asked for every 10 s while the page is open in the front window,
//! at once when the page opens or the window comes back to the front, and once at launch so it
//! opens filled (that one isn't retried off the page if it fails). Off the page or out of the
//! front, the loop sleeps until one of those kicks it.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::poll::{self, Kicked, Kicker};
use super::presence::Presence;
use super::{Page, Store, update_or_log};
use emberhut_api::machines::MachinesPage;

/// How often the machines are asked for while their page is open.
const EVERY: Duration = Duration::from_secs(10);

/// The machines, as last fetched.
#[derive(Debug)]
pub(crate) struct MachinesFeed {
    page: Option<MachinesPage>,
    problem: Option<String>,
    /// Wakes the loop; the loop's end of it waits here until the loop starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for MachinesFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            page: None,
            problem: None,
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl MachinesFeed {
    /// The last page, if one came.
    #[must_use]
    pub(crate) fn page(&self) -> Option<&MachinesPage> {
        self.page.as_ref()
    }

    /// Why the last fetch failed, if it did.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Asks for the machines now if they are wanted, rather than at the next tick.
    pub(super) fn kick(&self) {
        self.kicker.kick();
    }

    /// Takes a fetch's outcome in; whether anything changed. A failure keeps the last page.
    fn take(&mut self, page: Result<MachinesPage, crate::backend::Error>) -> bool {
        match page {
            Ok(page) => {
                let changed = self.page.as_ref() != Some(&page) || self.problem.is_some();
                (self.page, self.problem) = (Some(page), None);
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

/// What a failed fetch of the machines says: a server without the page is on older code; the
/// server's own words when it gave them.
fn problem(error: &crate::backend::Error) -> String {
    match error {
        crate::backend::Error::Refused {
            status: Some(404), ..
        } => "The server is on older code: it has no machines page yet.".to_owned(),
        crate::backend::Error::Refused {
            message: Some(message),
            ..
        } => message.clone(),
        _ => format!("Couldn't read the machines: {error}"),
    }
}

/// Whether to ask for the machines now: the `first` time (so the page opens filled), and then
/// while their page is open (`on_page`) in the front window. Behind another app's window or out
/// of sight no one watches them.
#[must_use]
fn wanted(first: bool, on_page: bool, presence: Presence) -> bool {
    first || (on_page && presence == Presence::Front)
}

impl Store {
    /// Whether the machines are [`wanted`] now; `first` for the fetch at launch.
    fn machines_wanted(&self, first: bool) -> bool {
        wanted(first, self.shows_page(Page::Machines), self.presence())
    }
}

/// Runs the machines' loop for the store in `cx`. It fetches when they are [`wanted`], then
/// waits [`EVERY`] (or a kick) while they still are, and only a kick while they aren't: what
/// makes them wanted again (their page opening, the window coming to the front) kicks it.
fn run(kicked: Kicked, cx: &mut Context<'_, Store>) {
    cx.spawn(async move |this, cx| {
        loop {
            let Some(due) = update_or_log(&this, cx, |store, _| {
                let first = store.machines.page.is_none() && store.machines.problem.is_none();
                store
                    .machines_wanted(first)
                    .then(|| Arc::clone(&store.backend))
            }) else {
                return;
            };
            if let Some(backend) = due {
                let page = cx.background_spawn(async move { backend.machines() }).await;
                update_or_log(&this, cx, |store, cx| {
                    if store.machines.take(page) {
                        cx.notify();
                    }
                });
            }
            let Some(again) = update_or_log(&this, cx, |store, _| store.machines_wanted(false))
            else {
                return;
            };
            if again {
                let timer = cx.background_executor().timer(EVERY);
                smol::future::or(
                    async {
                        timer.await;
                    },
                    kicked.wait(),
                )
                .await;
            } else {
                kicked.wait().await;
            }
        }
    })
    .detach();
}

impl Store {
    /// Starts the machines' loop (once).
    pub(super) fn start_machines(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.machines.kicked.take() {
            run(kicked, cx);
        }
    }

    /// The machines.
    #[must_use]
    pub(crate) fn machines(&self) -> &MachinesFeed {
        &self.machines
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backend::Error;

    fn page(name: &str) -> MachinesPage {
        serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "machines": [
            {"id": "a", "name": name, "state": "ok", "status": "Fine"}]}))
        .unwrap()
    }

    #[test]
    fn a_failure_says_what_the_server_said_or_that_it_is_older() {
        let refused = |status, message: Option<&str>| Error::Refused {
            status: Some(status),
            message: message.map(str::to_owned),
        };
        assert_eq!(
            problem(&refused(404, None)),
            "The server is on older code: it has no machines page yet."
        );
        assert_eq!(problem(&refused(500, Some("Runner down"))), "Runner down");
        assert!(
            problem(&Error::Unreachable("down".into())).starts_with("Couldn't read the machines")
        );
    }

    #[test]
    fn the_machines_are_asked_for_at_launch_and_on_their_page_in_front() {
        assert!(
            wanted(true, false, Presence::Hidden),
            "at launch, wherever the window is"
        );
        assert!(wanted(false, true, Presence::Front));
        assert!(!wanted(false, false, Presence::Front), "another page");
        assert!(
            !wanted(false, true, Presence::Behind),
            "the user is in another app"
        );
        assert!(
            !wanted(false, true, Presence::Hidden),
            "minimised or hidden"
        );
    }

    #[test]
    fn only_a_change_counts() {
        let mut feed = MachinesFeed::default();
        assert!(feed.take(Ok(page("Example"))));
        assert!(!feed.take(Ok(page("Example"))), "the same page again");
        assert!(feed.take(Err(Error::Unreachable("down".into()))));
        assert_eq!(
            feed.page().map(|page| page.machines.len()),
            Some(1),
            "the last page stays"
        );
        assert!(feed.problem().is_some());
        assert!(
            !feed.take(Err(Error::Unreachable("down".into()))),
            "the same problem again"
        );
        assert!(feed.take(Ok(page("Example"))));
        assert_eq!(feed.problem(), None);
    }
}
