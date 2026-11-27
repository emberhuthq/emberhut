//! Areas and the overviews the assistant reads: each project's areas with their trees, the
//! `tasks` tool's outline, and the `projects` tool's list.

use std::collections::{BTreeMap, BTreeSet};

use emberhut_core::task::area::{
    self, AreaStatus, AreaTree, OverviewParts, OverviewProject, TaskFilter, count_line, outline,
    overview,
};
use emberhut_core::task::{AreaId, ProjectId, Task, TaskId};
use emberhut_db::AreaChange;
use serde_json::Value;

use super::{TaskError, TaskService, shape};

impl TaskService {
    /// An area's tree: its tasks and findings (canceled ones left out), the scans and fixes
    /// they name looked up wherever they are.
    async fn tree_of(&self, a: &area::Area) -> Result<AreaTree, TaskError> {
        let rows = self.tasks.in_area(a.id).await?;
        let inside: BTreeSet<TaskId> = rows.iter().map(|t| t.id).collect();
        let outside: Vec<TaskId> = rows
            .iter()
            .flat_map(|t| t.parent.into_iter().chain(t.fixed_by))
            .filter(|id| !inside.contains(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let others: BTreeMap<TaskId, Task> = self
            .tasks
            .many(&outside)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        Ok(area::tree(a, &rows, |id| others.get(&id)))
    }

    /// Every area of a project (every project's: none) with its tree; done ones folded.
    pub(super) async fn area_trees(
        &self,
        pid: Option<ProjectId>,
    ) -> Result<Vec<AreaTree>, TaskError> {
        let mut trees = Vec::new();
        for a in self.areas.list(pid).await? {
            trees.push(if a.status == AreaStatus::Done {
                area::bare(&a)
            } else {
                self.tree_of(&a).await?
            });
        }
        Ok(trees)
    }

    /// Makes or changes an area; answers it in a line: its counts, what's left open, its
    /// findings not landed.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a new area with no project or name, a name taken, or no such
    /// area.
    pub async fn set_area(&self, change: AreaChange) -> Result<String, TaskError> {
        let new = change.id.is_none_or(|id| id.get() == 0);
        let id = self.areas.set(change, self.now()).await?;
        let a = self
            .areas
            .get(id)
            .await?
            .ok_or_else(|| TaskError::refused(format!("no area {id}")))?;
        let t = self.tree_of(&a).await?;
        self.activity
            .note(format!(
                "Area {}: {}",
                if new { "added" } else { "updated" },
                a.name
            ))
            .await?;
        let project = self.projects.get(a.pid).await?.map_or_default(|p| p.name);
        let open = t.counts.total() - t.counts.done;
        let left = if a.status == AreaStatus::Done && open > 0 {
            format!(". It still has {open} open: move or finish them")
        } else {
            String::new()
        };
        let findings = if t.findings.open() > 0 {
            format!("; {} findings not landed", t.findings.open())
        } else {
            String::new()
        };
        Ok(format!(
            "Area {id} \"{}\" in {project} ({}): {}{left}{findings}.",
            a.name,
            a.status.as_str(),
            count_line(t.counts)
        ))
    }

    /// The area named `which` (its id, or its name in `pid`), if there is one.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn find_area(
        &self,
        which: &str,
        pid: Option<ProjectId>,
    ) -> Result<Option<AreaId>, TaskError> {
        Ok(self.areas.find(which, pid).await?.map(|a| a.id))
    }

    /// One area in full: every task and its findings not landed.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn area_outline(&self, id: AreaId) -> Result<String, TaskError> {
        let a = self
            .areas
            .get(id)
            .await?
            .ok_or_else(|| TaskError::refused(format!("no area {id}")))?;
        Ok(outline(&self.tree_of(&a).await?, true, "", true))
    }

