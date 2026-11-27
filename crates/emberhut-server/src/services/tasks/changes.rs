//! The person's changes from the app (ticks, parks, archive, quick add, a task's or a project's
//! fields, a posted X reply), a program's job, findings filed in bulk and subtasks kept in step.

use emberhut_api as api;
use emberhut_core::task::board::{ProjectChange, ProjectStatus};
use emberhut_core::task::finding::{FindingIn, same_title};
use emberhut_core::task::rollup::Family;
use emberhut_core::task::{
    ArtifactId, GoalId, Landed, Owner, ProjectId, Size, Status, Task, TaskChange, TaskId, Who,
    date_only, day,
};
use emberhut_db::NewTask;
use serde_json::Value;

use super::{TaskError, TaskService, shape};

/// The history line of a task the person archived.
const ARCHIVED: &str = "Archived by the owner";
/// The history line of a task the person took out of the archive.
const UNARCHIVED: &str = "Unarchived by the owner";

fn archived_with(parent: TaskId) -> String {
    format!("Archived with {}", parent.key())
}

/// One subtask to keep in step, by id or by title under the parent (made when missing).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressItem {
    /// The subtask.
    pub id: Option<TaskId>,
    /// Its title.
    pub title: Option<String>,
    /// Its status.
    pub status: Option<Status>,
    /// Its count so far.
    pub progress: Option<i64>,
    /// What its count goes to.
    pub total: Option<i64>,
    /// What it counts.
    pub unit: Option<String>,
}

/// Findings to file under a scan.
pub type FindingsItem = FindingIn;

