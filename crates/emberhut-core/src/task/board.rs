//! Projects and the board: each project's health from its hard deadlines, each open task's
//! score, and Today: the tasks to do now, ranked by how close a hard deadline is against the
//! work left before it, then by the project's priority. The week starts on Sunday.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan as _};

use super::{Deadline, GoalId, Owner, ProjectId, Status, Task, TaskId};

/// Where a project is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectStatus {
    /// Worked on.
    Active,
    /// Off Today, its room folded, until it comes back.
    Paused,
    /// Its room out of the switcher; its open tasks parked.
    Archived,
    /// Finished.
    Done,
}

impl ProjectStatus {
    /// Every status.
    pub const ALL: [Self; 4] = [Self::Active, Self::Paused, Self::Archived, Self::Done];

    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Archived => "archived",
            Self::Done => "done",
        }
    }

    /// The status `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == word)
    }
}

/// How a project runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectKind {
    /// It ends with something delivered.
    Deliverable,
    /// It goes on.
    Ongoing,
}

impl ProjectKind {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deliverable => "deliverable",
            Self::Ongoing => "ongoing",
        }
    }

    /// The kind `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "deliverable" => Some(Self::Deliverable),
            "ongoing" => Some(Self::Ongoing),
            _ => None,
        }
    }
}

/// A project, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// Its id.
    pub id: ProjectId,
    /// Its name.
    pub name: String,
    /// The goal it serves.
    pub goal: Option<GoalId>,
    /// 0 none, 1 urgent ... 4 low.
    pub priority: i32,
    /// How it runs.
    pub kind: ProjectKind,
    /// When the whole project ends.
    pub deadline: Option<Date>,
    /// That deadline is someone else's.
    pub hard: bool,
    /// Where it is.
    pub status: ProjectStatus,
    /// A paused project's day to come back.
    pub paused_until: Option<Date>,
    /// When it was made.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

impl Project {
    /// Its deadline as its tasks inherit it.
    #[must_use]
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline.map(|date| Deadline {
            date,
            hard: self.hard,
        })
    }
}

/// The fields of a project to set; left out: unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectChange {
    /// Its name.
    pub name: Option<String>,
    /// Its goal.
    pub goal: Option<Option<GoalId>>,
    /// Its priority.
    pub priority: Option<i32>,
    /// Its kind.
    pub kind: Option<ProjectKind>,
    /// Its deadline.
    pub deadline: Option<Option<Date>>,
    /// Whether that deadline is hard.
    pub hard: Option<bool>,
    /// Its status.
    pub status: Option<ProjectStatus>,
    /// A pause's day to come back.
    pub paused_until: Option<Option<Date>>,
}

/// A project's health, from its hard deadlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Health {
    /// More than a day of slack before every hard deadline.
    OnTrack,
    /// Less than a day of slack before one.
    AtRisk,
    /// A hard deadline has passed.
    Late,
}

impl Health {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnTrack => "on_track",
            Self::AtRisk => "at_risk",
            Self::Late => "late",
        }
    }
}

/// A project's numbers on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectLine {
    /// The project.
    pub id: ProjectId,
    /// Its open top-level tasks.
    pub open: u32,
    /// Its top-level tasks done this week.
    pub done_week: u32,
    /// Its health; none without a hard deadline.
    pub health: Option<Health>,
    /// The nearest deadline of its open tasks.
    pub next: Option<Date>,
    /// The person's sized work left, in hours.
    pub hours_left: f64,
}

/// A task's place on the board.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scored {
    /// Its deadline as it counts (its own, else its project's); none once done.
    pub deadline: Option<Deadline>,
    /// How much it matters now (one decimal).
    pub score: f64,
}

/// The board: every project's line, every task's score, and Today.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    /// The projects, in board order.
    pub projects: Vec<ProjectLine>,
    /// Each listed task's deadline and score.
    pub scores: BTreeMap<TaskId, Scored>,
    /// Today's tasks, ranked.
    pub today: Vec<TaskId>,
}

/// The hours of the person's focus in a day, by default.
pub const FOCUS_HOURS: f64 = 4.0;

/// A project's priority's weight: none, urgent, high, medium, low.
const WEIGHT: [f64; 5] = [0.0, 8.0, 5.0, 3.0, 1.0];

