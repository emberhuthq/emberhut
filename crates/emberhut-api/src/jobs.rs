//! `GET /api/jobs`: every background job running on the machines now (the assistant's workers,
//! detached units, long runs, runner sessions, the Mac's own runs), each with its task, how
//! long it has run, its CPU and memory and its last activity; failed ones too. `?fresh=1` looks
//! again rather than answering from the server's few-seconds cache.
//!
//! `POST /api/jobs/stop` and `POST /api/jobs/clear` (`{ id }`) stop a job, or take a failed one
//! off the list; the client asks the person first. A job that has ended, or one that can't be
//! stopped from there, is refused with 400 and an [`crate::error::ErrorBody`].

use serde::{Deserialize, Serialize};

use crate::ids::{MachineJobId, TaskId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// The query that asks for a fresh look.
pub const FRESH: &str = "fresh=1";

open_enum! {
    /// What a job is.
    pub enum JobKind {
        /// The assistant's worker on a task.
        Worker = "worker",
        /// A detached unit.
        Unit = "unit",
        /// A file server or a page: always on.
        Server = "server",
        /// A service: always on.
        Service = "service",
        /// A process outside those.
        Process = "process",
        /// A runner session.
        Session = "session",
        /// One of the Mac's own jobs.
        Launchd = "launchd",
    }
}

open_enum! {
    /// Where a job stands.
    pub enum JobState {
        /// Running.
        Running = "running",
        /// Starting.
        Starting = "starting",
        /// Failed.
        Failed = "failed",
        /// Ended.
        Exited = "exited",
    }
}

/// The list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JobsPage {
    /// When it was made.
    pub at: Timestamp,
    /// The jobs.
    pub jobs: Vec<Job>,
    /// Whether the Mac answered.
    pub mac: JobsMac,
}

/// Whether the Mac's runner answered, and why not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JobsMac {
    /// It answered.
    pub ok: bool,
    /// Why not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Job {
    /// Its id.
    pub id: MachineJobId,
    /// What it is.
    pub kind: JobKind,
    /// The machine it runs on, in words (`Linux PC`, `Mac`).
    pub place: String,
    /// Its name there (a unit's, a process's).
    pub name: String,
    /// Its title.
    pub title: String,
    /// What it is, in a word or two (`Codex run`, `Worker`).
    pub what: String,
    /// The task it works for.
    pub task: Option<JobTask>,
    /// Every task named in it.
    #[serde(default)]
    pub tasks: Vec<TaskId>,
    /// How its task was found (`runner`, `worker`, `name`, `folder`, `session`).
    #[serde(default)]
    pub linked: Option<String>,
    /// Where it stands.
    pub state: JobState,
    /// When it started.
    #[serde(default)]
    pub since: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub ended: Option<Timestamp>,
    /// How it ended (`exit-code (exit 1)`).
    #[serde(default)]
    pub exit: Option<String>,
    /// Its CPU now, in % of one core.
    #[serde(default)]
    pub cpu: Option<f64>,
    /// Its CPU time, in seconds.
    #[serde(default)]
    pub cpu_time: Option<f64>,
    /// Its memory, in bytes.
    #[serde(default)]
    pub memory: Option<u64>,
    /// What it last said or did.
    #[serde(default)]
    pub last: Option<JobActivity>,
    /// Its last file change.
    #[serde(default)]
    pub file: Option<JobActivity>,
    /// Its last sign of life.
    #[serde(default)]
    pub active: Option<Timestamp>,
    /// Its folder.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Its command.
    #[serde(default)]
    pub command: Option<String>,
    /// The ports it serves on.
    #[serde(default)]
    pub ports: Vec<u16>,
    /// It can be stopped from the app.
    #[serde(default)]
    pub stop: bool,
    /// It failed and can be taken off the list.
    #[serde(default)]
    pub clear: bool,
}

impl Job {
    /// Whether it failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.state == JobState::Failed
    }

    /// Whether it runs (or is starting).
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self.state, JobState::Running | JobState::Starting)
    }

    /// Whether it is always on (a service, a file server): listed folded.
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        matches!(self.kind, JobKind::Service | JobKind::Server)
    }
}

/// A job's task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JobTask {
    /// Its id.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Its status.
    pub status: String,
}

/// What a job last said or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JobActivity {
    /// What it said, or the file it changed.
    pub text: String,
    /// When.
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// Where it came from: `codex`, `claude`, `log` or `file`.
    pub from: String,
}

impl JobActivity {
    /// Whether it is a file change.
    #[must_use]
    pub fn is_file(&self) -> bool {
        self.from == "file"
    }

    /// Whether it is a line of the job's log.
    #[must_use]
    pub fn is_log(&self) -> bool {
        self.from == "log"
    }
}

/// What the person does to a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobAction {
    /// Stop it.
    Stop,
    /// Take a failed one off the list.
    Clear,
}

impl JobAction {
    /// Its route's last part.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Clear => "clear",
        }
    }
}

/// `POST /api/jobs/stop` and `/api/jobs/clear`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct JobActionRequest {
    /// The job.
    pub id: MachineJobId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_is_quiet_and_a_failed_one_is_not_live() {
        let job: Job = serde_json::from_value(serde_json::json!({
            "id": "unit:files", "kind": "service", "place": "Linux PC", "name": "files",
            "title": "Files", "what": "Service", "task": null, "state": "failed",
            "stop": false, "clear": true
        }))
        .unwrap();
        assert!(job.is_quiet() && job.is_failed() && !job.is_live());
        assert_eq!(JobAction::Clear.as_str(), "clear");
    }
}
