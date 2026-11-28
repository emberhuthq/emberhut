//! A task's plan in words and shapes, on a server that keeps plans: how a row shows it (its
//! lead mark, its one line, its n/m and its bar, one segment a step), when a running task has
//! gone quiet, the page's summary line (`4 of 9 done · 1 blocked · 1 skipped`), and the
//! checklist, folded around the current step when the plan is long. An empty plan reads "No
//! plan yet".
//!
//! Counting is the server's: a skipped step counts in neither done nor total, and a step with
//! sub-steps counts as its sub-steps.

use emberhut_api::ids::TaskId;
use emberhut_api::task_plan::{Plan, PlanStep, PlanStepId, PlanSummary, StepState};
use emberhut_api::tasks::{Issue, TaskStatus};
use emberhut_api::work::RunnerSession;
use jiff::Timestamp;

use super::clock;
use super::words::{self, Doing};

/// What an empty plan says.
pub(crate) const NO_PLAN: &str = "No plan yet";

/// Past this many steps a bar is one plain fill rather than a segment a step.
const MOST_SEGMENTS: u32 = 12;

/// A running task with no word from its plan or its sessions for this long has gone quiet, in
/// seconds.
const QUIET_AFTER: i64 = 20 * 60;

/// Past this many top-level steps the checklist folds around the current step.
const FOLD_PAST: usize = 8;

/// How many steps after the current one show while the checklist is folded.
const NEXT_SHOWN: usize = 2;

/// The mark a row leads with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead {
    /// It runs: an amber ring that spins.
    Running,
    /// It is stuck: a red mark.
    Blocked,
    /// It waits on the user: a filled ember dot.
    NeedsYou,
    /// Its usual state icon.
    Usual,
}

/// The colour of a row's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Grey.
    Muted,
    /// Quieter grey.
    Faint,
    /// Red: blocked.
    Danger,
    /// The accent, semibold: it waits on the user.
    Accent,
}

/// One segment of a plan's bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Done: filled.
    Done,
    /// Being worked on: amber.
    Doing,
    /// Blocked: red.
    Blocked,
    /// Not started: empty.
    Todo,
}

/// A plan's bar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Bar {
    /// A segment a step.
    Segments(Vec<Segment>),
    /// One fill, done of total (a long plan).
    Plain(f64),
}

/// How a task's row shows its plan.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowPlan {
    /// The mark it leads with.
    pub(crate) lead: Lead,
    /// Its one line under the title, and its colour.
    pub(crate) line: (String, Tone),
    /// `4/9`; none while the plan is empty.
    pub(crate) ticks: Option<String>,
    /// Its bar; none while the plan is empty.
    pub(crate) bar: Option<Bar>,
}

/// How `task`'s row shows its plan; none for one done or archived, or on a server that sent no
/// plan.
#[must_use]
pub(crate) fn row_plan(task: &Issue) -> Option<RowPlan> {
    let summary = task.plan.as_ref()?;
    let doing = words::doing(task);
    let step = summary.current.as_ref().map(|step| step.title.as_str());
    let stuck = summary
        .current
        .as_ref()
        .is_some_and(|step| step.state == StepState::Blocked);
    let blocked = |note: &str| match step.filter(|_| stuck) {
        Some(step) if step != note => format!("Blocked: {step} · {note}"),
        _ => format!("Blocked: {note}"),
    };
    let (lead, line) = match doing {
        Doing::Done | Doing::Archived => return None,
        Doing::Waiting => {
            let asks = task
                .action
                .clone()
                .or_else(|| task.decision.as_ref().map(|d| d.question.clone()))
                .unwrap_or_else(|| "an answer".to_owned());
            (
                Lead::NeedsYou,
                (format!("Waiting on you: {asks}"), Tone::Accent),
            )
        }
        Doing::Running | Doing::Queued if stuck => (
            Lead::Blocked,
            (
                blocked(summary.blocker.as_deref().or(step).unwrap_or_default()),
                Tone::Danger,
            ),
        ),
        Doing::Running => (
            Lead::Running,
            step.map_or_else(
                || (NO_PLAN.to_owned(), Tone::Faint),
                |step| (step.to_owned(), Tone::Muted),
            ),
        ),
        Doing::Queued => (
            Lead::Usual,
            (
                step.map_or_else(
                    || "Queued".to_owned(),
                    |step| format!("Queued · first: {step}"),
                ),
                Tone::Muted,
            ),
        ),
        _ => match &summary.blocker {
            Some(note) => (Lead::Blocked, (blocked(note), Tone::Danger)),
            None if summary.is_empty() => (Lead::Usual, (empty_line(task), Tone::Faint)),
            None => (Lead::Usual, (open_line(summary), Tone::Muted)),
        },
    };
    Some(RowPlan {
        lead,
        line,
        ticks: ticks(summary),
        bar: bar_of_summary(summary, doing == Doing::Running),
    })
}