impl TaskService {
    /// The person's tick on a step: done (the assistant hears it, to line up what's next), or
    /// back to to do.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn todo_done(&self, id: TaskId, done: bool) -> Result<(), TaskError> {
        let status = if done { Status::Done } else { Status::Todo };
        let task = self
            .update(id, TaskChange::status(status), Who::Person, None)
            .await?;
        if task.status == Status::Done {
            self.tell_about(
                &task,
                "plan",
                format!(
                    "The person did {}: {}. Line up what's next for that goal if it helps.",
                    task.key(),
                    task.title
                ),
                "was marked done by the owner",
                None,
            )
            .await?;
        }
        Ok(())
    }

    /// The person parks a step until `until` (none: back now).
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn todo_park(&self, id: TaskId, until: Option<String>) -> Result<(), TaskError> {
        let task = self
            .update(
                id,
                TaskChange {
                    parked_until: Some(until.clone()),
                    ..TaskChange::default()
                },
                Who::Person,
                None,
            )
            .await?;
        let when = until
            .as_deref()
            .filter(|u| !u.is_empty())
            .map_or_default(|u| format!(" until {}", u.chars().take(10).collect::<String>()));
        self.activity
            .note(format!("The person parked {}{when}", task.title))
            .await?;
        Ok(())
    }

    /// The person's Archive: put away for good (canceled, shown as Archived), with its open
    /// subtasks at any depth; their workers stop, and the assistant hears it once.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a done task or no such task.
    pub async fn archive(&self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task(id).await?;
        if task.status == Status::Canceled {
            return Ok(());
        }
        if task.status == Status::Done {
            return Err(TaskError::refused(format!(
                "{} is done already",
                task.key()
            )));
        }
        let family = self.tasks.family().await?;
        let kids: Vec<TaskId> = family
            .descendants(id)
            .into_iter()
            .filter(|kid| {
                family
                    .get(*kid)
                    .is_some_and(|row| !row.finding && row.status.is_open())
            })
            .collect();
        let away = TaskChange {
            status: Some(Status::Canceled),
            today: Some(false),
            ..TaskChange::default()
        };
        self.update(id, away.clone(), Who::Person, Some(ARCHIVED))
            .await?;
        let with = archived_with(id);
        for kid in &kids {
            self.update(*kid, away.clone(), Who::Person, Some(&with))
                .await?;
        }
        let mut stopped = Vec::new();
        for at in std::iter::once(id).chain(kids.iter().copied()) {
            if self
                .tasks
                .get(at)
                .await?
                .is_some_and(|t| t.worker.is_some())
            {
                self.stop_worker(at).await?;
                stopped.push(format!("{}'s worker", at.key()));
            }
        }
        let subs = subtasks_line(&kids, "and its open subtask");
        let stopped = if stopped.is_empty() {
            String::new()
        } else {
            format!("stopped already: {}. ", stopped.join(", "))
        };
        self.tell_about(
            &task,
            "owner",
            format!(
                "The person archived {} \"{}\"{subs} from the app: put away for good, not done. Don't raise it again or count it; {stopped}Stop any thread, watch or check-in still tied to it. If they unarchive it, you'll hear.",
                task.key(),
                task.title
            ),
            "was archived by the owner: put away for good, not done; don't carry it on",
            Some(format!("archived:{}:{}", id.get(), self.iso())),
        )
        .await?;
        self.activity
            .note(format!(
                "The person archived {}: {}{subs}",
                task.key(),
                task.title
            ))
            .await?;
        Ok(())
    }

    /// Takes a task out of the archive, back to to do, with the subtasks the archive took with
    /// it (not ones closed before it).
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for no such task.
    pub async fn unarchive(&self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task(id).await?;
        if task.status != Status::Canceled {
            return Ok(());
        }
        self.update(
            id,
            TaskChange::status(Status::Todo),
            Who::Person,
            Some(UNARCHIVED),
        )
        .await?;
        let with = archived_with(id);
        let family = self.tasks.family().await?;
        let mut back = Vec::new();
        for kid in family.descendants(id) {
            let Some(row) = self.tasks.get(kid).await? else {
                continue;
            };
            if row.status == Status::Canceled && row.last.as_deref() == Some(with.as_str()) {
                self.update(
                    kid,
                    TaskChange::status(Status::Todo),
                    Who::Person,
                    Some(UNARCHIVED),
                )
                .await?;
                back.push(kid);
            }
        }
        let subs = subtasks_line(&back, "with");
        self.tell_about(
            &task,
            "owner",
            format!(
                "The person unarchived {} \"{}\"{subs}: it's back in to do. Pick it up again as it was (a brief: its worker; else what it needs next).",
                task.key(),
                task.title
            ),
            "is back in to do: the owner took it out of the archive",
            Some(format!("unarchived:{}:{}", id.get(), self.iso())),
        )
        .await?;
        self.activity
            .note(format!(
                "The person unarchived {}: {}{subs}",
                task.key(),
                task.title
            ))
            .await?;
        Ok(())
    }

    /// The person's quick add: a task of their own, with its goal and project.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn quick_add(
        &self,
        title: &str,
        goal: Option<i64>,
        project: Option<i64>,
    ) -> Result<(), TaskError> {
        self.create(
            NewTask {
                title: title.trim().to_owned(),
                owner: Some(Owner::Person),
                goal: goal.map(GoalId::new),
                pid: project.map(ProjectId::new),
                ..NewTask::default()
            },
            Who::Person,
        )
        .await?;
        Ok(())
    }

    /// The person sets a task's deadline, size, project, decide-by day, description or ultra
    /// mode from the app (`taskSet`); the assistant hears what it works from.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a bad day or a change the rules refuse.
    pub async fn task_set(&self, set: &api::settings::TaskChange) -> Result<(), TaskError> {
        let id = TaskId::new(set.id.get());
        let mut change = TaskChange::default();
        if let Some(deadline) = patched(&set.deadline) {
            change.due =
                Some(day(deadline.map(api::time::Day::as_str)).map_err(TaskError::Refused)?);
        }
        if let Some(hard) = set.hard {
            change.hard = Some(hard);
        }
        if let Some(size) = patched(&set.size) {
            change.size = Some(size.and_then(|s| Size::parse(s)));
        }
        if let Some(project) = patched(&set.project) {
            change.pid = Some(project.map(|p| ProjectId::new(p.get())));
        }
        let decide_by = patched(&set.decide_by).map(|d| d.map(api::time::Day::as_str));
        if let Some(decide_by) = decide_by {
            change.decide_by = Some(match decide_by.filter(|d| !d.is_empty()) {
                Some(text) => Some(date_only(text, &self.zone).map_err(TaskError::Refused)?),
                None => None,
            });
        }
        let body = set.body.as_deref().map(str::trim);
        if let Some(body) = body {
            change.body = Some((!body.is_empty()).then(|| body.to_owned()));
        }
        if let Some(ultra) = set.ultra {
            change.ultra = Some(ultra);
        }
        let task = self.update(id, change, Who::Person, None).await?;
        if let Some(decide_by) = decide_by {
            let line = match decide_by.filter(|d| !d.is_empty()) {
                Some(_) => format!(
                    "Decide by {}",
                    task.decide_by
                        .map_or_else(|| "null".to_owned(), |d| d.to_string())
                ),
                None => "No decide-by date".to_owned(),
            };
            self.note(id, Who::Person, &line, false).await?;
        }
        if let Some(ultra) = set.ultra {
            self.ultra_set(&task, ultra).await?;
        }
        if let Some(body) = body {
            self.note(id, Who::Person, "Changed the description", false)
                .await?;
            let said = if body.is_empty() {
                format!(
                    "The person cleared the description of {} \"{}\"",
                    task.key(),
                    task.title
                )
            } else {
                format!(
                    "The person wrote the description of {} \"{}\": {}",
                    task.key(),
                    task.title,
                    body.chars().take(1500).collect::<String>()
                )
            };
            self.tell_about(
                &task,
                "plan",
                said,
                "has a new description from the owner",
                None,
            )
            .await?;
        }
        Ok(())
    }

    /// The person turned ultra mode on or off for `task`: its history says so, and whoever it
    /// is for hears it.
    async fn ultra_set(&self, task: &Task, ultra: bool) -> Result<(), TaskError> {
        let line = if ultra {
            "Turned ultra mode on"
        } else {
            "Turned ultra mode off"
        };
        self.note(task.id, Who::Person, line, false).await?;
        self.tell_about(
            task,
            "plan",
            format!(
                "The person turned ultra mode {} for {} \"{}\"{}.",
                if ultra { "on" } else { "off" },
                task.key(),
                task.title,
                if ultra {
                    ": its next run splits the work across sub-agents"
                } else {
                    ""
                }
            ),
            "had ultra mode changed by the owner",
            None,
        )
        .await?;
        Ok(())
    }

    /// Makes or changes a project; the assistant hears the person's changes (`projectSet`).
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a new project with no name or no such project.
    pub async fn set_project(
        &self,
        id: Option<ProjectId>,
        change: ProjectChange,
    ) -> Result<ProjectId, TaskError> {
        Ok(self.projects.set(id, change, self.now()).await?)
    }

    /// The person sets a project's priority, deadline, status or pause from the app.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a bad day or no such project.
    pub async fn project_set(&self, set: &api::settings::ProjectChange) -> Result<(), TaskError> {
        let id = ProjectId::new(set.id.get());
        let days = |text: Option<Option<&api::time::Day>>| -> Result<Option<Option<jiff::civil::Date>>, TaskError> {
            text.map(|d| day(d.map(api::time::Day::as_str)).map_err(TaskError::Refused))
                .transpose()
        };
        let change = ProjectChange {
            priority: set.priority.map(i32::from),
            deadline: days(patched(&set.deadline))?,
            hard: set.hard,
            status: set
                .status
                .as_ref()
                .and_then(|s| ProjectStatus::parse(s.as_str())),
            paused_until: days(set.paused_until.as_ref().map(Some))?,
            ..ProjectChange::default()
        };
        self.set_project(Some(id), change).await?;
        let p = self
            .projects
            .get(id)
            .await?
            .ok_or_else(|| TaskError::refused(format!("no project {id}")))?;
        let priority = ["none", "urgent", "high", "medium", "low"]
            .get(usize::try_from(p.priority).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or("undefined");
        let deadline = p.deadline.map_or_else(
            || ", no deadline".to_owned(),
            |d| format!(", deadline {d} ({})", if p.hard { "hard" } else { "soft" }),
        );
        let until = p.paused_until.map_or_default(|d| format!(" until {d}"));
        self.tell(
            "plan",
            format!(
                "The person set project \"{}\": priority {priority}{deadline}, {}{until}.",
                p.name,
                p.status.as_str()
            ),
            None,
        )
        .await
    }

    /// The person posted reply `option` of an X post's suggestions: kept on it, written to its
    /// task's history, and the task done.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn x_posted(&self, artifact: ArtifactId, option: usize) -> Result<(), TaskError> {
        let Some(found) = self.artifacts.get(artifact).await? else {
            return Ok(());
        };
        if found.kind != emberhut_db::X_REPLY {
            return Ok(());
        }
        let Some(mut x) = found
            .data
            .as_deref()
            .and_then(|d| serde_json::from_str::<api::x_reply::XReply>(d).ok())
        else {
            return Ok(());
        };
        let Some(reply) = x.replies.get(option).cloned() else {
            return Ok(());
        };
        x.posted = Some(api::x_reply::XPosted {
            option: u32::try_from(option).unwrap_or(u32::MAX),
            text: reply.text.clone(),
            at: crate::convert::timestamp_to_api(self.now()),
        });
        let data =
            serde_json::to_string(&x).map_err(|error| TaskError::refused(error.to_string()))?;
        self.artifacts.set_data(artifact, &data).await?;
        self.note(
            found.task,
            Who::Person,
            &format!(
                "Posted reply {} to @{} on X: \"{}\" ({})",
                option + 1,
                x.author.as_deref().unwrap_or("?"),
                reply.text,
                x.post_url
            ),
            false,
        )
        .await?;
        if self.task(found.task).await?.status != Status::Done {
            self.update(
                found.task,
                TaskChange::status(Status::Done),
                Who::Person,
                None,
            )
            .await?;
        }
        Ok(())
    }

    /// Keeps X posts' reply suggestions on a task (any of the finder's shapes: one post, a
    /// list, or `{ picks: [...] }`); sent again, a post's are replaced and what was posted
    /// stays. Returns how many posts were kept.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn attach_replies(&self, id: TaskId, values: &[Value]) -> Result<usize, TaskError> {
        let mut kept = 0;
        for value in values {
            let Some(mut x) = x_reply(value) else {
                continue;
            };
            if let Some(was) = self.artifacts.find(id, &x.post_url).await?
                && let Some(old) = was
                    .data
                    .as_deref()
                    .and_then(|d| serde_json::from_str::<api::x_reply::XReply>(d).ok())
            {
                x.posted = old.posted;
            }
            let data =
                serde_json::to_string(&x).map_err(|error| TaskError::refused(error.to_string()))?;
            self.artifacts
                .put_reply(id, &x.post_url, &data, self.now())
                .await?;
            kept += 1;
        }
        Ok(kept)
    }

    /// A program on the server's machine hands the assistant a job the person asked it for:
    /// a task of the assistant's with its brief, marked as the person's request (their ask
    /// there is their OK).
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] without a title and a body.
    pub async fn from_program(
        &self,
        request: &api::tasks::NewTaskRequest,
    ) -> Result<api::tasks::NewTaskReply, TaskError> {
        let cut = |text: Option<&str>, n: usize| {
            text.unwrap_or_default()
                .trim()
                .chars()
                .take(n)
                .collect::<String>()
        };
        let title = cut(request.title.as_deref(), 200);
        let body = cut(request.body.as_deref(), 20_000);
        let from = cut(request.from.as_deref().or(Some("a program")), 80);
        if title.is_empty() || body.is_empty() {
            return Err(TaskError::refused(
                "a task needs a title and a body (the brief)",
            ));
        }
        let made = self
            .tasks
            .create(
                NewTask {
                    title,
                    body: Some(body),
                    owner: Some(Owner::Assistant),
                    requested: Some(self.requested()),
                    ..NewTask::default()
                },
                Who::Person,
                self.now(),
            )
            .await?;
        self.note(
            made.id,
            Who::Assistant,
            &format!(
                "Requested by the person in {from}: their ask there is their OK (not payments, passwords, codes or account security; a message or post as them needs its text OK'd once)"
            ),
            false,
        )
        .await?;
        self.activity
            .note(format!("Task {}: {} (from {from})", made.key(), made.title))
            .await?;
        Ok(api::tasks::NewTaskReply {
            ok: true,
            id: shape::task_to_api(made.id),
            key: made.key(),
        })
    }

    /// Files a scan's or a review's findings under it: new ones made, known ones (same id, or
    /// same title there) updated; a duplicate is put away. Returns what changed, in words.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for a scan that is a finding, an id that isn't one of its
    /// findings, or a new finding without a title.
    #[expect(clippy::too_many_lines, reason = "each finding's three cases, in turn")]
    pub async fn file_findings(
        &self,
        scan: TaskId,
        items: Vec<FindingIn>,
        by: Who,
    ) -> Result<Vec<String>, TaskError> {
        let task = self.task(scan).await?;
        if task.finding {
            return Err(TaskError::refused(format!(
                "{} is a finding: file findings under their scan task",
                task.key()
            )));
        }
        let mut known = self.tasks.findings(scan).await?;
        let mut said = Vec::new();
        for f in items {
            let title = f.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
            let k = match f.id {
                Some(id) => known.iter().find(|k| k.id == id),
                None => known
                    .iter()
                    .find(|k| title.is_some_and(|title| same_title(&k.title, title))),
            }
            .cloned();
            let Some(k) = k else {
                if let Some(id) = f.id {
                    return Err(TaskError::refused(format!(
                        "{} isn't a finding of {}",
                        id.key(),
                        scan.key()
                    )));
                }
                let title =
                    title.ok_or_else(|| TaskError::refused("a new finding needs a title"))?;
                let made = self
                    .tasks
                    .create(
                        NewTask {
                            title: title.to_owned(),
                            body: f.body.clone(),
                            parent: Some(scan),
                            finding: true,
                            landed: Some(f.landed.unwrap_or(Landed::Unchecked)),
                            fixed_by: f.fixed_by,
                            ..NewTask::default()
                        },
                        by,
                        self.now(),
                    )
                    .await?;
                said.push(format!(
                    "{} filed ({})",
                    made.key(),
                    made.landed.unwrap_or(Landed::Unchecked).words()
                ));
                known.push(made);
                continue;
            };
            if let Some(of) = f.duplicate_of {
                let other = self.tasks.get(of).await?;
                if of == k.id || !other.is_some_and(|o| o.finding) {
                    return Err(TaskError::refused(format!(
                        "duplicate_of: {} isn't another finding",
                        of.key()
                    )));
                }
                if k.status == Status::Canceled {
                    continue;
                }
                let body = f
                    .body
                    .clone()
                    .unwrap_or_else(|| format!("duplicate of {}", of.key()));
                self.update(
                    k.id,
                    TaskChange {
                        landed: Some(Landed::Superseded),
                        body: Some(Some(body)),
                        status: Some(Status::Canceled),
                        ..TaskChange::default()
                    },
                    by,
                    Some(&format!("Put away: a duplicate of {}", of.key())),
                )
                .await?;
                said.push(format!(
                    "{} put away (a duplicate of {})",
                    k.key(),
                    of.key()
                ));
                continue;
            }
            let mut change = TaskChange::default();
            if let Some(landed) = f.landed.filter(|l| Some(*l) != k.landed) {
                change.landed = Some(landed);
            }
            if f.fixed_by_given && f.fixed_by != k.fixed_by {
                change.fixed_by = Some(f.fixed_by);
            }
            if let Some(body) = f.body.as_deref().filter(|b| !b.trim().is_empty())
                && body.trim() != k.body.as_deref().unwrap_or_default().trim()
            {
                change.body = Some(Some(body.to_owned()));
            }
            if let (Some(title), Some(_)) = (title, f.id)
                && title != k.title
            {
                change.title = Some(title.to_owned());
            }
            if change.is_empty() {
                continue;
            }
            let landed = change.landed.is_some();
            let updated = self.update(k.id, change, by, None).await?;
            said.push(format!(
                "{} {}",
                updated.key(),
                if landed {
                    updated.landed.unwrap_or(Landed::Unchecked).words()
                } else {
                    "updated"
                }
            ));
        }
        Ok(said)
    }

    /// Keeps many subtasks of one parent in step at once: each found by id or by title (made
    /// when missing), changed only where it differs. Returns the keys changed, the parent's
    /// rollup and its status.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for an item with no id or title that matches.
    pub async fn progress(
        &self,
        parent: TaskId,
        items: Vec<ProgressItem>,
    ) -> Result<Value, TaskError> {
        let mut kids = self.tasks.subtasks(parent).await?;
        let mut changed = Vec::new();
        for item in items {
            let title = item.title.as_deref().map(str::trim);
            let found = match item.id {
                Some(id) => kids.iter().find(|k| k.id == id),
                None => kids.iter().find(|k| {
                    title.is_some_and(|t| k.title.trim().to_lowercase() == t.to_lowercase())
                }),
            }
            .cloned();
            let k = match (found, title.filter(|t| !t.is_empty())) {
                (Some(k), _) => k,
                (None, Some(title)) => {
                    let made = self
                        .tasks
                        .create(
                            NewTask {
                                title: title.to_owned(),
                                parent: Some(parent),
                                ..NewTask::default()
                            },
                            Who::Assistant,
                            self.now(),
                        )
                        .await?;
                    kids.push(made.clone());
                    made
                }
                (None, None) => {
                    return Err(TaskError::refused(format!(
                        "no subtask {}under {}",
                        item.id.map_or_default(|id| format!("{} ", id.key())),
                        parent.key()
                    )));
                }
            };
            let change = TaskChange {
                status: item.status.filter(|s| *s != k.status),
                progress: item.progress.filter(|p| Some(*p) != k.progress),
                total: item.total.filter(|t| Some(*t) != k.total),
                unit: item.unit.filter(|u| Some(u) != k.unit.as_ref()),
                ..TaskChange::default()
            };
            if !change.is_empty() {
                self.update(k.id, change, Who::Assistant, None).await?;
                changed.push(k.key());
            }
        }
        let family: Family = self.tasks.family().await?;
        let rollup = family.rollup(parent).map(shape::rollup);
        let mut answer = serde_json::json!({ "changed": changed, "rollup": rollup });
        if let Some(p) = self.tasks.get(parent).await? {
            answer["parentStatus"] = Value::from(p.status.as_str());
        }
        Ok(answer)
    }
}

