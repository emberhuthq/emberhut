//! A transcript laid out as its page shows it: turn by turn, each a run of items: what was
//! said, the tool calls in between (each with its result and, for a helper agent, the helper's
//! own steps), a question card, the to-do list, the runner's notes. Plus what a tool call's line
//! says, its icon, the fold of a finished turn's calls, the glance at what a call did, and the
//! step a running turn is on. Pure functions over the steps.

use std::collections::HashMap;

use emberhut_api::ids::{ToolCallId, TurnId};
use emberhut_api::runner::steps::{SessionStep, StepKind, TodoItem, TodoStatus};

/// The tools the page doesn't list: the to-do list shows what they do, and loading a tool says
/// nothing.
const QUIET_TOOLS: [&str; 6] = [
    "TodoWrite",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "ToolSearch",
];

/// The tool a session asks the user a question with: its question step is the card.
const ASK_TOOL: &str = "AskUserQuestion";

/// How many of a running turn's newest calls show on their own; older ones fold.
pub(crate) const LIVE_CALLS: usize = 4;

/// How many lines an input or output block shows until More, and how long it may be.
pub(crate) const BLOCK_LINES: usize = 12;
const BLOCK_CHARS: usize = 900;

/// How many lines of an edit's diff show until the rest.
pub(crate) const DIFF_LINES: usize = 16;

/// How many lines a glance at a diff shows.
const PEEK_DIFF_LINES: usize = 6;

/// One turn: its steps in order.
#[derive(Debug, PartialEq)]
pub(crate) struct Turn<'a> {
    /// Its turn; none for the steps before the first turn.
    pub(crate) id: Option<&'a TurnId>,
    /// Its steps.
    pub(crate) steps: Vec<&'a SessionStep>,
}

/// The steps grouped by turn: a step of another turn starts a new group; one with no turn goes
/// with the group before it.
#[must_use]
pub(crate) fn turns(steps: &[SessionStep]) -> Vec<Turn<'_>> {
    let mut out: Vec<Turn<'_>> = Vec::new();
    for step in steps {
        match (step.turn.as_ref(), out.last_mut()) {
            (Some(turn), Some(last)) if last.id == Some(turn) => last.steps.push(step),
            (Some(turn), _) => out.push(Turn {
                id: Some(turn),
                steps: vec![step],
            }),
            (None, Some(last)) => last.steps.push(step),
            (None, None) => out.push(Turn {
                id: None,
                steps: vec![step],
            }),
        }
    }
    out
}

/// A tool call (or a finished thinking block) with what came of it.
#[derive(Debug, PartialEq)]
pub(crate) struct ToolCall<'a> {
    /// The call, or the thinking step.
    pub(crate) call: &'a SessionStep,
    /// What it returned.
    pub(crate) result: Option<&'a SessionStep>,
    /// A helper agent's own steps, when the call started one.
    pub(crate) inside: Vec<&'a SessionStep>,
}

impl ToolCall<'_> {
    /// Whether it is a finished thinking block.
    #[must_use]
    pub(crate) fn is_thought(&self) -> bool {
        self.call.kind == StepKind::Thinking
    }

    /// Whether it returned an error.
    #[must_use]
    pub(crate) fn failed(&self) -> bool {
        self.result.is_some_and(SessionStep::failed)
    }

    /// Whether it started a helper agent.
    #[must_use]
    pub(crate) fn is_agent(&self) -> bool {
        is_agent(self.call.name.as_deref())
    }

    /// The helper's tool calls so far.
    #[must_use]
    pub(crate) fn helper_calls(&self) -> usize {
        self.inside
            .iter()
            .filter(|step| step.kind == StepKind::Tool)
            .count()
    }
}

