//! The state's `tasks`: the assistant's background work (agents and commands it started and
//! left running), the running ones first, then the last few that finished. Not to be confused
//! with the person's tasks ([`crate::tasks::Issue`]).

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Where a piece of background work stands: running, or the tools' own word once it ended.
    pub enum WorkStatus {
        /// Still running.
        Running = "running",
    }
}

impl Default for WorkStatus {
    /// An older server that leaves the status out: the empty word, not running.
    fn default() -> Self {
        Self::Other(String::new())
    }
}

/// One piece of background work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BackgroundTask {
    /// Its id, as the assistant's tools gave it.
    pub id: String,
    /// What it does, in a line.
    #[serde(default)]
    pub description: String,
    /// Its kind, as the tools name it (an agent, a shell command); shown as it is.
    #[serde(default, rename = "type")]
    pub kind: String,
    /// `running` while it runs; the tools' own word once it ended.
    #[serde(default)]
    pub status: WorkStatus,
    /// When it started.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
    /// What it came to.
    #[serde(default)]
    pub summary: Option<String>,
    /// The room it was started from; left out for Main on older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomId>,
}

impl BackgroundTask {
    /// Whether it is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == WorkStatus::Running
    }
}
