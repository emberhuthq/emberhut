//! Tasks: every piece of work, the person's and the assistant's, with an owner, a status, a
//! project, a place in a tree of subtasks and a history.
//!
//! A task moves `todo` → `working` → `waiting` (on the person) → `done`, or is `canceled`
//! (shown as Archived). Waiting means it needs the person, so a waiting task always has an
//! action: one short instruction for them. Work the person and the assistant only discuss stays
//! working. A scan's or a review's results are filed under it as findings: tasks of their own
//! kind, done once landed or superseded.
//!
//! This module holds the rules a change goes through ([`settle_change`]); the database applies
//! them and what follows from them (its parent's rollup, its subtree's area) in one transaction.

use std::fmt;

use jiff::Timestamp;
use jiff::civil::Date;

pub mod area;
pub mod ask;
pub mod board;
pub mod finding;
pub mod live;
pub mod rollup;
pub mod search;
pub mod wire;

macro_rules! task_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i64);

        impl $name {
            /// The id with this number.
            #[must_use]
            pub const fn new(id: i64) -> Self {
                Self(id)
            }

            /// The id's number, as the database and the API write it.
            #[must_use]
            pub const fn get(self) -> i64 {
                self.0
            }
        }

        impl From<i64> for $name {
            fn from(id: i64) -> Self {
                Self(id)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

task_id!(
    /// A task (shown by its key, [`TaskId::key`]).
    TaskId
);
task_id!(
    /// A project.
    ProjectId
);
task_id!(
    /// An area of a project.
    AreaId
);
task_id!(
    /// Something a task produced.
    ArtifactId
);
task_id!(
    /// One turn of work on a task.
    RunId
);
task_id!(
    /// One of the person's goals.
    GoalId
);
task_id!(
    /// One ask put to the person on a task: the number of the history line it wrote, so every
    /// ask (a re-check too) has one of its own, and an answer names the ask it answered.
    AskId
);

impl TaskId {
    /// The task's key as the apps show it: the prefix and the number (`T-12`).
    #[must_use]
    pub fn key(self) -> String {
        format!("{}{}", wire::KEY_PREFIX, self.0)
    }
}

/// How deep tasks go under an area (or a project): a top-level task is level 1.
pub const MAX_DEPTH: usize = 6;

/// The longest a task's title may be, in characters: the same for every way a task is made or
/// changed (the app, the assistant's tools, a program).
pub const TITLE_MAX: usize = 500;

/// The longest a task's brief (its body) may be, in characters, for every way it is set.
pub const BODY_MAX: usize = 20_000;

/// Whether a task's `title` and `body` (each when given) are within [`TITLE_MAX`] and
/// [`BODY_MAX`].
///
/// # Errors
///
/// [`TaskRefusal::TitleTooLong`] or [`TaskRefusal::BodyTooLong`]: too long is refused, never cut.
pub fn check_lengths(title: Option<&str>, body: Option<&str>) -> Result<(), TaskRefusal> {
    if title.is_some_and(|title| title.chars().count() > TITLE_MAX) {
        return Err(TaskRefusal::TitleTooLong);
    }
    if body.is_some_and(|body| body.chars().count() > BODY_MAX) {
        return Err(TaskRefusal::BodyTooLong);
    }
    Ok(())
}

/// The far `parked_until` an archived project gives its open tasks: unarchiving takes back
/// exactly those.
pub const PARKED_BY_ARCHIVE: &str = "9999-12-31";

/// Where a task is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Not started.
    Todo,
    /// Being worked on (or discussed).
    Working,
    /// Waiting on the person: it has an action.
    Waiting,
    /// Done.
    Done,
    /// Put away for good (Archived).
    Canceled,
}

impl Status {
    /// Every status, in the order they are worked through.
    pub const ALL: [Self; 5] = [
        Self::Todo,
        Self::Working,
        Self::Waiting,
        Self::Done,
        Self::Canceled,
    ];

    /// Its word, as stored and sent.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }

    /// The status `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == word)
    }

    /// Still to be finished: to do, working or waiting.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Todo | Self::Working | Self::Waiting)
    }

    /// Done or canceled.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        !self.is_open()
    }

    /// The history line a change to this status writes.
    #[must_use]
    pub const fn note(self) -> &'static str {
        match self {
            Self::Todo => "Back to to do",
            Self::Working => "Started",
            Self::Waiting => "Waiting on the owner",
            Self::Done => "Done",
            Self::Canceled => "Canceled",
        }
    }
}

