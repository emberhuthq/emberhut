//! Runs: every turn of work on a task (one turn of a thread, or one worker session), so a
//! task's state is a fact, not a label. From its runs, a task's live state: running (a run is
//! open and started), queued (sent, waiting behind another turn), waiting (an action for the
//! person), stalled (open, nothing running, nothing asked), done or canceled.

use std::borrow::Cow;
use std::sync::LazyLock;

use jiff::Timestamp;
use regex::Regex;

use super::ask::plain_text;
use super::{RunId, Status, Task, TaskId};

/// What a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunKind {
    /// A turn of a thread or a runner session.
    Thread,
    /// A worker's session.
    Worker,
}

impl RunKind {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Worker => "worker",
        }
    }

    /// The kind `word` names; anything else is a thread's turn.
    #[must_use]
    pub fn parse(word: &str) -> Self {
        if word == "worker" {
            Self::Worker
        } else {
            Self::Thread
        }
    }
}

/// One turn of work on a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Its id.
    pub id: RunId,
    /// The task it was for.
    pub task: Option<TaskId>,
    /// The machine it ran on (`worker` for a worker's).
    pub machine: String,
    /// Its thread, session or unit.
    pub thread: String,
    /// Its kind.
    pub kind: RunKind,
    /// What it was asked (400 characters at most).
    pub asked: Option<String>,
    /// When it was sent.
    pub requested: Option<Timestamp>,
    /// When it started.
    pub started: Option<Timestamp>,
    /// When it ended.
    pub ended: Option<Timestamp>,
    /// How it ended.
    pub outcome: Option<String>,
    /// Its last answer.
    pub last: Option<String>,
    /// Its status line.
    pub status_line: Option<String>,
    /// It was matched to its task by a guess.
    pub inferred: bool,
    /// Its end was applied to its task.
    pub applied: bool,
}

/// A run to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRun {
    /// Its task.
    pub task: TaskId,
    /// Its machine.
    pub machine: String,
    /// Its thread, session or unit.
    pub thread: String,
    /// Its kind.
    pub kind: RunKind,
    /// What it was asked.
    pub asked: Option<String>,
    /// It is already running (a worker's session), not only sent.
    pub started: bool,
}

/// Where a task stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiveState {
    /// A run is on.
    Running,
    /// Sent, waiting behind another turn.
    Queued,
    /// Waiting on the person.
    Waiting,
    /// Open, with nothing running and nothing asked.
    Stalled,
    /// Not started.
    Todo,
    /// Parked until a day.
    Parked,
    /// Held by its parent, which waits on the person.
    Held,
    /// Done.
    Done,
    /// Put away.
    Canceled,
}

impl LiveState {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Waiting => "waiting",
            Self::Stalled => "stalled",
            Self::Todo => "todo",
            Self::Parked => "parked",
            Self::Held => "held",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}

/// The run doing a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveRun {
    /// Its machine.
    pub machine: String,
    /// That machine in words.
    pub place: String,
    /// Its thread, session or unit.
    pub thread: String,
    /// It is a worker's session.
    pub worker: bool,
    /// When it was sent.
    pub requested: Option<Timestamp>,
    /// When it started.
    pub started: Option<Timestamp>,
}

/// How a task's last run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveLast {
    /// When.
    pub ended: Option<Timestamp>,
    /// Its last line, plain.
    pub line: Option<String>,
}

/// A task's live state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// Where it stands.
    pub state: LiveState,
    /// Since when.
    pub since: Option<Timestamp>,
    /// Its run was matched by a guess.
    pub inferred: Option<bool>,
    /// The run doing it.
    pub run: Option<LiveRun>,
    /// Parked until.
    pub until: Option<String>,
    /// The parent holding it.
    pub by: Option<TaskId>,
    /// How its last run ended.
    pub last: Option<LiveLast>,
}

impl Live {
    fn bare(state: LiveState) -> Self {
        Self {
            state,
            since: None,
            inferred: None,
            run: None,
            until: None,
            by: None,
            last: None,
        }
    }
}

/// A machine in words, for the app: a worker runs on the server's own machine.
#[must_use]
pub fn place_of(machine: &str) -> String {
    if machine == RunKind::Worker.as_str() {
        "this machine".to_owned()
    } else {
        machine.to_owned()
    }
}

