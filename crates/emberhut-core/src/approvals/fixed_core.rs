//! The fixed core: what no configuration can loosen.
//!
//! - A fixed-core kind ([`ActionKind::is_fixed_core`]) is the owner's, whatever a policy says.
//!   A policy naming anyone else, or nobody, for one is refused when written
//!   ([`check_policy`]) and ignored if it is in the table anyway ([`effective_approver`]).
//! - A request is classified by its words too ([`raise`]): an outward step whose text or
//!   summary is about money, a credential, a secret or account security is raised to the
//!   matching fixed-core kind. The words are the guard rails' own
//!   ([`crate::guard::hard_words`]), plus the words for secrets and the money words (a
//!   subscription, a top-up, a transfer, an amount) only approvals need.
//! - The default agent's every approval is the owner's.
//!
//! A request is made in this order: [`raise`] the kind the agent asked for, look up the agent's
//! policy for the raised kind, then [`effective_approver`].

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use super::approver::Approver;
use super::kind::ActionKind;
use crate::agents::AgentId;
use crate::guard::hard_words::{self, HardTopic};

static SECRET: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(secret|secrets|api[ -]?keys?|access tokens?|auth tokens?|bearer tokens?|private keys?|ssh keys?|signing keys?|client secrets?|credentials)\b").ok()
});

/// Money words the guard rails' own list leaves out, which approvals raise too: a
/// subscription, a top-up, a transfer, a refund, a renewal, a charge, and an amount of money.
/// Kept apart from the guard rails' list so their behaviour does not change.
static MONEY: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\b(subscribe|subscribes|subscribing|subscription|subscriptions|top[ -]?up|top[ -]?ups|topping up|transfer|transfers|transferring|wire|wires|wiring|refund|refunds|refunding|renew|renews|renewing|renewal|renewals|charge|charges|charging|invoice payment)\b|[$€£]\s?\d|\b\d+([.,]\d+)*\s?(usd|eur|gbp|sar|aed)\b)",
    )
    .ok()
});

/// Whether `kind` is the fixed core's.
#[must_use]
pub const fn is_fixed_core(kind: ActionKind) -> bool {
    kind.is_fixed_core()
}

/// The kind a request is for once its words are read: `kind` itself, or the fixed-core kind
/// its `text` or `summary` is about.
///
/// Only the outward kinds whose words are what goes out are raised: `post`, `send`, `create`
/// and `other`. A merge's or deploy's summary describes code, and code is often about
/// payments or passwords without moving money or handing one over; the fixed core reaches
/// those through the tools and the guard rails instead. A fixed-core kind stays as it is.
///
/// ```
/// use emberhut_core::approvals::ActionKind;
/// use emberhut_core::approvals::fixed_core::raise;
///
/// assert_eq!(raise(ActionKind::Post, Some("Our new pricing page"), "a post"), ActionKind::Post);
/// assert_eq!(
///     raise(ActionKind::Post, Some("Reset your password here"), "a post"),
///     ActionKind::Credential
/// );
/// ```
#[must_use]
pub fn raise(kind: ActionKind, text: Option<&str>, summary: &str) -> ActionKind {
    if !matches!(
        kind,
        ActionKind::Post | ActionKind::Send | ActionKind::Create | ActionKind::Other
    ) {
        return kind;
    }
    let about = |words: &str| {
        hard_words::topic(words)
            .map(|topic| match topic {
                HardTopic::Money => ActionKind::Payment,
                HardTopic::Credential => ActionKind::Credential,
                HardTopic::AccountSecurity => ActionKind::AccountSecurity,
            })
            .or_else(|| {
                MONEY
                    .as_ref()
                    .is_some_and(|re| re.is_match(words))
                    .then_some(ActionKind::Payment)
            })
            .or_else(|| {
                SECRET
                    .as_ref()
                    .is_some_and(|re| re.is_match(words))
                    .then_some(ActionKind::Secret)
            })
    };
    text.and_then(about)
        .or_else(|| about(summary))
        .unwrap_or(kind)
}