/// An open task's line with no plan: `To do · No plan yet` for the user's own to-do, else "No
/// plan yet".
fn empty_line(task: &Issue) -> String {
    if task.status == TaskStatus::Todo && !crate::store::lookup::is_assistants(task) {
        format!("To do · {NO_PLAN}")
    } else {
        NO_PLAN.to_owned()
    }
}

/// An open task's line with a plan: `Not started · first: <step>`, else `Next: <step>`.
fn open_line(summary: &PlanSummary) -> String {
    let step = summary.current.as_ref().map(|step| step.title.as_str());
    match (summary.done, step) {
        (0, Some(step)) => format!("Not started · first: {step}"),
        (0, None) => "Not started".to_owned(),
        (_, Some(step)) => format!("Next: {step}"),
        (_, None) => "Nothing left to do".to_owned(),
    }
}

/// `4/9`; none while the plan is empty.
#[must_use]
pub(crate) fn ticks(summary: &PlanSummary) -> Option<String> {
    (!summary.is_empty()).then(|| format!("{}/{}", summary.done.min(summary.total), summary.total))
}

/// How far along the plan is, 0 to 1 (0 while it is empty).
#[must_use]
pub(crate) fn fraction(summary: &PlanSummary) -> f64 {
    if summary.is_empty() {
        0.0
    } else {
        f64::from(summary.done.min(summary.total)) / f64::from(summary.total)
    }
}

/// A row's bar from the plan at a glance: the done steps, then the current one (amber while the
/// task runs, red when it is blocked), then a red one for what blocks the plan elsewhere, then
/// the rest; one fill past [`MOST_SEGMENTS`]; none while the plan is empty.
#[must_use]
pub(crate) fn bar_of_summary(summary: &PlanSummary, running: bool) -> Option<Bar> {
    if summary.is_empty() {
        return None;
    }
    if summary.total > MOST_SEGMENTS {
        return Some(Bar::Plain(fraction(summary)));
    }
    let total = summary.total as usize;
    let mut segments = vec![Segment::Done; summary.done.min(summary.total) as usize];
    let current = summary.current.as_ref().map(|step| &step.state);
    match current {
        Some(StepState::Blocked) => segments.push(Segment::Blocked),
        Some(_) if running => segments.push(Segment::Doing),
        _ => {}
    }
    let elsewhere =
        summary.blocker.is_some() && current.is_none_or(|state| *state != StepState::Blocked);
    if elsewhere && segments.len() < total {
        segments.push(Segment::Blocked);
    }
    segments.resize(total, Segment::Todo);
    Some(Bar::Segments(segments))
}

/// The steps that count, in order: each top-level step without sub-steps, and each sub-step;
/// skipped ones left out.
fn counted(plan: &Plan) -> impl Iterator<Item = &PlanStep> {
    plan.steps.iter().filter(move |step| {
        step.state != StepState::Skipped
            && (step.parent.is_some() || !plan.steps.iter().any(|sub| sub.parent == Some(step.id)))
    })
}

/// The page's bar: a segment for each step that counts, in its state; one fill (the summary's
/// done of total) past [`MOST_SEGMENTS`]; none while the plan is empty.
#[must_use]
pub(crate) fn bar_of_plan(plan: &Plan) -> Option<Bar> {
    let steps: Vec<&PlanStep> = counted(plan).collect();
    if steps.is_empty() {
        return None;
    }
    if steps.len() > MOST_SEGMENTS as usize {
        return Some(Bar::Plain(fraction(&plan.summary)));
    }
    Some(Bar::Segments(
        steps
            .iter()
            .map(|step| match step.state {
                StepState::Done => Segment::Done,
                StepState::Doing => Segment::Doing,
                StepState::Blocked => Segment::Blocked,
                _ => Segment::Todo,
            })
            .collect(),
    ))
}

/// The line over the checklist: `4 of 9 done · 1 blocked · 1 skipped`, counted as the server
/// counts (a skipped step leaves the total, a step with sub-steps counts as them); "No plan
/// yet" while it is empty.
#[must_use]
pub(crate) fn summary_line(plan: &Plan) -> String {
    let summary = &plan.summary;
    if summary.is_empty() {
        return NO_PLAN.to_owned();
    }
    let mut parts = vec![format!(
        "{} of {} done",
        summary.done.min(summary.total),
        summary.total
    )];
    let blocked = counted(plan)
        .filter(|step| step.state == StepState::Blocked)
        .count();
    if blocked > 0 {
        parts.push(format!("{blocked} blocked"));
    }
    let skipped = plan
        .steps
        .iter()
        .filter(|step| step.state == StepState::Skipped)
        .count();
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    parts.join(" · ")
}

