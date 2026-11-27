//! Why an agent, or one of its lists, can't be what was asked.

use super::id::InvalidAgentId;

/// Why an agent, its channels, tools, accounts or trusted requesters are refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    /// Not an agent's id.
    #[error(transparent)]
    Id(#[from] InvalidAgentId),
    /// A field that must say something is blank.
    #[error("{0} must not be blank")]
    Blank(&'static str),
    /// A field is longer than it may be.
    #[error("{field} is longer than {max} characters")]
    TooLong {
        /// The field.
        field: &'static str,
        /// Its longest.
        max: usize,
    },
    /// A spending cap's currency that is not a currency code.
    #[error("{0:?} is not a currency code: three capital letters, such as EUR")]
    Currency(String),
    /// An agent other than the default one given Main, the default agent's room.
    #[error("Main is the default agent's room; another agent has a room of its own")]
    MainRoom,
    /// A change the default agent does not take.
    #[error("the default agent {0}")]
    DefaultAgent(&'static str),
    /// A channel that can't be one.
    #[error("channels[{index}]: {why}")]
    Channel {
        /// Its place in the list.
        index: usize,
        /// Why not.
        why: &'static str,
    },
    /// A tool that can't be one.
    #[error("tools[{index}]: {why}")]
    Tool {
        /// Its place in the list.
        index: usize,
        /// Why not.
        why: &'static str,
    },
    /// One of the server's tools that act as the owner, given to an agent that isn't the
    /// default one.
    #[error("tools[{index}]: {name} acts as the owner, so only the default agent has it")]
    OwnerTool {
        /// Its place in the list.
        index: usize,
        /// The tool's name.
        name: String,
    },
    /// An account that can't be one.
    #[error("accounts[{index}]: {why}")]
    Account {
        /// Its place in the list.
        index: usize,
        /// Why not.
        why: &'static str,
    },
    /// A trusted requester that can't be one.
    #[error("requesters[{index}]: {why}")]
    Requester {
        /// Its place in the list.
        index: usize,
        /// Why not.
        why: &'static str,
    },
}

/// `text`, refused when blank or longer than `max` characters.
pub(crate) fn text_field(text: &str, field: &'static str, max: usize) -> Result<(), AgentError> {
    if text.trim().is_empty() {
        return Err(AgentError::Blank(field));
    }
    if text.chars().count() > max {
        return Err(AgentError::TooLong { field, max });
    }
    Ok(())
}
