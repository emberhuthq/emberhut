//! Decisions, each counted only as its channel proves who made it:
//!
//! - the owner, from a signed-in device (the route checks the session), on requests that wait
//!   on the owner;
//! - a Slack button: the request checked with the workspace's signing secret (at most five
//!   minutes old), then the click's team and user must be the approver's, the button must be
//!   on the very message the server posted (its channel and timestamp), and the button's hash
//!   the request's;
//! - a signed link: the token's MAC (keyed by the link key) covers the request and its hash,
//!   its nonce must still be the request's (the first decision clears it), and the form must
//!   carry the hash the page showed;
//! - the approver's own app: the call signed with the approver's secret, at most five minutes
//!   old; its `by` is recorded as the app reports it.
//!
//! Every decision is one conditional update (`ApprovalStore::decide`): pending, unexpired, the
//! same hash, the same approver. Of two at once, one counts. Until a call is proven, every
//! refusal reads the same ([`UNPROVEN`]), so an outsider learns nothing of which requests,
//! approvers or workspaces there are. What follows a decision (the live event, the DM's
//! update) is done after it is recorded, in the background, so Slack is answered at once.

use emberhut_api::approvals::WebhookDecision;
use emberhut_core::approvals::link::{self, LinkClaims};
use emberhut_core::approvals::slack::signing_secret_name;
use emberhut_core::approvals::{
    ApprovalRequest, Approver, ApproverRecord, DecidedVia, Decision, PayloadHash, SlackIdentity,
    slack, webhook,
};
use emberhut_core::secret_link::SecretName;
use emberhut_core::{ApprovalId, DeviceId};
use emberhut_db::DecisionMade;
use garde::Validate as _;
use serde_json::Value;

use super::channels::{APPROVE_ACTION, REJECT_ACTION};
use super::{ApprovalError, ApprovalService};

/// A call that signs itself: its headers' timestamp and signature, and its raw body.
#[derive(Clone, Copy, Debug)]
pub struct InboundCall<'a> {
    /// The timestamp header, if sent.
    pub timestamp: Option<&'a str>,
    /// The signature header, if sent.
    pub signature: Option<&'a str>,
    /// The body, as signed.
    pub body: &'a [u8],
}

impl<'a> InboundCall<'a> {
    /// Its timestamp and signature; refused without both.
    fn headers(&self) -> Result<(&'a str, &'a str), ApprovalError> {
        match (self.timestamp, self.signature) {
            (Some(timestamp), Some(signature)) => Ok((timestamp, signature)),
            _ => Err(ApprovalError::Unproven(UNPROVEN)),
        }
    }
}

/// A signed link's page: the request it decides, and the names it is shown with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    /// The request.
    pub request: ApprovalRequest,
    /// The agent's name.
    pub agent_name: String,
    /// The currency of a payment's amount: the agent's spending currency.
    pub currency: Option<String>,
}

/// What every call that doesn't prove itself is told.
pub(crate) const UNPROVEN: &str = "the call is not proven";

/// The payload hash written as `hex`; refused when it is not 64 hex digits.
fn hash_of(hex: &str) -> Result<PayloadHash, ApprovalError> {
    PayloadHash::from_hex(hex).ok_or(ApprovalError::Malformed(
        "the payload hash is 64 hex digits",
    ))
}

