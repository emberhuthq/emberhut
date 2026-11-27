//! Agents and their approval requests in the store: the pending requests (asked for every
//! 10 s while their list or a request's page shows in the front window, and at once when one
//! opens, the window comes forward, or the live stream says a request changed), each open
//! request's page as last read on its own, and the user's answers on them.
//!
//! A decision carries the payload hash of the request on screen, so a request that changed in
//! between decides nothing: the server refuses it (409), the request is read again and shown as
//! it is, and the refusal says why. Approve asks first; Reject and Withdraw (asked first too)
//! go one at a time. Every answer reads the state again, whose `approvals` and `agents` carry
//! the counts Needs you and the rooms show.
//!
//! The agents themselves come with the state: each but the default one has a room of its own.

pub(crate) mod lists;
pub(crate) mod shown;
pub(crate) mod words;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::{
    Approval, ApprovalReply, ApprovalState, ApprovalsEvent, ApprovalsPage, ApprovalsQuery,
    Decision, DecisionRequest, WithdrawRequest,
};
use emberhut_api::ids::{ApprovalId, ConfiguredAgentId, RoomId};
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use self::lists::{AgentRoom, Summary};
use super::poll::{self, Kicked, Kicker};
use super::presence::Presence;
use super::working::follow::{self, Feed};
use super::working::keys::{Key, Keys};
use super::{Page, Store, update_or_log};
use crate::backend::Error;

/// How often the pending requests are asked for while they show.
const EVERY: Duration = Duration::from_secs(10);

/// The key of an agent, for a list of its requests alone ([`Page::Approvals`]).
pub(crate) type AgentKey = Key<ConfiguredAgentId>;

/// What waits for the user's yes before it is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    /// An Approve, on the payload hash of the request on screen when it was asked.
    Approve {
        /// That hash: the decision carries it.
        payload_hash: String,
    },
    /// A withdrawal.
    Withdraw,
}

/// What became of the user's last answer on a request, for the line that says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    /// What it says.
    pub(crate) text: String,
    /// It went wrong (said in the danger colour).
    pub(crate) failed: bool,
}

/// Why a request's page has nothing to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// The server doesn't have it (404).
    Gone,
    /// It couldn't be read, worded for the user; reading it again may work.
    Failed(String),
}

