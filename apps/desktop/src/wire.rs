//! The wire words the contract leaves to the server: values the server writes and the app reads
//! (or writes back), spelled once here. Each moves to `emberhut-api` once the contract names it.

use emberhut_api::task_message::MessageTo;

/// The server when nothing else names one: a server on this machine, on the server's default
/// port.
pub(crate) const DEFAULT_SERVER: &str = "http://localhost:18990";

/// The owner id the server gives the assistant's own tasks: the assistant's id, which the
/// contract names once ([`MessageTo::ASSISTANT`]).
pub(crate) const ASSISTANT_OWNER: &str = MessageTo::ASSISTANT;

/// The owner id the server gives the person's own tasks, and the `who` of the person's steps in
/// a task's history.
pub(crate) const PERSON_OWNER: &str = emberhut_api::wire::PERSON;

/// How task keys start (`T-12`): an action line naming another task uses it.
pub(crate) const TASK_KEY_PREFIX: &str = "T-";

/// How a link to a task starts, before the task's number: the server writes such links in notes
/// and messages, and the app opens them: the app's own scheme ([`crate::links::SCHEME`]).
pub(crate) const TASK_LINK: &str = "emberhut://task/";

/// The steps of a task's history that only change its state, in the words the server writes.
pub(crate) const QUIET_STEPS: [&str; 11] = [
    "Started",
    "Back to to do",
    "Waiting on the owner",
    "Done",
    "Canceled",
    "Archived by the owner",
    "Unarchived by the owner",
    "Created",
    "A worker started on it",
    "Picked up again with the owner's answer",
    "Picked up again with its new brief",
];

/// How the history step of a task made by someone starts, their name after it.
pub(crate) const CREATED_BY: &str = "Created by ";

/// How the server starts the titles of the sessions it opens for tasks; a title reads without
/// it.
pub(crate) const ASSISTANT_TITLE_PREFIX: &str = "Emberhut · ";

/// What the server adds to every brief it sends a session; a message reads without it.
pub(crate) const BRIEF_TAILS: [&str; 2] = [
    "\n\nEnd every reply with one last line",
    "\n\nEnd your reply with its STATUS line",
];

/// Who drives the assistant's browser: its id ([`ASSISTANT_OWNER`]), which the app reads in any
/// case; a worker of its is this name and `'s worker on <task key>`.
pub(crate) const ASSISTANT_DRIVER: &str = ASSISTANT_OWNER;

/// How the line a server that keeps plans writes in a task's history when a step is ticked done
/// starts, the step's title after it.
pub(crate) const PLAN_TICKED: &str = "Ticked: ";

/// How its line for a step skipped starts, the step's title after it.
pub(crate) const PLAN_SKIPPED: &str = "Skipped: ";

/// How its line for a step blocked starts, the step's title and ` — <note>` after it.
pub(crate) const PLAN_BLOCKED: &str = "Blocked: ";

/// How its lines for the plan's other changes start: the plan written (`Plan set: 6 steps, 2
/// done`), a step started, reopened, added or renamed (`Renamed step: A → B`), a step's note
/// (`Note on <step>: …`) and the agent's estimate.
pub(crate) const PLAN_CHANGES: [&str; 7] = [
    "Plan set: ",
    "Started: ",
    "Reopened: ",
    "Added step: ",
    "Renamed step: ",
    "Note on ",
    "Plan estimate: ",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_link_is_an_app_link() {
        let scheme = TASK_LINK.split_once("://").map(|(scheme, _)| scheme);
        assert_eq!(scheme, Some(crate::links::SCHEME));
    }
}
