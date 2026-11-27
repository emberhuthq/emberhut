//! Coding threads: the sessions of coding agents the server knows about, on every machine, and
//! the ones the assistant started and follows.
//!
//! A thread is one coding agent's conversation on one machine: a runner session, or a thread a
//! connected coding tool reports. The server keeps the latest look at each one (its status,
//! what it was asked, its latest answer) so the assistant and the app know what every thread is
//! about. A *followed* thread is one the assistant handed work to: when it ends (finished,
//! failed, stopped, or waiting for an answer), the assistant hears of it at once, and it stops
//! being followed unless it is only waiting.

use std::fmt;

use jiff::Timestamp;

/// Where a thread is. The words are the runners' own; one the server doesn't know is kept as
/// it came ([`ThreadStatus::Other`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThreadStatus {
    /// Waiting to start its first turn.
    Queued,
    /// A turn runs now.
    Running,
    /// It asks a question and waits for the answer.
    Waiting,
    /// Between turns, nothing asked.
    Idle,
    /// Its turn finished.
    Completed,
    /// Its turn failed.
    Failed,
    /// Its turn was stopped.
    Interrupted,
    /// It was cancelled before it ran.
    Cancelled,
    /// A word this server doesn't know.
    Other(String),
}

impl ThreadStatus {
    /// The status's word.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
            Self::Other(word) => word,
        }
    }

    /// Whether a thread in this status still holds the person's attention: it waits to start,
    /// runs, or waits for an answer. The coding-thread app's threads in one of these are listed
    /// beside the runner's sessions.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Waiting)
    }

    /// Whether a thread in this status has come to an end of what it was given: finished,
    /// failed, stopped, cancelled, or waiting for an answer. Those are what a followed thread's
    /// follower hears.
    #[must_use]
    pub fn is_end(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Interrupted | Self::Waiting | Self::Cancelled
        )
    }
}

