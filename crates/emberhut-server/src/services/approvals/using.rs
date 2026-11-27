//! Using an approval: the one step that lets something go out. The step is hashed again from
//! itself (its words, and its files' bytes read now), so a changed word or a file edited after
//! the approval matches nothing; the use is one conditional update that also checks the
//! agent's policy names the same approver now (`ApprovalStore::use_approval`).
//!
//! [`ApprovalService::gate`] is the guard rails' way in: a step whose kind nobody approves goes
//! through; one with a matching approval goes through once; any other asks and is held.
//! Expiring what is due runs in the background ([`ApprovalService::keep_expiring`]).

use std::path::PathBuf;
use std::time::Duration;

use emberhut_core::ApprovalId;
use emberhut_core::agents::AgentId;
use emberhut_core::approvals::{ActionKind, ApprovalRequest, Payload, PayloadHash, RequestState};
use emberhut_db::{ApprovalStoreError, StepToUse};
use tokio_util::sync::CancellationToken;

use super::{ApprovalError, ApprovalService, Prepared, StepAsk, files};

/// A step about to use an approval: its exact words and files as they are now, and what uses
/// it (a task or a tool), for the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepUse {
    /// The exact words, if any.
    pub text: Option<String>,
    /// The absolute paths of its files, read now.
    pub files: Vec<PathBuf>,
    /// What uses it: `task 12`, `approval_use`, a plugin's name.
    pub by: String,
}

/// What using an approval came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UseOutcome {
    /// Used, once: the step may go out.
    Used(Box<ApprovalRequest>),
    /// Not used, and why: nothing goes out.
    Refused(String),
}

/// What the guard rails do with a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// It goes through: the line says why (no approval needed, or whose approval it used).
    Through(String),
    /// It waits: the line says on whom, and for which request.
    Held(String),
}

impl ApprovalService {
    /// Uses the approval of request `id` (that one, no other) for `agent`'s `step`: the payload
    /// is the request's agent, kind, target and amount with the step's own words and files,
    /// hashed now.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NotFound`] for no such request or another agent's, or the database's.
    /// A step that may not use it (another payload, not approved, used, expired, another
    /// approver now, over the cap, a file that can't be read) is [`UseOutcome::Refused`].
    pub async fn use_for_step(
        &self,
        agent: &AgentId,
        id: ApprovalId,
        step: &StepUse,
    ) -> Result<UseOutcome, ApprovalError> {
        let request = self.status(agent, id).await?;
        let owner = self
            .inner
            .agents
            .get(&request.agent)
            .await?
            .ok_or_else(|| ApprovalError::NoAgent(request.agent.clone()))?;
        let within = self.within(&owner).await?;
        let files = match files::read_all(&step.files, &self.inner.private, within.as_deref()).await
        {
            Ok(files) => files,
            Err(refused) => return Ok(UseOutcome::Refused(refused.to_string())),
        };
        let digests: Vec<_> = files.into_iter().map(|file| file.digest).collect();
        let hash = Payload {
            agent: &request.agent,
            kind: request.kind,
            target: &request.target,
            text: step.text.as_deref(),
            amount_cents: request.amount_cents,
            files: &digests,
        }
        .hash();
        match self
            .spend(&request.agent, Some(id), request.kind, &hash, &step.by)
            .await
        {
            Ok(Some(used)) => Ok(UseOutcome::Used(Box::new(used))),
            Ok(None) => Ok(UseOutcome::Refused(self.why_not(&request, &hash).await?)),
            Err(ApprovalError::OverCap(over)) => Ok(UseOutcome::Refused(over.to_string())),
            Err(ApprovalError::Disabled(_)) => Ok(UseOutcome::Refused(
                "The agent is disabled: it uses no approval.".to_owned(),
            )),
            Err(error) => Err(error),
        }
    }