/// The ETA line: `about 2 hours · updated 3 minutes ago`, either part alone; none when the plan
/// says neither.
#[must_use]
pub(crate) fn eta_line(summary: &PlanSummary, now: Timestamp) -> Option<String> {
    let updated = summary
        .updated
        .as_ref()
        .and_then(clock::instant)
        .map(|at| format!("updated {}", clock::relative(at, now)));
    let parts: Vec<String> = summary.eta.clone().into_iter().chain(updated).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The quiet line under an empty plan when a turn of work ended without one: `A turn ended
/// 20 minutes ago without a plan`.
#[must_use]
pub(crate) fn unplanned_line(summary: &PlanSummary, now: Timestamp) -> Option<String> {
    if !summary.is_empty() {
        return None;
    }
    let at = clock::instant(summary.unplanned_turn.as_ref()?)?;
    Some(format!(
        "A turn ended {} without a plan",
        clock::relative(at, now)
    ))
}

/// How long a running task has gone quiet: `quiet 25m` when neither its plan nor a session on
/// it (among `sessions`) has moved for [`QUIET_AFTER`] or more, counted from when its run
/// started when nothing moved since; none for a task that isn't running, or has news.
#[must_use]
pub(crate) fn quiet(task: &Issue, sessions: &[&RunnerSession], now: Timestamp) -> Option<String> {
    if words::doing(task) != Doing::Running {
        return None;
    }
    let plan = task.plan.as_ref().and_then(|plan| plan.updated.as_ref());
    let since = task.live.as_ref().and_then(|live| live.since.as_ref());
    let latest = sessions
        .iter()
        .filter(|session| session.tasks.contains(&task.id))
        .map(|session| &session.updated_at)
        .chain(plan)
        .chain(since)
        .filter_map(clock::instant)
        .max()?;
    (now.as_second() - latest.as_second() >= QUIET_AFTER)
        .then(|| format!("quiet {}", clock::span(latest, now)))
}

/// The current step's id: the summary's, else the first being worked on.
fn current_id(plan: &Plan) -> Option<PlanStepId> {
    plan.summary
        .current
        .as_ref()
        .map(|step| step.id)
        .or_else(|| {
            plan.steps
                .iter()
                .find(|step| step.state == StepState::Doing)
                .map(|step| step.id)
        })
}

/// The step being worked on as the page names it: the current step, and its sub-step being
/// worked on after `›` (`Build the installer › Sign it`).
#[must_use]
pub(crate) fn now_line(plan: &Plan) -> Option<String> {
    let current = current_id(plan)?;
    let step = plan.steps.iter().find(|step| step.id == current)?;
    let parent = step
        .parent
        .and_then(|parent| plan.steps.iter().find(|s| s.id == parent));
    let sub = plan
        .steps
        .iter()
        .find(|sub| sub.parent == Some(step.id) && sub.state == StepState::Doing);
    Some(match (parent, sub) {
        (Some(parent), _) => format!("{} › {}", parent.title, step.title),
        (None, Some(sub)) => format!("{} › {}", step.title, sub.title),
        (None, None) => step.title.clone(),
    })
}

/// One step of the checklist as the page draws it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StepRow<'a> {
    /// The step.
    pub(crate) step: &'a PlanStep,
    /// A sub-step: drawn indented under its step.
    pub(crate) nested: bool,
    /// The step being worked on now, or its sub-step being worked on.
    pub(crate) current: bool,
}

/// A line of the checklist.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item<'a> {
    /// A step.
    Step(StepRow<'a>),
    /// The earlier steps, folded: how many, and whether every one is done.
    Earlier {
        /// How many top-level steps it holds.
        count: usize,
        /// Whether each is done (or skipped).
        all_done: bool,
    },
    /// The later steps, folded: how many top-level steps.
    Later(usize),
}