impl From<&str> for ThreadStatus {
    fn from(word: &str) -> Self {
        match word {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "idle" => Self::Idle,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            "cancelled" => Self::Cancelled,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A coding thread, as last seen. A thread is known by its machine and its id together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodingThread {
    /// The machine it runs on, as its runner names itself.
    pub machine: String,
    /// Its id on that machine.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its project, by name (empty when it has none).
    pub project: String,
    /// Where it is.
    pub status: ThreadStatus,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// The thread that started it, for a helper agent's thread; `None` for a top-level one.
    pub parent: Option<String>,
    /// What it was first asked.
    pub ask: Option<String>,
    /// What it was asked last.
    pub last_ask: Option<String>,
    /// Its latest answer.
    pub last: Option<String>,
    /// The status line its latest answer ended with.
    pub status_line: Option<String>,
    /// What it is doing now, while a turn runs.
    pub doing: Option<String>,
    /// The account it runs on.
    pub account: Option<String>,
    /// The folder it works in.
    pub cwd: Option<String>,
    /// The branch it works on.
    pub branch: Option<String>,
    /// Since when its current turn runs.
    pub since: Option<Timestamp>,
    /// The person opened it and drives it themselves (not work the assistant handed off).
    pub direct: bool,
    /// The model it runs on.
    pub model: Option<String>,
    /// The reasoning effort it runs with.
    pub effort: Option<String>,
    /// A question it waits on the person to answer.
    pub question: Option<String>,
}

impl CodingThread {
    /// A new top-level thread with nothing known about it yet beyond these.
    #[must_use]
    pub fn new(
        machine: impl Into<String>,
        id: impl Into<String>,
        title: impl Into<String>,
        status: ThreadStatus,
        updated_at: Timestamp,
    ) -> Self {
        Self {
            machine: machine.into(),
            id: id.into(),
            title: title.into(),
            project: String::new(),
            status,
            updated_at,
            parent: None,
            ask: None,
            last_ask: None,
            last: None,
            status_line: None,
            doing: None,
            account: None,
            cwd: None,
            branch: None,
            since: None,
            direct: false,
            model: None,
            effort: None,
            question: None,
        }
    }

    /// Whether this is a top-level thread (not a helper agent's).
    #[must_use]
    pub fn is_top_level(&self) -> bool {
        self.parent.as_deref().is_none_or(str::is_empty)
    }
}

/// A thread the assistant started, or took on, and follows: why, and since when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Followed {
    /// The thread's id.
    pub thread: String,
    /// The machine it runs on.
    pub machine: String,
    /// Why it is followed: the work it was given, in a few words.
    pub why: String,
    /// When it was followed.
    pub at: Timestamp,
}

/// The longest `why` kept for a followed thread, in characters.
pub const WHY_LENGTH: usize = 80;

/// What a followed thread's new status means for its follower.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// It came to an end: tell the follower, and stop following it.
    Ended,
    /// It waits for an answer: tell the follower, and keep following it.
    Waiting,
}

impl Ending {
    /// Whether the thread stays followed.
    #[must_use]
    pub fn keeps_following(self) -> bool {
        self == Self::Waiting
    }
}

/// What `now`, a followed thread's new look, means for its follower, given its status at the
/// previous look (`None` when it wasn't seen before): an [`Ending`] when it has just come to an
/// end, `None` otherwise.
///
/// Only a change counts, so a thread seen again in the same status is news once. A thread the
/// person drives themselves that they stopped is not news: they stopped it.
#[must_use]
pub fn ending(was: Option<&ThreadStatus>, now: &CodingThread) -> Option<Ending> {
    if was == Some(&now.status) || !now.status.is_end() {
        return None;
    }
    if now.direct && now.status == ThreadStatus::Interrupted {
        return None;
    }
    Some(if now.status == ThreadStatus::Waiting {
        Ending::Waiting
    } else {
        Ending::Ended
    })
}

/// The most items a thread's activity shows: its latest steps.
pub const ACTIVITY_ITEMS: usize = 40;

/// The longest an activity item's text is shown, in characters.
pub const ACTIVITY_TEXT: usize = 1200;

/// The kind of activity item that only marks a checkpoint: never shown.
const CHECKPOINT: &str = "checkpoint";

/// One step of a thread's activity: a message, a tool call, a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityItem {
    /// Its kind, in the coding-thread app's word (`user_message`, `assistant_message`, ...).
    pub kind: String,
    /// Its title, when it has one.
    pub title: Option<String>,
    /// What it says.
    pub text: String,
    /// When it last changed, as the app wrote it.
    pub at: Option<String>,
}

/// The items of a thread's latest steps worth showing, in their order: checkpoints and items
/// with nothing to say are left out, and each text is cut to [`ACTIVITY_TEXT`] characters.
#[must_use]
pub fn shown_activity(items: Vec<ActivityItem>) -> Vec<ActivityItem> {
    items
        .into_iter()
        .filter(|item| item.kind != CHECKPOINT && !item.text.trim().is_empty())
        .map(|mut item| {
            if let Some((cut, _)) = item.text.char_indices().nth(ACTIVITY_TEXT) {
                item.text.truncate(cut);
            }
            item
        })
        .collect()
}

/// Where to start reading a thread of `count` items so the read ends with its newest `limit`.
#[must_use]
pub fn tail_start(count: u64, limit: usize) -> u64 {
    count.saturating_sub(u64::try_from(limit).unwrap_or(u64::MAX))
}

/// The id a thread is known by on `machine`, from `id` as given. Threads the coding-thread
/// app started for another tool carry a prefix (`mcp:<uuid>`), and an id written from memory
/// often drops it: when no known thread there has `id` itself, the one top-level thread whose
/// id ends with `:<id>` is the one meant. Otherwise `id` as given (the app answers that it
/// doesn't know it).
#[must_use]
pub fn resolve_thread_id<'a>(known: &'a [CodingThread], machine: &str, id: &'a str) -> &'a str {
    let mine = || known.iter().filter(|t| t.machine == machine);
    if mine().any(|t| t.id == id) {
        return id;
    }
    let suffix = format!(":{id}");
    let mut like = mine().filter(|t| t.is_top_level() && t.id.ends_with(&suffix));
    match (like.next(), like.next()) {
        (Some(only), None) => &only.id,
        _ => id,
    }
}

