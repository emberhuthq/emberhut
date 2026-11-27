//! An agent's lines: who it works for, Off while disabled, and its requests waiting on the
//! user.
use emberhut_api::agents::AgentRow;

use crate::store::approvals::shown::visible;

/// The line under an agent's name in the sidebar: who it works for, and Off while disabled.
#[must_use]
pub(crate) fn agent_line(agent: &AgentRow) -> String {
    let off = if agent.enabled { "" } else { " · Off" };
    format!("Works for {}{off}", visible(&agent.principal))
}

/// The line under an agent room's name in the chat's bar: what the assistant is doing (Off
/// while the agent is disabled) and who the agent works for.
#[must_use]
pub(crate) fn agent_caption(agent: &AgentRow, status: &str) -> String {
    let doing = if agent.enabled { status } else { "Off" };
    format!("{doing} · works for {}", visible(&agent.principal))
}

/// An agent's requests waiting on the user, as its button says them: `2 wait on you`, or
/// `Approvals` for none.
#[must_use]
pub(crate) fn approvals_count(count: usize) -> String {
    match count {
        0 => "Approvals".to_owned(),
        1 => "1 waits on you".to_owned(),
        n => format!("{n} wait on you"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_says_what_waits_on_you() {
        assert_eq!(approvals_count(0), "Approvals");
        assert_eq!(approvals_count(1), "1 waits on you");
        assert_eq!(approvals_count(3), "3 wait on you");
    }
}