/// The checklist: every step in order, each sub-step right under its step (a sub-step whose
/// step isn't in the plan stands on its own), the current one marked. Past [`FOLD_PAST`]
/// top-level steps it folds around the current step unless `earlier` or `later` is open: the
/// steps before it but the last one fold into one line, and those after the next
/// [`NEXT_SHOWN`] into another.
#[must_use]
pub(crate) fn checklist(plan: &Plan, earlier: bool, later: bool) -> Vec<Item<'_>> {
    let current = current_id(plan);
    let is_top = |step: &PlanStep| {
        step.parent.is_none_or(|parent| {
            !plan
                .steps
                .iter()
                .any(|s| s.id == parent && s.parent.is_none())
        })
    };
    let groups: Vec<Vec<StepRow<'_>>> = plan
        .steps
        .iter()
        .filter(|step| is_top(step))
        .map(|step| {
            let subs = plan
                .steps
                .iter()
                .filter(|sub| step.parent.is_none() && sub.parent == Some(step.id));
            std::iter::once(StepRow {
                step,
                nested: false,
                current: current == Some(step.id),
            })
            .chain(subs.map(|sub| StepRow {
                step: sub,
                nested: true,
                current: current == Some(sub.id)
                    || (current == Some(step.id) && sub.state == StepState::Doing),
            }))
            .collect()
        })
        .collect();
    let settled =
        |row: &StepRow<'_>| matches!(row.step.state, StepState::Done | StepState::Skipped);
    let at = groups
        .iter()
        .position(|group| group.iter().any(|row| row.current))
        .or_else(|| groups.iter().position(|group| !settled(&group[0])))
        .unwrap_or(groups.len().saturating_sub(1));
    let folds = groups.len() > FOLD_PAST;
    // Folding one step would save nothing: it shows.
    let hidden = if folds && !earlier && at >= 2 {
        at - 1
    } else {
        0
    };
    let shown_until = if folds && !later {
        (at + 1 + NEXT_SHOWN).min(groups.len())
    } else {
        groups.len()
    };
    let mut items = Vec::new();
    if hidden > 0 {
        items.push(Item::Earlier {
            count: hidden,
            all_done: groups[..hidden].iter().flatten().all(settled),
        });
    }
    for group in &groups[hidden..shown_until] {
        items.extend(group.iter().cloned().map(Item::Step));
    }
    if shown_until < groups.len() {
        items.push(Item::Later(groups.len() - shown_until));
    }
    items
}

/// How long the current step has run: `8m so far`.
#[must_use]
pub(crate) fn so_far(step: &PlanStep, now: Timestamp) -> Option<String> {
    let started = clock::instant(step.started.as_ref()?)?;
    Some(format!("{} so far", clock::span(started, now)))
}

/// The steps' results, in plan order: each link and the step that made it.
#[must_use]
pub(crate) fn results(plan: &Plan) -> Vec<(&str, &PlanStep)> {
    plan.steps
        .iter()
        .filter_map(|step| Some((step.link.as_deref()?, step)))
        .collect()
}

/// How long a link's words run before they are cut.
const LINK_CHARS: usize = 48;

/// A step's link in short words: a file on the server by its name, a web address without its
/// scheme, `www.` and a trailing `/`, cut at [`LINK_CHARS`].
#[must_use]
pub(crate) fn link_label(link: &str) -> String {
    let bare = link.split(['?', '#']).next().unwrap_or(link);
    let short = if bare.starts_with('/') {
        bare.rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(bare)
    } else {
        let rest = bare.split_once("://").map_or(bare, |(_, rest)| rest);
        rest.strip_prefix("www.")
            .unwrap_or(rest)
            .trim_end_matches('/')
    };
    match short.char_indices().nth(LINK_CHARS) {
        Some((cut, _)) => format!("{}…", &short[..cut]),
        None => short.to_owned(),
    }
}

/// A step's state in a word, for a screen reader.
#[must_use]
pub(crate) fn state_word(state: &StepState) -> &'static str {
    match state {
        StepState::Todo => "To do",
        StepState::Doing => "Doing now",
        StepState::Done => "Done",
        StepState::Blocked => "Blocked",
        StepState::Skipped => "Skipped",
        _ => "Unknown",
    }
}

/// A step as a screen reader says it: `Done: Build the Mac app`, `Blocked: Sign the build,
/// waiting on the certificate`.
#[must_use]
pub(crate) fn spoken(step: &PlanStep) -> String {
    let mut line = format!("{}: {}", state_word(&step.state), step.title);
    if let Some(note) = step.note.as_deref().filter(|note| !note.trim().is_empty()) {
        line.push_str(", ");
        line.push_str(note.trim());
    }
    line
}

/// What a screen reader adds for a row's plan: its line, and `4 of 9 steps done`.
#[must_use]
pub(crate) fn spoken_row(planned: &RowPlan) -> String {
    match &planned.ticks {
        Some(ticks) => format!(
            "{}, {} steps done",
            planned.line.0,
            ticks.replace('/', " of ")
        ),
        None => planned.line.0.clone(),
    }
}

/// The id a step's row is found by.
#[must_use]
pub(crate) fn step_key(task: TaskId, step: PlanStepId) -> String {
    format!("plan-{task}-{step}")
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
