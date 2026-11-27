//! How sessions read: their status in words and as a mark, a row's muted line, short titles and
//! paths, the plain preview of what one said, who sent a message, the model and effort names,
//! the word for what a running turn does, a timer and a token count. Pure functions.

use emberhut_api::ids::TaskId;
use emberhut_api::runner::accounts::ClaudeAccount;
use emberhut_api::runner::agents::AgentNow;
use emberhut_api::runner::steps::Sender;
use emberhut_api::work::RunnerSession;
use jiff::Timestamp;

use crate::brand::APP_NAME;
use crate::wire::{ASSISTANT_TITLE_PREFIX, BRIEF_TAILS};

/// The server keeps up to 1,500 characters of what a session said last; a text this long that
/// doesn't end as an answer does was cut.
const CUT_NEAR: usize = 1400;

/// A session's state as a mark, as tasks mark theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Running or about to: the live dot.
    Running,
    /// Waiting on the user: a raised hand, in the accent.
    Waiting,
    /// Failed: a red cross.
    Failed,
    /// Stopped: an orange stop.
    Stopped,
    /// Done: a green tick.
    Done,
}

impl Mark {
    /// The mark for a status word.
    #[must_use]
    pub(crate) fn of(status: &str) -> Self {
        match status {
            "running" | "queued" => Self::Running,
            "waiting" => Self::Waiting,
            "failed" => Self::Failed,
            "interrupted" => Self::Stopped,
            _ => Self::Done,
        }
    }

    /// Its state in one word, for a sidebar row: running, waiting, failed, stopped, done.
    #[must_use]
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Done => "done",
        }
    }
}

/// A status in words: `Running`, `Starting`, `Finished its turn`, `Failed`, `Stopped`,
/// `Waiting for you`, `Ready`, else the word itself.
#[must_use]
pub(crate) fn status_text(status: &str) -> String {
    match status {
        "running" => "Running".to_owned(),
        "queued" => "Starting".to_owned(),
        "completed" => "Finished its turn".to_owned(),
        "failed" => "Failed".to_owned(),
        "interrupted" => "Stopped".to_owned(),
        "waiting" => "Waiting for you".to_owned(),
        "idle" => "Ready".to_owned(),
        other => crate::format::capitalized(other),
    }
}

/// A title without the prefix the server starts its sessions' titles with.
#[must_use]
pub(crate) fn short_title(title: &str) -> &str {
    title.strip_prefix(ASSISTANT_TITLE_PREFIX).unwrap_or(title)
}

/// A path with its home folder as `~`: `/home/example/work` reads `~/work`, as does
/// `/Users/example/work`.
#[must_use]
pub(crate) fn short_path(path: &str) -> String {
    for home in ["/home/", "/Users/"] {
        if let Some(rest) = path.strip_prefix(home) {
            return match rest.split_once('/') {
                Some((_, inside)) => format!("~/{inside}"),
                None => "~".to_owned(),
            };
        }
    }
    path.to_owned()
}

/// An account by name: the one the server gave, else its id looked up in the state's
/// `accounts`, else the id itself.
#[must_use]
pub(crate) fn account_name(
    name: Option<&str>,
    id: Option<&str>,
    accounts: &[ClaudeAccount],
) -> Option<String> {
    if let Some(name) = name {
        return Some(name.to_owned());
    }
    let id = id?;
    Some(
        accounts
            .iter()
            .find(|account| account.id == id)
            .map_or_else(|| id.to_owned(), |account| account.name.clone()),
    )
}