/// Who approves `kind` for `agent`, given the `policy` its table names (none: no row).
///
/// The fixed core and the default agent are always the owner's; no policy is the owner's too.
///
/// ```
/// use emberhut_core::agents::AgentId;
/// use emberhut_core::approvals::fixed_core::effective_approver;
/// use emberhut_core::approvals::{ActionKind, Approver};
///
/// let agent = AgentId::parse("client-a").unwrap();
/// assert_eq!(effective_approver(&agent, ActionKind::Post, Some(&Approver::Nobody)), Approver::Nobody);
/// assert_eq!(effective_approver(&agent, ActionKind::Payment, Some(&Approver::Nobody)), Approver::Owner);
/// assert_eq!(effective_approver(&agent, ActionKind::Post, None), Approver::Owner);
/// ```
#[must_use]
pub fn effective_approver(
    agent: &AgentId,
    kind: ActionKind,
    policy: Option<&Approver>,
) -> Approver {
    match policy {
        Some(approver) if !kind.is_fixed_core() && !agent.is_main() => approver.clone(),
        _ => Approver::Owner,
    }
}

/// One row of an agent's policies: who approves one kind of action.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Policy {
    /// The kind of action.
    pub kind: ActionKind,
    /// Who approves it.
    pub approver: Approver,
}

/// Why a policy can't be written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// A fixed-core kind given to anyone but the owner.
    #[error("{0} is the fixed core's: only the owner approves it")]
    FixedCore(ActionKind),
    /// A policy for the default agent naming anyone but the owner.
    #[error("the default agent's every approval is the owner's")]
    DefaultAgent,
    /// Two policies for one kind.
    #[error("{0} has two policies")]
    Duplicate(ActionKind),
}

/// Checks one policy of `agent`'s: a fixed-core kind, or any kind of the default agent's,
/// names the owner.
///
/// # Errors
///
/// [`PolicyError::FixedCore`] or [`PolicyError::DefaultAgent`].
pub fn check_policy(agent: &AgentId, policy: &Policy) -> Result<(), PolicyError> {
    if policy.approver == Approver::Owner {
        return Ok(());
    }
    if policy.kind.is_fixed_core() {
        return Err(PolicyError::FixedCore(policy.kind));
    }
    if agent.is_main() {
        return Err(PolicyError::DefaultAgent);
    }
    Ok(())
}

