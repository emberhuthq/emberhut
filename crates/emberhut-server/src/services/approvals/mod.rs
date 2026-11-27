//! Approvals: every OK an agent's outward step needs, for exactly the payload the server hashed,
//! decided once by the approver the agent's policy names, and used once.
//!
//! - A step asks ([`ApprovalService::request`]): its kind is raised by the fixed core when its
//!   words are about money, a credential, a secret or account security; the policy names who
//!   approves; the files are read by the server from the paths given; a payment must fit the
//!   agent's monthly cap. The request is made, or the pending one for the same payload found,
//!   and a new one is routed to its approver (`route`).
//! - A decision counts only as its channel proves it (`decide`): the owner from a signed-in
//!   device, a Slack button under Slack's signature from the approver's own team and user, a
//!   one-time signed link, or the approver's app under their HMAC. An agent's word never does.
//! - A step uses an approval once (`using`), hashed again from the step itself, while the
//!   agent's policy still names the approver who gave it; [`ApprovalService::gate`] is the
//!   same for the guard rails, asking when nothing matches.
//! - Requests expire in the background ([`ApprovalService::keep_expiring`]); every change is
//!   an `approvals` event on the live stream ([`ApprovalEvents`]).

pub mod channels;

mod decide;
mod events;
pub(crate) mod files;
mod route;
mod secrets;
mod using;
mod view;

use std::path::PathBuf;
use std::sync::Arc;

