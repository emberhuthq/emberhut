//! A task's plan: the ordered steps its agent works through, ticked as it goes, so a task shows
//! its own progress.
//!
//! Steps nest one level: a top-level step may have sub-steps, listed right after it; a sub-step
//! has none. Every step has an id that is stable for the life of its task's plan and never
//! given to another step of that task, so an agent can name a step by it across turns.
//!
//! This module holds the rules: what a plan may hold ([`MAX_STEPS`], [`TITLE_MAX`], one
//! nesting level), how a whole new plan replaces the old one while keeping the ticks of the
//! steps it names again ([`set`]), how one step changes ([`edit`]), what a step's state does to
//! its times ([`move_to`]), the plan at a glance ([`summary`]), the history line each change
//! writes, and the checklist an agent reads back ([`checklist`]). The database keeps the steps
//! and applies these in one transaction.

use std::collections::BTreeMap;

use jiff::Timestamp;

/// A step's id: stable for the life of its plan, unique within its task.
pub type StepId = u32;

/// The most steps a plan holds, sub-steps included.
pub const MAX_STEPS: usize = 100;

/// The longest a step's title may be, in characters: one short line.
pub const TITLE_MAX: usize = 200;

/// The longest a step's note may be, in characters.
pub const NOTE_MAX: usize = 2_000;

/// The longest a step's link may be, in characters.
pub const LINK_MAX: usize = 2_000;

/// The longest a plan's estimate may be, in characters (`about 2 hours`).
pub const ETA_MAX: usize = 120;

/// Where a step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepState {
    /// Not started.
    Todo,
    /// Being worked on now.
    Doing,
    /// Done: ticked.
    Done,
    /// Stuck: its note says on what.
    Blocked,
    /// Left out: not needed after all.
    Skipped,
}

impl StepState {
    /// Every state.
    pub const ALL: [Self; 5] = [
        Self::Todo,
        Self::Doing,
        Self::Done,
        Self::Blocked,
        Self::Skipped,
    ];

    /// Its word, as stored and sent.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
        }
    }

    /// The state `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == word)
    }

    /// Whether the step is over: done or skipped.
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }

    /// Its box in the checklist an agent reads back.
    #[must_use]
    pub const fn mark(self) -> &'static str {
        match self {
            Self::Todo => "[ ]",
            Self::Doing => "[>]",
            Self::Done => "[x]",
            Self::Blocked => "[!]",
            Self::Skipped => "[-]",
        }
    }
}

/// One step of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Its id.
    pub id: StepId,
    /// The top-level step it is a sub-step of; none for a top-level step.
    pub parent: Option<StepId>,
    /// What the step is, one short line.
    pub title: String,
    /// Where it is.
    pub state: StepState,
    /// What it came to, or what blocks it.
    pub note: Option<String>,
    /// A link to what it produced.
    pub link: Option<String>,
    /// When it was first worked on.
    pub started: Option<Timestamp>,
    /// When it was done or skipped.
    pub finished: Option<Timestamp>,
    /// When it was added.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

/// A step of a whole new plan ([`set`]), with its sub-steps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewStep {
    /// Its title.
    pub title: String,
    /// Its state; none: as it was, for a step the plan had, else to do.
    pub state: Option<StepState>,
    /// Its note; none: as it was. Empty clears it.
    pub note: Option<String>,
    /// Its link; none: as it was. Empty clears it.
    pub link: Option<String>,
    /// Its sub-steps (a sub-step has none).
    pub steps: Vec<Self>,
}

/// A step named by its id or by its exact title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepRef {
    /// By its id.
    Id(StepId),
    /// By its title (surrounding spaces and case aside).
    Title(String),
}

/// A step to add ([`Edit::add`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddStep {
    /// Its title.
    pub title: String,
    /// The top-level step it is a sub-step of; none: a top-level step.
    pub parent: Option<StepRef>,
    /// The step it comes right after; none: last (among its parent's sub-steps, for a
    /// sub-step).
    pub after: Option<StepRef>,
}

