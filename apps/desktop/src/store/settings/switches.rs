//! The MCP servers and skills with the user's switches: asked for every 10 s while Settings is
//! open in the front window, at once when it opens or the window comes back to the front, and
//! not at all otherwise. A switch (or a server's Reconnect) goes one at a time: the row on its
//! way shows a spinner and every switch waits, and the page comes back as it is after.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::store::poll::{self, Kicked, Kicker};
use crate::store::presence::Presence;
use crate::store::{Store, update_or_log};
use emberhut_api::switches::{McpServer, McpState, SwitchAction, SwitchPage};

/// How often the switches are asked for while Settings is open.
const EVERY: Duration = Duration::from_secs(10);

/// The switches, as last fetched.
#[derive(Debug)]
pub(crate) struct SwitchesFeed {
    page: Option<SwitchPage>,
    problem: Option<String>,
    /// The server or skill whose switch (or Reconnect) is on its way.
    busy: Option<String>,
    /// The server or skill shown in full.
    open: Option<String>,
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for SwitchesFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            page: None,
            problem: None,
            busy: None,
            open: None,
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl SwitchesFeed {
    /// The last page, if one came.
    #[must_use]
    pub(crate) fn page(&self) -> Option<&SwitchPage> {
        self.page.as_ref()
    }

    /// Why the last fetch or switch failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The server or skill whose switch is on its way, while one is.
    #[must_use]
    pub(crate) fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    /// Whether the row of `key` ([`row_key`]) is open in full.
    #[must_use]
    pub(crate) fn is_open(&self, key: &str) -> bool {
        self.open.as_deref() == Some(key)
    }

    /// Asks for the switches now if they are wanted, rather than at the next tick.
    pub(in crate::store) fn kick(&self) {
        self.kicker.kick();
    }

    /// Takes a fetch's outcome in; whether anything changed. A failure keeps the last page.
    fn take(&mut self, page: Result<SwitchPage, Error>) -> bool {
        let (page, problem) = match page {
            Ok(page) => (Some(page), None),
            Err(error) => (self.page.take(), Some(read_problem(&error))),
        };
        let changed = self.page != page || self.problem != problem;
        (self.page, self.problem) = (page, problem);
        changed
    }

    /// Takes a switch's outcome in: the page as it is after, or why it wasn't done.
    fn take_switched(&mut self, page: Result<SwitchPage, Error>) {
        self.busy = None;
        match page {
            Ok(page) => (self.page, self.problem) = (Some(page), None),
            Err(error) => self.problem = Some(switch_problem(&error)),
        }
    }
}

/// The key of a row that opens in place: servers and skills apart, as a server and a skill may
/// share a name.
#[must_use]
pub(crate) fn row_key(kind: RowKind, name: &str) -> String {
    match kind {
        RowKind::Server => format!("mcp:{name}"),
        RowKind::Skill => format!("skill:{name}"),
    }
}

/// What a row of the switches is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// An MCP server.
    Server,
    /// A skill.
    Skill,
}

/// Why the switches couldn't be read, as the reference client words it.
fn read_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        Error::Refused {
            status: Some(404), ..
        } => format!("{APP_NAME}'s server is on older code: this comes with its next update"),
        other => other.server_message().map_or_else(
            || format!("Couldn't read {APP_NAME}'s servers and skills"),
            str::to_owned,
        ),
    }
}

/// Why a switch wasn't done: the server's words, else what went wrong.
fn switch_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        other => other
            .server_message()
            .map_or_else(|| format!("{APP_NAME} couldn't do that"), str::to_owned),
    }
}

/// What a server is now, in words.
#[must_use]
pub(crate) fn server_status(server: &McpServer) -> String {
    match &server.state {
        McpState::Connected => "Connected".to_owned(),
        McpState::Failed => server
            .error
            .as_deref()
            .map_or_else(|| "Failed".to_owned(), |error| format!("Failed: {error}")),
        McpState::NeedsAuth => "Needs a sign-in".to_owned(),
        McpState::Pending => "Connecting…".to_owned(),
        McpState::Disabled => "Turned off in Claude Code".to_owned(),
        McpState::Off => "Off".to_owned(),
        _ => "Not loaded right now".to_owned(),
    }
}

/// How a server's state shows: its dot's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateTone {
    /// Connected.
    Good,
    /// Failed.
    Bad,
    /// Waiting on a sign-in or connecting.
    Waiting,
    /// Off, or not loaded.
    Quiet,
}

/// The tone of a server's state.
#[must_use]
pub(crate) fn server_tone(server: &McpServer) -> StateTone {
    match server.state {
        McpState::Connected => StateTone::Good,
        McpState::Failed => StateTone::Bad,
        McpState::NeedsAuth | McpState::Pending => StateTone::Waiting,
        _ => StateTone::Quiet,
    }
}

