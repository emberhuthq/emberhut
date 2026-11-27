//! `GET /api/tasks`: tasks from the whole list, done and archived ones too, newest first: by key
//! (a task's key, or its number alone: `212`) or the words of a title, narrowed by the server so a big list comes back
//! small.

use serde::{Deserialize, Serialize};

use crate::ids::{AreaId, ProjectId};
use crate::query::push_encoded;
use crate::tasks::{Issue, TaskStatus};

/// Which tasks to find. Every narrowing is left out by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskQuery {
    /// A key or the words of a title; empty finds every task the rest allows.
    pub text: String,
    /// At most this many; the server's own default when none.
    pub limit: Option<u32>,
    /// One project's tasks, or those of none.
    pub project: Option<ProjectScope>,
    /// Only finished ones.
    pub closed: bool,
    /// Only archived ones.
    pub archived: bool,
    /// Only tasks with one of these statuses; any when empty.
    pub statuses: Vec<TaskStatus>,
    /// Only tasks of this owner (an [`Issue::owner`] value).
    pub owner: Option<String>,
    /// Only tasks in this area of a project.
    pub area: Option<AreaId>,
}

/// The project a search keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectScope {
    /// Tasks in no project.
    NoProject,
    /// That project's tasks.
    Project(ProjectId),
}

impl TaskQuery {
    /// Tasks whose key or title matches `text`.
    #[must_use]
    pub fn search(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// The query string, without the `?`, values percent-encoded:
    /// `q=212&limit=50&status=todo,working`.
    ///
    /// ```
    /// use emberhut_api::task_search::TaskQuery;
    /// use emberhut_api::tasks::TaskStatus;
    ///
    /// let query = TaskQuery {
    ///     limit: Some(50),
    ///     statuses: vec![TaskStatus::Todo, TaskStatus::Working],
    ///     ..TaskQuery::search("the report")
    /// };
    /// assert_eq!(query.to_query(), "q=the%20report&limit=50&status=todo%2Cworking");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("q=");
        push_encoded(&mut query, &self.text);
        if let Some(limit) = self.limit {
            query.push_str("&limit=");
            query.push_str(&limit.to_string());
        }
        match self.project {
            Some(ProjectScope::NoProject) => query.push_str("&project=none"),
            Some(ProjectScope::Project(id)) => {
                query.push_str("&project=");
                query.push_str(&id.to_string());
            }
            None => {}
        }
        if self.closed {
            query.push_str("&closed=1");
        }
        if self.archived {
            query.push_str("&archived=1");
        }
        if !self.statuses.is_empty() {
            let words: Vec<&str> = self.statuses.iter().map(TaskStatus::as_str).collect();
            query.push_str("&status=");
            push_encoded(&mut query, &words.join(","));
        }
        if let Some(owner) = &self.owner {
            query.push_str("&owner=");
            push_encoded(&mut query, owner);
        }
        if let Some(area) = self.area {
            query.push_str("&area=");
            query.push_str(&area.to_string());
        }
        query
    }
}

/// `GET /api/tasks`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FoundTasks {
    /// The tasks found, newest first.
    pub tasks: Vec<Issue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_narrowing_goes_in_the_query() {
        let query = TaskQuery {
            text: "report".to_owned(),
            limit: Some(200),
            project: Some(ProjectScope::NoProject),
            closed: true,
            archived: true,
            statuses: vec![TaskStatus::Done],
            owner: Some("you".to_owned()),
            area: Some(AreaId::new(4)),
        };
        assert_eq!(
            query.to_query(),
            "q=report&limit=200&project=none&closed=1&archived=1&status=done&owner=you&area=4"
        );
        let one = TaskQuery {
            project: Some(ProjectScope::Project(ProjectId::new(3))),
            ..TaskQuery::default()
        };
        assert_eq!(one.to_query(), "q=&project=3");
    }
}