/// Whose a task is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    /// The person's to do.
    Person,
    /// The assistant's work (its workers take the ones with a brief).
    Assistant,
}

impl Owner {
    /// Its wire word ([`wire::PERSON`], [`wire::ASSISTANT`]).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => wire::PERSON,
            Self::Assistant => wire::ASSISTANT,
        }
    }

    /// The owner `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            wire::PERSON => Some(Self::Person),
            wire::ASSISTANT => Some(Self::Assistant),
            _ => None,
        }
    }
}

/// Who wrote a line of a task's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    /// The person (in the app).
    Person,
    /// The assistant.
    Assistant,
    /// A worker: a session doing one task.
    Worker,
}

impl Who {
    /// Its wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => wire::PERSON,
            Self::Assistant => wire::ASSISTANT,
            Self::Worker => wire::WORKER,
        }
    }

    /// The author `word` names; an unknown word reads as the assistant.
    #[must_use]
    pub fn parse(word: &str) -> Self {
        match word {
            wire::PERSON => Self::Person,
            wire::WORKER => Self::Worker,
            _ => Self::Assistant,
        }
    }
}

/// A task's rough size, in the person's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    /// About an hour.
    S,
    /// About three hours.
    M,
    /// About a day.
    L,
    /// Several days.
    Xl,
}

impl Size {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
            Self::Xl => "XL",
        }
    }

    /// The size `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "S" => Some(Self::S),
            "M" => Some(Self::M),
            "L" => Some(Self::L),
            "XL" => Some(Self::Xl),
            _ => None,
        }
    }

    /// The hours of the person's time it stands for.
    #[must_use]
    pub const fn hours(self) -> f64 {
        match self {
            Self::S => 1.0,
            Self::M => 3.0,
            Self::L => 8.0,
            Self::Xl => 20.0,
        }
    }
}

/// Whether a finding (a scan's or a review's result) landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Landed {
    /// It landed: done.
    Landed,
    /// Partly landed: still open.
    Partly,
    /// Not landed: open.
    NotLanded,
    /// Made moot by later work: done.
    Superseded,
    /// Not checked against the work yet: open.
    Unchecked,
}

impl Landed {
    /// Every state, as the tools list them.
    pub const ALL: [Self; 5] = [
        Self::Landed,
        Self::Partly,
        Self::NotLanded,
        Self::Superseded,
        Self::Unchecked,
    ];

    /// Its stored word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Landed => "landed",
            Self::Partly => "partly",
            Self::NotLanded => "not_landed",
            Self::Superseded => "superseded",
            Self::Unchecked => "unchecked",
        }
    }

    /// The state `word` names (its stored word), if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|landed| landed.as_str() == word)
    }

    /// In words, for a history line.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Landed => "landed",
            Self::Partly => "partly landed",
            Self::NotLanded => "not landed",
            Self::Superseded => "superseded",
            Self::Unchecked => "unchecked",
        }
    }

    /// The status it gives its finding: landed and superseded are done, the rest open.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Self::Landed | Self::Superseded => Status::Done,
            Self::Partly | Self::NotLanded | Self::Unchecked => Status::Todo,
        }
    }

    /// Still open: not landed, partly or unchecked.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self.status(), Status::Todo)
    }
}