fn subtasks_line(kids: &[TaskId], lead: &str) -> String {
    if kids.is_empty() {
        return String::new();
    }
    let keys: Vec<String> = kids.iter().map(|k| k.key()).collect();
    if lead == "with" {
        format!(" with {}", keys.join(", "))
    } else {
        format!(
            " {lead}{} {}",
            if kids.len() == 1 { "" } else { "s" },
            keys.join(", ")
        )
    }
}

/// A post's suggestions read loosely, from any of the reply finder's shapes; none without a
/// post id or a reply.
fn x_reply(o: &Value) -> Option<api::x_reply::XReply> {
    let text = |v: Option<&Value>| match v? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let post = o.get("post");
    let field = |key: &str| o.get(key).or_else(|| post.and_then(|p| p.get(key)));
    let url = text(
        o.get("post_url")
            .or_else(|| o.get("url"))
            .or_else(|| post.and_then(|p| p.get("url"))),
    );
    let author = text(
        o.get("author")
            .or_else(|| o.get("handle"))
            .or_else(|| o.get("user"))
            .or_else(|| post.and_then(|p| p.get("author")))
            .or_else(|| post.and_then(|p| p.get("handle"))),
    )
    .map(|a| a.trim_start_matches('@').to_owned());
    let from_url = url.as_deref().and_then(|u| {
        let at = u
            .find("/status/")
            .map(|i| i + 8)
            .or_else(|| u.find("/statuses/").map(|i| i + 10))?;
        let digits: String = u[at..].chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty()).then_some(digits)
    });
    let id = text(
        o.get("post_id")
            .or_else(|| o.get("tweet_id"))
            .or_else(|| post.and_then(|p| p.get("id"))),
    )
    .or(from_url)?;
    if !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let list = ["reply_options", "replies", "options", "suggestions"]
        .iter()
        .find_map(|key| o.get(*key).and_then(Value::as_array))?;
    let replies: Vec<api::x_reply::XReplyOption> = list
        .iter()
        .filter_map(|r| match r {
            Value::String(s) => Some(api::x_reply::XReplyOption {
                text: s.trim().to_owned(),
                ..api::x_reply::XReplyOption::default()
            }),
            other => Some(api::x_reply::XReplyOption {
                text: text(other.get("text").or_else(|| other.get("reply")))?,
                why: text(
                    other
                        .get("why")
                        .or_else(|| other.get("reason"))
                        .or_else(|| other.get("rationale")),
                ),
                label: text(other.get("label")),
                style: text(other.get("style")),
                intent_url: text(other.get("intent_url")).filter(|u| {
                    u.starts_with("https://x.com/intent/")
                        || u.starts_with("https://twitter.com/intent/")
                }),
            }),
        })
        .filter(|r| !r.text.is_empty())
        .collect();
    if replies.is_empty() {
        return None;
    }
    Some(api::x_reply::XReply {
        post_url: url.unwrap_or_else(|| {
            format!(
                "https://x.com/{}/status/{id}",
                author.as_deref().unwrap_or("i")
            )
        }),
        post_id: id,
        author,
        author_name: text(o.get("author_name")),
        followers: o.get("followers").and_then(Value::as_u64),
        posted_at: text(o.get("posted_at")).map(api::time::Timestamp::from),
        post_text: text(
            o.get("post_text")
                .or_else(|| o.get("text"))
                .or_else(|| field("text")),
        ),
        replies,
        check: text(o.get("check")),
        metrics: None,
        room: None,
        posted: None,
    })
}

