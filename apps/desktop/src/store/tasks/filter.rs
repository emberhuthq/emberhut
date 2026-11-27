//! The filter bar over the Tasks list: by status, owner, when it has to be decided, and project
//! (only while projects show). The list in the state is filtered here; a search over every task
//! is narrowed by the server the same way. The last pick is kept on this device
//! (`filters.json` in the app's directory), so the list opens as it was left.

use std::sync::Mutex;

use emberhut_api::ids::ProjectId;
use emberhut_api::task_search::{ProjectScope, TaskQuery};
use emberhut_api::tasks::{Issue, TaskStatus};
use emberhut_api::time::Day;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::config::AppFiles;
use crate::json_file;
use crate::store::lookup;
use crate::wire::ASSISTANT_OWNER;

/// A task's status, as the filter words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StatusPick {
    /// Waiting on the user.
    NeedsYou,
    /// Being worked on.
    Working,
    /// Not started.
    NotStarted,
    /// Done.
    Done,
}

impl StatusPick {
    /// Every pick, in the bar's order.
    pub(crate) const ALL: [Self; 4] = [Self::NeedsYou, Self::Working, Self::NotStarted, Self::Done];

    /// Its chip's words.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Working => "Working",
            Self::NotStarted => "Not started",
            Self::Done => "Done",
        }
    }

    /// The pick a task's status falls under.
    #[must_use]
    pub(crate) fn of(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Waiting => Self::NeedsYou,
            TaskStatus::Todo => Self::NotStarted,
            TaskStatus::Done => Self::Done,
            _ => Self::Working,
        }
    }

    /// The server's status for it, for a search.
    #[must_use]
    fn server(self) -> TaskStatus {
        match self {
            Self::NeedsYou => TaskStatus::Waiting,
            Self::Working => TaskStatus::Working,
            Self::NotStarted => TaskStatus::Todo,
            Self::Done => TaskStatus::Done,
        }
    }
}

/// Whose task it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerPick {
    /// The user's own.
    Yours,
    /// The assistant's.
    Assistants,
}

impl OwnerPick {
    /// Both, in the bar's order.
    pub(crate) const ALL: [Self; 2] = [Self::Yours, Self::Assistants];

    /// Its chip's words.
    #[must_use]
    pub(crate) fn name(self) -> String {
        match self {
            Self::Yours => "Yours".to_owned(),
            Self::Assistants => format!("{}'s", crate::brand::APP_NAME),
        }
    }

    /// The owner id the server keeps, for a search.
    #[must_use]
    fn server(self) -> &'static str {
        match self {
            Self::Yours => lookup::PERSON_OWNER,
            Self::Assistants => ASSISTANT_OWNER,
        }
    }
}

/// When a task has to be decided (or is due), in the buckets Needs you groups by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum When {
    /// Today, or late.
    Today,
    /// Within six days.
    ThisWeek,
    /// Within a month.
    ThisMonth,
    /// More than a month out.
    Later,
}

impl When {
    /// Every bucket, soonest first.
    pub(crate) const ALL: [Self; 4] = [Self::Today, Self::ThisWeek, Self::ThisMonth, Self::Later];

    /// Its words.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::ThisWeek => "This week",
            Self::ThisMonth => "This month",
            Self::Later => "Later",
        }
    }

    /// The bucket of `day` seen from `today`; a day that doesn't read is this week's.
    #[must_use]
    pub(crate) fn of(day: &Day, today: Date) -> Self {
        let days = lookup::civil_day(day.as_str()).and_then(|day| lookup::days_from(today, day));
        match days {
            Some(..=0) => Self::Today,
            None | Some(1..=6) => Self::ThisWeek,
            Some(7..=30) => Self::ThisMonth,
            Some(_) => Self::Later,
        }
    }
}

/// The project a filter keeps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectPick {
    /// Tasks in no project.
    NoProject,
    /// That project's tasks.
    Project(ProjectId),
}

/// The pick on the Tasks list; each part left out narrows nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskFilter {
    /// By status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<StatusPick>,
    /// By owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<OwnerPick>,
    /// By when it has to be decided (its decide-by day, else its deadline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) when: Option<When>,
    /// By project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<ProjectPick>,
}

impl TaskFilter {
    /// Whether it narrows nothing.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// How many parts are on (the bar offers Clear from two).
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        [
            self.status.is_some(),
            self.owner.is_some(),
            self.when.is_some(),
            self.project.is_some(),
        ]
        .into_iter()
        .filter(|&on| on)
        .count()
    }

    /// The filter as it applies while projects are hidden or shown: hidden, a project picked
    /// before no longer narrows the list.
    #[must_use]
    pub(crate) fn shown(&self, hide_projects: bool) -> Self {
        Self {
            project: self.project.filter(|_| !hide_projects),
            ..self.clone()
        }
    }

    /// Whether it keeps `task`, seen from `today`.
    #[must_use]
    pub(crate) fn keeps(&self, task: &Issue, today: Date) -> bool {
        if let Some(project) = self.project {
            let matches = match project {
                ProjectPick::NoProject => task.pid.is_none(),
                ProjectPick::Project(id) => task.pid == Some(id),
            };
            if !matches {
                return false;
            }
        }
        if self
            .status
            .is_some_and(|status| StatusPick::of(&task.status) != status)
        {
            return false;
        }
        if let Some(owner) = self.owner
            && (owner == OwnerPick::Assistants) != lookup::is_assistants(task)
        {
            return false;
        }
        if let Some(when) = self.when {
            let day = task.decide_by.as_ref().or(task.deadline.as_ref());
            return day.is_some_and(|day| When::of(day, today) == when);
        }
        true
    }

    /// `query` narrowed by the server as this filter narrows the list (its decide-by part is
    /// the list's alone: the server doesn't take it).
    #[must_use]
    pub(crate) fn narrow(&self, query: TaskQuery) -> TaskQuery {
        TaskQuery {
            statuses: self
                .status
                .map(|status| vec![status.server()])
                .unwrap_or(query.statuses),
            owner: self
                .owner
                .map(|owner| owner.server().to_owned())
                .or(query.owner),
            project: self
                .project
                .map(|project| match project {
                    ProjectPick::NoProject => ProjectScope::NoProject,
                    ProjectPick::Project(id) => ProjectScope::Project(id),
                })
                .or(query.project),
            ..query
        }
    }
}