/// Checks `agent`'s whole list of policies: each one, and one per kind.
///
/// # Errors
///
/// As [`check_policy`], or [`PolicyError::Duplicate`].
pub fn validate_policies(agent: &AgentId, policies: &[Policy]) -> Result<(), PolicyError> {
    let mut seen = HashSet::new();
    for policy in policies {
        check_policy(agent, policy)?;
        if !seen.insert(policy.kind) {
            return Err(PolicyError::Duplicate(policy.kind));
        }
    }
    Ok(())
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    SECRET.is_some() && MONEY.is_some() && hard_words::patterns_compile()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::ApproverId;

    fn agent() -> AgentId {
        AgentId::parse("client-a").unwrap()
    }

    fn external() -> Approver {
        Approver::External(ApproverId::parse("example-lead").unwrap())
    }

    #[test]
    fn patterns_compile_here() {
        assert!(patterns_compile());
    }

    #[test]
    fn a_post_about_money_or_secrets_is_raised() {
        let cases = [
            ("Pay the hosting invoice today", ActionKind::Payment),
            ("Buy the ad credits", ActionKind::Payment),
            ("The password is in the vault", ActionKind::Credential),
            ("Your one-time code is 123456", ActionKind::Credential),
            (
                "Turn on two-factor for the team",
                ActionKind::AccountSecurity,
            ),
            (
                "Close my account at the registrar",
                ActionKind::AccountSecurity,
            ),
            ("Here is the API key for staging", ActionKind::Secret),
            ("Rotate the SSH keys", ActionKind::Secret),
            (
                "Subscribe the workspace to the Pro plan",
                ActionKind::Payment,
            ),
            ("Top up the ad account with $500", ActionKind::Payment),
            ("Transfer €2,000 to the supplier", ActionKind::Payment),
            ("Renew the domain for 3 years", ActionKind::Payment),
            ("Charge the client's card", ActionKind::Payment),
            ("Send them 1,200 EUR by Friday", ActionKind::Payment),
            ("A refund for the order", ActionKind::Payment),
        ];
        for (text, raised) in cases {
            for kind in [
                ActionKind::Post,
                ActionKind::Send,
                ActionKind::Create,
                ActionKind::Other,
            ] {
                assert_eq!(
                    raise(kind, Some(text), "an update"),
                    raised,
                    "{kind}: {text}"
                );
            }
        }
    }

    #[test]
    fn the_summary_raises_too() {
        assert_eq!(
            raise(ActionKind::Post, Some("See you there"), "pay the venue"),
            ActionKind::Payment
        );
        assert_eq!(
            raise(ActionKind::Create, None, "a password for the new account"),
            ActionKind::Credential
        );
    }

    #[test]
    fn ordinary_words_stay() {
        assert_eq!(
            raise(
                ActionKind::Post,
                Some("We shipped dark mode"),
                "release post"
            ),
            ActionKind::Post
        );
        assert_eq!(
            raise(ActionKind::Send, None, "weekly update to the team"),
            ActionKind::Send
        );
    }

    #[test]
    fn code_steps_and_the_fixed_core_are_not_raised() {
        assert_eq!(
            raise(
                ActionKind::Merge,
                None,
                "fix the payment form's password field"
            ),
            ActionKind::Merge
        );
        assert_eq!(
            raise(ActionKind::Deploy, Some("api key rotation"), "deploy"),
            ActionKind::Deploy
        );
        assert_eq!(
            raise(ActionKind::AsOwner, Some("pay"), "x"),
            ActionKind::AsOwner
        );
    }

    #[test]
    fn the_fixed_core_is_the_owners_whatever_the_policy() {
        for kind in ActionKind::ALL.iter().filter(|k| k.is_fixed_core()) {
            for policy in [Some(&external()), Some(&Approver::Nobody), None] {
                assert_eq!(
                    effective_approver(&agent(), *kind, policy),
                    Approver::Owner,
                    "{kind}"
                );
            }
        }
    }

    #[test]
    fn a_configurable_kind_follows_its_policy() {
        assert_eq!(
            effective_approver(&agent(), ActionKind::Post, Some(&external())),
            external()
        );
        assert_eq!(
            effective_approver(&agent(), ActionKind::Merge, None),
            Approver::Owner
        );
    }

    #[test]
    fn the_default_agent_is_always_the_owners() {
        assert_eq!(
            effective_approver(&AgentId::MAIN, ActionKind::Post, Some(&Approver::Nobody)),
            Approver::Owner
        );
    }

    #[test]
    fn policies_that_escalate_are_refused() {
        for approver in [external(), Approver::Nobody] {
            let policy = Policy {
                kind: ActionKind::Payment,
                approver: approver.clone(),
            };
            assert_eq!(
                check_policy(&agent(), &policy),
                Err(PolicyError::FixedCore(ActionKind::Payment))
            );
            let main_post = Policy {
                kind: ActionKind::Post,
                approver,
            };
            assert_eq!(
                check_policy(&AgentId::MAIN, &main_post),
                Err(PolicyError::DefaultAgent)
            );
        }
        let owner_payment = Policy {
            kind: ActionKind::Payment,
            approver: Approver::Owner,
        };
        assert_eq!(check_policy(&agent(), &owner_payment), Ok(()));
    }

    #[test]
    fn one_policy_per_kind() {
        let post = |approver| Policy {
            kind: ActionKind::Post,
            approver,
        };
        assert_eq!(
            validate_policies(&agent(), &[post(external()), post(Approver::Nobody)]),
            Err(PolicyError::Duplicate(ActionKind::Post))
        );
        assert_eq!(
            validate_policies(
                &agent(),
                &[
                    post(external()),
                    Policy {
                        kind: ActionKind::Merge,
                        approver: Approver::Nobody
                    }
                ]
            ),
            Ok(())
        );
    }
}