/// A row's muted line: its tasks (by `key`), project, machine and account by name (`EX-12 ·
/// Example · Mac · Personal`), the account looked up in `accounts` when the row has only its id.
#[must_use]
pub(crate) fn meta(
    session: &RunnerSession,
    accounts: &[ClaudeAccount],
    key: impl Fn(TaskId) -> String,
) -> String {
    let tasks = session
        .tasks
        .iter()
        .map(|task| key(*task))
        .collect::<Vec<_>>()
        .join(", ");
    [
        Some(tasks),
        Some(session.project.clone()),
        Some(session.place.clone()),
        account_name(
            session.account_name.as_deref(),
            session.account.as_deref(),
            accounts,
        ),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// A sidebar row's help: whose it is, where it runs, its status.
#[must_use]
pub(crate) fn whose(session: &RunnerSession) -> String {
    let whose = if session.direct {
        "Your session".to_owned()
    } else {
        format!("{APP_NAME}'s session")
    };
    let place = if session.place.trim().is_empty() {
        machine_name(session.machine.as_str()).to_owned()
    } else {
        session.place.clone()
    };
    format!("{whose} · {place} · {}", status_text(&session.status))
}

/// A runner machine's name when the server gave none: the Linux PC for the runner an account
/// with no machine is on, else the machine's own word.
#[must_use]
pub(crate) fn machine_name(machine: &str) -> &str {
    if machine == emberhut_api::runner::accounts::LINUX_RUNNER {
        "Linux PC"
    } else {
        machine
    }
}

/// A runner machine's name as the state places it: the place the server gives an account or a
/// session on it (`Mac`), else [`machine_name`].
#[must_use]
pub(crate) fn machine_place(state: Option<&emberhut_api::state::State>, machine: &str) -> String {
    let given = state.and_then(|state| {
        let accounts = state.accounts.iter().filter_map(|account| {
            let on = account
                .machine
                .as_ref()
                .is_some_and(|on| on.as_str() == machine);
            on.then_some(account.place.as_deref()).flatten()
        });
        let sessions = state
            .sessions
            .iter()
            .chain(&state.my_sessions)
            .filter(|session| session.machine.as_str() == machine)
            .map(|session| session.place.as_str());
        accounts
            .chain(sessions)
            .find(|place| !place.trim().is_empty())
    });
    given.map_or_else(|| machine_name(machine).to_owned(), str::to_owned)
}

/// A session's words for a preview: without markdown's `*` and backticks, and without the
/// English coach's line an answer can open with (it is for the chat); none when nothing is
/// left.
#[must_use]
pub(crate) fn plain(text: &str) -> Option<String> {
    let text = without_coach(text).replace(['*', '`'], "");
    (!text.trim().is_empty()).then_some(text)
}

/// The text without a leading `💬 More natural: "…" — 8/10` or `💬 Nicely phrased — 10/10`
/// line (the server's coach).
fn without_coach(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("💬 ") else {
        return text;
    };
    let Some(score) = rest.find("/10") else {
        return text;
    };
    let head = &rest[..score];
    if !(head.starts_with("More natural: \"") || head.starts_with("Nicely phrased — ")) {
        return text;
    }
    let mut after = &rest[score + 3..];
    if let Some(tags) = after.strip_prefix(" · [")
        && let Some(end) = tags.find(']')
    {
        after = &tags[end + 1..];
    }
    after.trim_start()
}

/// What a finished session said last (or, while it waits, what it asks), plain and whole; cut
/// at a word when the server cut it mid-word.
#[must_use]
pub(crate) fn summary(session: &RunnerSession) -> Option<String> {
    let raw = if session.is_waiting() {
        session.question.as_deref().or(session.last.as_deref())
    } else {
        session.last.as_deref().or(session.status_line.as_deref())
    }?;
    let text = plain(raw)?;
    let whole = text.ends_with(['.', '!', '?', ')', '"'])
        || text
            .rfind("STATUS: ")
            .is_some_and(|at| !text[at..].contains('.'));
    if text.chars().count() < CUT_NEAR || whole {
        return Some(text);
    }
    match text.rfind(' ') {
        Some(cut) => Some(format!("{} …", &text[..cut])),
        None => Some(text),
    }
}

/// Who sent a message to a session, as its bubble says it.
#[must_use]
pub(crate) fn sender(by: Option<&Sender>) -> String {
    match by {
        Some(Sender::Person) => "You".to_owned(),
        Some(Sender::Assistant) => format!("From {APP_NAME}"),
        Some(Sender::Runner) => format!("{APP_NAME}'s runner"),
        Some(Sender::Itself) => "Carried on by itself".to_owned(),
        _ => APP_NAME.to_owned(),
    }
}

/// A message as it reads: without what the server adds to every brief.
#[must_use]
pub(crate) fn brief(text: &str) -> &str {
    BRIEF_TAILS
        .iter()
        .filter_map(|tail| text.find(tail))
        .min()
        .map_or(text, |cut| &text[..cut])
}

/// The models a session of the user's may run on: id and name.
pub(crate) const MODELS: [(&str, &str); 4] = [
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-haiku-5-5", "Haiku 5.5"),
];

/// A model's name, else its id.
#[must_use]
pub(crate) fn model_name(id: &str) -> &str {
    MODELS
        .iter()
        .find(|(model, _)| *model == id)
        .map_or(id, |(_, name)| name)
}

/// The word for what a running turn does now: its to-do's own words while one is in progress,
/// else one that fits the moment, changing with `tick` (every few seconds).
#[must_use]
pub(crate) fn live_word(
    thinking: bool,
    typing: bool,
    tool: Option<&str>,
    todo: Option<&str>,
    tick: usize,
) -> String {
    if let Some(todo) = todo.filter(|todo| !todo.trim().is_empty()) {
        return todo.to_owned();
    }
    let words: &[&str] = if thinking {
        &["Thinking", "Reasoning", "Weighing it up", "Working it out"]
    } else if typing {
        &["Writing", "Putting it into words"]
    } else {
        match tool {
            Some("Bash") => &["Running", "Executing", "Crunching"],
            Some("Read" | "Grep" | "Glob") => &["Reading", "Exploring", "Tracing the code"],
            Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit") => {
                &["Editing", "Shaping the code", "Writing code"]
            }
            Some("Agent" | "Task") => &["Orchestrating", "Coordinating agents", "Delegating"],
            Some("WebSearch" | "WebFetch") => &["Researching", "Reading the web"],
            _ => &["Working", "Building", "Tinkering", "Figuring it out"],
        }
    };
    words[tick % words.len()].to_owned()
}

/// How long each word for what a running turn does stays, in milliseconds, before the next.
const WORD_EVERY_MS: i64 = 3500;

/// Which of the words for what a running turn does shows at `now`: the next one every 3.5 s.
#[must_use]
pub(crate) fn word_tick(now: Timestamp) -> usize {
    usize::try_from(now.as_millisecond().div_euclid(WORD_EVERY_MS)).unwrap_or(0)
}

/// A step as a line under the live word: a command reads `$ npm test`.
#[must_use]
pub(crate) fn step_detail(doing: &str) -> String {
    doing
        .strip_prefix("Running: ")
        .map_or_else(|| doing.to_owned(), |command| format!("$ {command}"))
}

/// A running timer from `since` to `now`: `0:42`, `12:05`, `1:02:09`.
#[must_use]
pub(crate) fn timer(since: Timestamp, now: Timestamp) -> String {
    let seconds = (now.as_second() - since.as_second()).max(0);
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A token count, short: `950`, `12k`, `1.4M`.
#[must_use]
pub(crate) fn tokens(n: f64) -> String {
    let n = crate::format::whole(n).max(0);
    if n >= 1_000_000 {
        let tenths = (n + 50_000) / 100_000;
        if tenths % 10 == 0 {
            format!("{}M", tenths / 10)
        } else {
            format!("{}.{}M", tenths / 10, tenths % 10)
        }
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// A working agent's row, in words: `EX-12 An example task`, then `Worker · Running checks`.
#[must_use]
pub(crate) fn agent_lines(agent: &AgentNow, lone: bool) -> (String, String) {
    let title = agent.title.as_deref().unwrap_or_default();
    let name = agent.name.as_deref().unwrap_or("Agent");
    if agent.lead {
        let doing = agent.line.as_deref().unwrap_or("Thinking…");
        (
            format!("{} {title}", agent.key),
            format!("{name} · {doing}"),
        )
    } else if lone {
        let doing = agent.line.as_deref().unwrap_or(title);
        (name.to_owned(), format!("{} · {doing}", agent.key))
    } else {
        (
            name.to_owned(),
            agent.line.clone().unwrap_or_else(|| "Working".to_owned()),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn session(extra: serde_json::Value) -> RunnerSession {
        let title = format!("{ASSISTANT_TITLE_PREFIX}A session");
        let mut base = json!({"id": "r-1", "machine": "runner-b", "title": title,
            "project": "Example", "status": "completed", "updatedAt": "2026-11-25T08:00:00Z",
            "place": "Mac", "doing": null, "account": "a-1", "accountName": "Personal",
            "ask": null, "last": null, "direct": false, "model": null, "tasks": [12]});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn a_status_reads_as_words_and_a_mark() {
        assert_eq!(status_text("queued"), "Starting");
        assert_eq!(status_text("completed"), "Finished its turn");
        assert_eq!(status_text("paused"), "Paused");
        assert_eq!(Mark::of("interrupted"), Mark::Stopped);
        assert_eq!(Mark::of("waiting").word(), "waiting");
        assert_eq!(Mark::of("idle"), Mark::Done);
        assert_eq!(Mark::of("queued").word(), "running");
    }

    #[test]
    fn titles_and_paths_read_short() {
        assert_eq!(
            short_title(&format!("{ASSISTANT_TITLE_PREFIX}Fix it")),
            "Fix it"
        );
        assert_eq!(short_title("Fix it"), "Fix it");
        assert_eq!(short_path("/home/example/work/emberhut"), "~/work/emberhut");
        assert_eq!(short_path("/Users/example"), "~");
        assert_eq!(short_path("/srv/work"), "/srv/work");
    }

    #[test]
    fn a_rows_line_has_its_tasks_project_place_and_account() {
        let key = |task: TaskId| format!("EX-{task}");
        let s = session(json!({}));
        assert_eq!(meta(&s, &[], key), "EX-12 · Example · Mac · Personal");
        let bare =
            session(json!({"tasks": [], "project": "", "accountName": null, "account": null}));
        assert_eq!(meta(&bare, &[], key), "Mac");
        let by_id =
            session(json!({"tasks": [], "project": "", "accountName": null, "account": "a-1"}));
        assert_eq!(
            meta(&by_id, &[], key),
            "Mac · a-1",
            "an unknown id as it is"
        );
        let accounts: Vec<ClaudeAccount> =
            serde_json::from_value(json!([{"id": "a-1", "name": "Work"}])).unwrap();
        assert_eq!(
            meta(&by_id, &accounts, key),
            "Mac · Work",
            "looked up by its id"
        );
        assert_eq!(
            whose(&session(json!({"direct": true}))),
            "Your session · Mac · Finished its turn"
        );
    }

    #[test]
    fn a_preview_loses_its_marks_and_the_coach() {
        assert_eq!(plain("**Done** with `x`").as_deref(), Some("Done with x"));
        assert_eq!(
            plain("💬 More natural: \"I did\" — 8/10 · [tense] It works.").as_deref(),
            Some("It works.")
        );
        assert_eq!(
            plain("💬 Nicely phrased — 10/10\nAll good.").as_deref(),
            Some("All good.")
        );
        assert_eq!(plain(" ** "), None);
    }

    #[test]
    fn a_cut_answer_ends_at_a_word() {
        let long = "word ".repeat(300) + "unfinish";
        let s = session(json!({ "last": long }));
        let text = summary(&s).unwrap();
        assert!(text.ends_with("word …"), "{}", &text[text.len() - 10..]);
        let whole = session(json!({ "last": "word ".repeat(300) + "done." }));
        assert!(summary(&whole).unwrap().ends_with("done."));
        let waiting = session(json!({"status": "waiting", "question": "Which?", "last": "Old"}));
        assert_eq!(summary(&waiting).as_deref(), Some("Which?"));
        assert_eq!(summary(&session(json!({}))), None);
    }

    #[test]
    fn a_message_says_who_sent_it_and_loses_the_brief_tail() {
        assert_eq!(sender(Some(&Sender::Person)), "You");
        assert_eq!(sender(Some(&Sender::Itself)), "Carried on by itself");
        assert_eq!(
            brief("Fix it\n\nEnd your reply with its STATUS line: ..."),
            "Fix it"
        );
        assert_eq!(brief("Fix it"), "Fix it");
    }

    #[test]
    fn the_live_word_follows_what_it_does() {
        assert_eq!(live_word(true, false, None, None, 1), "Reasoning");
        assert_eq!(live_word(false, false, Some("Bash"), None, 3), "Running");
        assert_eq!(
            live_word(false, true, None, Some("Fixing the bug"), 0),
            "Fixing the bug"
        );
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("other"), "other");
    }

    #[test]
    fn the_live_word_moves_on_every_three_and_a_half_seconds() {
        let at = |ms: i64| Timestamp::from_millisecond(ms).unwrap();
        assert_eq!(word_tick(at(0)), 0);
        assert_eq!(word_tick(at(3499)), 0);
        assert_eq!(word_tick(at(3500)), 1);
        assert_eq!(word_tick(at(7000)), 2);
        assert_eq!(step_detail("Running: npm test"), "$ npm test");
        assert_eq!(step_detail("Reading main.rs"), "Reading main.rs");
    }

    #[test]
    fn timers_and_token_counts_read_short() {
        let at = |s: i64| Timestamp::from_second(s).unwrap();
        assert_eq!(timer(at(0), at(42)), "0:42");
        assert_eq!(timer(at(0), at(3729)), "1:02:09");
        assert_eq!(tokens(950.), "950");
        assert_eq!(tokens(12_400.), "12k");
        assert_eq!(tokens(1_400_000.), "1.4M");
        assert_eq!(tokens(2_000_000.), "2M");
    }
}