/// The posts of a reply finder's answer: a list, `{ picks }`, `{ posts }`, or one post.
#[must_use]
pub fn reply_posts(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(list) => list.clone(),
        other => other
            .get("picks")
            .or_else(|| other.get("posts"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![other.clone()]),
    }
}

/// A change request's field as the rules read it: left out is `None` (unchanged), `null` is
/// `Some(None)` (cleared), a value is `Some(Some(value))`.
#[expect(
    clippy::option_option,
    reason = "a partial change: left out is unchanged, null clears"
)]
fn patched<T>(patch: &api::patch::Patch<T>) -> Option<Option<&T>> {
    match patch {
        api::patch::Patch::Keep => None,
        api::patch::Patch::Clear => Some(None),
        api::patch::Patch::Set(value) => Some(Some(value)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_posts_suggestions_are_read_loosely() {
        let x = x_reply(&json!({
            "url": "https://x.com/someone/status/123?s=20",
            "handle": "@someone",
            "replies": ["Nice", { "text": "Agreed", "why": "short", "intent_url": "https://example.com" }]
        }))
        .unwrap();
        assert_eq!(x.post_id, "123");
        assert_eq!(x.author.as_deref(), Some("someone"));
        assert_eq!(x.replies.len(), 2);
        assert_eq!(x.replies[1].intent_url, None, "only X's own compose page");
        assert!(x_reply(&json!({ "post_id": "abc", "replies": ["x"] })).is_none());
        assert!(x_reply(&json!({ "post_id": "1", "replies": [] })).is_none());
        assert_eq!(reply_posts(&json!({ "picks": [1, 2] })).len(), 2);
        assert_eq!(reply_posts(&json!({ "post_id": "1" })).len(), 1);
    }
}