impl ApprovalService {
    /// Records `decision` from the owner's signed-in `device` on the request `id`, for the
    /// payload `hash` they were shown. Only a request that waits on the owner is decided here:
    /// one waiting on an external approver is theirs alone.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::NotFound`], [`ApprovalError::NotYours`] for a request waiting on an
    /// external approver, [`ApprovalError::Malformed`] or [`ApprovalError::Mismatch`] for a
    /// hash that isn't the request's, [`ApprovalError::Closed`], or the database's.
    pub async fn decide_as_owner(
        &self,
        id: ApprovalId,
        decision: Decision,
        hash: &str,
        device: DeviceId,
        reason: Option<&str>,
    ) -> Result<ApprovalRequest, ApprovalError> {
        let hash = hash_of(hash)?;
        let request = self
            .inner
            .approvals
            .get(id)
            .await?
            .ok_or(ApprovalError::NotFound)?;
        if request.approver != Approver::Owner {
            return Err(ApprovalError::NotYours(
                "this request waits on its approver, not on the owner",
            ));
        }
        self.record(
            &request,
            &hash,
            decision,
            &format!("the owner (device {device})"),
            DecidedVia::App,
            reason,
            ApprovalError::Closed,
        )
        .await
    }

    /// Records a decision whose proof was checked: the one conditional update, then the
    /// request as it is now. `closed` is what a request that no longer takes it answers.
    #[expect(
        clippy::too_many_arguments,
        reason = "one decision's parts, each from a different channel's proof"
    )]
    async fn record(
        &self,
        request: &ApprovalRequest,
        hash: &PayloadHash,
        decision: Decision,
        by: &str,
        via: DecidedVia,
        reason: Option<&str>,
        closed: ApprovalError,
    ) -> Result<ApprovalRequest, ApprovalError> {
        if *hash != request.payload_hash {
            return Err(ApprovalError::Mismatch);
        }
        let made = DecisionMade {
            id: request.id,
            payload_hash: hash,
            approver: &request.approver,
            decision,
            by,
            via,
            reason,
        };
        if !self.inner.approvals.decide(&made, self.now()).await? {
            return Err(closed);
        }
        let decided = self
            .inner
            .approvals
            .get(request.id)
            .await?
            .ok_or(ApprovalError::NotFound)?;
        let service = self.clone();
        let told = decided.clone();
        self.inner.followups.spawn(async move {
            service.publish(&told).await;
            service.tell_slack(&told).await;
        });
        Ok(decided)
    }

    /// The external approver whose request `request` is.
    async fn approver_of(
        &self,
        request: &ApprovalRequest,
    ) -> Result<Option<ApproverRecord>, ApprovalError> {
        match &request.approver {
            Approver::External(id) => Ok(self.inner.agents.approver(id).await?),
            Approver::Owner | Approver::Nobody => Ok(None),
        }
    }

    /// Records a Slack button click (`POST /api/approvals/slack`): Slack's form body with its
    /// `payload`, checked with its workspace's signing secret before anything in it counts.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Unproven`] for a call Slack's signature doesn't prove (none, wrong,
    /// stale, or no signing secret for its workspace), [`ApprovalError::Malformed`],
    /// [`ApprovalError::NotFound`], [`ApprovalError::NotYours`] for a click from anyone but
    /// the request's approver or on any message but the request's own, [`ApprovalError::Mismatch`], [`ApprovalError::Closed`], or the
    /// database's.
    pub async fn slack_action(
        &self,
        call: InboundCall<'_>,
    ) -> Result<ApprovalRequest, ApprovalError> {
        let (timestamp, signature) = call.headers()?;
        let payload = form_payload(call.body).ok_or(ApprovalError::Unproven(UNPROVEN))?;
        let team = payload["team"]["id"]
            .as_str()
            .filter(|team| is_slack_id(team))
            .ok_or(ApprovalError::Unproven(UNPROVEN))?;
        let secret = SecretName::parse(&signing_secret_name(team))
            .map_err(|_| ApprovalError::Unproven(UNPROVEN))?;
        let Some(secret) = self.inner.secrets.read(&secret).await? else {
            return Err(ApprovalError::Unproven(UNPROVEN));
        };
        slack::verify(
            secret.as_bytes(),
            timestamp,
            signature,
            call.body,
            self.now(),
        )
        .map_err(|_| ApprovalError::Unproven(UNPROVEN))?;

        let user = payload["user"]["id"]
            .as_str()
            .ok_or(ApprovalError::Malformed("the click names no user"))?;
        let action = &payload["actions"][0];
        let decision = match action["action_id"].as_str() {
            Some(APPROVE_ACTION) => Decision::Approve,
            Some(REJECT_ACTION) => Decision::Reject,
            _ => return Err(ApprovalError::Malformed("not an approval's button")),
        };
        let (id, hash) = action["value"]
            .as_str()
            .and_then(|value| value.split_once('.'))
            .ok_or(ApprovalError::Malformed("not an approval's button"))?;
        let id = request_id(id).ok_or(ApprovalError::NotFound)?;
        let hash = hash_of(hash)?;
        let request = self
            .inner
            .approvals
            .get(id)
            .await?
            .ok_or(ApprovalError::NotFound)?;
        let clicked = SlackIdentity {
            workspace: team.to_owned(),
            user: user.to_owned(),
        };
        let theirs = self
            .approver_of(&request)
            .await?
            .and_then(|record| record.slack)
            .is_some_and(|slack| slack == clicked);
        if !theirs {
            return Err(ApprovalError::NotYours(
                "only the request's approver decides it",
            ));
        }
        let container = &payload["container"];
        let on_its_message = request.shown.as_ref().is_some_and(|shown| {
            container["channel_id"].as_str() == Some(shown.channel.as_str())
                && container["message_ts"].as_str() == Some(shown.ts.as_str())
        });
        if !on_its_message {
            return Err(ApprovalError::NotYours(
                "the button is not on the request's own message",
            ));
        }
        self.record(
            &request,
            &hash,
            decision,
            &format!("Slack user {user} of {team}"),
            DecidedVia::Slack,
            None,
            ApprovalError::Closed,
        )
        .await
    }

    /// The request a signed link's `token` opens, while the link works: its MAC is the link
    /// key's, it has not expired, its nonce is still the request's, it is for the request's
    /// payload, the request still waits, and its approver may have a link.
    async fn link_open(
        &self,
        token: &str,
    ) -> Result<(LinkClaims, ApprovalRequest, ApproverRecord), ApprovalError> {
        let key = self
            .inner
            .secrets
            .link_key()
            .await?
            .ok_or(ApprovalError::Gone)?;
        let now = self.now();
        let claims = link::verify(&key, token, now).map_err(|_| ApprovalError::Gone)?;
        let request = self
            .inner
            .approvals
            .by_link_nonce(&claims.nonce.hash())
            .await?
            .filter(|request| {
                claims.is_for(request.id, &request.payload_hash) && request.is_open(now)
            })
            .ok_or(ApprovalError::Gone)?;
        let record = self
            .approver_of(&request)
            .await?
            .filter(|record| record.link)
            .ok_or(ApprovalError::Gone)?;
        Ok((claims, request, record))
    }

    /// What a signed link's page shows.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Gone`] for a link that doesn't work, or the database's.
    pub async fn link_view(&self, token: &str) -> Result<LinkView, ApprovalError> {
        let (_, request, _) = self.link_open(token).await?;
        let agent = self.inner.agents.get(&request.agent).await?;
        let currency = agent
            .as_ref()
            .and_then(|agent| agent.spend_cap.as_ref())
            .map(|cap| cap.currency.as_str().to_owned());
        let agent_name = agent.map_or_else(|| request.agent.to_string(), |agent| agent.name);
        Ok(LinkView {
            request,
            agent_name,
            currency,
        })
    }

    /// Records the decision of a signed link's form: `decision` on the hash the page showed.
    /// The link works once.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Gone`] for a link that doesn't work (any more),
    /// [`ApprovalError::Malformed`] for a form without a decision or a hash,
    /// [`ApprovalError::Mismatch`], or the database's.
    pub async fn link_decide(
        &self,
        token: &str,
        decision: Option<Decision>,
        hash: Option<&str>,
    ) -> Result<ApprovalRequest, ApprovalError> {
        let (_, request, record) = self.link_open(token).await?;
        let decision = decision.ok_or(ApprovalError::Malformed("approve or reject"))?;
        let hash = hash_of(hash.unwrap_or_default())?;
        self.record(
            &request,
            &hash,
            decision,
            &format!("{} (signed link)", record.name),
            DecidedVia::Link,
            None,
            ApprovalError::Gone,
        )
        .await
    }

    /// Records an approver's app's answer on the request `id` (its route's text): the call
    /// signed with the approver's secret over `path`, at most five minutes old, then its body,
    /// a `WebhookDecision`.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Unproven`] for a call the approver's secret doesn't prove (none, wrong,
    /// stale, no such request, or one that takes no webhook answer: all alike, so an unsigned
    /// caller learns nothing), [`ApprovalError::Malformed`] for a body that isn't a decision,
    /// [`ApprovalError::Mismatch`], [`ApprovalError::Closed`], or the database's.
    pub async fn webhook_decision(
        &self,
        id: &str,
        path: &str,
        call: InboundCall<'_>,
    ) -> Result<ApprovalRequest, ApprovalError> {
        let (timestamp, signature) = call.headers()?;
        let unproven = || ApprovalError::Unproven(UNPROVEN);
        let id = request_id(id).ok_or_else(unproven)?;
        let request = self.inner.approvals.get(id).await?.ok_or_else(unproven)?;
        let target = self
            .approver_of(&request)
            .await?
            .and_then(|record| record.webhook)
            .ok_or_else(unproven)?;
        let secret = self
            .inner
            .secrets
            .read(&target.secret)
            .await?
            .ok_or_else(unproven)?;
        webhook::verify(
            secret.as_bytes(),
            timestamp,
            signature,
            "POST",
            path,
            call.body,
            self.now(),
        )
        .map_err(|_| unproven())?;
        let answer: WebhookDecision = serde_json::from_slice(call.body)
            .map_err(|_| ApprovalError::Malformed("not a decision"))?;
        answer
            .validate()
            .map_err(|_| ApprovalError::Malformed("not a decision"))?;
        let decision = match answer.decision {
            emberhut_api::approvals::Decision::Approve => Decision::Approve,
            emberhut_api::approvals::Decision::Reject => Decision::Reject,
        };
        let hash = hash_of(&answer.payload_hash)?;
        self.record(
            &request,
            &hash,
            decision,
            answer.by.trim(),
            DecidedVia::Webhook,
            None,
            ApprovalError::Closed,
        )
        .await
    }
}