    /// The `tasks` tool's overview, cut by `filter`.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn overview(&self, filter: &TaskFilter) -> Result<String, TaskError> {
        let (board, listed, projects, _) = self.board().await?;
        let only = match filter.area {
            Some(area) => self.areas.get(area).await?.map(|a| a.pid),
            None => filter.pid.flatten(),
        };
        let areas = self.area_trees(only).await?;
        let open_ids: Vec<TaskId> = listed.iter().map(|t| t.id).collect();
        let scans = area::scan_counts(
            &self
                .tasks
                .findings_of(&open_ids)
                .await?
                .iter()
                .collect::<Vec<_>>(),
        );
        let health: BTreeMap<ProjectId, _> =
            board.projects.iter().map(|l| (l.id, l.health)).collect();
        let parts = OverviewParts {
            projects: projects
                .iter()
                .map(|p| OverviewProject {
                    project: p,
                    health: health.get(&p.id).copied().flatten(),
                })
                .collect(),
            areas,
            open: &listed,
            scans,
            today: &board.today,
        };
        Ok(overview(&parts, filter))
    }

    /// The board's tasks with every field, subtasks nested under their parents, and Today: the
    /// `tasks` tool's full list.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn full_list(&self) -> Result<Value, TaskError> {
        fn nest(
            t: &emberhut_api::tasks::Issue,
            depth: usize,
            by_parent: &BTreeMap<Option<i64>, Vec<&emberhut_api::tasks::Issue>>,
        ) -> Value {
            let mut value = serde_json::to_value(t).unwrap_or(Value::Null);
            if let Some(kids) = by_parent.get(&Some(t.id.get()))
                && depth < emberhut_core::task::MAX_DEPTH
                && let Value::Object(object) = &mut value
            {
                object.insert(
                    "subtasks".to_owned(),
                    Value::Array(kids.iter().map(|k| nest(k, depth + 1, by_parent)).collect()),
                );
            }
            value
        }
        let (board, listed, _, family) = self.board().await?;
        let shaped = self.shape_with(&listed, Some(&board), &family).await?;
        let mut by_parent: BTreeMap<Option<i64>, Vec<&emberhut_api::tasks::Issue>> =
            BTreeMap::new();
        for t in &shaped {
            by_parent
                .entry(t.parent.map(emberhut_api::ids::TaskId::get))
                .or_default()
                .push(t);
        }
        let tasks: Vec<Value> = by_parent
            .get(&None)
            .map_or_default(|top| top.iter().map(|t| nest(t, 1, &by_parent)).collect());
        let today: Vec<i64> = board.today.iter().map(|id| id.get()).collect();
        Ok(serde_json::json!({ "tasks": tasks, "today": today }))
    }

    /// The projects on the board, each with its areas in a line apiece: the `projects` tool.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn projects_json(&self) -> Result<Value, TaskError> {
        let (board, _, projects, _) = self.board().await?;
        let areas = self.area_trees(None).await?;
        let lines: BTreeMap<ProjectId, _> = board.projects.iter().map(|l| (l.id, l)).collect();
        let list: Vec<Value> = projects
            .iter()
            .map(|p| {
                let mut value = serde_json::to_value(shape::project(p, lines.get(&p.id).copied()))
                    .unwrap_or(Value::Null);
                let mine: Vec<Value> = areas
                    .iter()
                    .filter(|a| a.area.pid == p.id)
                    .map(|a| {
                        serde_json::json!({
                            "id": a.area.id.get(),
                            "name": a.area.name,
                            "status": a.area.status.as_str(),
                            "goal": a.area.goal,
                            "about": a.area.about,
                            "state": a.state.as_str(),
                            "tasks": count_line(a.counts),
                            "needsYou": a.questions.len(),
                            "findingsOpen": a.findings.open(),
                        })
                    })
                    .collect();
                if let Value::Object(object) = &mut value {
                    object.insert("areas".to_owned(), Value::Array(mine));
                }
                value
            })
            .collect();
        Ok(Value::Array(list))
    }

    /// The person's projects' names, by id.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn project_name(&self, id: ProjectId) -> Result<Option<String>, TaskError> {
        Ok(self.projects.get(id).await?.map(|p| p.name))
    }

    /// The tasks a filter keeps, as the overview's filter takes them.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a state that isn't one.
    pub fn filter(
        pid: Option<Option<ProjectId>>,
        area: Option<AreaId>,
        states: &[&str],
        owner: Option<emberhut_core::task::Owner>,
    ) -> Result<TaskFilter, TaskError> {
        let mut set = BTreeSet::new();
        for word in states {
            set.insert(area::NodeState::parse_filter(word).ok_or_else(|| {
                TaskError::refused(format!(
                    "status is done, working, needs_you or not_started, not \"{word}\""
                ))
            })?);
        }
        Ok(TaskFilter {
            pid,
            area,
            states: set,
            owner,
        })
    }
}