/// One item of a turn.
#[derive(Debug, PartialEq)]
pub(crate) enum Item<'a> {
    /// A message to the session, or the model's words.
    Said(&'a SessionStep),
    /// A run of tool calls between them.
    Tools(Vec<ToolCall<'a>>),
    /// A turn's end or the runner's note.
    Note(&'a SessionStep),
    /// A question to the user.
    Question(&'a SessionStep),
    /// The to-do list as it changed.
    Todos(&'a SessionStep),
}

impl Item<'_> {
    /// Its first step's place, as its id.
    #[must_use]
    pub(crate) fn seq(&self) -> u64 {
        match self {
            Self::Said(step) | Self::Note(step) | Self::Question(step) | Self::Todos(step) => {
                step.seq
            }
            Self::Tools(calls) => calls.first().map_or(0, |call| call.call.seq),
        }
    }
}

/// Whether a tool started a helper agent.
#[must_use]
pub(crate) fn is_agent(name: Option<&str>) -> bool {
    matches!(name, Some("Agent" | "Task"))
}

/// The items of `steps` directly under `within` (none: the session's own, else a helper
/// agent's call): results and a helper's steps hang off the call they belong to.
#[must_use]
pub(crate) fn items<'a>(steps: &[&'a SessionStep], within: Option<&ToolCallId>) -> Vec<Item<'a>> {
    let mut results: HashMap<&ToolCallId, &SessionStep> = HashMap::new();
    let mut inside: HashMap<&ToolCallId, Vec<&SessionStep>> = HashMap::new();
    for step in steps {
        if step.kind == StepKind::Result
            && let Some(call) = &step.call
        {
            results.entry(call).or_insert(step);
        }
        if let Some(parent) = &step.parent
            && Some(parent) != within
        {
            inside.entry(parent).or_default().push(step);
        }
    }
    let mut out = Vec::new();
    let mut run: Vec<ToolCall<'a>> = Vec::new();
    for step in steps
        .iter()
        .copied()
        .filter(|s| s.parent.as_ref() == within)
    {
        match step.kind {
            StepKind::Tool => {
                let name = step.name.as_deref().unwrap_or_default();
                if name == ASK_TOOL || QUIET_TOOLS.contains(&name) {
                    continue;
                }
                let call = step.call.as_ref();
                run.push(ToolCall {
                    call: step,
                    result: call.and_then(|c| results.get(c).copied()),
                    inside: call
                        .and_then(|c| inside.get(c).cloned())
                        .unwrap_or_default(),
                });
            }
            StepKind::Thinking => run.push(ToolCall {
                call: step,
                result: None,
                inside: Vec::new(),
            }),
            StepKind::Result => {}
            _ => {
                if !run.is_empty() {
                    out.push(Item::Tools(std::mem::take(&mut run)));
                }
                out.push(match step.kind {
                    StepKind::User | StepKind::Text => Item::Said(step),
                    StepKind::Question => Item::Question(step),
                    StepKind::Todos => Item::Todos(step),
                    _ => Item::Note(step),
                });
            }
        }
    }
    if !run.is_empty() {
        out.push(Item::Tools(run));
    }
    out
}

/// Whether a message of turn `turn` waits behind the running turn `now`: its number is past it.
#[must_use]
pub(crate) fn is_queued(turn: Option<&TurnId>, now: Option<&TurnId>, running: bool) -> bool {
    match (
        running,
        turn.and_then(TurnId::number),
        now.and_then(TurnId::number),
    ) {
        (true, Some(turn), Some(now)) => turn > now,
        _ => false,
    }
}

/// What a tool call's line says: `$ cargo test` (without a leading `cd` into the session's
/// own folder), the file, the search.
#[must_use]
pub(crate) fn tool_line(call: &SessionStep) -> String {
    let name = call.name.as_deref().unwrap_or_default();
    if name == "Bash"
        && let Some(summary) = call.summary.as_deref().filter(|s| !s.is_empty())
    {
        return format!("$ {}", without_cd(summary));
    }
    let doing = call.doing.as_deref().unwrap_or(name);
    let doing = doing.strip_prefix("Running: ").unwrap_or(doing);
    match (name, call.summary.as_deref()) {
        ("Grep" | "Glob", Some(summary)) => format!("{doing} {summary}"),
        _ => doing.to_owned(),
    }
}

