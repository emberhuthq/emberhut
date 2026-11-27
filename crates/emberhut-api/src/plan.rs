//! The person's week, in the state: each goal's list this week (`todos`), what each goal's
//! steps this week were (a goal's `week`), and the weekly report (`report`).
//!
//! A goal's ring is its list this week: the done ones of all of them. The list is every top-level
//! task that serves the goal (its own goal, or its project's), open ones first, then this week's
//! done ones; a parked task is off it until its day.
//!
//! Also here: the open loops (`loops`) and what the assistant keeps an eye on (`focus`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{GoalId, ProjectId, RoomId, TaskId};
use crate::tasks::ProjectHealth;
use crate::time::Timestamp;

/// A goal's steps done this week, counted by who did them: each key an
/// [`Issue::owner`](crate::tasks::Issue::owner) value (the person's, the assistant's). A key
/// whose value isn't a count is skipped, as unknown fields are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GoalWeek(pub BTreeMap<String, u32>);

impl GoalWeek {
    /// The steps `owner` did this week.
    #[must_use]
    pub fn by(&self, owner: &str) -> u32 {
        self.0.get(owner).copied().unwrap_or(0)
    }
}

impl<'de> Deserialize<'de> for GoalWeek {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// A value under a key: a count, or something else (skipped).
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Count(u32),
            Other(serde::de::IgnoredAny),
        }
        let all = BTreeMap::<String, Value>::deserialize(deserializer)?;
        Ok(Self(
            all.into_iter()
                .filter_map(|(who, value)| match value {
                    Value::Count(count) => Some((who, count)),
                    Value::Other(_) => None,
                })
                .collect(),
        ))
    }
}

/// A task on a goal's list this week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Todo {
    /// The task.
    pub id: TaskId,
    /// The goal it serves; none for a task that serves none.
    pub goal: Option<GoalId>,
    /// Its title.
    pub text: String,
    /// Who does it (an [`Issue::owner`](crate::tasks::Issue::owner) value).
    pub who: String,
    /// One of today's.
    pub today: bool,
    /// When it was done this week; none while it is open.
    pub done: Option<Timestamp>,
    /// Parked until then.
    #[serde(default)]
    pub parked_until: Option<Timestamp>,
    /// When it last changed.
    pub at: Timestamp,
}

impl Todo {
    /// Whether it is done.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done.is_some()
    }
}

/// The weekly report: why it all matters, how the weeks went, each goal with the projects that
/// moved it, and the week's finished tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WeekReport {
    /// The person's ultimate concern, once it is put into words.
    #[serde(default)]
    pub concern: Option<Concern>,
    /// Tasks finished in each of the last weeks, this one last.
    #[serde(default)]
    pub weeks: Vec<u32>,
    /// When each of those weeks began (older servers don't send it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week_starts: Option<Vec<Timestamp>>,
    /// When the first task was made: weeks before it weren't tracked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// Each goal, with its projects.
    #[serde(default)]
    pub goals: Vec<ReportGoal>,
    /// This week's finished top-level tasks, newest first (30 at most).
    #[serde(default)]
    pub done: Vec<ReportDone>,
}

/// The ultimate concern: what everything serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Concern {
    /// It, in a line.
    pub title: String,
    /// Why.
    pub why: String,
}

/// A goal in the weekly report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportGoal {
    /// The goal.
    pub id: GoalId,
    /// Its title.
    pub title: String,
    /// The label under its ring.
    #[serde(default)]
    pub short: Option<String>,
    /// Its tasks finished this week.
    pub done: u32,
    /// The projects that serve it.
    #[serde(default)]
    pub projects: Vec<ReportProject>,
}

/// A project under its goal in the weekly report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportProject {
    /// The project.
    pub id: ProjectId,
    /// Its name.
    pub name: String,
    /// Its tasks done this week.
    pub done_week: u32,
    /// Its open tasks.
    pub open: u32,
    /// Its health; none without a hard deadline.
    #[serde(default)]
    pub health: Option<ProjectHealth>,
}

/// A task finished this week, in the weekly report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportDone {
    /// The task.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Who did it (an [`Issue::owner`](crate::tasks::Issue::owner) value).
    pub owner: String,
    /// Its project's name.
    #[serde(default)]
    pub project: Option<String>,
}

/// An open loop the assistant keeps: someone waiting on the person, a promise, a deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Loop {
    /// Its id.
    pub id: i64,
    /// Who it is about.
    pub who: String,
    /// What is open.
    pub what: String,
    /// When it is due (a day or an instant), if it is.
    #[serde(default)]
    pub due: Option<String>,
    /// Where it came from, in words.
    #[serde(default)]
    pub source: String,
    /// When it was opened.
    pub since: Timestamp,
    /// The chat it lives in, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    /// When it was last touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched: Option<Timestamp>,
    /// The person wrote in its chat after it opened: it may be answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maybe: Option<Timestamp>,
    /// The room that opened it, when it isn't Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomId>,
}

/// Something the assistant keeps an eye on behind the scenes (a Focus item).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FocusItem {
    /// The goal it serves.
    #[serde(default)]
    pub goal: Option<GoalId>,
    /// Quiet until then.
    #[serde(default)]
    pub parked_until: Option<String>,
    /// Why it was parked.
    #[serde(default)]
    pub parked_why: Option<String>,
    /// Its id.
    pub id: i64,
    /// Its title.
    pub title: String,
    /// `needs_you`, `moving`, `personal` or `quiet`.
    pub section: String,
    /// Where it stands, in a sentence or two.
    pub line: String,
    /// Its next step.
    #[serde(default)]
    pub next: Option<String>,
    /// A few short chips.
    #[serde(default)]
    pub tags: Vec<String>,
    /// What it ties to.
    #[serde(default)]
    pub links: Vec<FocusLink>,
    /// The person keeps it: the assistant doesn't drop it on its own.
    #[serde(default)]
    pub pinned: bool,
    /// When it was added.
    pub since: Timestamp,
    /// When it last changed.
    pub at: Timestamp,
    /// What happened, newest first (30 at most).
    #[serde(default)]
    pub timeline: Vec<FocusEvent>,
}

/// What a Focus item ties to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FocusLink {
    /// `Thread`, `Watch`, `Loop`, `Ticket`, `PR`, `Person`, `Note`, ...
    pub kind: String,
    /// In plain words.
    pub label: String,
}

/// A line of a Focus item's timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FocusEvent {
    /// When.
    pub at: Timestamp,
    /// What happened.
    pub text: String,
}