/// What a thread was about, read from its items: its first request, its latest request and
/// its latest answer, each with its runs of white space made one space and cut short.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Digest {
    /// Its first request, at most [`DIGEST_ASK`] characters.
    pub ask: Option<String>,
    /// Its latest request, at most [`DIGEST_ASK`] characters.
    pub last_ask: Option<String>,
    /// Its latest answer, at most [`DIGEST_ANSWER`] characters.
    pub last: Option<String>,
}

/// The longest request a digest keeps, in characters.
pub const DIGEST_ASK: usize = 600;

/// The longest answer a digest keeps, in characters.
pub const DIGEST_ANSWER: usize = 1500;

/// The kind of item the person wrote.
const USER_MESSAGE: &str = "user_message";

/// The kind of item the agent answered.
const ASSISTANT_MESSAGE: &str = "assistant_message";

impl Digest {
    /// The digest of a thread whose first items are `head` and latest items `tail`, both in
    /// their order.
    #[must_use]
    pub fn of(head: &[ActivityItem], tail: &[ActivityItem]) -> Self {
        fn first<'a>(
            kind: &str,
            mut items: impl Iterator<Item = &'a ActivityItem>,
            max: usize,
        ) -> Option<String> {
            items
                .find(|item| item.kind == kind)
                .map(|item| clip(&item.text, max))
        }
        Self {
            ask: first(USER_MESSAGE, head.iter(), DIGEST_ASK),
            last_ask: first(USER_MESSAGE, tail.iter().rev(), DIGEST_ASK),
            last: first(ASSISTANT_MESSAGE, tail.iter().rev(), DIGEST_ANSWER),
        }
    }
}