/// One change to a plan ([`edit`]): a step added, or one step changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    /// The step to change (none when one is added).
    pub step: Option<StepRef>,
    /// A step to add; the other fields then apply to it.
    pub add: Option<AddStep>,
    /// Its new title.
    pub title: Option<String>,
    /// Its new state.
    pub state: Option<StepState>,
    /// Its new note; empty clears it.
    pub note: Option<String>,
    /// Its new link; empty clears it.
    pub link: Option<String>,
}

/// Why a plan change is refused; the text is for the agent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PlanRefusal {
    /// A step with no title.
    #[error("a step needs a title")]
    NoTitle,
    /// A title over [`TITLE_MAX`] characters.
    #[error("a step's title is at most {TITLE_MAX} characters: one short line")]
    TitleTooLong,
    /// A note over [`NOTE_MAX`] characters.
    #[error("a step's note is at most {NOTE_MAX} characters")]
    NoteTooLong,
    /// A link over [`LINK_MAX`] characters.
    #[error("a step's link is at most {LINK_MAX} characters")]
    LinkTooLong,
    /// An estimate over [`ETA_MAX`] characters.
    #[error("the estimate is at most {ETA_MAX} characters (about 2 hours)")]
    EtaTooLong,
    /// More than [`MAX_STEPS`] steps.
    #[error("a plan has at most {MAX_STEPS} steps, sub-steps included")]
    TooManySteps,
    /// A sub-step of a sub-step.
    #[error("steps nest one level: a sub-step has no sub-steps of its own")]
    TooDeep,
    /// No step has this id or title.
    #[error("no step {0} in the plan (plan_read lists them)")]
    NoStep(String),
    /// A step to add under a parent, after a step that isn't under it.
    #[error("the step it goes after is not that parent or one of its sub-steps")]
    NotUnder,
    /// Neither a step to change nor one to add.
    #[error("say which step (its id or exact title), or add one")]
    WhichStep,
    /// Nothing to change.
    #[error("nothing to change: give a state, note, link, title or estimate")]
    Nothing,
}

/// What a change came to: the whole plan after it, the last id given out, and the one history
/// line it writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Every step, in order: each top-level step, then its sub-steps.
    pub steps: Vec<Step>,
    /// The highest step id the task has given out.
    pub last_id: StepId,
    /// The task's history line for the change.
    pub line: String,
}

/// Fails unless `text` is a title a step may have; the title trimmed.
fn title(text: &str) -> Result<String, PlanRefusal> {
    let text = one_line(text);
    if text.is_empty() {
        return Err(PlanRefusal::NoTitle);
    }
    if text.chars().count() > TITLE_MAX {
        return Err(PlanRefusal::TitleTooLong);
    }
    Ok(text)
}

/// `text` on one line: its runs of white space, line breaks too, one space each.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A note or link as given: trimmed, empty is none.
fn field(text: &str, max: usize, too_long: PlanRefusal) -> Result<Option<String>, PlanRefusal> {
    let text = text.trim();
    if text.chars().count() > max {
        return Err(too_long);
    }
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

/// Fails unless `eta` is an estimate a plan may carry; the estimate trimmed, empty is none.
///
/// # Errors
///
/// [`PlanRefusal::EtaTooLong`].
pub fn eta(eta: &str) -> Result<Option<String>, PlanRefusal> {
    field(eta, ETA_MAX, PlanRefusal::EtaTooLong)
}

/// The key titles are matched by: one line, in lower case.
fn title_key(text: &str) -> String {
    one_line(text).to_lowercase()
}

/// Moves `step` to `to` at `now`: starting it dates its start (the first time only), finishing
/// it (done or skipped) dates its finish, and reopening it takes the finish off. Whether its
/// state changed.
pub fn move_to(step: &mut Step, to: StepState, now: Timestamp) -> bool {
    if step.state == to {
        return false;
    }
    step.state = to;
    if to == StepState::Doing && step.started.is_none() {
        step.started = Some(now);
    }
    step.finished = to.is_finished().then_some(now);
    step.updated = now;
    true
}

/// Sets `slot` to `value` (a field as given: empty clears) at `now`; whether it changed.
fn put(
    slot: &mut Option<String>,
    value: Option<&str>,
    max: usize,
    too_long: PlanRefusal,
) -> Result<bool, PlanRefusal> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value = field(value, max, too_long)?;
    if *slot == value {
        return Ok(false);
    }
    *slot = value;
    Ok(true)
}