/// A command without its leading `cd <folder> &&` (or `;`).
fn without_cd(command: &str) -> &str {
    let Some(rest) = command.strip_prefix("cd ") else {
        return command;
    };
    let rest = rest.trim_start();
    let folder_end = rest
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(rest.len());
    let after = rest[folder_end..].trim_start();
    after
        .strip_prefix("&&")
        .or_else(|| after.strip_prefix(';'))
        .map_or(command, str::trim_start)
}

/// A tool's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolIcon {
    /// A command.
    Terminal,
    /// A file read.
    Document,
    /// An edit.
    Pencil,
    /// A search.
    Search,
    /// A helper agent.
    People,
    /// The web.
    Globe,
    /// The to-do list.
    Checklist,
    /// Anything else.
    Wrench,
}

impl ToolIcon {
    /// The icon for a tool's name.
    #[must_use]
    pub(crate) fn of(name: Option<&str>) -> Self {
        match name {
            Some("Bash") => Self::Terminal,
            Some("Read") => Self::Document,
            Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit") => Self::Pencil,
            Some("Grep" | "Glob") => Self::Search,
            Some("Agent" | "Task") => Self::People,
            Some("WebSearch" | "WebFetch") => Self::Globe,
            Some("TodoWrite") => Self::Checklist,
            _ => Self::Wrench,
        }
    }
}

/// A finished turn's calls folded into one line: `12 steps · Ran the tests · 1 failed`, or
/// `Thought` when it only thought.
#[must_use]
pub(crate) fn folded(calls: &[ToolCall<'_>]) -> String {
    let tools: Vec<&ToolCall<'_>> = calls.iter().filter(|c| !c.is_thought()).collect();
    let mut line = match tools.len() {
        0 => "Thought".to_owned(),
        1 => "1 step".to_owned(),
        n => format!("{n} steps"),
    };
    if let Some(last) = tools.last().and_then(|c| c.call.doing.as_deref()) {
        line.push_str(" · ");
        line.push_str(last.strip_prefix("Running: ").unwrap_or(last));
    }
    let failed = calls.iter().filter(|c| c.failed()).count();
    if failed > 0 {
        line.push_str(" · ");
        line.push_str(&failed.to_string());
        line.push_str(" failed");
    }
    line
}

/// Whether a block of text is long enough to fold until More.
#[must_use]
pub(crate) fn is_long_block(text: &str) -> bool {
    text.lines().count() > BLOCK_LINES || text.chars().count() > BLOCK_CHARS
}

/// How many paragraphs a folded long answer shows.
const FOLDED_PARAGRAPHS: usize = 3;

/// A long answer folded: its first three paragraphs (blank-line separated).
#[must_use]
pub(crate) fn folded_text(text: &str) -> &str {
    let mut end = 0;
    let mut seen = 0;
    let mut rest = text;
    while let Some(at) = rest.find("\n\n") {
        seen += 1;
        end += at;
        if seen == FOLDED_PARAGRAPHS {
            return text[..end].trim_end();
        }
        end += 2;
        rest = &text[end..];
    }
    text
}

/// A glance at what a finished call did, under its line in a running turn.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Peek<'a> {
    /// Nothing to show.
    Nothing,
    /// An edit's first changed lines, and how many more there are.
    Diff(Vec<&'a str>, usize),
    /// How many lines it read.
    Read(u64),
    /// How many results a search found.
    Found(u64),
    /// A command's last lines (and how many came before them), or another tool's first.
    Lines(Vec<&'a str>, u64),
}

/// The glance at `call`.
#[must_use]
pub(crate) fn peek<'a>(call: &ToolCall<'a>) -> Peek<'a> {
    let step = call.call;
    if let Some(diff) = step.diff.as_deref().filter(|d| !d.trim().is_empty()) {
        let lines: Vec<&str> = diff.lines().collect();
        let more = lines.len().saturating_sub(PEEK_DIFF_LINES);
        return Peek::Diff(lines.into_iter().take(PEEK_DIFF_LINES).collect(), more);
    }
    let Some(result) = call.result else {
        return Peek::Nothing;
    };
    let lines: Vec<&'a str> = result
        .text
        .as_deref()
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .collect();
    let count = result
        .lines
        .unwrap_or_else(|| u64::try_from(lines.len()).unwrap_or(u64::MAX));
    match step.name.as_deref() {
        Some("Grep" | "Glob") => Peek::Found(if lines.is_empty() { 0 } else { count }),
        _ if lines.is_empty() => Peek::Nothing,
        Some("Read") => Peek::Read(count),
        Some("Bash") => {
            let tail: Vec<&str> = lines[lines.len().saturating_sub(3)..].to_vec();
            Peek::Lines(tail, count.saturating_sub(3))
        }
        _ => Peek::Lines(lines.into_iter().take(2).collect(), 0),
    }
}