/// `text` with its runs of white space made one space, trimmed, and cut to `max` characters.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(status: ThreadStatus) -> CodingThread {
        CodingThread::new(
            "runner",
            "r-1",
            "Fix the build",
            status,
            Timestamp::UNIX_EPOCH,
        )
    }

    #[test]
    fn statuses_keep_their_words() {
        for word in [
            "queued",
            "running",
            "waiting",
            "idle",
            "completed",
            "failed",
            "interrupted",
            "cancelled",
            "parked",
        ] {
            assert_eq!(ThreadStatus::from(word).as_str(), word);
        }
        assert_eq!(
            ThreadStatus::from("parked"),
            ThreadStatus::Other("parked".to_owned())
        );
        assert_eq!(ThreadStatus::Running.to_string(), "running");
    }

    #[test]
    fn the_ends_are_what_a_follower_hears() {
        let ends: Vec<&str> = [
            "queued",
            "running",
            "waiting",
            "idle",
            "completed",
            "failed",
            "interrupted",
            "cancelled",
            "parked",
        ]
        .into_iter()
        .filter(|word| ThreadStatus::from(*word).is_end())
        .collect();
        assert_eq!(
            ends,
            ["waiting", "completed", "failed", "interrupted", "cancelled"]
        );
    }

    #[test]
    fn an_end_is_news_once_and_waiting_keeps_it_followed() {
        let done = thread(ThreadStatus::Completed);
        assert_eq!(
            ending(Some(&ThreadStatus::Running), &done),
            Some(Ending::Ended)
        );
        assert_eq!(ending(None, &done), Some(Ending::Ended), "first seen ended");
        assert_eq!(ending(Some(&ThreadStatus::Completed), &done), None);
        assert_eq!(
            ending(Some(&ThreadStatus::Running), &thread(ThreadStatus::Running)),
            None
        );
        let waiting = ending(Some(&ThreadStatus::Running), &thread(ThreadStatus::Waiting));
        assert_eq!(waiting, Some(Ending::Waiting));
        assert!(waiting.unwrap().keeps_following());
        assert!(!Ending::Ended.keeps_following());
    }

    #[test]
    fn the_person_stopping_their_own_thread_is_not_news() {
        let mut stopped = thread(ThreadStatus::Interrupted);
        stopped.direct = true;
        assert_eq!(ending(Some(&ThreadStatus::Running), &stopped), None);
        stopped.status = ThreadStatus::Failed;
        assert_eq!(
            ending(Some(&ThreadStatus::Running), &stopped),
            Some(Ending::Ended)
        );
    }

    #[test]
    fn a_helper_s_thread_is_not_top_level() {
        let mut helper = thread(ThreadStatus::Running);
        assert!(helper.is_top_level());
        helper.parent = Some(String::new());
        assert!(helper.is_top_level(), "an empty parent is none");
        helper.parent = Some("r-0".to_owned());
        assert!(!helper.is_top_level());
    }

    fn item(kind: &str, text: &str) -> ActivityItem {
        ActivityItem {
            kind: kind.to_owned(),
            title: None,
            text: text.to_owned(),
            at: None,
        }
    }

    #[test]
    fn activity_leaves_out_checkpoints_and_blank_items_and_cuts_long_texts() {
        let long = "é".repeat(ACTIVITY_TEXT + 5);
        let shown = shown_activity(vec![
            item("checkpoint", "saved"),
            item("tool_call", "  \n "),
            item("assistant_message", &long),
            item("user_message", "Go on"),
        ]);
        let kinds: Vec<&str> = shown.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["assistant_message", "user_message"]);
        assert_eq!(shown[0].text.chars().count(), ACTIVITY_TEXT);
        assert_eq!(shown[1].text, "Go on");
    }

    #[test]
    fn the_tail_starts_so_it_ends_with_the_newest() {
        assert_eq!(tail_start(100, 40), 60);
        assert_eq!(tail_start(10, 40), 0);
    }

    #[test]
    fn a_thread_id_without_its_prefix_finds_the_one_it_ends() {
        let at = Timestamp::UNIX_EPOCH;
        let mut helper = CodingThread::new("laptop", "mcp:x-3", "Lane", ThreadStatus::Running, at);
        helper.parent = Some("mcp:x-1".to_owned());
        let known = vec![
            CodingThread::new("laptop", "mcp:x-1", "One", ThreadStatus::Running, at),
            CodingThread::new("laptop", "mcp:x-2", "Two", ThreadStatus::Running, at),
            CodingThread::new(
                "laptop",
                "other:x-2",
                "Two again",
                ThreadStatus::Running,
                at,
            ),
            CodingThread::new("desk", "mcp:x-4", "Four", ThreadStatus::Running, at),
            helper,
        ];
        assert_eq!(resolve_thread_id(&known, "laptop", "mcp:x-1"), "mcp:x-1");
        assert_eq!(resolve_thread_id(&known, "laptop", "x-1"), "mcp:x-1");
        assert_eq!(
            resolve_thread_id(&known, "laptop", "x-2"),
            "x-2",
            "two match"
        );
        assert_eq!(
            resolve_thread_id(&known, "laptop", "x-3"),
            "x-3",
            "a helper's"
        );
        assert_eq!(
            resolve_thread_id(&known, "laptop", "x-4"),
            "x-4",
            "another machine's"
        );
    }

    #[test]
    fn a_digest_has_the_first_and_latest_ask_and_the_latest_answer() {
        let head = [
            item("user_message", "Fix   the\nbuild"),
            item("assistant_message", "On it"),
        ];
        let tail = [
            item("user_message", "And the tests"),
            item("assistant_message", "Tests pass"),
            item("tool_call", "cargo test"),
        ];
        let digest = Digest::of(&head, &tail);
        assert_eq!(digest.ask.as_deref(), Some("Fix the build"));
        assert_eq!(digest.last_ask.as_deref(), Some("And the tests"));
        assert_eq!(digest.last.as_deref(), Some("Tests pass"));
        assert_eq!(Digest::of(&[], &[]), Digest::default());
        assert_eq!(clip(&"a ".repeat(DIGEST_ASK), 5), "a a a");
    }

    #[test]
    fn active_statuses_hold_attention() {
        let active: Vec<&str> = [
            "queued",
            "running",
            "waiting",
            "idle",
            "completed",
            "failed",
        ]
        .into_iter()
        .filter(|word| ThreadStatus::from(*word).is_active())
        .collect();
        assert_eq!(active, ["queued", "running", "waiting"]);
    }
}