/// A task, as stored.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a column of its own"
)]
pub struct Task {
    /// Its id.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Its brief.
    pub body: Option<String>,
    /// Whose it is.
    pub owner: Owner,
    /// Where it is.
    pub status: Status,
    /// Its own goal.
    pub goal: Option<GoalId>,
    /// Its project's name, as given (older tasks name their project only by name).
    pub project: Option<String>,
    /// Its project.
    pub pid: Option<ProjectId>,
    /// Its own deadline is someone else's.
    pub hard: bool,
    /// Its rough size.
    pub size: Option<Size>,
    /// 0 none, 1 urgent ... 4 low.
    pub priority: i32,
    /// Its own deadline.
    pub due: Option<Date>,
    /// Pinned to Today.
    pub today: bool,
    /// Out of sight until then: a day or an instant, as given.
    pub parked_until: Option<String>,
    /// An ask's day to decide by.
    pub decide_by: Option<Date>,
    /// What it asks, as stored (JSON in the wire shape).
    pub decision: Option<String>,
    /// What the person should do, while it waits on them.
    pub action: Option<String>,
    /// When it was made.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
    /// When it was done.
    pub done_at: Option<Timestamp>,
    /// The newest line of its history.
    pub last: Option<String>,
    /// Its parent, for a subtask (or the scan, for a finding).
    pub parent: Option<TaskId>,
    /// Its count so far.
    pub progress: Option<i64>,
    /// What its count goes to.
    pub total: Option<i64>,
    /// What it counts.
    pub unit: Option<String>,
    /// Done elsewhere: the assistant only keeps its status, no worker takes it.
    pub tracked: bool,
    /// When it is expected done, in words.
    pub eta: Option<String>,
    /// Its runs split the work across sub-agents.
    pub ultra: bool,
    /// The model its work runs on (none: the default).
    pub model: Option<String>,
    /// Its area in the project.
    pub area: Option<AreaId>,
    /// It is a finding, not a task.
    pub finding: bool,
    /// A finding's state.
    pub landed: Option<Landed>,
    /// A finding's: the task that fixes it.
    pub fixed_by: Option<TaskId>,
    /// What does the work when it isn't a turn: `unit:<name>` or `watch:<id>`.
    pub runner: Option<String>,
    /// What its worker carries on with when it next runs (the person's answers, a new brief).
    pub next: Option<String>,
    /// Its worker's unit, while one runs.
    pub worker: Option<String>,
    /// Its worker's session, once one ran.
    pub session: Option<String>,
    /// It came from the person's own request: when and where (JSON, the wire shape).
    pub requested: Option<String>,
    /// It came from the ask of someone other than the owner: who, when and how (JSON, the wire
    /// shape).
    pub requested_by: Option<String>,
}

impl Task {
    /// Its key, as the apps show it.
    #[must_use]
    pub fn key(&self) -> String {
        self.id.key()
    }

    /// Its deadline as it counts, when it has its own: the day and whether it is hard.
    #[must_use]
    pub fn own_deadline(&self) -> Option<Deadline> {
        self.due.map(|date| Deadline {
            date,
            hard: self.hard,
        })
    }

    /// The hours of the person's time it stands for (unsized: one).
    #[must_use]
    pub fn hours(&self) -> f64 {
        self.size.map_or(1.0, Size::hours)
    }
}

/// A deadline as it counts: its day, and whether someone else holds the person to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    /// The day.
    pub date: Date,
    /// Someone else's (hard), or the person's own target (soft).
    pub hard: bool,
}

/// A field of a change: `None` leaves it, `Some(None)` clears it, `Some(Some(v))` sets it.
pub type Patch<T> = Option<Option<T>>;