/// The running turn's to-do list: the newest list, when it changed in that turn.
#[must_use]
pub(crate) fn live_todos<'a>(
    steps: &'a [SessionStep],
    turn: Option<&TurnId>,
) -> Option<&'a [TodoItem]> {
    let step = steps
        .iter()
        .rev()
        .find(|s| s.kind == StepKind::Todos && s.parent.is_none())?;
    (turn.is_some() && step.turn.as_ref() == turn && !step.todos.is_empty())
        .then_some(step.todos.as_slice())
}

/// The item in progress on a to-do list, in its words for while it is done.
#[must_use]
pub(crate) fn todo_now(todos: &[TodoItem]) -> Option<&str> {
    todos
        .iter()
        .find(|t| t.status == TodoStatus::InProgress)
        .map(|t| t.active_form.as_deref().unwrap_or(&t.content))
}

/// How far along a to-do list is: done and in all.
#[must_use]
pub(crate) fn todo_progress(todos: &[TodoItem]) -> (usize, usize) {
    let done = todos
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    (done, todos.len())
}

/// The longest running to-do list that shows open by itself; a longer one, or a finished
/// turn's, folds to its count and the item in progress.
pub(crate) const TODOS_OPEN_UP_TO: usize = 5;

/// Whether a to-do list of `count` items shows open: by itself while it runs and is short, the
/// other way once the user `flipped` it.
#[must_use]
pub(crate) fn todos_open(live: bool, count: usize, flipped: bool) -> bool {
    flipped != (live && count <= TODOS_OPEN_UP_TO)
}

/// How far along a to-do list is, from 0 to 1 (an empty one hasn't started).
#[must_use]
pub(crate) fn todo_share(todos: &[TodoItem]) -> f32 {
    let (done, all) = todo_progress(todos);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a to-do list is a few dozen items at most"
    )]
    let share = done as f32 / all.max(1) as f32;
    share
}

/// The step a running turn is on, for its pinned bar.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Now<'a> {
    /// The newest tool call still running.
    pub(crate) tool: Option<&'a str>,
}

/// What the running turn is doing: its newest own tool call, when it hasn't returned yet.
#[must_use]
pub(crate) fn now(steps: &[SessionStep]) -> Now<'_> {
    let Some(last) = steps.iter().rev().find(|s| {
        s.kind == StepKind::Tool
            && s.parent.is_none()
            && !QUIET_TOOLS.contains(&s.name.as_deref().unwrap_or_default())
    }) else {
        return Now::default();
    };
    let returned = steps
        .iter()
        .any(|s| s.kind == StepKind::Result && s.call.is_some() && s.call == last.call);
    Now {
        tool: (!returned).then(|| last.name.as_deref().unwrap_or_default()),
    }
}