use emberhut_api::approvals::{Approval, ApprovalsEvent};
use emberhut_core::agents::{Agent, AgentId};
use emberhut_core::approvals::fixed_core::raise;
use emberhut_core::approvals::{
    ActionKind, ApprovalRequest, Approver, MonthKey, NewApproval, RequestError, RequestState,
    within_cap,
};
use emberhut_core::task::TaskId;
use emberhut_core::{ApprovalId, Clock, RandomSource};
use emberhut_db::{
    AgentStore, AgentStoreError, ApprovalFilter, ApprovalStore, ApprovalStoreError, DbError,
    OverCap,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use tokio_util::task::TaskTracker;

use self::channels::{ApprovalDeps, SlackDm, Webhook};
pub use self::decide::{InboundCall, LinkView};
pub use self::events::ApprovalEvents;
use self::files::StepFile;
pub use self::files::{FILE_MAX, FileRefused};
pub use self::route::Delivery;
use self::secrets::SecretFiles;
pub use self::secrets::{LINK_KEY, SecretError};
pub use self::using::{Gate, StepUse, UseOutcome};
use crate::services::agents::scope::AgentRoots;
use crate::services::private::PrivatePaths;

/// Why an approval could not be asked for, decided, withdrawn or used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalError {
    /// There is no such agent.
    #[error("no agent {0}")]
    NoAgent(AgentId),
    /// The agent is disabled: it asks for and uses no approval.
    #[error("agent {0} is disabled")]
    Disabled(AgentId),
    /// There is no such request (or not one of this agent's).
    #[error("no such approval request")]
    NotFound,
    /// A step's file can't be sent.
    #[error(transparent)]
    File(#[from] FileRefused),
    /// The request is not what a request may be.
    #[error(transparent)]
    Invalid(#[from] RequestError),
    /// A payment the agent's monthly cap does not leave room for.
    #[error(transparent)]
    OverCap(#[from] OverCap),
    /// A call that can't be read.
    #[error("{0}")]
    Malformed(&'static str),
    /// A call that doesn't prove where it came from: no signature, a wrong one, a stale one.
    #[error("{0}")]
    Unproven(&'static str),
    /// A decision from someone who is not the request's approver.
    #[error("{0}")]
    NotYours(&'static str),
    /// A decision on a payload hash that isn't the request's.
    #[error("the payload hash is not the request's: the request changed, or was not shown")]
    Mismatch,
    /// The request no longer takes this: decided, withdrawn, used or expired.
    #[error("the request is no longer open: it was decided, withdrawn or used, or has expired")]
    Closed,
    /// A signed link that no longer works, or never did.
    #[error("this link has expired or was already used")]
    Gone,
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// A secret could not be read or made.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The approvals' store failed otherwise.
    #[error("the approvals' store failed")]
    Store(#[source] ApprovalStoreError),
    /// The agents' store failed otherwise.
    #[error("the agents' store failed")]
    AgentStore(#[source] AgentStoreError),
}

impl From<ApprovalStoreError> for ApprovalError {
    fn from(error: ApprovalStoreError) -> Self {
        match error {
            ApprovalStoreError::Invalid(error) => Self::Invalid(error),
            ApprovalStoreError::NoAgent(agent) => Self::NoAgent(agent),
            ApprovalStoreError::OverCap(over) => Self::OverCap(over),
            ApprovalStoreError::Disabled(agent) => Self::Disabled(agent),
            ApprovalStoreError::Db(error) => Self::Db(error),
            other => Self::Store(other),
        }
    }
}

impl From<AgentStoreError> for ApprovalError {
    fn from(error: AgentStoreError) -> Self {
        match error {
            AgentStoreError::NoAgent(agent) => Self::NoAgent(agent),
            AgentStoreError::Db(error) => Self::Db(error),
            other => Self::AgentStore(other),
        }
    }
}

/// What a step asks approval for, as the agent (or the guard rails) gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepAsk {
    /// The kind of action, before the fixed core reads its words.
    pub kind: ActionKind,
    /// Where it goes.
    pub target: String,
    /// One line of the agent's.
    pub summary: String,
    /// The exact words, if any.
    pub text: Option<String>,
    /// The absolute paths of its files, read by the server.
    pub files: Vec<PathBuf>,
    /// A payment's amount, in cents.
    pub amount_cents: Option<u64>,
    /// The task whose step asks, if one does.
    pub task: Option<TaskId>,
}

/// A request asked for: made now or found pending, and how its approver hears of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requested {
    /// The request.
    pub request: ApprovalRequest,
    /// Whether it was made now (not found pending).
    pub made: bool,
    /// How its approver hears of it.
    pub delivery: Delivery,
}

/// What asking for an approval came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Nobody approves this kind for the agent: the step needs no request.
    NotNeeded(ActionKind),
    /// The request waits on its approver.
    Waiting(Box<Requested>),
}

/// What the approvals service is built from.
#[derive(Clone, Debug)]
pub struct ApprovalParts {
    /// The requests.
    pub approvals: ApprovalStore,
    /// The agents, their policies, channels and approvers.
    pub agents: AgentStore,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where link nonces and keys come from.
    pub random: Arc<dyn RandomSource>,
    /// The server's time zone: the month a payment counts in.
    pub zone: TimeZone,
    /// The secrets directory.
    pub secrets_dir: PathBuf,
    /// What never leaves the server: no request carries one of these files.
    pub private: PrivatePaths,
    /// The outside services requests are routed through.
    pub deps: ApprovalDeps,
    /// The folders a configured agent's files may come from.
    pub roots: AgentRoots,
}

/// Asks for, routes, decides, uses and expires approvals. Cheap to clone.
#[derive(Clone, Debug)]
pub struct ApprovalService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    approvals: ApprovalStore,
    agents: AgentStore,
    clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
    zone: TimeZone,
    secrets: SecretFiles,
    private: PrivatePaths,
    roots: AgentRoots,
    slack: Arc<dyn SlackDm>,
    webhook: Arc<dyn Webhook>,
    public_url: Option<String>,
    events: ApprovalEvents,
    /// What follows a decision (its live event, its DM's update), done in the background.
    followups: TaskTracker,
}

/// A request ready to be made: its agent, the request, and its files as read.
#[derive(Clone, Debug)]
struct Ready {
    agent: Agent,
    new: NewApproval,
    files: Vec<StepFile>,
}

/// A step's ask, read: nobody approves it, or it is ready to be made.
#[derive(Clone, Debug)]
enum Prepared {
    NotNeeded(ActionKind),
    Ready(Box<Ready>),
}

impl ApprovalService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: ApprovalParts) -> Self {
        let ApprovalParts {
            approvals,
            agents,
            clock,
            random,
            zone,
            secrets_dir,
            private,
            deps,
            roots,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                approvals,
                agents,
                clock,
                random,
                zone,
                secrets: SecretFiles::new(secrets_dir),
                private,
                roots,
                slack: deps.slack,
                webhook: deps.webhook,
                public_url: deps
                    .public_url
                    .map(|url| url.trim_end_matches('/').to_owned())
                    .filter(|url| !url.is_empty()),
                events: ApprovalEvents::default(),
                followups: TaskTracker::new(),
            }),
        }
    }

    /// The live stream's approvals events.
    #[must_use]
    pub fn events(&self) -> &ApprovalEvents {
        &self.inner.events
    }

    /// Waits until what follows every decision so far (its live event, its DM's update) is
    /// done.
    pub async fn settled(&self) {
        let followups = &self.inner.followups;
        followups.close();
        followups.wait().await;
        followups.reopen();
    }

    /// What an agent is told of an approval it used: whose (as a role, never who) and when.
    #[must_use]
    pub fn receipt(&self, used: &ApprovalRequest) -> String {
        let at = used.decided.as_ref().map_or_else(String::new, |decided| {
            format!(
                " at {}",
                self.inner.zone.to_datetime(decided.at).strftime("%H:%M")
            )
        });
        format!(
            "Used request {} on {}'s approval{at}.",
            used.id,
            used.approver.role()
        )
    }

    fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    fn month(&self, at: Timestamp) -> MonthKey {
        MonthKey::of(at, &self.inner.zone)
    }

    /// Asks for an approval of `ask` for `agent`, which is the agent whose room's token called
    /// (never an argument of the call): the request made, or the pending one for the same
    /// payload; or that nobody approves this kind for the agent.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NoAgent`], [`ApprovalError::Disabled`] for a disabled agent,
    /// [`ApprovalError::File`] for a file that can't be sent,
    /// [`ApprovalError::Invalid`], [`ApprovalError::OverCap`] for a payment the month's cap
    /// does not leave room for, or the database's.
    pub async fn request(&self, agent: &AgentId, ask: &StepAsk) -> Result<Asked, ApprovalError> {
        match self.prepare(agent, ask).await? {
            Prepared::NotNeeded(kind) => Ok(Asked::NotNeeded(kind)),
            Prepared::Ready(ready) => self
                .make(*ready)
                .await
                .map(|made| Asked::Waiting(Box::new(made))),
        }
    }

    /// The folders `agent`'s files must be in: a configured agent's roots; none for the
    /// default agent, whose steps are the owner's own.
    async fn within(&self, agent: &Agent) -> Result<Option<Vec<PathBuf>>, ApprovalError> {
        if agent.id.is_main() {
            return Ok(None);
        }
        Ok(Some(self.inner.roots.of(agent).await?))
    }

    /// Reads `ask` for `agent`: the fixed core's raise, then the agent's policy for the raised
    /// kind, the files, the request's own checks, and a payment's cap.
    async fn prepare(&self, agent: &AgentId, ask: &StepAsk) -> Result<Prepared, ApprovalError> {
        let agents = &self.inner.agents;
        let agent = agents
            .get(agent)
            .await?
            .ok_or_else(|| ApprovalError::NoAgent(agent.clone()))?;
        if !agent.enabled {
            return Err(ApprovalError::Disabled(agent.id));
        }
        let kind = raise(ask.kind, ask.text.as_deref(), &ask.summary);
        let approver = agents.approver_for(&agent.id, kind).await?;
        if approver == Approver::Nobody {
            return Ok(Prepared::NotNeeded(kind));
        }
        let within = self.within(&agent).await?;
        let files = files::read_all(&ask.files, &self.inner.private, within.as_deref()).await?;
        let new = NewApproval {
            agent: agent.id.clone(),
            kind,
            target: ask.target.clone(),
            summary: ask.summary.clone(),
            text: ask.text.clone(),
            files: files.iter().map(|file| file.digest.clone()).collect(),
            approver,
            amount_cents: ask.amount_cents,
            task: ask.task,
        };
        new.validate()?;
        if let Some(amount) = new.amount_cents {
            let month = self.month(self.now());
            let spent = self.inner.approvals.month_spend(&agent.id, &month).await?;
            if !within_cap(spent, amount, agent.spend_cap.as_ref()) {
                return Err(OverCap {
                    amount_cents: amount,
                    spent_cents: spent,
                    cap_cents: agent.spend_cap.as_ref().map_or(0, |cap| cap.cents),
                }
                .into());
            }
        }
        Ok(Prepared::Ready(Box::new(Ready { agent, new, files })))
    }

    /// Makes the request (or finds the pending one), and routes a new one to its approver.
    async fn make(&self, ready: Ready) -> Result<Requested, ApprovalError> {
        let (request, made) = self
            .inner
            .approvals
            .create_or_find_pending(&ready.new, self.now())
            .await?;
        if !made {
            return Ok(Requested {
                request,
                made,
                delivery: Delivery::AskedBefore,
            });
        }
        self.publish(&request).await;
        let delivery = self.route(&ready.agent, &request, &ready.files).await;
        Ok(Requested {
            request,
            made,
            delivery,
        })
    }

    /// The request `id`, if it is `agent`'s.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NotFound`] for no such request or another agent's, or the database's.
    pub async fn status(
        &self,
        agent: &AgentId,
        id: ApprovalId,
    ) -> Result<ApprovalRequest, ApprovalError> {
        self.inner
            .approvals
            .get(id)
            .await?
            .filter(|request| request.agent == *agent)
            .ok_or(ApprovalError::NotFound)
    }

    /// Withdraws the request `id`, pending or approved and unused: by its own agent (`agent`),
    /// or by the owner (none). The request as it is now.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NotFound`] for no such request (or another agent's),
    /// [`ApprovalError::Closed`] when it can no longer be withdrawn, or the database's.
    pub async fn withdraw(
        &self,
        id: ApprovalId,
        agent: Option<&AgentId>,
        reason: Option<&str>,
    ) -> Result<ApprovalRequest, ApprovalError> {
        let approvals = &self.inner.approvals;
        let found = approvals.get(id).await?;
        if found
            .as_ref()
            .is_none_or(|request| agent.is_some_and(|agent| request.agent != *agent))
        {
            return Err(ApprovalError::NotFound);
        }
        if !approvals.withdraw(id, agent, reason, self.now()).await? {
            return Err(ApprovalError::Closed);
        }
        self.changed(id).await
    }

    /// The request `id` as it is now, after a change: told on the live stream and, when it
    /// was posted to its approver in Slack, said there.
    async fn changed(&self, id: ApprovalId) -> Result<ApprovalRequest, ApprovalError> {
        let request = self
            .inner
            .approvals
            .get(id)
            .await?
            .ok_or(ApprovalError::NotFound)?;
        self.publish(&request).await;
        self.tell_slack(&request).await;
        Ok(request)
    }

    /// Tells the live stream that `request` changed.
    async fn publish(&self, request: &ApprovalRequest) {
        let pending = match self.inner.approvals.pending_counts().await {
            Ok(counts) => counts.iter().map(|(_, n)| *n).sum(),
            Err(error) => {
                tracing::warn!(%error, "approvals: the pending count could not be read");
                0
            }
        };
        self.inner.events.send(ApprovalsEvent {
            id: emberhut_api::ids::ApprovalId::new(request.id.get()),
            state: view::state(request.state),
            pending,
        });
    }

    /// The requests in `state` (a word of the contract) of `agent`, newest first. A state or
    /// agent no request can have answers none.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn list(
        &self,
        state: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Vec<Approval>, ApprovalError> {
        let state = match state {
            Some(word) => match RequestState::parse(word) {
                Some(state) => Some(state),
                None => return Ok(Vec::new()),
            },
            None => None,
        };
        let agent = match agent {
            Some(id) => match AgentId::parse(id) {
                Ok(agent) => Some(agent),
                Err(_) => return Ok(Vec::new()),
            },
            None => None,
        };
        let found = self
            .inner
            .approvals
            .list(&ApprovalFilter {
                state,
                agent,
                limit: None,
            })
            .await?;
        self.shown(&found).await
    }

    /// The request `id`, as the apps show it.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NotFound`], or the database's.
    pub async fn approval(&self, id: ApprovalId) -> Result<Approval, ApprovalError> {
        let request = self
            .inner
            .approvals
            .get(id)
            .await?
            .ok_or(ApprovalError::NotFound)?;
        self.shown(std::slice::from_ref(&request))
            .await?
            .into_iter()
            .next()
            .ok_or(ApprovalError::NotFound)
    }

    /// `request` as the apps show it.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn show(&self, request: &ApprovalRequest) -> Result<Approval, ApprovalError> {
        self.shown(std::slice::from_ref(request))
            .await?
            .into_iter()
            .next()
            .ok_or(ApprovalError::NotFound)
    }

    /// The requests waiting on an answer, newest first: the state's `approvals`.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn pending(&self) -> Result<Vec<Approval>, ApprovalError> {
        let found = self
            .inner
            .approvals
            .list(&ApprovalFilter {
                state: Some(RequestState::Pending),
                agent: None,
                limit: None,
            })
            .await?;
        self.shown(&found).await
    }

    /// `requests` as the apps show them, with the agents' and approvers' names.
    async fn shown(&self, requests: &[ApprovalRequest]) -> Result<Vec<Approval>, ApprovalError> {
        let agents = self.inner.agents.list().await?;
        let names = view::Names {
            currencies: agents
                .iter()
                .filter_map(|agent| {
                    let cap = agent.spend_cap.as_ref()?;
                    Some((
                        agent.id.as_str().to_owned(),
                        cap.currency.as_str().to_owned(),
                    ))
                })
                .collect(),
            agents: agents
                .into_iter()
                .map(|agent| (agent.id.as_str().to_owned(), agent.name))
                .collect(),
            approvers: self
                .inner
                .agents
                .approvers()
                .await?
                .into_iter()
                .map(|approver| (approver.id.as_str().to_owned(), approver.name))
                .collect(),
        };
        let now = self.now();
        Ok(requests
            .iter()
            .map(|request| view::approval(request, &names, now))
            .collect())
    }
}