/// A pattern of this module's: none if it didn't compile (its tests say so), and then what it
/// would find isn't found.
fn rx(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

static STATUS: LazyLock<Option<Regex>> =
    LazyLock::new(|| rx(r"(?i)STATUS:\s*(done|working|blocked|needs\s+\w+)\b\s*:?\s*([^\n]*)"));
static LINKS: LazyLock<Option<Regex>> =
    LazyLock::new(|| rx(r"(?:https?://|~/|\.{0,2}/[\w.-]+/)\S*"));
static HASHES: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\b[0-9a-f]{7,40}\b"));
static COACHING: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\A💬[^\n]*\n+"));
static STATUS_LINES: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"STATUS:[^\n]*"));
static ASK_TAIL: LazyLock<Option<Regex>> =
    LazyLock::new(|| rx(r"(?is)\s*End your reply with its STATUS line.*\z"));

/// `text` with `pattern`'s matches replaced (as is when the pattern is missing).
fn replace<'t>(
    pattern: &LazyLock<Option<Regex>>,
    text: &'t str,
    with: impl regex::Replacer,
) -> Cow<'t, str> {
    pattern
        .as_ref()
        .map_or(Cow::Borrowed(text), |re| re.replace_all(text, with))
}

/// A run's last status line (`STATUS: done`, `working: ...`, `blocked: ...`, `needs ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    /// `done`, `working`, `blocked` or `needs`.
    pub kind: String,
    /// What it says after the word.
    pub text: String,
    /// The line as written.
    pub raw: String,
}

/// The last `STATUS:` line of a run's answer.
#[must_use]
pub fn parse_status(text: &str) -> Option<StatusLine> {
    let caps = STATUS.as_ref()?.captures_iter(text).last()?;
    let word = caps[1].to_lowercase();
    Some(StatusLine {
        kind: if word.starts_with("needs") {
            "needs".to_owned()
        } else {
            word
        },
        text: caps[2].trim().replace(['*', '_', '`'], ""),
        raw: caps[0].trim().to_owned(),
    })
}

/// A run's answer as its task's page shows it: without the coaching line a turn may open with
/// (`💬 ...`) and without its `STATUS:` lines.
#[must_use]
pub fn answer_text(text: &str) -> String {
    let text = replace(&COACHING, text, "");
    replace(&STATUS_LINES, &text, "").into_owned()
}

/// What a run was asked, as its task's page shows it: without the closing instruction to end
/// the reply with a STATUS line.
#[must_use]
pub fn asked_text(text: &str) -> String {
    replace(&ASK_TAIL, text, "").into_owned()
}

/// A run's answer as one plain line for the app: no status lines, links, paths or commit
/// hashes; the first line with words, 200 characters at most.
#[must_use]
pub fn plain_line(text: &str) -> Option<String> {
    let text = replace(&STATUS, text, "");
    let text = plain_text(&text);
    let text = replace(&LINKS, &text, "");
    let text = replace(&HASHES, &text, |caps: &regex::Captures<'_>| {
        if caps[0].bytes().any(|b| b.is_ascii_digit()) {
            String::new()
        } else {
            caps[0].to_owned()
        }
    });
    let line = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|line| line.chars().any(char::is_alphanumeric))?;
    Some(line.chars().take(200).collect())
}