/// The fields of a task to set. A field left out stays as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskChange {
    /// Its title.
    pub title: Option<String>,
    /// Its brief.
    pub body: Patch<String>,
    /// Whose it is.
    pub owner: Option<Owner>,
    /// Where it is.
    pub status: Option<Status>,
    /// Its own goal.
    pub goal: Patch<GoalId>,
    /// 0 none, 1 urgent ... 4 low.
    pub priority: Option<i32>,
    /// Its own deadline.
    pub due: Patch<Date>,
    /// Pinned to Today.
    pub today: Option<bool>,
    /// Out of sight until then.
    pub parked_until: Patch<String>,
    /// Its project.
    pub pid: Patch<ProjectId>,
    /// Its deadline is someone else's.
    pub hard: Option<bool>,
    /// Its rough size.
    pub size: Patch<Size>,
    /// Its parent.
    pub parent: Patch<TaskId>,
    /// Its count so far.
    pub progress: Option<i64>,
    /// What its count goes to.
    pub total: Option<i64>,
    /// What it counts.
    pub unit: Option<String>,
    /// Done elsewhere.
    pub tracked: Option<bool>,
    /// When it is expected done.
    pub eta: Patch<String>,
    /// What the person should do: it then waits on them; cleared, it goes back to work.
    pub action: Patch<String>,
    /// What does the work when it isn't a turn.
    pub runner: Patch<String>,
    /// An ask's day to decide by.
    pub decide_by: Patch<Date>,
    /// Ultra mode.
    pub ultra: Option<bool>,
    /// Its model.
    pub model: Patch<String>,
    /// Its area.
    pub area: Patch<AreaId>,
    /// A finding's state.
    pub landed: Option<Landed>,
    /// A finding's fixing task.
    pub fixed_by: Patch<TaskId>,
}

impl TaskChange {
    /// A change of the status alone.
    #[must_use]
    pub fn status(status: Status) -> Self {
        Self {
            status: Some(status),
            ..Self::default()
        }
    }

    /// Whether it changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Why a change or a new task is refused. The text is for the assistant (or the app).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TaskRefusal {
    /// There is no such task.
    #[error("no task {}", .0.key())]
    NoTask(TaskId),
    /// A title over [`TITLE_MAX`] characters.
    #[error("a task's title is at most {TITLE_MAX} characters")]
    TitleTooLong,
    /// A brief over [`BODY_MAX`] characters.
    #[error("a task's brief is at most {BODY_MAX} characters")]
    BodyTooLong,
    /// There is no such area.
    #[error("no area {0}")]
    NoArea(AreaId),
    /// There is no such project.
    #[error("no project {0}")]
    NoProject(ProjectId),
    /// An action was given with a status other than waiting.
    #[error("an action means it waits on the person; leave status out or use waiting")]
    ActionNotWaiting,
    /// Waiting was asked for with no action.
    #[error(
        "waiting means it needs the person: give it an action, one short instruction for them (\"Approve the deploy\"). Work you're only discussing with them stays working."
    )]
    WaitingWithoutAction,
    /// `landed` or `fixed_by` on a task that isn't a finding.
    #[error("{} isn't a finding: landed and fixed_by are a finding's", .0.key())]
    NotAFinding(TaskId),
    /// `landed` or `fixed_by` on a new task that isn't a finding.
    #[error("landed and fixed_by are a finding's (kind finding)")]
    FindingFieldsOnTask,
    /// A finding with no scan to go under.
    #[error("a finding goes under the scan or review task that found it: give parent_id")]
    FindingWithoutScan,
    /// A finding taken out from under its scan.
    #[error("a finding stays under a scan or review task")]
    FindingLeavesScan,
    /// Something filed under a finding.
    #[error("{} is a finding: nothing goes under a finding (file it under its scan task)", .0.key())]
    UnderAFinding(TaskId),
    /// A task under itself.
    #[error("{} can't be its own parent", .0.key())]
    OwnParent(TaskId),
    /// A task put under one of its own subtasks.
    #[error("{} is under {}: putting {} under it would make a loop", .parent.key(), .task.key(), .task.key())]
    Loop {
        /// The task moved.
        task: TaskId,
        /// The parent asked for.
        parent: TaskId,
    },
    /// A tree deeper than [`MAX_DEPTH`].
    #[error("too deep: tasks go at most {MAX_DEPTH} levels under an area")]
    TooDeep,
    /// A subtask's area changed on its own.
    #[error(
        "{} is a subtask of {}: it sits in its parent's area. Move {}, or give parent_id null to take it out",
        .task.key(), .parent.key(), .parent.key()
    )]
    SubtaskArea {
        /// The subtask.
        task: TaskId,
        /// Its parent.
        parent: TaskId,
    },
    /// An area given with another project.
    #[error("area {0} is in another project")]
    AreaElsewhere(AreaId),
    /// A finding's fix that is no task (or itself).
    #[error("no task {} to fix it", .0.key())]
    NoFix(TaskId),
    /// A task with no title.
    #[error("a task needs a title")]
    NoTitle,
}