/// The filters kept on this device, one per screen.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    tasks: TaskFilter,
}

/// The Tasks list's last filter on this device: none when there is no file, or it doesn't read
/// (then it is logged, and the next pick writes a good one).
#[must_use]
pub(crate) fn load(files: Option<&AppFiles>) -> TaskFilter {
    files
        .and_then(|files| {
            json_file::read::<Kept>(&files.filters(), "the kept filters", "showing every task")
        })
        .map_or_else(TaskFilter::default, |kept| kept.tasks)
}

/// Keeps `filter` as the Tasks list's on this device, as write `number` of the file; a failure
/// is logged (the pick still holds for this launch). It touches the disk: call it off the UI
/// thread.
pub(crate) fn save(files: &AppFiles, filter: &TaskFilter, number: u64, written: &Mutex<u64>) {
    let kept = Kept {
        tasks: filter.clone(),
    };
    json_file::write(&files.filters(), &kept, number, written, "the Tasks filter");
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;

    const TODAY: Date = jiff::civil::date(2026, 11, 25);

    #[test]
    fn each_part_narrows_the_list() {
        let mine = issue(
            1,
            "todo",
            json!({"owner": "owner", "pid": 2, "deadline": "2026-11-27"}),
        );
        let theirs = issue(
            2,
            "waiting",
            json!({"owner": ASSISTANT_OWNER, "decideBy": "2026-11-25"}),
        );
        let filter = TaskFilter {
            owner: Some(OwnerPick::Yours),
            ..TaskFilter::default()
        };
        assert!(filter.keeps(&mine, TODAY) && !filter.keeps(&theirs, TODAY));
        let filter = TaskFilter {
            status: Some(StatusPick::NeedsYou),
            ..TaskFilter::default()
        };
        assert!(!filter.keeps(&mine, TODAY) && filter.keeps(&theirs, TODAY));
        let filter = TaskFilter {
            when: Some(When::ThisWeek),
            ..TaskFilter::default()
        };
        assert!(filter.keeps(&mine, TODAY), "its deadline, in two days");
        assert!(!filter.keeps(&theirs, TODAY), "decided today");
        let filter = TaskFilter {
            project: Some(ProjectPick::NoProject),
            ..TaskFilter::default()
        };
        assert!(!filter.keeps(&mine, TODAY) && filter.keeps(&theirs, TODAY));
        assert!(TaskFilter::default().keeps(&mine, TODAY));
    }

    #[test]
    fn a_task_with_no_day_has_no_when_to_match() {
        let filter = TaskFilter {
            when: Some(When::Later),
            ..TaskFilter::default()
        };
        assert!(!filter.keeps(&issue(1, "todo", json!({})), TODAY));
        assert_eq!(When::of(&Day::new("2027-01-30"), TODAY), When::Later);
        assert_eq!(When::of(&Day::new("2026-11-20"), TODAY), When::Today);
        assert_eq!(When::of(&Day::new("2026-12-10"), TODAY), When::ThisMonth);
    }

    #[test]
    fn a_project_pick_is_dropped_while_projects_are_hidden() {
        let filter = TaskFilter {
            project: Some(ProjectPick::Project(ProjectId::new(3))),
            owner: Some(OwnerPick::Assistants),
            ..TaskFilter::default()
        };
        assert_eq!(filter.count(), 2);
        let hidden = filter.shown(true);
        assert_eq!(hidden.project, None);
        assert_eq!(hidden.count(), 1);
        assert_eq!(filter.shown(false), filter);
    }

    #[test]
    fn a_search_is_narrowed_by_the_server_the_same_way() {
        let filter = TaskFilter {
            status: Some(StatusPick::NotStarted),
            owner: Some(OwnerPick::Assistants),
            project: Some(ProjectPick::Project(ProjectId::new(3))),
            when: Some(When::Today),
        };
        let query = filter.narrow(TaskQuery::search("report"));
        assert_eq!(
            query.to_query(),
            format!("q=report&project=3&status=todo&owner={ASSISTANT_OWNER}")
        );
    }

    #[test]
    fn the_filter_is_kept_on_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("app"));
        assert!(load(Some(&files)).is_empty(), "nothing kept yet");
        let filter = TaskFilter {
            status: Some(StatusPick::Done),
            when: Some(When::ThisMonth),
            ..TaskFilter::default()
        };
        save(&files, &filter, 1, &Mutex::new(0));
        assert_eq!(load(Some(&files)), filter);
        std::fs::write(files.filters(), "not json").unwrap();
        assert!(
            load(Some(&files)).is_empty(),
            "a file that doesn't read is no filter"
        );
        assert!(load(None).is_empty());
    }
}