    /// The one use: the matching approval becomes used, told on the live stream and in its
    /// DM. None when nothing matched; [`ApprovalError::OverCap`] for a payment over the cap.
    async fn spend(
        &self,
        agent: &AgentId,
        request: Option<ApprovalId>,
        kind: ActionKind,
        hash: &PayloadHash,
        by: &str,
    ) -> Result<Option<ApprovalRequest>, ApprovalError> {
        let now = self.now();
        let month = self.month(now);
        let step = StepToUse {
            agent,
            request,
            kind,
            payload_hash: hash,
            by,
            month: &month,
        };
        match self.inner.approvals.use_approval(&step, now).await {
            Ok(Some(used)) => {
                tracing::info!(request = %used.id, agent = %agent, by, "approvals: an approval was used");
                self.publish(&used).await;
                self.tell_slack(&used).await;
                Ok(Some(used))
            }
            Ok(None) => Ok(None),
            Err(ApprovalStoreError::Disabled(agent)) => Err(ApprovalError::Disabled(agent)),
            Err(ApprovalStoreError::OverCap(over)) => {
                tracing::info!(agent = %agent, %over, "approvals: a payment over the cap was refused");
                Err(ApprovalError::OverCap(over))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Why `request` could not be used for a step hashed `hash`.
    async fn why_not(
        &self,
        request: &ApprovalRequest,
        hash: &PayloadHash,
    ) -> Result<String, ApprovalError> {
        if request.payload_hash != *hash {
            return Ok(
                "The step is not what was approved: its words, target or files changed. \
                       Ask again for this payload."
                    .to_owned(),
            );
        }
        if request.state != RequestState::Approved {
            return Ok(format!("The request is {}, not approved.", request.state));
        }
        let now_approver = self
            .inner
            .agents
            .approver_for(&request.agent, request.kind)
            .await?;
        if now_approver != request.approver {
            return Ok("The agent's policy names another approver now: ask again.".to_owned());
        }
        if request.expires_at().is_none_or(|at| self.now() >= at) {
            return Ok("The approval has expired: ask again.".to_owned());
        }
        Ok("The approval was used already.".to_owned())
    }

    /// The guard rails' check of `agent`'s step `ask`, used by `by`: through when nobody
    /// approves its kind, or once on a matching approval (used now); otherwise the request is
    /// made (or found) and the step is held.
    ///
    /// # Errors
    ///
    /// As [`ApprovalService::request`].
    pub async fn gate(
        &self,
        agent: &AgentId,
        ask: &StepAsk,
        by: &str,
    ) -> Result<Gate, ApprovalError> {
        let ready = match self.prepare(agent, ask).await {
            Ok(Prepared::NotNeeded(kind)) => {
                return Ok(Gate::Through(format!(
                    "No approval is needed for a {kind}."
                )));
            }
            Ok(Prepared::Ready(ready)) => ready,
            Err(ApprovalError::OverCap(over)) => return Ok(Gate::Held(over.to_string())),
            Err(error) => return Err(error),
        };
        let hash = ready.new.payload_hash();
        let spent = match self.spend(agent, None, ready.new.kind, &hash, by).await {
            Err(ApprovalError::OverCap(over)) => return Ok(Gate::Held(over.to_string())),
            other => other?,
        };
        if let Some(used) = spent {
            return Ok(Gate::Through(format!(
                "Went through. {}",
                self.receipt(&used)
            )));
        }
        let role = ready.new.approver.role();
        let requested = self.make(*ready).await?;
        Ok(Gate::Held(format!(
            "Held: it waits on {role}'s approval (request {}). {}",
            requested.request.id,
            requested.delivery.describe()
        )))
    }

    /// Expires what is due now: requests unanswered for 3 days and approvals unused for 24
    /// hours, each told on the live stream and in its DM. The ids expired.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn expire_due(&self) -> Result<Vec<ApprovalId>, ApprovalError> {
        let expired = self.inner.approvals.expire_due(self.now()).await?;
        for id in &expired {
            self.changed(*id).await?;
        }
        Ok(expired)
    }

    /// Expires what is due every `every`, until `stop` is cancelled.
    pub async fn keep_expiring(self, every: Duration, stop: CancellationToken) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            if let Err(error) = self.expire_due().await {
                tracing::warn!(%error, "approvals: expiring what is due failed");
            }
        }
    }
}