/// Projects in board order: active ones first, then by priority (none last), deadline (none
/// last) and name.
pub fn sort_projects(projects: &mut [Project]) {
    projects.sort_by(|a, b| {
        let rank = |p: &Project| if p.priority == 0 { 9 } else { p.priority };
        (b.status == ProjectStatus::Active)
            .cmp(&(a.status == ProjectStatus::Active))
            .then_with(|| rank(a).cmp(&rank(b)))
            .then_with(|| match (a.deadline, b.deadline) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Days from `now` to the end of `date` (23:59:59 on the server's clock); negative once past.
#[must_use]
pub fn days_to(date: Date, now: Timestamp, zone: &TimeZone) -> f64 {
    let end = date
        .to_datetime(Time::constant(23, 59, 59, 0))
        .to_zoned_datetime(zone.clone())
        .map_or(now, |zoned| zoned.timestamp());
    end.duration_since(now).as_secs_f64() / 86_400.0
}

/// Where this week starts: the last Sunday, 00:00 on the server's clock.
#[must_use]
pub fn week_start(now: Timestamp, zone: &TimeZone) -> Timestamp {
    let today = now.to_zoned_datetime(zone.clone()).date();
    let back = i64::from(today.weekday().to_sunday_zero_offset());
    let sunday = today.checked_sub(back.days()).unwrap_or(today);
    sunday
        .to_zoned_datetime(zone.clone())
        .map_or(now, |zoned| zoned.timestamp())
}

/// The day `days` from now on the server's clock.
#[must_use]
pub fn day_in(now: Timestamp, zone: &TimeZone, days: i64) -> Date {
    (now + SignedDuration::from_hours(24 * days))
        .to_zoned_datetime(zone.clone())
        .date()
}

fn round1(score: f64) -> f64 {
    (score * 10.0 + 0.5).floor() / 10.0
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The board of `projects` (in board order) and `tasks` (the listed ones: open, this week's
/// done ones, and open parents' subtasks; canceled ones are left out). `waits_below` are the
/// tasks with a subtask waiting on the person: they go first on Today, as waiting ones do.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one pass of the board's rules, in the order they apply"
)]
pub fn board(
    projects: &[Project],
    tasks: &[Task],
    waits_below: &BTreeSet<TaskId>,
    now: Timestamp,
    zone: &TimeZone,
    focus_hours: f64,
) -> Board {
    let by_id: BTreeMap<ProjectId, &Project> = projects.iter().map(|p| (p.id, p)).collect();
    let project_of = |t: &Task| t.pid.and_then(|pid| by_id.get(&pid).copied());
    let due = |t: &Task| t.own_deadline().or_else(|| project_of(t)?.deadline());
    let list: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.status != Status::Canceled)
        .collect();
    let open: Vec<&Task> = list
        .iter()
        .copied()
        .filter(|t| t.status != Status::Done && t.parent.is_none())
        .collect();
    let mut slack: BTreeMap<TaskId, f64> = BTreeMap::new();
    let mut health: BTreeMap<ProjectId, Option<Health>> = BTreeMap::new();
    let groups = projects.iter().map(|p| Some(p.id)).chain([None]);
    for pid in groups {
        let mut mine: Vec<(&Task, Deadline)> = open
            .iter()
            .filter(|t| t.pid == pid)
            .filter_map(|t| due(t).map(|d| (*t, d)))
            .collect();
        mine.sort_by_key(|(_, d)| d.date);
        let mut work = 0.0;
        let mut worst = f64::INFINITY;
        let mut late = false;
        for (t, d) in mine {
            if t.owner == Owner::Person || t.status == Status::Waiting {
                work += t.hours();
            }
            let days = days_to(d.date, now, zone);
            let s = days - work / focus_hours;
            slack.insert(t.id, s);
            if d.hard {
                worst = worst.min(s);
                late = late || days < 0.0;
            }
        }
        if let Some(pid) = pid {
            health.insert(
                pid,
                if late {
                    Some(Health::Late)
                } else if worst.is_infinite() {
                    None
                } else if worst < 1.0 {
                    Some(Health::AtRisk)
                } else {
                    Some(Health::OnTrack)
                },
            );
        }
    }
    let score = |t: &Task| {
        let p = project_of(t);
        let d = due(t);
        let pull = slack
            .get(&t.id)
            .map_or(0.0, |s| ((10.0 - s) / 10.0).clamp(0.0, 1.5));
        let deadline = match d {
            None => 0.0,
            Some(d) if d.hard => 12.0 * pull,
            Some(d) if days_to(d.date, now, zone) < -3.0 => 0.0,
            Some(_) => 4.0 * pull.min(1.0),
        };
        let weight = p
            .filter(|p| p.status == ProjectStatus::Active)
            .and_then(|p| usize::try_from(p.priority).ok())
            .and_then(|priority| WEIGHT.get(priority).copied())
            .unwrap_or(0.0);
        let state = match t.status {
            Status::Waiting => 4.0,
            Status::Working => 2.0,
            _ => 0.0,
        };
        weight + deadline + state
    };
    let scores: BTreeMap<TaskId, Scored> = list
        .iter()
        .map(|t| {
            let done = t.status == Status::Done;
            (
                t.id,
                Scored {
                    deadline: if done { None } else { due(t) },
                    score: if done { 0.0 } else { round1(score(t)) },
                },
            )
        })
        .collect();
    let score_of = |t: &Task| scores.get(&t.id).map_or(0.0, |s| s.score);
    let mut ranked: Vec<&Task> = open.clone();
    ranked.sort_by(|a, b| score_of(b).total_cmp(&score_of(a)));
    let waits = |t: &Task| t.status == Status::Waiting || waits_below.contains(&t.id);
    let mut today: Vec<&Task> = ranked.iter().copied().filter(|t| waits(t)).collect();
    today.extend(ranked.iter().copied().filter(|t| t.today && !waits(t)));
    let mut used: f64 = today.iter().map(|t| t.hours()).sum();
    for t in ranked.iter().copied().filter(|t| {
        t.owner == Owner::Person && t.status == Status::Todo && !t.today && score_of(t) > 0.0
    }) {
        if used >= focus_hours {
            break;
        }
        today.push(t);
        used += t.hours();
    }
    let lines = projects
        .iter()
        .map(|p| {
            let mine: Vec<&Task> = list
                .iter()
                .copied()
                .filter(|t| t.pid == Some(p.id) && t.parent.is_none())
                .collect();
            let left: Vec<&Task> = mine
                .iter()
                .copied()
                .filter(|t| t.status != Status::Done)
                .collect();
            ProjectLine {
                id: p.id,
                open: count(left.len()),
                done_week: count(mine.len() - left.len()),
                health: health.get(&p.id).copied().flatten(),
                next: left
                    .iter()
                    .filter_map(|t| scores.get(&t.id)?.deadline.map(|d| d.date))
                    .min(),
                hours_left: left
                    .iter()
                    .filter(|t| t.owner == Owner::Person)
                    .map(|t| t.hours())
                    .sum(),
            }
        })
        .collect();
    Board {
        projects: lines,
        scores,
        today: today.into_iter().map(|t| t.id).collect(),
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;
    use crate::task::tests::task;
    use crate::task::{ProjectId, Size};

    fn project(id: i64, priority: i32, deadline: Option<Date>, hard: bool) -> Project {
        Project {
            id: ProjectId::new(id),
            name: format!("Project {id}"),
            goal: None,
            priority,
            kind: ProjectKind::Deliverable,
            deadline,
            hard,
            status: ProjectStatus::Active,
            paused_until: None,
            created: Timestamp::UNIX_EPOCH,
            updated: Timestamp::UNIX_EPOCH,
        }
    }

    fn at(day: Date) -> Timestamp {
        day.to_datetime(Time::constant(9, 0, 0, 0))
            .to_zoned_datetime(TimeZone::UTC)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn a_hard_deadline_close_ahead_puts_a_project_at_risk_and_its_task_on_today() {
        let now = at(date(2026, 11, 25));
        let projects = [
            project(1, 1, Some(date(2026, 11, 26)), true),
            project(2, 4, None, false),
        ];
        let mut urgent = task(1);
        urgent.owner = Owner::Person;
        urgent.pid = Some(ProjectId::new(1));
        urgent.size = Some(Size::L);
        let mut later = task(2);
        later.owner = Owner::Person;
        later.pid = Some(ProjectId::new(2));
        let mut waiting = task(3);
        waiting.status = Status::Waiting;
        waiting.action = Some("Pick one".to_owned());
        let mut done = task(4);
        done.status = Status::Done;
        done.pid = Some(ProjectId::new(1));
        let board = board(
            &projects,
            &[urgent, later, waiting, done],
            &BTreeSet::new(),
            now,
            &TimeZone::UTC,
            FOCUS_HOURS,
        );
        assert_eq!(board.projects[0].health, Some(Health::AtRisk));
        assert_eq!(board.projects[0].open, 1);
        assert_eq!(board.projects[0].done_week, 1);
        assert_eq!(board.projects[0].next, Some(date(2026, 11, 26)));
        assert!((board.projects[0].hours_left - 8.0).abs() < f64::EPSILON);
        assert_eq!(board.projects[1].health, None);
        assert_eq!(
            board.today,
            [TaskId::new(3), TaskId::new(1)],
            "the waiting one first, then the best scored until the day's hours are used"
        );
        let first = board.scores[&TaskId::new(1)];
        assert!(first.deadline.is_some_and(|d| d.hard));
        assert!(first.score > board.scores[&TaskId::new(2)].score);
        assert!(board.scores[&TaskId::new(4)].deadline.is_none());
    }

    #[test]
    fn a_passed_hard_deadline_is_late() {
        let now = at(date(2026, 11, 25));
        let projects = [project(1, 2, Some(date(2026, 11, 20)), true)];
        let mut t = task(1);
        t.pid = Some(ProjectId::new(1));
        let board = board(&projects, &[t], &BTreeSet::new(), now, &TimeZone::UTC, 4.0);
        assert_eq!(board.projects[0].health, Some(Health::Late));
    }

    #[test]
    fn the_week_starts_on_sunday() {
        let wednesday = at(date(2026, 11, 25));
        assert_eq!(
            week_start(wednesday, &TimeZone::UTC).to_string(),
            "2026-11-22T00:00:00Z"
        );
        let sunday = at(date(2026, 11, 22));
        assert_eq!(
            week_start(sunday, &TimeZone::UTC).to_string(),
            "2026-11-22T00:00:00Z"
        );
    }

    #[test]
    fn projects_sort_active_first_then_by_priority() {
        let mut paused = project(1, 1, None, false);
        paused.status = ProjectStatus::Paused;
        let mut projects = vec![
            paused,
            project(2, 0, None, false),
            project(3, 2, None, false),
        ];
        sort_projects(&mut projects);
        let ids: Vec<i64> = projects.iter().map(|p| p.id.get()).collect();
        assert_eq!(ids, [3, 2, 1]);
    }
}