/// Replaces the plan `old` with `new`, as an agent writes the whole plan at `now`. A step whose
/// title matches one of the old plan's (spaces and case aside, each old step matched once, in
/// order) keeps its id, state, note, link and times unless `new` gives them; a new step gets
/// the next id after `last_id`. The line says how many steps there are.
///
/// # Errors
///
/// [`PlanRefusal`] when a step has no title or one too long, a note or link too long, a
/// sub-step has sub-steps, or there are more than [`MAX_STEPS`].
pub fn set(
    old: &[Step],
    last_id: StepId,
    new: &[NewStep],
    now: Timestamp,
) -> Result<Outcome, PlanRefusal> {
    let count: usize = new.iter().map(|step| 1 + step.steps.len()).sum();
    if count > MAX_STEPS {
        return Err(PlanRefusal::TooManySteps);
    }
    if new
        .iter()
        .flat_map(|step| &step.steps)
        .any(|sub| !sub.steps.is_empty())
    {
        return Err(PlanRefusal::TooDeep);
    }
    let mut pool: BTreeMap<String, Vec<&Step>> = BTreeMap::new();
    for step in old {
        pool.entry(title_key(&step.title)).or_default().push(step);
    }
    let mut next = last_id;
    let mut steps = Vec::with_capacity(count);
    for top in new {
        let id = place(&mut steps, &mut pool, &mut next, top, None, now)?;
        for sub in &top.steps {
            place(&mut steps, &mut pool, &mut next, sub, Some(id), now)?;
        }
    }
    let line = set_line(&steps);
    Ok(Outcome {
        steps,
        last_id: next,
        line,
    })
}

/// Adds `new` to `steps` under `parent`: the old step it matches, changed as `new` says, or a
/// new one. Its id.
fn place(
    steps: &mut Vec<Step>,
    pool: &mut BTreeMap<String, Vec<&Step>>,
    next: &mut StepId,
    new: &NewStep,
    parent: Option<StepId>,
    now: Timestamp,
) -> Result<StepId, PlanRefusal> {
    let title = title(&new.title)?;
    let matched = pool
        .get_mut(&title_key(&title))
        .and_then(|found| (!found.is_empty()).then(|| found.remove(0)));
    let mut step = if let Some(old) = matched {
        let mut kept = old.clone();
        if kept.title != title || kept.parent != parent {
            kept.updated = now;
        }
        kept.title = title;
        kept.parent = parent;
        kept
    } else {
        *next = next.saturating_add(1);
        Step {
            id: *next,
            parent,
            title,
            state: StepState::Todo,
            note: None,
            link: None,
            started: None,
            finished: None,
            created: now,
            updated: now,
        }
    };
    if let Some(state) = new.state {
        move_to(&mut step, state, now);
    }
    if put(
        &mut step.note,
        new.note.as_deref(),
        NOTE_MAX,
        PlanRefusal::NoteTooLong,
    )? | put(
        &mut step.link,
        new.link.as_deref(),
        LINK_MAX,
        PlanRefusal::LinkTooLong,
    )? {
        step.updated = now;
    }
    let id = step.id;
    steps.push(step);
    Ok(id)
}

/// The index in `steps` of the step `which` names.
fn find(steps: &[Step], which: &StepRef) -> Result<usize, PlanRefusal> {
    match which {
        StepRef::Id(id) => steps
            .iter()
            .position(|step| step.id == *id)
            .ok_or_else(|| PlanRefusal::NoStep(id.to_string())),
        StepRef::Title(title) => {
            let key = title_key(title);
            steps
                .iter()
                .position(|step| title_key(&step.title) == key)
                .ok_or_else(|| PlanRefusal::NoStep(format!("\"{}\"", one_line(title))))
        }
    }
}