/// The approval requests, as last read.
#[derive(Debug)]
pub(crate) struct ApprovalsFeed {
    /// The pending requests, newest first; none until the first read.
    pending: Option<Vec<Approval>>,
    /// Why the last read failed, worded for the user.
    problem: Option<String>,
    /// Each request whose page is open, as last read on its own or answered.
    seen: HashMap<ApprovalId, Approval>,
    /// The requests known to be no longer pending (decided, withdrawn, used, expired or gone):
    /// a read that started before that and lands after it never brings one back as pending,
    /// as a request never goes back to pending.
    settled: HashSet<ApprovalId>,
    /// Why an open request's page couldn't be read, until it is.
    unread: HashMap<ApprovalId, Unread>,
    /// The request whose answer is on its way.
    busy: Option<ApprovalId>,
    /// The answer waiting for the user's yes, and on which request.
    asking: Option<(ApprovalId, Ask)>,
    /// What became of the last answer, and on which request.
    notice: Option<(ApprovalId, Notice)>,
    agents: Keys<ConfiguredAgentId>,
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for ApprovalsFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            pending: None,
            problem: None,
            seen: HashMap::new(),
            settled: HashSet::new(),
            unread: HashMap::new(),
            busy: None,
            asking: None,
            notice: None,
            agents: Keys::default(),
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl ApprovalsFeed {
    /// Why the last read failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Whether the pending requests have been read at least once.
    #[must_use]
    pub(crate) fn has_read(&self) -> bool {
        self.pending.is_some()
    }

    /// The request whose answer is on its way.
    #[must_use]
    pub(crate) fn busy(&self) -> Option<ApprovalId> {
        self.busy
    }

    /// The answer on `id` waiting for the user's yes.
    #[must_use]
    pub(crate) fn asking(&self, id: ApprovalId) -> Option<&Ask> {
        self.asking
            .as_ref()
            .filter(|(on, _)| *on == id)
            .map(|(_, ask)| ask)
    }

    /// What became of the last answer on `id`.
    #[must_use]
    pub(crate) fn notice(&self, id: ApprovalId) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|(on, _)| *on == id)
            .map(|(_, notice)| notice)
    }

    /// Why request `id`'s page has nothing to show, if it couldn't be read.
    #[must_use]
    pub(crate) fn unread(&self, id: ApprovalId) -> Option<&Unread> {
        self.unread.get(&id)
    }

    /// The agent of `key`, for its list.
    #[must_use]
    pub(crate) fn agent_of(&self, key: AgentKey) -> Option<&ConfiguredAgentId> {
        self.agents.get(key)
    }

    /// Takes a read of the pending requests in; whether anything changed. A failure keeps the
    /// last list. A request known to be settled is left out: the read started before it was.
    /// An open request's page takes the list's copy of the others.
    fn take(&mut self, page: Result<ApprovalsPage, Error>) -> bool {
        let (pending, problem) = match page {
            Ok(mut page) => {
                page.approvals
                    .retain(|approval| !self.settled.contains(&approval.id));
                (Some(page.approvals), None)
            }
            Err(error) => (self.pending.take(), Some(words::read_problem(&error))),
        };
        for approval in pending.iter().flatten() {
            if let Some(seen) = self.seen.get_mut(&approval.id) {
                seen.clone_from(approval);
            }
        }
        let changed = self.pending != pending || self.problem != problem;
        (self.pending, self.problem) = (pending, problem);
        changed
    }

    /// Takes `approval` as it is now: its page shows it, and the list holds it as it is (one no
    /// longer pending leaves the list's groups). A pending copy of a request known to be settled
    /// is a read that started before it was, and is left out.
    fn take_one(&mut self, approval: Approval) {
        if approval.state == ApprovalState::Pending && self.settled.contains(&approval.id) {
            return;
        }
        if approval.state != ApprovalState::Pending {
            self.settled.insert(approval.id);
        }
        self.unread.remove(&approval.id);
        if let Some(listed) = self
            .pending
            .iter_mut()
            .flatten()
            .find(|listed| listed.id == approval.id)
        {
            listed.clone_from(&approval);
        }
        self.seen.insert(approval.id, approval);
    }

    /// Request `id` couldn't be read on its own: a 404 lets go of every copy of it (its page
    /// says it isn't there); anything else keeps what is shown and says why.
    fn take_unread(&mut self, id: ApprovalId, error: &Error) {
        if matches!(
            error,
            Error::Refused {
                status: Some(404),
                ..
            }
        ) {
            self.seen.remove(&id);
            self.settled.insert(id);
            if let Some(pending) = &mut self.pending {
                pending.retain(|approval| approval.id != id);
            }
            self.unread.insert(id, Unread::Gone);
        } else {
            self.unread
                .insert(id, Unread::Failed(words::read_one_problem(error)));
        }
    }
}

/// The read of the pending requests the list shows (every agent's: a list of one agent's
/// requests filters it).
fn pending_query() -> ApprovalsQuery {
    ApprovalsQuery {
        state: Some(ApprovalState::Pending),
        agent: None,
    }
}

/// Whether to ask for the pending requests now: while their list or a request's page shows,
/// in front.
#[must_use]
fn wanted(on_page: bool, presence: Presence) -> bool {
    on_page && presence == Presence::Front
}

/// Request `id` among `list`.
fn listed(list: &[Approval], id: ApprovalId) -> Option<&Approval> {
    list.iter().find(|approval| approval.id == id)
}

/// Whether `page` shows approval requests.
fn shows_approvals(page: Page) -> bool {
    matches!(page, Page::Approvals(_) | Page::Approval(_))
}

impl Store {
    /// Starts the pending requests' loop (once).
    pub(super) fn start_approvals(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.approvals.kicked.take() {
            follow::follow(
                Feed {
                    every: EVERY,
                    wanted: Self::approvals_wanted,
                    fetch: |backend| backend.approvals(&pending_query()),
                    take: Self::take_approvals,
                },
                kicked,
                cx,
            );
        }
    }

    /// Whether the pending requests are [`wanted`] now.
    fn approvals_wanted(&self) -> bool {
        wanted(self.columns.shows_any(shows_approvals), self.presence())
    }

    fn take_approvals(&mut self, page: Result<ApprovalsPage, Error>) -> bool {
        self.approvals.take(page)
    }

