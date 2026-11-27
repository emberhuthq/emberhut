//! A request's life: its states, the moves between them, and when it expires.
//!
//! ```text
//! pending ──> approved ──> used
//!    │           ├──> expired (unused after 24 hours)
//!    │           └──> withdrawn
//!    ├──> rejected
//!    ├──> expired (unanswered after 3 days)
//!    └──> withdrawn
//! ```
//!
//! Every other move is refused: a decision is made once, an approval is used once, and an
//! ended request stays ended. The same payload approved again is a new request.

use jiff::{SignedDuration, Timestamp};

use crate::wire::wire_enum;

/// How long a request waits for its answer.
pub const PENDING_LIFETIME: SignedDuration = SignedDuration::from_hours(72);
/// How long an approval waits to be used.
pub const APPROVED_LIFETIME: SignedDuration = SignedDuration::from_hours(24);

wire_enum! {
    /// Where a request is in its life.
    pub enum RequestState {
        /// Waiting for its approver.
        Pending = "pending",
        /// Approved, not used yet.
        Approved = "approved",
        /// Rejected.
        Rejected = "rejected",
        /// Approved, and used once.
        Used = "used",
        /// Unanswered, or approved and unused, for too long.
        Expired = "expired",
        /// Taken back by the agent or the owner.
        Withdrawn = "withdrawn",
    }
}

impl RequestState {
    /// Whether a request may move from this state to `next`.
    #[must_use]
    pub const fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Pending,
                Self::Approved | Self::Rejected | Self::Expired | Self::Withdrawn
            ) | (Self::Approved, Self::Used | Self::Expired | Self::Withdrawn)
        )
    }

    /// Whether nothing more happens to a request in this state.
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Used | Self::Expired | Self::Withdrawn
        )
    }

    /// The states a request may be withdrawn from.
    pub const WITHDRAWABLE: [Self; 2] = [Self::Pending, Self::Approved];
}

wire_enum! {
    /// An approver's answer.
    pub enum Decision {
        /// Yes.
        Approve = "approve",
        /// No.
        Reject = "reject",
    }
}

impl Decision {
    /// The state a pending request moves to on this answer.
    #[must_use]
    pub const fn state(self) -> RequestState {
        match self {
            Self::Approve => RequestState::Approved,
            Self::Reject => RequestState::Rejected,
        }
    }
}

wire_enum! {
    /// The channel a decision came through, which is what proved who made it.
    pub enum DecidedVia {
        /// The apps, from a signed-in device: the owner.
        App = "app",
        /// A button in the approver's Slack DM, under Slack's signature.
        Slack = "slack",
        /// A one-time signed link.
        Link = "link",
        /// The approver's own app, under their signature.
        Webhook = "webhook",
    }
}

/// When a request made at `created_at` stops waiting for its answer.
#[must_use]
pub fn pending_expires_at(created_at: Timestamp) -> Timestamp {
    created_at + PENDING_LIFETIME
}

/// When an approval given at `decided_at` stops waiting to be used.
#[must_use]
pub fn approval_expires_at(decided_at: Timestamp) -> Timestamp {
    decided_at + APPROVED_LIFETIME
}

/// Requests still pending that were made before this instant are due to expire at `now`.
#[must_use]
pub fn pending_cutoff(now: Timestamp) -> Timestamp {
    now - PENDING_LIFETIME
}

/// Approvals still unused that were given before this instant are due to expire at `now`.
#[must_use]
pub fn approved_cutoff(now: Timestamp) -> Timestamp {
    now - APPROVED_LIFETIME
}

/// When a request in `state`, made at `created_at` and decided at `decided_at`, expires; none
/// for a request that no longer can.
#[must_use]
pub fn expires_at(
    state: RequestState,
    created_at: Timestamp,
    decided_at: Option<Timestamp>,
) -> Option<Timestamp> {
    match state {
        RequestState::Pending => Some(pending_expires_at(created_at)),
        RequestState::Approved => decided_at.map(approval_expires_at),
        _ => None,
    }
}

/// Whether such a request has expired at `now` (its expiry is the first instant it no longer
/// counts).
#[must_use]
pub fn is_expired(
    state: RequestState,
    created_at: Timestamp,
    decided_at: Option<Timestamp>,
    now: Timestamp,
) -> bool {
    expires_at(state, created_at, decided_at).is_some_and(|at| now >= at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RequestState::{Approved, Expired, Pending, Rejected, Used, Withdrawn};

    #[test]
    fn the_allowed_moves() {
        let allowed = [
            (Pending, Approved),
            (Pending, Rejected),
            (Pending, Expired),
            (Pending, Withdrawn),
            (Approved, Used),
            (Approved, Expired),
            (Approved, Withdrawn),
        ];
        for from in RequestState::ALL {
            for to in RequestState::ALL {
                assert_eq!(
                    from.can_become(*to),
                    allowed.contains(&(*from, *to)),
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn no_decision_twice_and_no_use_twice() {
        assert!(!Approved.can_become(Approved));
        assert!(!Approved.can_become(Rejected));
        assert!(!Rejected.can_become(Approved));
        assert!(!Used.can_become(Approved));
        assert!(!Used.can_become(Used));
        assert!(!Expired.can_become(Approved));
        assert!(!Withdrawn.can_become(Approved));
        assert!(!Pending.can_become(Used), "used needs an approval first");
    }

    #[test]
    fn final_states() {
        let finals: Vec<_> = RequestState::ALL
            .iter()
            .filter(|s| s.is_final())
            .copied()
            .collect();
        assert_eq!(finals, [Rejected, Used, Expired, Withdrawn]);
        for state in finals {
            assert!(
                RequestState::ALL
                    .iter()
                    .all(|next| !state.can_become(*next))
            );
        }
    }

    #[test]
    fn decisions_move_a_pending_request() {
        assert_eq!(Decision::Approve.state(), Approved);
        assert_eq!(Decision::Reject.state(), Rejected);
        assert_eq!(Decision::parse("approve"), Some(Decision::Approve));
        assert_eq!(DecidedVia::Webhook.as_str(), "webhook");
    }

    #[test]
    fn expiry() {
        let made = Timestamp::from_second(1_800_000_000).unwrap();
        let three_days = made + SignedDuration::from_hours(72);
        assert!(!is_expired(
            Pending,
            made,
            None,
            three_days - SignedDuration::from_secs(1)
        ));
        assert!(is_expired(Pending, made, None, three_days));
        let decided = made + SignedDuration::from_hours(1);
        let day_later = decided + SignedDuration::from_hours(24);
        assert!(!is_expired(
            Approved,
            made,
            Some(decided),
            day_later - SignedDuration::from_secs(1)
        ));
        assert!(is_expired(Approved, made, Some(decided), day_later));
        assert!(!is_expired(Used, made, Some(decided), three_days));
        assert_eq!(expires_at(Rejected, made, Some(decided)), None);
        assert_eq!(pending_cutoff(three_days), made);
        assert_eq!(approved_cutoff(day_later), decided);
    }
}