/// A server's kind and where it is configured, as one line: `http · your Claude config`.
#[must_use]
pub(crate) fn server_origin(server: &McpServer) -> String {
    let source = server.source.as_deref().map(|source| {
        if source == "user" {
            "your Claude config"
        } else {
            source
        }
    });
    [server.kind.as_deref(), source]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The MCP section's footer: the page's own note, else which account the servers are on and
/// what Off does.
#[must_use]
pub(crate) fn servers_footer(page: &SwitchPage) -> String {
    if let Some(note) = &page.note {
        return note.clone();
    }
    let account = page
        .account
        .as_deref()
        .map_or_default(|account| format!(" (on {account})"));
    format!(
        "What {APP_NAME}'s own session has{account}. Off: {APP_NAME}'s sessions start again \
         without it once their turn is over; the runners' at their next start."
    )
}

/// The skills section's footer.
#[must_use]
pub(crate) fn skills_footer() -> String {
    format!(
        "Off: {APP_NAME}, its workers and its runner sessions don't see it. Your own sessions \
         keep it."
    )
}

/// Whether the switches are wanted now: while Settings' front door or Connections is open in
/// the front window.
#[must_use]
fn wanted(on_page: bool, presence: Presence) -> bool {
    on_page && presence == Presence::Front
}

impl Store {
    /// Whether the switches are [`wanted`] now.
    fn switches_wanted(&self) -> bool {
        let on_page = self.shows_settings(&[super::SettingsPane::Connections]);
        wanted(on_page, self.presence())
    }

    /// The switches.
    #[must_use]
    pub(crate) fn switches(&self) -> &SwitchesFeed {
        &self.settings.switches
    }

    /// Opens a server's or skill's row in full, or folds it again.
    pub(crate) fn toggle_switch_row(&mut self, key: String, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.switches;
        feed.open = if feed.open.as_deref() == Some(key.as_str()) {
            None
        } else {
            Some(key)
        };
        cx.notify();
    }

    /// Turns a server or skill on or off, or reconnects a server; ignored while another one is
    /// on its way.
    pub(crate) fn switch(&mut self, action: SwitchAction, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.switches;
        if feed.busy.is_some() {
            return;
        }
        feed.busy = Some(action.name().to_owned());
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { backend.switch(&action) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.settings.switches.take_switched(page);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Starts the switches' loop (once).
    pub(in crate::store) fn start_switches(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.settings.switches.kicked.take() {
            run(kicked, cx);
        }
    }
}

/// Runs the switches' loop: it fetches while they are [`wanted`], every [`EVERY`] or at a kick,
/// and waits for a kick while they aren't (Settings opening, the window coming forward).
fn run(kicked: Kicked, cx: &mut Context<'_, Store>) {
    cx.spawn(async move |this, cx| {
        loop {
            let Some(due) = update_or_log(&this, cx, |store, _| {
                store.switches_wanted().then(|| Arc::clone(&store.backend))
            }) else {
                return;
            };
            if let Some(backend) = due {
                let page = cx.background_spawn(async move { backend.switches() }).await;
                update_or_log(&this, cx, |store, cx| {
                    if store.settings.switches.take(page) {
                        cx.notify();
                    }
                });
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page() -> SwitchPage {
        serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "account": "main",
            "live": true, "mcp": [], "skills": []}))
        .unwrap()
    }

    fn server(state: &str, error: Option<&str>) -> McpServer {
        serde_json::from_value(json!({"name": "files", "state": state, "error": error,
            "source": "user", "kind": "http", "on": true, "fixed": false}))
        .unwrap()
    }

    #[test]
    fn the_switches_are_asked_for_on_settings_in_front() {
        assert!(wanted(true, Presence::Front));
        assert!(!wanted(true, Presence::Behind));
        assert!(!wanted(false, Presence::Front));
    }

    #[test]
    fn a_server_says_what_it_is_now() {
        assert_eq!(
            server_status(&server("failed", Some("refused"))),
            "Failed: refused"
        );
        assert_eq!(
            server_status(&server("needs-auth", None)),
            "Needs a sign-in"
        );
        assert_eq!(server_status(&server("pending", None)), "Connecting…");
        assert_eq!(
            server_status(&server("disabled", None)),
            "Turned off in Claude Code"
        );
        assert_eq!(
            server_status(&server("something-new", None)),
            "Not loaded right now"
        );
        assert_eq!(server_tone(&server("pending", None)), StateTone::Waiting);
        assert_eq!(
            server_origin(&server("connected", None)),
            "http · your Claude config"
        );
    }

    #[test]
    fn a_failed_read_says_why_and_keeps_the_page() {
        let mut feed = SwitchesFeed::default();
        assert!(feed.take(Ok(page())));
        assert!(!feed.take(Ok(page())), "the same page again");
        assert!(feed.take(Err(Error::Refused {
            status: Some(404),
            message: None
        })));
        assert!(feed.page().is_some(), "the last page stays");
        assert!(feed.problem().unwrap().contains("older code"));
        feed.take(Err(Error::Unreachable("down".into())));
        assert_eq!(feed.problem(), Some("Can't reach Emberhut"));
        feed.take(Err(Error::Refused {
            status: Some(500),
            message: Some("broken".into()),
        }));
        assert_eq!(feed.problem(), Some("broken"));
    }

    #[test]
    fn a_switch_ends_its_wait_either_way() {
        let mut feed = SwitchesFeed {
            busy: Some("files".into()),
            ..SwitchesFeed::default()
        };
        feed.take_switched(Err(Error::Refused {
            status: Some(400),
            message: None,
        }));
        assert_eq!(feed.busy(), None);
        assert_eq!(feed.problem(), Some("Emberhut couldn't do that"));
        feed.busy = Some("files".into());
        feed.take_switched(Ok(page()));
        assert_eq!((feed.busy(), feed.problem()), (None, None));
    }

    #[test]
    fn the_footer_names_the_account() {
        let mut page = page();
        assert!(servers_footer(&page).contains("(on main)"));
        page.note = Some("What it had last".into());
        assert_eq!(servers_footer(&page), "What it had last");
    }
}
