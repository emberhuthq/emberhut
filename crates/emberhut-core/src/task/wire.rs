//! The wire words of the task area: the apps and the assistant's tools read and write these
//! words, so they are values, not prose. Each is written once, here.

/// How a task's owner, a step's `who` and a history line's author name the person the
/// assistant works for.
pub const PERSON: &str = "owner";

/// How the same places name the assistant itself.
pub const ASSISTANT: &str = "assistant";

/// How a history line names a worker (a session doing one task) as its author.
pub const WORKER: &str = "worker";

/// How a task's key starts: task 12's key is this followed by 12.
pub const KEY_PREFIX: &str = "T-";