    /// The approval requests as last read.
    #[must_use]
    pub(crate) fn approvals(&self) -> &ApprovalsFeed {
        &self.approvals
    }

    /// The agents, the default agent first, as the state has them.
    #[must_use]
    pub(crate) fn agents(&self) -> &[AgentRow] {
        self.state
            .as_ref()
            .map_or(&[], |state| state.agents.as_slice())
    }

    /// The agents with rooms of their own, with their rooms' lines.
    #[must_use]
    pub(crate) fn agent_rooms(&self) -> Vec<AgentRoom<'_>> {
        self.state.as_ref().map_or_else(Vec::new, |state| {
            lists::agent_rooms(&state.agents, &state.rooms, &state.approvals)
        })
    }

    /// How many of agent `id`'s requests wait on the user, from the state's pending ones.
    #[must_use]
    pub(crate) fn waiting_on_you(&self, id: &ConfiguredAgentId) -> usize {
        self.state
            .as_ref()
            .map_or(0, |state| lists::waiting_on_you(&state.approvals, id))
    }

    /// The agent whose own room `room` is.
    #[must_use]
    pub(crate) fn agent_of_room(&self, room: RoomId) -> Option<&AgentRow> {
        lists::agent_of_room(self.agents(), room)
    }

    /// The pending requests: the list as last read, else the state's (which has the pending
    /// ones too, before the list's first read or from a server without the list).
    #[must_use]
    pub(crate) fn pending_approvals(&self) -> &[Approval] {
        match &self.approvals.pending {
            Some(pending) => pending,
            None => self
                .state
                .as_ref()
                .map_or(&[], |state| state.approvals.as_slice()),
        }
    }

    /// Request `id` as its page shows it: as last read on its own, else as listed.
    #[must_use]
    pub(crate) fn approval(&self, id: ApprovalId) -> Option<&Approval> {
        if self.approvals.unread(id) == Some(&Unread::Gone) {
            return None;
        }
        self.approvals
            .seen
            .get(&id)
            .or_else(|| listed(self.pending_approvals(), id))
            .or_else(|| listed(&self.state.as_ref()?.approvals, id))
    }

    /// Needs you's row of the state's pending requests.
    #[must_use]
    pub(crate) fn approvals_summary(&self) -> Summary {
        self.state
            .as_ref()
            .map_or_default(|state| Summary::of(&state.approvals))
    }

    /// The sidebar's Needs you count: the asks it counts and the requests waiting on the user.
    #[must_use]
    pub(crate) fn needs_count(&self) -> usize {
        self.needs().map_or(0, |needs| needs.count()) + self.approvals_summary().yours
    }

    /// The page key of agent `id`, to open its requests alone.
    pub(crate) fn agent_key(&mut self, id: ConfiguredAgentId) -> AgentKey {
        self.approvals.agents.key(id)
    }

    /// The requests whose pages are in a column, shown or under another.
    fn open_approvals(&self) -> Vec<ApprovalId> {
        self.columns
            .held()
            .filter_map(|page| match page {
                Page::Approval(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// A column shows `page` afresh: the pending requests are asked for now, and a request's
    /// own page reads it again.
    pub(super) fn approvals_page_shown(&self, page: Option<Page>, cx: &mut Context<'_, Self>) {
        match page {
            Some(Page::Approval(id)) => {
                self.approvals.kick();
                self.read_approval(id, cx);
            }
            Some(Page::Approvals(_)) => self.approvals.kick(),
            _ => {}
        }
    }

    /// Lets go of the requests read for pages no longer in a column, and of a question or a
    /// line about one of them.
    pub(super) fn forget_closed_approvals(&mut self) {
        let open = self.open_approvals();
        let feed = &mut self.approvals;
        feed.seen.retain(|id, _| open.contains(id));
        feed.unread.retain(|id, _| open.contains(id));
        if feed
            .asking
            .as_ref()
            .is_some_and(|(id, _)| !open.contains(id))
        {
            feed.asking = None;
        }
        if feed
            .notice
            .as_ref()
            .is_some_and(|(id, _)| !open.contains(id))
        {
            feed.notice = None;
        }
    }

    /// The window came to the front: what shows is read at once.
    pub(super) fn approvals_forward(&self) {
        self.approvals.kick();
    }

    /// The live stream says a request changed: the state (its counts) and what shows approval
    /// requests are read again.
    pub(super) fn approvals_changed(&self, changed: &ApprovalsEvent, cx: &mut Context<'_, Self>) {
        log::debug!(
            "approval {} is {} now; {} wait on the user",
            changed.id,
            changed.state,
            changed.pending
        );
        self.kicker.kick();
        self.approvals.kick();
        for id in self.open_approvals() {
            self.read_approval(id, cx);
        }
    }

    /// Reads request `id` again after a read failed (its page's Retry).
    pub(crate) fn retry_approval(&mut self, id: ApprovalId, cx: &mut Context<'_, Self>) {
        self.approvals.unread.remove(&id);
        self.read_approval(id, cx);
        cx.notify();
    }

    /// Reads request `id` on its own, for its page.
    fn read_approval(&self, id: ApprovalId, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move { backend.approval(id) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                match read {
                    Ok(ApprovalReply { approval }) => store.approvals.take_one(approval),
                    Err(error) => {
                        log::warn!("couldn't read approval {id}: {error}");
                        store.approvals.take_unread(id, &error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for the user's yes to `ask` on a request (`None`: they said no).
    pub(crate) fn ask_approval(
        &mut self,
        ask: Option<(ApprovalId, Ask)>,
        cx: &mut Context<'_, Self>,
    ) {
        self.approvals.asking = ask;
        cx.notify();
    }

    /// The user's answer on request `id`, on `payload_hash`, the hash of the request they
    /// were shown; one answer at a time.
    pub(crate) fn decide_approval(
        &mut self,
        id: ApprovalId,
        decision: Decision,
        payload_hash: String,
        cx: &mut Context<'_, Self>,
    ) {
        let request = DecisionRequest {
            decision,
            payload_hash,
            reason: None,
        };
        self.answer_approval(id, Some(decision), cx, move |backend| {
            backend.decide_approval(id, &request)
        });
    }

    /// Withdraws request `id` (the user said yes); one answer at a time.
    pub(crate) fn withdraw_approval(&mut self, id: ApprovalId, cx: &mut Context<'_, Self>) {
        self.answer_approval(id, None, cx, move |backend| {
            backend.withdraw_approval(id, &WithdrawRequest::default())
        });
    }

    /// Sends `send`, the user's `decision` on `id` (none: a withdrawal), and takes its answer
    /// in: the request as it is after, or why not, with the request read again when it
    /// changed. The state and the list are read again either way.
    fn answer_approval(
        &mut self,
        id: ApprovalId,
        decision: Option<Decision>,
        cx: &mut Context<'_, Self>,
        send: impl FnOnce(&dyn crate::backend::Backend) -> Result<ApprovalReply, Error> + Send + 'static,
    ) {
        let feed = &mut self.approvals;
        if feed.busy.is_some() {
            return;
        }
        feed.asking = None;
        feed.notice = None;
        feed.busy = Some(id);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let (outcome, again) = cx
                .background_spawn(async move {
                    let outcome = send(backend.as_ref()).map_err(|error| words::failure(&error));
                    let again = match &outcome {
                        Err(failure) if failure.refresh => Some(backend.approval(id)),
                        _ => None,
                    };
                    (outcome, again)
                })
                .await;
            update_or_log(&this, cx, |store, cx| {
                let feed = &mut store.approvals;
                feed.busy = None;
                let notice = match outcome {
                    Ok(ApprovalReply { approval }) => {
                        feed.take_one(approval);
                        Notice {
                            text: words::decided(decision).to_owned(),
                            failed: false,
                        }
                    }
                    Err(failure) => Notice {
                        text: failure.message,
                        failed: true,
                    },
                };
                feed.notice = Some((id, notice));
                match again {
                    Some(Ok(ApprovalReply { approval })) => feed.take_one(approval),
                    Some(Err(error)) => {
                        log::warn!("couldn't read approval {id} again: {error}");
                        feed.take_unread(id, &error);
                    }
                    None => {}
                }
                feed.kick();
                store.kicker.kick();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl ApprovalsFeed {
    /// Asks for the pending requests now if they are wanted, rather than at the next tick.
    fn kick(&self) {
        self.kicker.kick();
    }
}

#[cfg(test)]
mod tests;