/// The index just past the step at `at` and its sub-steps.
fn past_family(steps: &[Step], at: usize) -> usize {
    let id = steps[at].id;
    let mut end = at + 1;
    while end < steps.len() && steps[end].parent == Some(id) {
        end += 1;
    }
    end
}

/// Inserts the step `add` with the id `id` into `steps` (under its parent, after the step
/// named, or last); its index.
fn insert(
    steps: &mut Vec<Step>,
    add: &AddStep,
    id: StepId,
    now: Timestamp,
) -> Result<usize, PlanRefusal> {
    if steps.len() >= MAX_STEPS {
        return Err(PlanRefusal::TooManySteps);
    }
    let title = title(&add.title)?;
    let parent = match &add.parent {
        Some(which) => {
            let at = find(steps, which)?;
            if steps[at].parent.is_some() {
                return Err(PlanRefusal::TooDeep);
            }
            Some(steps[at].id)
        }
        None => None,
    };
    let after = add
        .after
        .as_ref()
        .map(|which| find(steps, which))
        .transpose()?;
    let (at, parent) = match (after, parent) {
        (Some(after), Some(parent)) => {
            if steps[after].id != parent && steps[after].parent != Some(parent) {
                return Err(PlanRefusal::NotUnder);
            }
            (after + 1, Some(parent))
        }
        // After a sub-step: its sibling; after a top-level step: after its sub-steps too.
        (Some(after), None) => match steps[after].parent {
            Some(parent) => (after + 1, Some(parent)),
            None => (past_family(steps, after), None),
        },
        (None, Some(parent)) => {
            let top = steps
                .iter()
                .position(|step| step.id == parent)
                .unwrap_or_default();
            (past_family(steps, top), Some(parent))
        }
        (None, None) => (steps.len(), None),
    };
    steps.insert(
        at,
        Step {
            id,
            parent,
            title,
            state: StepState::Todo,
            note: None,
            link: None,
            started: None,
            finished: None,
            created: now,
            updated: now,
        },
    );
    Ok(at)
}

/// Changes the plan `old` as one call of an agent's says, at `now`: adds a step (placed under
/// its parent, after the step named, or last) or changes one (its title, state, note, link).
/// The line says what happened to it.
///
/// # Errors
///
/// [`PlanRefusal`] when no step is named or added, the step named isn't there, a parent is a
/// sub-step, nothing would change, a field is too long, or the plan would have more than
/// [`MAX_STEPS`].
pub fn edit(
    old: &[Step],
    last_id: StepId,
    edit: &Edit,
    now: Timestamp,
) -> Result<Outcome, PlanRefusal> {
    let mut steps = old.to_vec();
    let mut next = last_id;
    let (at, added) = if let Some(add) = &edit.add {
        next = next.saturating_add(1);
        (insert(&mut steps, add, next, now)?, true)
    } else {
        let which = edit.step.as_ref().ok_or(PlanRefusal::WhichStep)?;
        (find(&steps, which)?, false)
    };
    let step = &mut steps[at];
    let was_title = step.title.clone();
    let mut renamed = false;
    if !added && let Some(new_title) = &edit.title {
        let new_title = title(new_title)?;
        if new_title != step.title {
            step.title = new_title;
            step.updated = now;
            renamed = true;
        }
    }
    let moved = edit.state.is_some_and(|state| move_to(step, state, now));
    let noted = put(
        &mut step.note,
        edit.note.as_deref(),
        NOTE_MAX,
        PlanRefusal::NoteTooLong,
    )?;
    let linked = put(
        &mut step.link,
        edit.link.as_deref(),
        LINK_MAX,
        PlanRefusal::LinkTooLong,
    )?;
    if noted || linked {
        step.updated = now;
    }
    let line = if added {
        match step.state {
            StepState::Todo => format!("Added step: {}", step.title),
            _ => format!("Added step: {} ({})", step.title, state_line(step)),
        }
    } else if moved {
        state_line(step)
    } else if renamed {
        format!("Renamed step: {was_title} → {}", step.title)
    } else if noted && step.note.is_some() {
        format!(
            "Note on {}: {}",
            step.title,
            step.note.as_deref().unwrap_or_default()
        )
    } else if noted || linked {
        format!("Updated step: {}", step.title)
    } else {
        return Err(PlanRefusal::Nothing);
    };
    Ok(Outcome {
        steps,
        last_id: next,
        line,
    })
}

