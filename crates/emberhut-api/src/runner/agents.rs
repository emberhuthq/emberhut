//! Every agent working now across the person's tasks (the state's `agentsNow`): each running
//! lead (a task's worker, or the runner session its run is in), then its helper agents, with who
//! runs it and where.

use serde::{Deserialize, Serialize};

use crate::ids::{MachineId, SessionId, TaskId, ToolCallId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Who runs an agent.
    pub enum AgentWho {
        /// The assistant's worker on a task.
        Worker = "worker",
        /// A runner session the assistant runs for a task.
        Session = "session",
        /// An ultra run's lead.
        Lead = "lead",
        /// A session the person opened.
        Direct = "direct",
        /// A helper agent a lead started.
        Helper = "helper",
    }
}

/// One agent working now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentNow {
    /// Its row's id (`lead-<task>`, `agent-<id>`).
    pub id: String,
    /// Its task.
    pub task: TaskId,
    /// The task's key.
    pub key: String,
    /// The task's title.
    #[serde(default)]
    pub title: Option<String>,
    /// Its name (`Worker`, `Lead`, a helper's own).
    #[serde(default)]
    pub name: Option<String>,
    /// What it is doing now.
    #[serde(default)]
    pub line: Option<String>,
    /// When it started.
    #[serde(default)]
    pub started: Option<Timestamp>,
    /// Its tokens so far.
    #[serde(default)]
    pub tokens: Option<f64>,
    /// The session to watch it in; none: nothing to watch.
    #[serde(default)]
    pub viewer: Option<SessionId>,
    /// A helper's own steps in that session: the call that started it.
    #[serde(default)]
    pub parent: Option<ToolCallId>,
    /// It is its task's lead.
    pub lead: bool,
    /// Who runs it; none from an older server.
    #[serde(default)]
    pub kind: Option<AgentWho>,
    /// The runner it is on.
    #[serde(default)]
    pub machine: Option<MachineId>,
    /// That runner, in words.
    #[serde(default)]
    pub place: Option<String>,
    /// The account it runs on.
    #[serde(default)]
    pub account: Option<String>,
    /// That account's name.
    #[serde(default)]
    pub account_name: Option<String>,
}

impl AgentNow {
    /// Who runs it: its kind, or for an older server's row, the lead's name says it and every
    /// other row is a helper.
    #[must_use]
    pub fn who(&self) -> AgentWho {
        if let Some(kind) = &self.kind {
            return kind.clone();
        }
        if !self.lead {
            return AgentWho::Helper;
        }
        match self.name.as_deref() {
            Some("Worker") => AgentWho::Worker,
            Some("Lead") => AgentWho::Lead,
            _ => AgentWho::Session,
        }
    }

    /// Its account, by name when it has one (the same account on two machines is one).
    #[must_use]
    pub fn account_label(&self) -> Option<&str> {
        self.account_name.as_deref().or(self.account.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(lead: bool, name: &str, kind: Option<AgentWho>) -> AgentNow {
        AgentNow {
            id: "lead-1".into(),
            task: TaskId::new(1),
            key: "EX-1".into(),
            title: None,
            name: Some(name.into()),
            line: None,
            started: None,
            tokens: None,
            viewer: None,
            parent: None,
            lead,
            kind,
            machine: None,
            place: None,
            account: Some("a-1".into()),
            account_name: None,
        }
    }

    #[test]
    fn an_older_servers_row_says_who_by_its_name() {
        assert_eq!(agent(true, "Worker", None).who(), AgentWho::Worker);
        assert_eq!(agent(true, "Lead", None).who(), AgentWho::Lead);
        assert_eq!(agent(true, "Session", None).who(), AgentWho::Session);
        assert_eq!(agent(false, "Part 1", None).who(), AgentWho::Helper);
        assert_eq!(
            agent(true, "Worker", Some(AgentWho::Direct)).who(),
            AgentWho::Direct
        );
        assert_eq!(agent(true, "Worker", None).account_label(), Some("a-1"));
    }
}