/// The change as it is applied to `before`: an action trimmed (a blank one clears it), an
/// action putting the task in front of the person and a cleared one sending it back to work,
/// a count that reaches its total finishing it, and a finding's state giving its status.
///
/// # Errors
///
/// [`TaskRefusal`] when the change breaks a rule: an action with another status, waiting
/// without an action, a finding's fields on a task, a finding taken from its scan.
pub fn settle_change(before: &Task, mut change: TaskChange) -> Result<TaskChange, TaskRefusal> {
    if !before.finding && (change.landed.is_some() || matches!(change.fixed_by, Some(Some(_)))) {
        return Err(TaskRefusal::NotAFinding(before.id));
    }
    if before.finding && matches!(change.parent, Some(None)) {
        return Err(TaskRefusal::FindingLeavesScan);
    }
    if before.finding
        && let Some(landed) = change.landed
        && change.status.is_none()
    {
        change.status = Some(landed.status());
    }
    if let Some(Some(action)) = &change.action {
        let trimmed = action.trim();
        change.action = Some((!trimmed.is_empty()).then(|| trimmed.to_owned()));
    }
    match &change.action {
        Some(Some(_)) => {
            if change
                .status
                .is_some_and(|status| status != Status::Waiting)
            {
                return Err(TaskRefusal::ActionNotWaiting);
            }
            change.status = Some(Status::Waiting);
        }
        Some(None) if before.status == Status::Waiting && change.status.is_none() => {
            change.status = Some(match before.owner {
                Owner::Person => Status::Todo,
                Owner::Assistant => Status::Working,
            });
        }
        _ => {}
    }
    let status = change.status.unwrap_or(before.status);
    let action = match &change.action {
        Some(action) => action.as_deref(),
        None => before.action.as_deref(),
    };
    if status == Status::Waiting && action.is_none_or(str::is_empty) {
        return Err(TaskRefusal::WaitingWithoutAction);
    }
    if change.status.is_none()
        && let Some(progress) = change.progress
        && let Some(total) = change.total.or(before.total)
        && total != 0
    {
        if progress >= total && before.status != Status::Done {
            change.status = Some(Status::Done);
        } else if progress > 0 && progress < total && before.status == Status::Todo {
            change.status = Some(Status::Working);
        }
    }
    Ok(change)
}

/// "2026-10-24" from a day or an instant as text, the instant's day in `zone`.
///
/// # Errors
///
/// The text, when it is neither.
pub fn date_only(text: &str, zone: &jiff::tz::TimeZone) -> Result<Date, String> {
    let text = text.trim();
    if let Ok(date) = text.parse::<Date>()
        && text.len() == 10
    {
        return Ok(date);
    }
    text.parse::<Timestamp>()
        .map(|at| at.to_zoned_datetime(zone.clone()).date())
        .map_err(|_| format!("decide_by must be a date like 2026-10-24, got \"{text}\""))
}

