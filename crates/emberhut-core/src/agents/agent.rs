//! An agent's own row: who it is, who it works for, its room, its project and its budget.

use std::fmt;

use super::error::{AgentError, text_field};
use super::id::AgentId;
use crate::RoomId;
use crate::task::ProjectId;

/// The longest name, in characters.
pub const NAME_MAX: usize = 80;
/// The longest principal, in characters.
pub const PRINCIPAL_MAX: usize = 120;
/// The longest persona, in characters.
pub const PERSONA_MAX: usize = 50_000;

/// The default agent's name, as the apps show it.
pub const DEFAULT_NAME: &str = "Main";
/// Who the default agent works for.
pub const DEFAULT_PRINCIPAL: &str = "the owner";

/// A configured assistant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    /// Its id.
    pub id: AgentId,
    /// Its name, as the apps show it.
    pub name: String,
    /// Its system prompt; none for the default agent, which runs on the server's own rules.
    pub persona: Option<String>,
    /// Who it works for, a display name only: the policies decide, not this.
    pub principal: String,
    /// Its own room.
    pub room: RoomId,
    /// The project it is scoped to: it reads that project's facts and never writes them.
    pub project: Option<ProjectId>,
    /// The most its approved spending may add up to in a calendar month; none: no spending.
    pub spend_cap: Option<SpendCap>,
    /// A disabled agent's room takes no turns and its approvals are not routed.
    pub enabled: bool,
    /// Its tasks' workers may run shell commands (as the server's user): for an agent the owner
    /// trusts. Off by default, and always off for the default agent, whose workers are the
    /// owner's own.
    pub worker_shell: bool,
}

impl Agent {
    /// The default agent: the assistant the server already runs, in Main, with no persona of
    /// its own, every tool, and every approval the owner's.
    #[must_use]
    pub fn main() -> Self {
        Self {
            id: AgentId::MAIN,
            name: DEFAULT_NAME.to_owned(),
            persona: None,
            principal: DEFAULT_PRINCIPAL.to_owned(),
            room: RoomId::MAIN,
            project: None,
            spend_cap: None,
            enabled: true,
            worker_shell: false,
        }
    }

    /// Checks the agent as it would be stored: a name and a principal that say something and
    /// are not too long, a persona (when given) that says something, and a room of its own.
    /// The default agent stays as [`Agent::main`] makes it, but for its spending cap: it
    /// cannot be renamed, given a persona, a project or another room, or disabled.
    ///
    /// # Errors
    ///
    /// [`AgentError`] naming the first field refused.
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.id.is_main() {
            return self.validate_main();
        }
        text_field(&self.name, "name", NAME_MAX)?;
        text_field(&self.principal, "principal", PRINCIPAL_MAX)?;
        if let Some(persona) = &self.persona {
            text_field(persona, "persona", PERSONA_MAX)?;
        }
        if self.room == RoomId::MAIN {
            return Err(AgentError::MainRoom);
        }
        Ok(())
    }

    fn validate_main(&self) -> Result<(), AgentError> {
        let main = Self {
            spend_cap: self.spend_cap.clone(),
            ..Self::main()
        };
        let refused = if self.name != main.name {
            Some("keeps its name")
        } else if self.principal != main.principal {
            Some("works for the owner")
        } else if self.persona.is_some() {
            Some("runs on the server's own rules, with no persona")
        } else if self.room != main.room {
            Some("keeps Main as its room")
        } else if self.project.is_some() {
            Some("is not scoped to a project")
        } else if !self.enabled {
            Some("cannot be disabled")
        } else if self.worker_shell {
            Some("has the owner's own workers, not an agent's")
        } else {
            None
        };
        refused.map_or(Ok(()), |why| Err(AgentError::DefaultAgent(why)))
    }
}

/// Refuses deleting the default agent.
///
/// # Errors
///
/// [`AgentError::DefaultAgent`] for `main`.
pub fn check_delete(id: &AgentId) -> Result<(), AgentError> {
    if id.is_main() {
        Err(AgentError::DefaultAgent("cannot be deleted"))
    } else {
        Ok(())
    }
}