/// The history line of a step's move to its state: `Ticked: Build the Mac app`,
/// `Blocked: Sign the build — needs the certificate`.
#[must_use]
pub fn state_line(step: &Step) -> String {
    let title = &step.title;
    match step.state {
        StepState::Done => format!("Ticked: {title}"),
        StepState::Doing => format!("Started: {title}"),
        StepState::Skipped => format!("Skipped: {title}"),
        StepState::Todo => format!("Reopened: {title}"),
        StepState::Blocked => match step.note.as_deref() {
            Some(note) => format!("Blocked: {title} — {note}"),
            None => format!("Blocked: {title}"),
        },
    }
}

/// The history line of a whole plan written: `Plan set: 7 steps`, with how many are done.
#[must_use]
pub fn set_line(steps: &[Step]) -> String {
    let glance = summary(steps);
    let count = match glance.total {
        1 => "1 step".to_owned(),
        n => format!("{n} steps"),
    };
    if glance.done == 0 {
        format!("Plan set: {count}")
    } else {
        format!("Plan set: {count}, {} done", glance.done)
    }
}

/// The history line of a plan's estimate changed alone.
#[must_use]
pub fn eta_line(eta: Option<&str>) -> String {
    eta.map_or_else(
        || "Plan estimate cleared".to_owned(),
        |eta| format!("Plan estimate: {eta}"),
    )
}

/// The history line of a turn of work on a task that ended with no plan.
pub const UNPLANNED_TURN_LINE: &str = "A turn ended with no plan";

/// A plan at a glance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary<'a> {
    /// Steps done.
    pub done: u32,
    /// Steps in all. 0: no plan yet.
    pub total: u32,
    /// The step being worked on, or next.
    pub current: Option<&'a Step>,
    /// What blocks the plan.
    pub blocker: Option<String>,
}

/// The plan at a glance: a top-level step with sub-steps counts as its sub-steps, a skipped
/// step in neither `done` nor `total`; the current step is the first being worked on, else
/// the first blocked, else the first not started (of the steps that count); the blocker is the
/// first blocked step's note, else its title.
#[must_use]
pub fn summary(steps: &[Step]) -> Summary<'_> {
    let has_subs = |step: &Step| steps.iter().any(|sub| sub.parent == Some(step.id));
    let counted: Vec<&Step> = steps
        .iter()
        .filter(|step| step.parent.is_some() || !has_subs(step))
        .collect();
    let tally = |keep: &dyn Fn(&Step) -> bool| {
        u32::try_from(counted.iter().filter(|step| keep(step)).count()).unwrap_or(u32::MAX)
    };
    let first = |state: StepState| counted.iter().find(|step| step.state == state).copied();
    Summary {
        done: tally(&|step| step.state == StepState::Done),
        total: tally(&|step| step.state != StepState::Skipped),
        current: first(StepState::Doing)
            .or_else(|| first(StepState::Blocked))
            .or_else(|| first(StepState::Todo)),
        blocker: steps
            .iter()
            .find(|step| step.state == StepState::Blocked)
            .map(|step| step.note.clone().unwrap_or_else(|| step.title.clone())),
    }
}