/// A task's live state from what is actually happening, first match wins: its own open turn or
/// worker session; its worker's unit; an action for the person; not started; parked; held by a
/// parent that waits on the person; else, when it is marked working, stalled.
#[must_use]
pub fn live_of(
    task: &Task,
    open: Option<&Run>,
    last: Option<&Run>,
    parent: Option<&Task>,
    now: Timestamp,
) -> Live {
    match task.status {
        Status::Done => return Live::bare(LiveState::Done),
        Status::Canceled => return Live::bare(LiveState::Canceled),
        _ => {}
    }
    let last_out = last.map(|run| LiveLast {
        ended: run.ended,
        line: run
            .last
            .as_deref()
            .and_then(|last| plain_line(&answer_text(last))),
    });
    if let Some(run) = open {
        return Live {
            state: if run.started.is_some() {
                LiveState::Running
            } else {
                LiveState::Queued
            },
            since: run.started.or(run.requested),
            inferred: Some(run.inferred),
            run: Some(LiveRun {
                machine: run.machine.clone(),
                place: place_of(&run.machine),
                thread: run.thread.clone(),
                worker: run.kind == RunKind::Worker,
                requested: run.requested,
                started: run.started,
            }),
            last: last_out,
            ..Live::bare(LiveState::Running)
        };
    }
    if let Some(unit) = &task.worker {
        return Live {
            since: Some(task.updated),
            run: Some(LiveRun {
                machine: RunKind::Worker.as_str().to_owned(),
                place: place_of(RunKind::Worker.as_str()),
                thread: unit.clone(),
                worker: true,
                requested: None,
                started: None,
            }),
            ..Live::bare(LiveState::Running)
        };
    }
    if task.status == Status::Waiting || task.action.is_some() {
        return Live {
            last: last_out,
            ..Live::bare(LiveState::Waiting)
        };
    }
    if task.status == Status::Todo {
        return Live::bare(LiveState::Todo);
    }
    if let Some(until) = &task.parked_until
        && until.as_str() > now.to_string().as_str()
    {
        return Live {
            until: Some(until.clone()),
            last: last_out,
            ..Live::bare(LiveState::Parked)
        };
    }
    if let Some(parent) = parent
        && (parent.status == Status::Waiting || parent.action.is_some())
    {
        return Live {
            by: Some(parent.id),
            last: last_out,
            ..Live::bare(LiveState::Held)
        };
    }
    Live {
        since: last.and_then(|run| run.ended).or(Some(task.updated)),
        last: last_out,
        ..Live::bare(LiveState::Stalled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::tests::task;

    #[test]
    fn every_pattern_compiles() {
        assert!(STATUS.is_some(), "STATUS");
        assert!(LINKS.is_some(), "LINKS");
        assert!(HASHES.is_some(), "HASHES");
        assert!(COACHING.is_some(), "COACHING");
        assert!(STATUS_LINES.is_some(), "STATUS_LINES");
        assert!(ASK_TAIL.is_some(), "ASK_TAIL");
    }

    fn run(started: bool, ended: bool) -> Run {
        Run {
            id: RunId::new(1),
            task: Some(TaskId::new(1)),
            machine: "runner".to_owned(),
            thread: "r-1".to_owned(),
            kind: RunKind::Thread,
            asked: None,
            requested: Some(Timestamp::UNIX_EPOCH),
            started: started.then_some(Timestamp::UNIX_EPOCH),
            ended: ended.then_some(Timestamp::UNIX_EPOCH),
            outcome: None,
            last: Some("Fixed it in https://example.com/pr/3\nSTATUS: done".to_owned()),
            status_line: None,
            inferred: false,
            applied: false,
        }
    }

    #[test]
    fn what_happens_on_a_task_gives_its_state() {
        let now = Timestamp::UNIX_EPOCH;
        let mut t = task(1);
        t.status = Status::Working;
        assert_eq!(
            live_of(&t, Some(&run(true, false)), None, None, now).state,
            LiveState::Running
        );
        assert_eq!(
            live_of(&t, Some(&run(false, false)), None, None, now).state,
            LiveState::Queued
        );
        let stalled = live_of(&t, None, Some(&run(true, true)), None, now);
        assert_eq!(stalled.state, LiveState::Stalled);
        assert_eq!(stalled.last.unwrap().line.as_deref(), Some("Fixed it in"));
        let mut parent = task(2);
        parent.status = Status::Waiting;
        parent.action = Some("Approve".to_owned());
        assert_eq!(
            live_of(&t, None, None, Some(&parent), now).by,
            Some(TaskId::new(2))
        );
        t.status = Status::Todo;
        assert_eq!(live_of(&t, None, None, None, now).state, LiveState::Todo);
        t.status = Status::Done;
        assert_eq!(live_of(&t, None, None, None, now).state, LiveState::Done);
    }

    #[test]
    fn a_runs_answer_loses_its_coaching_line_and_status_lines() {
        assert_eq!(
            answer_text("💬 Keep it short\n\nFixed the login.\nSTATUS: done: shipped\nMore"),
            "Fixed the login.\n\nMore"
        );
        assert_eq!(
            answer_text("Said 💬 here\nSTATUS: working"),
            "Said 💬 here\n",
            "only a first line that opens with it is coaching"
        );
        assert_eq!(answer_text(""), "");
    }

    #[test]
    fn what_a_run_was_asked_loses_the_status_instruction() {
        assert_eq!(
            asked_text("Check the build.\n\nend your reply with its STATUS line:\nSTATUS: done"),
            "Check the build."
        );
        assert_eq!(asked_text("Nothing to cut"), "Nothing to cut");
    }

    #[test]
    fn a_status_line_is_read_from_the_end() {
        let status =
            parse_status("STATUS: working: on it\nmore\nSTATUS: blocked: **no key**").unwrap();
        assert_eq!(status.kind, "blocked");
        assert_eq!(status.text, "no key");
        assert_eq!(parse_status("no status").map(|s| s.kind), None);
    }
}