/// The question a waiting session asks now: its newest question step.
#[must_use]
pub(crate) fn asking(steps: &[SessionStep]) -> Option<u64> {
    steps
        .iter()
        .rev()
        .find(|s| s.kind == StepKind::Question)
        .map(|s| s.seq)
}

/// What an option of a question sends as the answer: its label, with its question before it
/// when the card asks several.
#[must_use]
pub(crate) fn answer(step: &SessionStep, question: usize, option: usize) -> Option<String> {
    let q = step.questions.get(question)?;
    let label = &q.options.get(option)?.label;
    Some(if step.questions.len() > 1 {
        format!("{} {label}", q.question)
    } else {
        label.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(seq: u64, kind: StepKind) -> SessionStep {
        let mut step = SessionStep::new(seq, kind);
        step.turn = Some(TurnId::from("r-1:1"));
        step
    }

    fn tool(seq: u64, name: &str, call: &str) -> SessionStep {
        let mut s = step(seq, StepKind::Tool);
        s.name = Some(name.into());
        s.call = Some(ToolCallId::from(call));
        s.doing = Some(format!("Running: {name}"));
        s
    }

    fn result(seq: u64, call: &str, error: bool) -> SessionStep {
        let mut s = step(seq, StepKind::Result);
        s.call = Some(ToolCallId::from(call));
        s.error = Some(error);
        s.text = Some("a\nb\nc\nd".into());
        s
    }

    #[test]
    fn steps_group_by_turn() {
        let mut first = step(1, StepKind::User);
        first.turn = None;
        let mut second = step(3, StepKind::User);
        second.turn = Some(TurnId::from("r-1:2"));
        let mut note = step(4, StepKind::System);
        note.turn = None;
        let steps = [first, step(2, StepKind::Text), second, note];
        let turns = turns(&steps);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].id, None);
        assert_eq!(
            turns[2].steps.len(),
            2,
            "a note with no turn stays with the one before"
        );
    }

    #[test]
    fn calls_gather_their_results_and_their_helpers_steps() {
        let mut helper_text = step(5, StepKind::Text);
        helper_text.parent = Some(ToolCallId::from("agent"));
        let mut helper_call = tool(6, "Read", "inner");
        helper_call.parent = Some(ToolCallId::from("agent"));
        let steps = [
            step(1, StepKind::User),
            tool(2, "Bash", "a"),
            result(3, "a", true),
            tool(4, "Agent", "agent"),
            helper_text,
            helper_call,
            tool(7, "TodoWrite", "t"),
            step(8, StepKind::Todos),
            step(9, StepKind::Text),
            step(10, StepKind::End),
        ];
        let all: Vec<&SessionStep> = steps.iter().collect();
        let laid = items(&all, None);
        assert_eq!(laid.len(), 5);
        let Item::Tools(calls) = &laid[1] else {
            panic!("a run of calls");
        };
        assert_eq!(calls.len(), 2, "TodoWrite says nothing");
        assert!(calls[0].failed());
        assert!(calls[1].is_agent());
        assert_eq!(calls[1].helper_calls(), 1);
        assert_eq!(folded(calls), "2 steps · Agent · 1 failed");
        let inside = items(&calls[1].inside, Some(&ToolCallId::from("agent")));
        assert!(matches!(inside[0], Item::Said(_)));
        assert!(matches!(laid[2], Item::Todos(_)));
        assert!(matches!(laid[4], Item::Note(_)));
        assert_eq!(laid[3].seq(), 9);
    }

    #[test]
    fn a_tool_line_says_what_it_did() {
        let mut bash = tool(1, "Bash", "a");
        bash.summary = Some("cd /work/x && cargo test".into());
        assert_eq!(tool_line(&bash), "$ cargo test");
        bash.summary = Some("cd /work/x; ls".into());
        assert_eq!(tool_line(&bash), "$ ls");
        bash.summary = Some("cargo build".into());
        assert_eq!(tool_line(&bash), "$ cargo build");
        let mut grep = tool(2, "Grep", "b");
        grep.doing = Some("Searching".into());
        grep.summary = Some("fn main".into());
        assert_eq!(tool_line(&grep), "Searching fn main");
        assert_eq!(ToolIcon::of(Some("MultiEdit")), ToolIcon::Pencil);
        assert_eq!(ToolIcon::of(None), ToolIcon::Wrench);
        let thought = [ToolCall {
            call: &step(1, StepKind::Thinking),
            result: None,
            inside: Vec::new(),
        }];
        assert_eq!(folded(&thought), "Thought");
    }

    #[test]
    fn a_message_behind_the_running_turn_is_queued() {
        let (two, three) = (TurnId::from("r-1:2"), TurnId::from("r-1:3"));
        assert!(is_queued(Some(&three), Some(&two), true));
        assert!(!is_queued(Some(&two), Some(&two), true));
        assert!(!is_queued(Some(&three), Some(&two), false));
    }

    #[test]
    fn a_glance_shows_what_a_call_did() {
        let call = tool(1, "Bash", "a");
        let done = result(2, "a", false);
        let finished = ToolCall {
            call: &call,
            result: Some(&done),
            inside: Vec::new(),
        };
        assert_eq!(peek(&finished), Peek::Lines(vec!["b", "c", "d"], 1));
        let running = ToolCall {
            call: &call,
            result: None,
            inside: Vec::new(),
        };
        assert_eq!(peek(&running), Peek::Nothing);
        let mut read = tool(3, "Read", "r");
        read.diff = Some("- a\n+ b".into());
        let p = peek(&ToolCall {
            call: &read,
            result: None,
            inside: Vec::new(),
        });
        assert_eq!(p, Peek::Diff(vec!["- a", "+ b"], 0));
    }

    #[test]
    fn the_running_turn_has_its_todos_its_tool_and_its_question() {
        let mut todos = step(3, StepKind::Todos);
        todos.todos = vec![
            TodoItem {
                id: None,
                content: "Test".into(),
                status: TodoStatus::Completed,
                active_form: None,
            },
            TodoItem {
                id: None,
                content: "Fix".into(),
                status: TodoStatus::InProgress,
                active_form: Some("Fixing".into()),
            },
        ];
        let mut question = step(5, StepKind::Question);
        question.questions = vec![
            serde_json::from_value(serde_json::json!({
                "question": "Which?", "options": [{"label": "A"}, {"label": "B"}]
            }))
            .unwrap(),
        ];
        let steps = vec![
            tool(1, "Bash", "a"),
            result(2, "a", false),
            todos,
            tool(4, "Read", "b"),
            question,
        ];
        let turn = TurnId::from("r-1:1");
        let list = live_todos(&steps, Some(&turn)).unwrap();
        assert_eq!(todo_now(list), Some("Fixing"));
        assert_eq!(todo_progress(list), (1, 2));
        assert!((todo_share(list) - 0.5).abs() < f32::EPSILON);
        assert!(todo_share(&[]).abs() < f32::EPSILON);
        assert!(todos_open(true, 5, false), "a short running list opens");
        assert!(!todos_open(true, 6, false), "a long one folds");
        assert!(!todos_open(false, 2, false), "a finished turn's folds");
        assert!(todos_open(false, 2, true), "until opened");
        assert!(!todos_open(true, 3, true), "or folded by hand");
        assert_eq!(live_todos(&steps, Some(&TurnId::from("r-1:2"))), None);
        assert_eq!(now(&steps).tool, Some("Read"));
        assert_eq!(asking(&steps), Some(5));
        assert_eq!(answer(&steps[4], 0, 1).as_deref(), Some("B"));
        assert!(is_long_block(&"x\n".repeat(13)));
        assert_eq!(folded_text("a\n\nb\n\nc\n\nd"), "a\n\nb\n\nc");
        assert_eq!(folded_text("a\n\nb"), "a\n\nb");
    }
}
