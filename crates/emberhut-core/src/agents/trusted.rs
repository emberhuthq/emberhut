//! Trusted requesters: people whose requests on a task count as the owner's own request, for
//! the job's grant. They never cover the fixed core or anything said in the owner's name; that
//! is the approvals' rule, not this list's.

use std::collections::HashSet;

use super::error::{AgentError, text_field};
use super::id::AgentId;

/// The longest requester name, in characters.
pub const REQUESTER_MAX: usize = 120;

/// A person whose requests count as the owner's own, for one agent or for all.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrustedRequester {
    /// The name a task's request carries.
    pub requester: String,
    /// The agent whose tasks it counts for; none: every agent's.
    pub agent: Option<AgentId>,
}

impl TrustedRequester {
    /// Whether a request by `requester` on a task of `agent` counts as the owner's own.
    #[must_use]
    pub fn covers(&self, requester: &str, agent: &AgentId) -> bool {
        self.requester == requester && self.agent.as_ref().is_none_or(|a| a == agent)
    }
}

/// Checks the whole list: each name says something and is not too long, and no pair of name
/// and agent is listed twice.
///
/// # Errors
///
/// [`AgentError::Requester`] naming the first one refused.
pub fn validate_requesters(requesters: &[TrustedRequester]) -> Result<(), AgentError> {
    let mut seen = HashSet::new();
    for (index, requester) in requesters.iter().enumerate() {
        text_field(&requester.requester, "requester", REQUESTER_MAX).map_err(|_| {
            AgentError::Requester {
                index,
                why: "a name of 1 to 120 characters",
            }
        })?;
        if !seen.insert(requester) {
            return Err(AgentError::Requester {
                index,
                why: "listed twice",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted(name: &str, agent: Option<&str>) -> TrustedRequester {
        TrustedRequester {
            requester: name.to_owned(),
            agent: agent.map(|a| AgentId::parse(a).unwrap()),
        }
    }

    #[test]
    fn covers_its_agent_or_all() {
        let a = AgentId::parse("client-a").unwrap();
        let b = AgentId::parse("client-b").unwrap();
        assert!(trusted("Example Person", None).covers("Example Person", &a));
        assert!(trusted("Example Person", Some("client-a")).covers("Example Person", &a));
        assert!(!trusted("Example Person", Some("client-a")).covers("Example Person", &b));
        assert!(!trusted("Example Person", None).covers("Someone Else", &a));
    }

    #[test]
    fn the_list_is_checked() {
        assert_eq!(
            validate_requesters(&[
                trusted("Example Person", None),
                trusted("Example Person", Some("client-a")),
            ]),
            Ok(())
        );
        assert!(matches!(
            validate_requesters(&[trusted(" ", None)]),
            Err(AgentError::Requester { index: 0, .. })
        ));
        assert_eq!(
            validate_requesters(&[trusted("A", None), trusted("A", None)]),
            Err(AgentError::Requester {
                index: 1,
                why: "listed twice"
            })
        );
    }
}