/// A monthly spending cap: an amount in a currency's smallest unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendCap {
    /// The most, in cents (the currency's hundredth).
    pub cents: u64,
    /// Its currency.
    pub currency: Currency,
}

/// An ISO 4217 currency code: three capital letters (`EUR`, `USD`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Currency(String);

impl Currency {
    /// The code, if `code` is three capital ASCII letters.
    ///
    /// # Errors
    ///
    /// [`AgentError::Currency`].
    pub fn parse(code: &str) -> Result<Self, AgentError> {
        if code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()) {
            Ok(Self(code.to_owned()))
        } else {
            Err(AgentError::Currency(code.to_owned()))
        }
    }

    /// The code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> Agent {
        Agent {
            id: AgentId::parse("client-a").unwrap(),
            name: "Example helper".to_owned(),
            persona: Some("You help the example team.".to_owned()),
            principal: "Example Ltd".to_owned(),
            room: RoomId::new(5),
            project: Some(ProjectId::new(2)),
            spend_cap: Some(SpendCap {
                cents: 10_000,
                currency: Currency::parse("EUR").unwrap(),
            }),
            enabled: true,
            worker_shell: true,
        }
    }

    #[test]
    fn a_full_agent_is_valid() {
        assert_eq!(agent().validate(), Ok(()));
        assert_eq!(Agent::main().validate(), Ok(()));
    }

    #[test]
    fn blank_or_long_fields_are_refused() {
        let mut a = agent();
        a.name = "  ".to_owned();
        assert_eq!(a.validate(), Err(AgentError::Blank("name")));
        let mut a = agent();
        a.principal = "x".repeat(PRINCIPAL_MAX + 1);
        assert_eq!(
            a.validate(),
            Err(AgentError::TooLong {
                field: "principal",
                max: PRINCIPAL_MAX
            })
        );
        let mut a = agent();
        a.persona = Some(String::new());
        assert_eq!(a.validate(), Err(AgentError::Blank("persona")));
        let mut a = agent();
        a.persona = None;
        assert_eq!(a.validate(), Ok(()));
    }

    #[test]
    fn another_agent_never_takes_main() {
        let mut a = agent();
        a.room = RoomId::MAIN;
        assert_eq!(a.validate(), Err(AgentError::MainRoom));
    }

    /// A change to an agent, and why the default agent refuses it.
    type Refused = (fn(&mut Agent), &'static str);

    #[test]
    fn the_default_agent_keeps_its_shape() {
        let changes: [Refused; 7] = [
            (|a| a.name = "Renamed".to_owned(), "keeps its name"),
            (
                |a| a.principal = "Someone".to_owned(),
                "works for the owner",
            ),
            (
                |a| a.persona = Some("Be terse.".to_owned()),
                "runs on the server's own rules, with no persona",
            ),
            (|a| a.room = RoomId::new(4), "keeps Main as its room"),
            (
                |a| a.project = Some(ProjectId::new(1)),
                "is not scoped to a project",
            ),
            (|a| a.enabled = false, "cannot be disabled"),
            (
                |a| a.worker_shell = true,
                "has the owner's own workers, not an agent's",
            ),
        ];
        for (change, why) in changes {
            let mut main = Agent::main();
            change(&mut main);
            assert_eq!(main.validate(), Err(AgentError::DefaultAgent(why)));
        }
        let mut main = Agent::main();
        main.spend_cap = Some(SpendCap {
            cents: 500,
            currency: Currency::parse("USD").unwrap(),
        });
        assert_eq!(main.validate(), Ok(()), "its cap may change");
    }

    #[test]
    fn the_default_agent_is_never_deleted() {
        assert_eq!(
            check_delete(&AgentId::MAIN),
            Err(AgentError::DefaultAgent("cannot be deleted"))
        );
        assert_eq!(check_delete(&agent().id), Ok(()));
    }

    #[test]
    fn currencies() {
        assert_eq!(Currency::parse("EUR").unwrap().to_string(), "EUR");
        for bad in ["eur", "EU", "EURO", "E1R", ""] {
            assert_eq!(
                Currency::parse(bad),
                Err(AgentError::Currency(bad.to_owned()))
            );
        }
    }
}