/// A request's id written as `text`: a positive whole number.
fn request_id(text: &str) -> Option<ApprovalId> {
    text.parse::<i64>()
        .ok()
        .filter(|id| *id > 0 && text.bytes().all(|b| b.is_ascii_digit()))
        .map(ApprovalId::new)
}

/// Slack's interactivity body: a form whose `payload` field is JSON.
fn form_payload(body: &[u8]) -> Option<Value> {
    form_urlencoded::parse(body)
        .find(|(name, _)| name == "payload")
        .and_then(|(_, payload)| serde_json::from_str(&payload).ok())
}

/// Whether `text` could be a Slack team id: capital letters and digits.
fn is_slack_id(text: &str) -> bool {
    (2..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_payloads_are_read_strictly() {
        assert_eq!(request_id("7"), Some(ApprovalId::new(7)));
        assert_eq!(request_id("+7"), None);
        assert_eq!(request_id("0"), None);
        assert_eq!(request_id("{id}"), None);
        let body = b"payload=%7B%22team%22%3A%7B%22id%22%3A%22T0EXAMPLE%22%7D%7D";
        assert_eq!(form_payload(body).unwrap()["team"]["id"], "T0EXAMPLE");
        assert!(form_payload(b"other=1").is_none());
        assert!(is_slack_id("T0EXAMPLE") && !is_slack_id("t0example") && !is_slack_id("../x"));
    }
}
