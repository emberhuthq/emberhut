//! The keys of a runner session's `meta`: what the server writes when it opens a session (and
//! what anyone opening one through a runner's own API may write), which the runner keeps with
//! the session and sends back, as given, in every event and transcript head. The runner reads
//! none of them; the server reads them, on a server that keeps plans (`--task-plans`), to know
//! which task the session works for.
//!
//! A session is linked to a task once, when the server first takes a turn of it (or first sees
//! it): the task its meta names; else its parent's tasks; else, for one of the person's own
//! sessions first messaged after this server started making them, a task made for it. From
//! then on its links say which task it works for, not its meta: a session moved to another
//! task stays there.

/// `true`: the person opened the session and drives it themselves (not work the assistant
/// handed off).
pub const DIRECT: &str = "direct";

/// The task the session works for: its number (a JSON number, or its digits as text) or its
/// key (`T-12`).
pub const TASK: &str = "task";

/// The session that started this one, by id (`r-…`): a session with no task of its own works
/// for its parent's. A session that starts another through a runner's API directly sets it to
/// its own id.
pub const PARENT: &str = "parent";