/// The plan as the checklist an agent reads back: a line per step with its box and id
/// (`[x] 1 Build the server`, `[>] 2 Run the tests`, `[!] 3 Sign it — needs the key`),
/// sub-steps indented, then the plan at a glance and its estimate.
#[must_use]
pub fn checklist(steps: &[Step], eta: Option<&str>) -> String {
    if steps.is_empty() {
        return "No plan yet. Write it with plan_set: every step of the work, in order.".to_owned();
    }
    let mut lines: Vec<String> = steps
        .iter()
        .map(|step| {
            let indent = if step.parent.is_some() { "  " } else { "" };
            let mut line = format!("{indent}{} {} {}", step.state.mark(), step.id, step.title);
            if let Some(note) = &step.note {
                line.push_str(" — ");
                line.push_str(note);
            }
            if let Some(link) = &step.link {
                line.push_str(" (");
                line.push_str(link);
                line.push(')');
            }
            line
        })
        .collect();
    let glance = summary(steps);
    let mut tail = vec![format!("{} of {} done", glance.done, glance.total)];
    if let Some(current) = glance.current {
        tail.push(format!("now: {} {}", current.id, current.title));
    }
    if let Some(blocker) = &glance.blocker {
        tail.push(format!("blocked: {blocker}"));
    }
    if let Some(eta) = eta {
        tail.push(format!("estimate: {eta}"));
    }
    lines.push(String::new());
    lines.push(tail.join("; "));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + seconds).unwrap()
    }

    fn new(title: &str) -> NewStep {
        NewStep {
            title: title.to_owned(),
            ..NewStep::default()
        }
    }

    fn with(title: &str, state: StepState) -> NewStep {
        NewStep {
            state: Some(state),
            ..new(title)
        }
    }

    fn by_id(id: StepId) -> StepRef {
        StepRef::Id(id)
    }

    fn plan(steps: &[NewStep]) -> Vec<Step> {
        set(&[], 0, steps, at(0)).unwrap().steps
    }

    #[test]
    fn a_new_plan_numbers_its_steps_in_order_with_sub_steps_after_their_step() {
        let out = set(
            &[],
            0,
            &[
                NewStep {
                    steps: vec![new("Write the code"), new("Write the tests")],
                    ..new("Build the server")
                },
                new("Ship it"),
            ],
            at(0),
        )
        .unwrap();
        let ids: Vec<(StepId, Option<StepId>)> =
            out.steps.iter().map(|s| (s.id, s.parent)).collect();
        assert_eq!(ids, [(1, None), (2, Some(1)), (3, Some(1)), (4, None)]);
        assert_eq!(out.last_id, 4);
        assert_eq!(
            out.line, "Plan set: 3 steps",
            "the sub-steps count for their step"
        );
    }

    #[test]
    fn setting_the_plan_again_keeps_the_ticks_of_the_steps_it_names_again() {
        let first = set(
            &[],
            0,
            &[
                with("Build the server", StepState::Done),
                with("Run the tests", StepState::Doing),
                new("Old step"),
            ],
            at(0),
        )
        .unwrap();
        let again = set(
            &first.steps,
            first.last_id,
            &[
                new("build the server "),
                new("Run the tests"),
                new("Sign the build"),
            ],
            at(60),
        )
        .unwrap();
        let built = &again.steps[0];
        assert_eq!((built.id, built.state), (1, StepState::Done));
        assert_eq!(built.finished, Some(at(0)), "its times kept");
        assert_eq!(built.title, "build the server", "the new title");
        assert_eq!(again.steps[1].state, StepState::Doing);
        assert_eq!(again.steps[1].started, Some(at(0)));
        assert_eq!(
            again.steps[2].id, 4,
            "a new step never takes a removed one's id"
        );
        assert_eq!(again.last_id, 4);
        assert_eq!(again.line, "Plan set: 3 steps, 1 done");
    }

    #[test]
    fn a_plan_nests_one_level_and_has_caps() {
        let deep = NewStep {
            steps: vec![NewStep {
                steps: vec![new("Too deep")],
                ..new("Sub")
            }],
            ..new("Top")
        };
        assert_eq!(set(&[], 0, &[deep], at(0)), Err(PlanRefusal::TooDeep));
        let many: Vec<NewStep> = (0..=MAX_STEPS).map(|n| new(&format!("Step {n}"))).collect();
        assert_eq!(set(&[], 0, &many, at(0)), Err(PlanRefusal::TooManySteps));
        assert_eq!(set(&[], 0, &[new("  ")], at(0)), Err(PlanRefusal::NoTitle));
        let long = "x".repeat(TITLE_MAX + 1);
        assert_eq!(
            set(&[], 0, &[new(&long)], at(0)),
            Err(PlanRefusal::TitleTooLong)
        );
        assert_eq!(eta(&"x".repeat(ETA_MAX + 1)), Err(PlanRefusal::EtaTooLong));
        assert_eq!(eta("  "), Ok(None));
    }

    #[test]
    fn a_state_dates_the_step() {
        let mut step = plan(&[new("Run the tests")]).remove(0);
        assert!(move_to(&mut step, StepState::Doing, at(10)));
        assert_eq!((step.started, step.finished), (Some(at(10)), None));
        assert!(move_to(&mut step, StepState::Done, at(20)));
        assert_eq!(step.finished, Some(at(20)));
        assert!(move_to(&mut step, StepState::Doing, at(30)));
        assert_eq!(
            (step.started, step.finished),
            (Some(at(10)), None),
            "reopened: the first start kept, the finish taken off"
        );
        assert!(move_to(&mut step, StepState::Skipped, at(40)));
        assert_eq!(step.finished, Some(at(40)));
        assert!(!move_to(&mut step, StepState::Skipped, at(50)));
        assert_eq!(step.updated, at(40), "no change, no new time");
    }

    #[test]
    fn the_summary_counts_sub_steps_for_their_step_and_leaves_skipped_ones_out() {
        let steps = plan(&[
            NewStep {
                steps: vec![
                    with("Compile", StepState::Done),
                    with("Link", StepState::Skipped),
                    new("Sign"),
                ],
                ..with("Build", StepState::Doing)
            },
            with("Test", StepState::Done),
            new("Ship"),
        ]);
        let glance = summary(&steps);
        assert_eq!((glance.done, glance.total), (2, 4));
        assert_eq!(
            glance.current.map(|s| s.title.as_str()),
            Some("Sign"),
            "the step being worked on counts as its sub-steps: the first one to do"
        );
        assert_eq!(glance.blocker, None);
        assert_eq!(summary(&[]), Summary::default());
    }

    #[test]
    fn the_current_step_is_the_one_doing_else_blocked_else_to_do() {
        let mut steps = plan(&[
            with("Done one", StepState::Done),
            new("Next"),
            with("Stuck", StepState::Blocked),
            with("Working", StepState::Doing),
        ]);
        assert_eq!(summary(&steps).current.map(|s| s.id), Some(4));
        steps[3].state = StepState::Done;
        assert_eq!(summary(&steps).current.map(|s| s.id), Some(3));
        assert_eq!(summary(&steps).blocker.as_deref(), Some("Stuck"));
        steps[2].note = Some("needs the certificate".to_owned());
        assert_eq!(
            summary(&steps).blocker.as_deref(),
            Some("needs the certificate")
        );
        steps[2].state = StepState::Done;
        assert_eq!(summary(&steps).current.map(|s| s.id), Some(2));
    }

    #[test]
    fn a_step_changed_writes_one_line_saying_how() {
        let steps = plan(&[new("Build the Mac app"), new("Sign the build")]);
        let tick = |state, note: Option<&str>| {
            edit(
                &steps,
                2,
                &Edit {
                    step: Some(by_id(1)),
                    state: Some(state),
                    note: note.map(str::to_owned),
                    ..Edit::default()
                },
                at(5),
            )
            .unwrap()
            .line
        };
        assert_eq!(tick(StepState::Done, None), "Ticked: Build the Mac app");
        assert_eq!(tick(StepState::Doing, None), "Started: Build the Mac app");
        assert_eq!(tick(StepState::Skipped, None), "Skipped: Build the Mac app");
        assert_eq!(
            tick(StepState::Blocked, Some("needs the certificate")),
            "Blocked: Build the Mac app — needs the certificate"
        );
        let renamed = edit(
            &steps,
            2,
            &Edit {
                step: Some(StepRef::Title("sign the BUILD".to_owned())),
                title: Some("Sign and notarize".to_owned()),
                ..Edit::default()
            },
            at(5),
        )
        .unwrap();
        assert_eq!(
            renamed.line,
            "Renamed step: Sign the build → Sign and notarize"
        );
        assert_eq!(renamed.steps[1].id, 2);
        assert_eq!(
            edit(
                &steps,
                2,
                &Edit {
                    step: Some(by_id(1)),
                    ..Edit::default()
                },
                at(5)
            ),
            Err(PlanRefusal::Nothing)
        );
        assert_eq!(
            edit(&steps, 2, &Edit::default(), at(5)),
            Err(PlanRefusal::WhichStep)
        );
        assert!(matches!(
            edit(
                &steps,
                2,
                &Edit {
                    step: Some(by_id(9)),
                    state: Some(StepState::Done),
                    ..Edit::default()
                },
                at(5)
            ),
            Err(PlanRefusal::NoStep(_))
        ));
    }

    #[test]
    fn an_added_step_goes_under_its_parent_after_the_step_named_or_last() {
        let steps = plan(&[
            NewStep {
                steps: vec![new("Compile")],
                ..new("Build")
            },
            new("Ship"),
        ]);
        let add = |add: AddStep, state: Option<StepState>| {
            edit(
                &steps,
                3,
                &Edit {
                    add: Some(add),
                    state,
                    ..Edit::default()
                },
                at(5),
            )
        };
        let last = add(
            AddStep {
                title: "Announce".to_owned(),
                ..AddStep::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(last.steps.last().map(|s| s.id), Some(4));
        assert_eq!(last.line, "Added step: Announce");
        let under = add(
            AddStep {
                title: "Sign".to_owned(),
                parent: Some(by_id(1)),
                ..AddStep::default()
            },
            Some(StepState::Doing),
        )
        .unwrap();
        let order: Vec<(StepId, Option<StepId>)> =
            under.steps.iter().map(|s| (s.id, s.parent)).collect();
        assert_eq!(order, [(1, None), (2, Some(1)), (4, Some(1)), (3, None)]);
        assert_eq!(under.line, "Added step: Sign (Started: Sign)");
        let after_top = add(
            AddStep {
                title: "Test".to_owned(),
                after: Some(by_id(1)),
                ..AddStep::default()
            },
            None,
        )
        .unwrap();
        let order: Vec<StepId> = after_top.steps.iter().map(|s| s.id).collect();
        assert_eq!(order, [1, 2, 4, 3], "after the step and its sub-steps");
        let after_sub = add(
            AddStep {
                title: "Strip".to_owned(),
                after: Some(by_id(2)),
                ..AddStep::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            after_sub.steps[2].parent,
            Some(1),
            "a sibling of the sub-step"
        );
        assert_eq!(
            add(
                AddStep {
                    title: "Deeper".to_owned(),
                    parent: Some(by_id(2)),
                    ..AddStep::default()
                },
                None
            ),
            Err(PlanRefusal::TooDeep)
        );
    }

    #[test]
    fn the_checklist_shows_each_step_with_its_box_and_id() {
        let mut steps = plan(&[
            with("Build the server", StepState::Done),
            NewStep {
                steps: vec![with("Unit", StepState::Doing)],
                ..new("Run the tests")
            },
            with("Sign it", StepState::Blocked),
        ]);
        steps[3].note = Some("needs the key".to_owned());
        assert_eq!(
            checklist(&steps, Some("about an hour")),
            "[x] 1 Build the server\n[ ] 2 Run the tests\n  [>] 3 Unit\n[!] 4 Sign it — needs the key\n\n1 of 3 done; now: 3 Unit; blocked: needs the key; estimate: about an hour"
        );
        assert!(checklist(&[], None).starts_with("No plan yet"));
    }

    #[test]
    fn every_state_reads_back_from_its_word() {
        for state in StepState::ALL {
            assert_eq!(StepState::parse(state.as_str()), Some(state));
        }
        assert_eq!(StepState::parse("later"), None);
    }
}
