//! Agents' ids: short slugs, as they appear in paths, fact categories and secret names.

use std::borrow::Cow;
use std::fmt;

/// What starts the category of an agent's own facts, before its id.
pub const AGENT_FACTS_PREFIX: &str = "agent-";

/// The longest slug, in characters.
const SLUG_MAX: usize = 40;

/// Whether `text` is a slug: `[a-z0-9][a-z0-9-]{0,39}`.
pub(crate) fn is_slug(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && text.len() <= SLUG_MAX
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An agent's id: 1 to 40 of `a-z`, `0-9` and `-`, not starting with `-`.
///
/// It is part of the agent's paths (`/api/agents/{id}`), its facts' category (`agent-<id>`) and
/// its inbound secret's name (`agent-<id>-inbound`), so it is kept to characters that are safe
/// in all three.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentId(Cow<'static, str>);

/// Text that is not an agent's id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an agent's id: 1 to 40 of a-z, 0-9 and '-', not starting with '-'")]
pub struct InvalidAgentId(pub String);

impl AgentId {
    /// The default agent's id, `main`: the assistant the server runs for its owner.
    pub const MAIN: Self = Self(Cow::Borrowed("main"));

    /// The id, if `text` is one.
    ///
    /// ```
    /// use emberhut_core::agents::AgentId;
    ///
    /// assert!(AgentId::parse("client-a").is_ok());
    /// assert!(AgentId::parse("Client A").is_err());
    /// assert!(AgentId::parse("-a").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`InvalidAgentId`].
    pub fn parse(text: &str) -> Result<Self, InvalidAgentId> {
        if is_slug(text) {
            Ok(Self(Cow::Owned(text.to_owned())))
        } else {
            Err(InvalidAgentId(text.to_owned()))
        }
    }

    /// Whether this is the default agent.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.0 == Self::MAIN.0
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The category of this agent's own facts, which is also their folder's name:
    /// `agent-<id>`. A dash, not a colon, so the folder's name is the same on every file system.
    #[must_use]
    pub fn facts_category(&self) -> String {
        format!("{AGENT_FACTS_PREFIX}{}", self.0)
    }

    /// The agent whose own facts' category `category` is, if it is one.
    #[must_use]
    pub fn from_facts_category(category: &str) -> Option<Self> {
        category
            .strip_prefix(AGENT_FACTS_PREFIX)
            .and_then(|id| Self::parse(id).ok())
    }

    /// The name of the secret holding the bearer token its bridge sends to
    /// `POST /api/agents/{id}/inbound`: `agent-<id>-inbound`.
    #[must_use]
    pub fn inbound_secret(&self) -> String {
        format!("agent-{}-inbound", self.0)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for AgentId {
    type Err = InvalidAgentId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl AsRef<str> for AgentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        for good in ["main", "a", "0", "client-a", "a-", &"a".repeat(40)] {
            assert!(AgentId::parse(good).is_ok(), "{good}");
        }
        for bad in ["", "-a", "A", "a_b", "a b", "a.b", "é", &"a".repeat(41)] {
            assert_eq!(
                AgentId::parse(bad),
                Err(InvalidAgentId(bad.to_owned())),
                "{bad}"
            );
        }
    }

    #[test]
    fn main_is_the_default_agent() {
        assert!(AgentId::MAIN.is_main());
        assert_eq!(AgentId::parse("main").unwrap(), AgentId::MAIN);
        assert!(!AgentId::parse("mains").unwrap().is_main());
    }

    #[test]
    fn derived_names() {
        let id = AgentId::parse("client-a").unwrap();
        assert_eq!(id.facts_category(), "agent-client-a");
        assert_eq!(id.inbound_secret(), "agent-client-a-inbound");
        assert_eq!(
            AgentId::from_facts_category("agent-client-a"),
            Some(id.clone())
        );
        assert_eq!(AgentId::from_facts_category("people"), None);
        assert_eq!(AgentId::from_facts_category("agent-"), None);
        assert_eq!(id.to_string(), "client-a");
        assert_eq!("client-a".parse::<AgentId>(), Ok(id));
    }
}