/// A deadline given as text: its first ten characters as a day (`2026-10-15`, or the day of
/// `2026-10-15T09:00`). Blank: none.
///
/// # Errors
///
/// The text, when it doesn't start with a day.
pub fn day(text: Option<&str>) -> Result<Option<Date>, String> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    text.get(..10)
        .and_then(|head| head.parse::<Date>().ok())
        .map(Some)
        .ok_or_else(|| format!("deadline must be a date like 2026-10-15, got \"{text}\""))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::task::wire::KEY_PREFIX;

    /// A plain open task of the assistant's, for the rules' tests.
    pub(crate) fn task(id: i64) -> Task {
        Task {
            id: TaskId::new(id),
            title: format!("Task {id}"),
            body: None,
            owner: Owner::Assistant,
            status: Status::Todo,
            goal: None,
            project: None,
            pid: None,
            hard: false,
            size: None,
            priority: 3,
            due: None,
            today: false,
            parked_until: None,
            decide_by: None,
            decision: None,
            action: None,
            created: Timestamp::UNIX_EPOCH,
            updated: Timestamp::UNIX_EPOCH,
            done_at: None,
            last: None,
            parent: None,
            progress: None,
            total: None,
            unit: None,
            tracked: false,
            eta: None,
            ultra: false,
            model: None,
            area: None,
            finding: false,
            landed: None,
            fixed_by: None,
            runner: None,
            next: None,
            worker: None,
            session: None,
            requested: None,
            requested_by: None,
        }
    }

    #[test]
    fn an_action_makes_it_wait_and_clearing_it_sends_it_back() {
        let before = task(1);
        let change = TaskChange {
            action: Some(Some("  Approve the deploy ".to_owned())),
            ..TaskChange::default()
        };
        let settled = settle_change(&before, change).unwrap();
        assert_eq!(settled.status, Some(Status::Waiting));
        assert_eq!(settled.action, Some(Some("Approve the deploy".to_owned())));

        let mut waiting = task(1);
        waiting.status = Status::Waiting;
        waiting.action = Some("Approve".to_owned());
        let back = settle_change(
            &waiting,
            TaskChange {
                action: Some(None),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(back.status, Some(Status::Working));
        waiting.owner = Owner::Person;
        let mine = settle_change(
            &waiting,
            TaskChange {
                action: Some(None),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(mine.status, Some(Status::Todo));
    }

    #[test]
    fn waiting_needs_an_action_and_an_action_needs_waiting() {
        assert_eq!(
            settle_change(&task(1), TaskChange::status(Status::Waiting)),
            Err(TaskRefusal::WaitingWithoutAction)
        );
        assert_eq!(
            settle_change(
                &task(1),
                TaskChange {
                    status: Some(Status::Done),
                    action: Some(Some("Do it".to_owned())),
                    ..TaskChange::default()
                }
            ),
            Err(TaskRefusal::ActionNotWaiting)
        );
        let blank = settle_change(
            &task(1),
            TaskChange {
                action: Some(Some("   ".to_owned())),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(blank.action, Some(None));
    }

    #[test]
    fn a_count_that_reaches_its_total_finishes_it() {
        let mut before = task(1);
        before.total = Some(12);
        let started = settle_change(
            &before,
            TaskChange {
                progress: Some(4),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(started.status, Some(Status::Working));
        let finished = settle_change(
            &before,
            TaskChange {
                progress: Some(12),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(finished.status, Some(Status::Done));
    }

    #[test]
    fn a_findings_state_gives_its_status() {
        assert_eq!(
            settle_change(
                &task(1),
                TaskChange {
                    landed: Some(Landed::Landed),
                    ..TaskChange::default()
                }
            ),
            Err(TaskRefusal::NotAFinding(TaskId::new(1)))
        );
        let mut finding = task(2);
        finding.finding = true;
        finding.parent = Some(TaskId::new(1));
        let landed = settle_change(
            &finding,
            TaskChange {
                landed: Some(Landed::Superseded),
                ..TaskChange::default()
            },
        )
        .unwrap();
        assert_eq!(landed.status, Some(Status::Done));
        assert_eq!(
            settle_change(
                &finding,
                TaskChange {
                    parent: Some(None),
                    ..TaskChange::default()
                }
            ),
            Err(TaskRefusal::FindingLeavesScan)
        );
    }

    #[test]
    fn days_are_read_from_days_and_instants() {
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(
            date_only("2026-10-24", &utc).unwrap().to_string(),
            "2026-10-24"
        );
        assert_eq!(
            date_only("2026-10-24T23:30:00Z", &utc).unwrap().to_string(),
            "2026-10-24"
        );
        assert!(date_only("soon", &utc).is_err());
        assert_eq!(
            day(Some("2026-10-15T09:00")).unwrap().unwrap().to_string(),
            "2026-10-15"
        );
        assert_eq!(day(Some(" ")).unwrap(), None);
        assert!(day(Some("next week")).is_err());
        assert_eq!(TaskId::new(12).key(), format!("{KEY_PREFIX}12"));
    }
}
