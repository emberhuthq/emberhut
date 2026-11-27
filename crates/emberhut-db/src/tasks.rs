//! Tasks, their history and their tree. A change goes through `emberhut-core`'s rules and, with all
//! that follows from it (its subtree's area and project, its parent's rollup, the asks that only
//! pointed at it), is one transaction.

use std::future::Future;
use std::pin::Pin;

use emberhut_core::RoomId;
use emberhut_core::task::rollup::{Family, Light};
use emberhut_core::task::search::TaskSearch;
use emberhut_core::task::{
    AreaId, AskId, GoalId, Landed, MAX_DEPTH, Owner, PARKED_BY_ARCHIVE, Patch, ProjectId, Size,
    Status, Task, TaskChange, TaskId, TaskRefusal, Who, settle_change,
};
use jiff::Timestamp;
use jiff::civil::Date;
use sea_orm::sea_query::{Expr, ExprTrait, Func, LikeExpr, SimpleExpr};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, IntoActiveModel, Order, QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::{area, issue, issue_event};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

mod rooms;

/// Why a task could not be made or changed: one of the rules said no, or the database failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TaskStoreError {
    /// A rule refused it; the text is for the caller.
    #[error(transparent)]
    Refused(#[from] TaskRefusal),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<DbErr> for TaskStoreError {
    fn from(error: DbErr) -> Self {
        Self::Db(DbError::Query(error))
    }
}

/// A task to make.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a column the new task starts with"
)]
pub struct NewTask {
    /// Its title.
    pub title: String,
    /// Its brief.
    pub body: Option<String>,
    /// Whose it is (none: the person's, or its parent's owner).
    pub owner: Option<Owner>,
    /// Its goal.
    pub goal: Option<GoalId>,
    /// Its project's name, as given.
    pub project: Option<String>,
    /// 0 none, 1 urgent ... 4 low (none: 3).
    pub priority: Option<i32>,
    /// Its own deadline.
    pub due: Option<Date>,
    /// Pinned to Today.
    pub today: bool,
    /// Out of sight until then.
    pub parked_until: Option<String>,
    /// Its project.
    pub pid: Option<ProjectId>,
    /// Its deadline is someone else's.
    pub hard: bool,
    /// Its rough size.
    pub size: Option<Size>,
    /// Its parent (a finding's scan).
    pub parent: Option<TaskId>,
    /// Its count so far (a subtask's).
    pub progress: Option<i64>,
    /// What its count goes to.
    pub total: Option<i64>,
    /// What it counts.
    pub unit: Option<String>,
    /// Done elsewhere.
    pub tracked: bool,
    /// Its area (a top-level task's; a subtask takes its parent's).
    pub area: Option<AreaId>,
    /// It is a finding.
    pub finding: bool,
    /// A finding's state (none: unchecked).
    pub landed: Option<Landed>,
    /// A finding's fixing task.
    pub fixed_by: Option<TaskId>,
    /// The person's own request for it, as JSON: the person's add is their request.
    pub requested: Option<String>,
    /// The room it is made in, when that isn't its project's: a configured agent's room makes
    /// the task that agent's, from the insert on. None for a subtask or a finding: its
    /// parent's.
    pub room: Option<RoomId>,
    /// Who asked for it when it was not the owner, as JSON (`who`, `at`, `how`); shown, never a
    /// grant by itself.
    pub requested_by: Option<String>,
}

/// A line of a task's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryLine {
    /// When.
    pub at: Timestamp,
    /// Who wrote it.
    pub who: Who,
    /// What happened.
    pub text: String,
}

/// A line of a task's history, with its place in every task's history and its task's title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLine {
    /// Its id: lines are numbered across every task, in the order they were written.
    pub id: i64,
    /// Its task.
    pub task: TaskId,
    /// The task's title.
    pub title: String,
    /// The line.
    pub line: HistoryLine,
}

/// Reads and writes tasks.
#[derive(Clone, Debug)]
pub struct TaskStore {
    conn: DatabaseConnection,
}

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T, TaskStoreError>> + Send + 'a>>;

impl TaskStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The task `id`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, id: TaskId) -> Result<Option<Task>, DbError> {
        find(&self.conn, id).await
    }

    /// The tasks `ids` that exist, by id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn many(&self, ids: &[TaskId]) -> Result<Vec<Task>, DbError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        rows(
            issue::Entity::find()
                .filter(issue::Column::Id.is_in(ids.iter().map(|id| id.get())))
                .order_by_asc(issue::Column::Id)
                .all(&self.conn)
                .await?,
        )
    }

    /// Every task's place in the tree.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn family(&self) -> Result<Family, DbError> {
        family(&self.conn).await
    }

    /// The tasks the app lists: open ones (those parked until later left out), those done since
    /// `since`, and the subtasks of open ones; findings never. Waiting first, then working, then
    /// to do; Today's first, then by priority, deadline and id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn listed(&self, since: Timestamp, now: Timestamp) -> Result<Vec<Task>, DbError> {
        let open = [Status::Todo, Status::Working, Status::Waiting].map(Status::as_str);
        let now_text = now.to_string();
        let open_ids: Vec<i64> = issue::Entity::find()
            .select_only()
            .column(issue::Column::Id)
            .filter(issue::Column::Status.is_in(open))
            .into_tuple()
            .all(&self.conn)
            .await?;
        let shown = issue::Column::Status.is_in(open).and(
            issue::Column::ParkedUntil
                .is_null()
                .or(issue::Column::ParkedUntil.lte(now_text)),
        );
        let done = issue::Column::Status
            .eq(Status::Done.as_str())
            .and(issue::Column::DoneAt.gte(millis(since)));
        let mut tasks = rows(
            issue::Entity::find()
                .filter(issue::Column::Kind.is_null())
                .filter(shown.or(done).or(issue::Column::Parent.is_in(open_ids)))
                .all(&self.conn)
                .await?,
        )?;
        let rank = |s: Status| match s {
            Status::Waiting => 0,
            Status::Working => 1,
            Status::Todo => 2,
            Status::Done | Status::Canceled => 3,
        };
        tasks.sort_by(|a, b| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then_with(|| b.today.cmp(&a.today))
                .then_with(|| a.priority.cmp(&b.priority))
                .then_with(|| match (a.due, b.due) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(tasks)
    }

    /// Any task, done and canceled ones too: a key that matches first, then tasks with every
    /// word in their title, then the newest (finished, else last touched). A list with no
    /// words leaves findings out; a search finds them too.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn search(&self, s: &TaskSearch) -> Result<Vec<Task>, DbError> {
        let like = |word: &str| {
            let escaped: String = word
                .chars()
                .flat_map(|c| {
                    if matches!(c, '\\' | '%' | '_') {
                        vec!['\\', c]
                    } else {
                        vec![c]
                    }
                })
                .collect();
            LikeExpr::new(format!("%{escaped}%")).escape('\\')
        };
        let mut query = issue::Entity::find();
        if !s.words.is_empty() {
            let mut all = sea_orm::Condition::all();
            for word in &s.words {
                all = all.add(
                    issue::Column::Title
                        .like(like(word))
                        .or(Expr::expr(Func::coalesce([
                            Expr::col(issue::Column::Body),
                            Expr::val(""),
                        ]))
                        .like(like(word))),
                );
            }
            let mut either = sea_orm::Condition::any().add(all);
            if let Some(key) = s.key {
                either = either.add(issue::Column::Id.eq(key.get()));
            }
            query = query.filter(either);
        }
        match s.project {
            Some(None) => query = query.filter(issue::Column::Pid.is_null()),
            Some(Some(pid)) => query = query.filter(issue::Column::Pid.eq(pid.get())),
            None => {}
        }
        if let Some(area) = s.area {
            query = query.filter(issue::Column::Area.eq(area));
        }
        if !s.statuses.is_empty() {
            query =
                query.filter(issue::Column::Status.is_in(s.statuses.iter().map(|st| st.as_str())));
        }
        if let Some(owner) = s.owner {
            query = query.filter(issue::Column::Owner.eq(owner.as_str()));
        }
        if s.is_list() {
            query = query.filter(issue::Column::Kind.is_null());
        }
        if s.closed {
            query = query.filter(
                issue::Column::Status.is_in([Status::Done.as_str(), Status::Canceled.as_str()]),
            );
        }
        if s.archived {
            query = query.filter(issue::Column::Status.eq(Status::Canceled.as_str()));
        }
        if let Some(key) = s.key {
            query = query.order_by(Expr::col(issue::Column::Id).eq(key.get()), Order::Desc);
        }
        if !s.words.is_empty() {
            let mut in_title: Option<SimpleExpr> = None;
            for word in &s.words {
                let hit = issue::Column::Title.like(like(word));
                in_title = Some(match in_title {
                    Some(all) => all.and(hit),
                    None => hit,
                });
            }
            if let Some(in_title) = in_title {
                query = query.order_by(in_title, Order::Desc);
            }
        }
        let newest = Expr::expr(Func::coalesce([
            Expr::col(issue::Column::DoneAt),
            Expr::col(issue::Column::Updated),
        ]));
        rows(
            query
                .order_by(newest, Order::Desc)
                .order_by_desc(issue::Column::Id)
                .limit(s.limit)
                .all(&self.conn)
                .await?,
        )
    }

    /// Its subtasks (not its findings), by id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn subtasks(&self, id: TaskId) -> Result<Vec<Task>, DbError> {
        under(&self.conn, id, false).await
    }

    /// The findings filed under a scan, by id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn findings(&self, id: TaskId) -> Result<Vec<Task>, DbError> {
        under(&self.conn, id, true).await
    }

    /// Every task and finding in `area`, canceled ones too.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn in_area(&self, area: AreaId) -> Result<Vec<Task>, DbError> {
        rows(
            issue::Entity::find()
                .filter(issue::Column::Area.eq(area.get()))
                .order_by_asc(issue::Column::Id)
                .all(&self.conn)
                .await?,
        )
    }

    /// The findings filed under any of `scans`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn findings_of(&self, scans: &[TaskId]) -> Result<Vec<Task>, DbError> {
        if scans.is_empty() {
            return Ok(Vec::new());
        }
        rows(
            issue::Entity::find()
                .filter(issue::Column::Kind.eq(FINDING))
                .filter(issue::Column::Parent.is_in(scans.iter().map(|id| id.get())))
                .all(&self.conn)
                .await?,
        )
    }

    /// Every open task (to do, working or waiting), findings left out.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn open(&self) -> Result<Vec<Task>, DbError> {
        rows(
            issue::Entity::find()
                .filter(issue::Column::Kind.is_null())
                .filter(
                    issue::Column::Status.is_in(
                        [Status::Todo, Status::Working, Status::Waiting].map(Status::as_str),
                    ),
                )
                .order_by_asc(issue::Column::Id)
                .all(&self.conn)
                .await?,
        )
    }

    /// Tasks (findings too) changed after `after` (every one without it), the least recently
    /// changed first, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn changed_after(
        &self,
        after: Option<Timestamp>,
        limit: u64,
    ) -> Result<Vec<Task>, DbError> {
        let mut query = issue::Entity::find();
        if let Some(after) = after {
            query = query.filter(issue::Column::Updated.gt(millis(after)));
        }
        rows(
            query
                .order_by_asc(issue::Column::Updated)
                .order_by_asc(issue::Column::Id)
                .limit(limit)
                .all(&self.conn)
                .await?,
        )
    }

    /// Every task's history lines after line `after`, oldest first, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn lines_after(&self, after: i64, limit: u64) -> Result<Vec<TaskLine>, DbError> {
        let lines = issue_event::Entity::find()
            .filter(issue_event::Column::Id.gt(after))
            .order_by_asc(issue_event::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?;
        let ids: Vec<i64> = lines.iter().map(|line| line.issue).collect();
        let titles: std::collections::HashMap<i64, String> = issue::Entity::find()
            .filter(issue::Column::Id.is_in(ids))
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| (row.id, row.title))
            .collect();
        lines
            .into_iter()
            .map(|row| {
                Ok(TaskLine {
                    id: row.id,
                    task: TaskId::new(row.issue),
                    title: titles.get(&row.issue).cloned().unwrap_or_default(),
                    line: HistoryLine {
                        at: instant(row.at)?,
                        who: Who::parse(&row.who),
                        text: row.text,
                    },
                })
            })
            .collect()
    }

    /// Its history, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn history(&self, id: TaskId) -> Result<Vec<HistoryLine>, DbError> {
        issue_event::Entity::find()
            .filter(issue_event::Column::Issue.eq(id.get()))
            .order_by_asc(issue_event::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(HistoryLine {
                    at: instant(row.at)?,
                    who: Who::parse(&row.who),
                    text: row.text,
                })
            })
            .collect()
    }

    /// Adds a line to its history (and makes it the task's last line); `false` when there is
    /// no such task.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn note(
        &self,
        id: TaskId,
        who: Who,
        text: &str,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        if find(&txn, id).await?.is_none() {
            return Ok(false);
        }
        note(&txn, id, who, text, now).await?;
        txn.commit().await?;
        Ok(true)
    }

    /// Dates a done task's finish `at` (the day it really happened, for its week): false when
    /// there is no such task or it isn't done.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn done_on(&self, id: TaskId, at: Timestamp) -> Result<bool, DbError> {
        let Some(row) = issue::Entity::find_by_id(id.get()).one(&self.conn).await? else {
            return Ok(false);
        };
        if row.status != Status::Done.as_str() {
            return Ok(false);
        }
        let mut active = row.into_active_model();
        active.done_at = Set(Some(millis(at)));
        active.update(&self.conn).await?;
        Ok(true)
    }

    /// Makes a task, as `by` asked, and settles its parent.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when a rule says no (no such parent or area, a finding with
    /// no scan, a tree too deep), [`TaskStoreError::Db`] when the database fails.
    pub async fn create(
        &self,
        new: NewTask,
        by: Who,
        now: Timestamp,
    ) -> Result<Task, TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let task = create(&txn, new, by, now).await?;
        txn.commit().await?;
        Ok(task)
    }

    /// Changes a task, as `by` asked (`said`: the history line for a status change, in place of
    /// the usual one), with everything that follows from it.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when a rule says no, [`TaskStoreError::Db`] when the
    /// database fails.
    pub async fn update(
        &self,
        id: TaskId,
        change: TaskChange,
        by: Who,
        said: Option<&str>,
        now: Timestamp,
    ) -> Result<Task, TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let task = update(&txn, id, change, by, said, now, 0).await?;
        txn.commit().await?;
        Ok(task)
    }

    /// Puts the ask in front of the person: the task waits, with its decision (JSON), its
    /// action and its decide-by day, and comes back from an archive's park; `line` goes to its
    /// history, and that line is the ask's id (every ask, a re-check too, has its own): the
    /// task's latest ask from now on.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when there is no such task, [`TaskStoreError::Db`].
    #[expect(clippy::too_many_arguments, reason = "an ask's parts, each a column")]
    pub async fn ask(
        &self,
        id: TaskId,
        decision: String,
        action: String,
        decide_by: Option<Date>,
        line: &str,
        by: Who,
        now: Timestamp,
    ) -> Result<(Task, AskId), TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let row = issue::Entity::find_by_id(id.get())
            .one(&txn)
            .await?
            .ok_or(TaskRefusal::NoTask(id))?;
        let mut active = row.clone().into_active_model();
        active.status = Set(Status::Waiting.as_str().to_owned());
        active.decision = Set(Some(decision));
        active.action = Set(Some(action));
        active.decide_by = Set(decide_by.map(|d| d.to_string()));
        active.updated = Set(millis(now));
        if row.parked_until.as_deref() == Some(PARKED_BY_ARCHIVE) {
            active.parked_until = Set(None);
        }
        active.update(&txn).await?;
        let ask = note(&txn, id, by, line, now).await?;
        issue::Entity::update_many()
            .col_expr(issue::Column::LastAsk, Expr::value(ask))
            .filter(issue::Column::Id.eq(id.get()))
            .exec(&txn)
            .await?;
        let task = find(&txn, id).await?.ok_or(TaskRefusal::NoTask(id))?;
        txn.commit().await?;
        Ok((task, AskId::new(ask)))
    }

    /// The person's answer `text` to the task's latest ask goes to its history, naming that
    /// ask, read in the same transaction; `false` when there is no such task.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn answer(
        &self,
        id: TaskId,
        who: Who,
        text: &str,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let Some(row) = issue::Entity::find_by_id(id.get()).one(&txn).await? else {
            return Ok(false);
        };
        let ask = row.last_ask.map(AskId::new);
        note_answering(&txn, id, who, text, ask, now).await?;
        txn.commit().await?;
        Ok(true)
    }

    /// The task's latest ask; none when it was never asked anything, or there is no such task.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn last_ask(&self, id: TaskId) -> Result<Option<AskId>, DbError> {
        Ok(issue::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .and_then(|row| row.last_ask)
            .map(AskId::new))
    }

    /// The person's answers to the ask `ask` of the task, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn answers_to(&self, id: TaskId, ask: AskId) -> Result<Vec<HistoryLine>, DbError> {
        issue_event::Entity::find()
            .filter(issue_event::Column::Issue.eq(id.get()))
            .filter(issue_event::Column::Answers.eq(ask.get()))
            .order_by_asc(issue_event::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(HistoryLine {
                    at: instant(row.at)?,
                    who: Who::parse(&row.who),
                    text: row.text,
                })
            })
            .collect()
    }

    /// Sets what its worker carries on with: `text` after what it has, and the task back to to
    /// do (its ask cleared) unless `keep_status`.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when there is no such task, [`TaskStoreError::Db`].
    pub async fn queue_next(
        &self,
        id: TaskId,
        text: &str,
        reopen: bool,
        now: Timestamp,
    ) -> Result<(), TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let row = issue::Entity::find_by_id(id.get())
            .one(&txn)
            .await?
            .ok_or(TaskRefusal::NoTask(id))?;
        let next = match row.next.as_deref() {
            Some(was) if !was.is_empty() => format!("{was}\n\n{text}"),
            _ => text.to_owned(),
        };
        let mut active = row.into_active_model();
        active.next = Set(Some(next));
        if reopen {
            active.status = Set(Status::Todo.as_str().to_owned());
            active.decision = Set(None);
            active.action = Set(None);
            active.decide_by = Set(None);
        }
        active.updated = Set(millis(now));
        active.update(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Top-level tasks done from `from` to just before `to`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn count_done(&self, from: Timestamp, to: Timestamp) -> Result<u64, DbError> {
        use sea_orm::PaginatorTrait as _;
        Ok(issue::Entity::find()
            .filter(issue::Column::Status.eq(Status::Done.as_str()))
            .filter(issue::Column::Parent.is_null())
            .filter(issue::Column::DoneAt.gte(millis(from)))
            .filter(issue::Column::DoneAt.lt(millis(to)))
            .count(&self.conn)
            .await?)
    }

    /// Top-level tasks done since `since`, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn done_since(&self, since: Timestamp) -> Result<Vec<Task>, DbError> {
        rows(
            issue::Entity::find()
                .filter(issue::Column::Status.eq(Status::Done.as_str()))
                .filter(issue::Column::Parent.is_null())
                .filter(issue::Column::DoneAt.gte(millis(since)))
                .order_by_desc(issue::Column::DoneAt)
                .all(&self.conn)
                .await?,
        )
    }

    /// When the first task was made.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn first_created(&self) -> Result<Option<Timestamp>, DbError> {
        let first: Option<i64> = issue::Entity::find()
            .select_only()
            .column(issue::Column::Created)
            .order_by_asc(issue::Column::Created)
            .into_tuple()
            .one(&self.conn)
            .await?;
        first.map(instant).transpose()
    }

    /// The tasks waiting on the person with a decision.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn asking(&self) -> Result<Vec<Task>, DbError> {
        rows(
            issue::Entity::find()
                .filter(issue::Column::Status.eq(Status::Waiting.as_str()))
                .filter(issue::Column::Decision.is_not_null())
                .all(&self.conn)
                .await?,
        )
    }

    /// Writes its worker's columns as the worker area keeps them: its unit while one runs, and
    /// its session once one ran.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when there is no such task, [`TaskStoreError::Db`].
    pub async fn set_worker(
        &self,
        id: TaskId,
        unit: Option<String>,
        session: Option<String>,
    ) -> Result<(), TaskStoreError> {
        let row = issue::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .ok_or(TaskRefusal::NoTask(id))?;
        let mut active = row.into_active_model();
        active.unit = Set(unit);
        if session.is_some() {
            active.session = Set(session);
        }
        active.update(&self.conn).await?;
        Ok(())
    }
}

/// The `kind` of a finding's row.
const FINDING: &str = "finding";

pub(crate) fn date(text: Option<&str>) -> Option<Date> {
    text.and_then(|text| text.parse::<Date>().ok())
}

fn from_row(row: issue::Model) -> Result<Task, DbError> {
    Ok(Task {
        id: TaskId::new(row.id),
        owner: Owner::parse(&row.owner).unwrap_or(Owner::Person),
        status: Status::parse(&row.status).unwrap_or(Status::Todo),
        goal: row.goal.map(GoalId::new),
        pid: row.pid.map(ProjectId::new),
        size: row.size.as_deref().and_then(Size::parse),
        due: date(row.due.as_deref()),
        decide_by: date(row.decide_by.as_deref()),
        created: instant(row.created)?,
        updated: instant(row.updated)?,
        done_at: row.done_at.map(instant).transpose()?,
        parent: row.parent.map(TaskId::new),
        area: row.area.map(AreaId::new),
        finding: row.kind.as_deref() == Some(FINDING),
        landed: row.landed.as_deref().and_then(Landed::parse),
        fixed_by: row.fixed_by.map(TaskId::new),
        title: row.title,
        body: row.body,
        project: row.project,
        hard: row.hard,
        priority: row.priority,
        today: row.today,
        parked_until: row.parked_until,
        decision: row.decision,
        action: row.action,
        last: row.last_note,
        progress: row.progress,
        total: row.total,
        unit: row.count_unit,
        tracked: row.tracked,
        eta: row.eta,
        ultra: row.ultra,
        model: row.model,
        runner: row.runner,
        next: row.next,
        worker: row.unit,
        session: row.session,
        requested: row.requested,
        requested_by: row.requested_by,
    })
}

fn rows(rows: Vec<issue::Model>) -> Result<Vec<Task>, DbError> {
    rows.into_iter().map(from_row).collect()
}

async fn find(conn: &impl ConnectionTrait, id: TaskId) -> Result<Option<Task>, DbError> {
    issue::Entity::find_by_id(id.get())
        .one(conn)
        .await?
        .map(from_row)
        .transpose()
}

async fn under(
    conn: &impl ConnectionTrait,
    id: TaskId,
    findings: bool,
) -> Result<Vec<Task>, DbError> {
    let kind = if findings {
        issue::Column::Kind.eq(FINDING)
    } else {
        issue::Column::Kind.is_null()
    };
    rows(
        issue::Entity::find()
            .filter(issue::Column::Parent.eq(id.get()))
            .filter(kind)
            .order_by_asc(issue::Column::Id)
            .all(conn)
            .await?,
    )
}

type LightRow = (
    i64,
    Option<i64>,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

async fn family(conn: &impl ConnectionTrait) -> Result<Family, DbError> {
    let rows: Vec<LightRow> = issue::Entity::find()
        .select_only()
        .columns([
            issue::Column::Id,
            issue::Column::Parent,
            issue::Column::Status,
            issue::Column::Progress,
            issue::Column::Total,
            issue::Column::CountUnit,
            issue::Column::Kind,
        ])
        .into_tuple()
        .all(conn)
        .await?;
    Ok(Family::new(rows.into_iter().map(
        |(id, parent, status, progress, total, unit, kind)| Light {
            id: TaskId::new(id),
            parent: parent.map(TaskId::new),
            status: Status::parse(&status).unwrap_or(Status::Todo),
            progress,
            total,
            unit,
            finding: kind.as_deref() == Some(FINDING),
        },
    )))
}

/// Adds a line to a task's history (and makes it the task's last line); its id.
async fn note(
    conn: &impl ConnectionTrait,
    id: TaskId,
    who: Who,
    text: &str,
    now: Timestamp,
) -> Result<i64, DbError> {
    note_answering(conn, id, who, text, None, now).await
}

/// [`note`], for the person's answer to `answers` (none: not an answer to an ask).
async fn note_answering(
    conn: &impl ConnectionTrait,
    id: TaskId,
    who: Who,
    text: &str,
    answers: Option<AskId>,
    now: Timestamp,
) -> Result<i64, DbError> {
    let line = issue_event::ActiveModel {
        issue: Set(id.get()),
        at: Set(millis(now)),
        who: Set(who.as_str().to_owned()),
        text: Set(text.to_owned()),
        answers: Set(answers.map(AskId::get)),
        ..Default::default()
    }
    .insert(conn)
    .await?;
    issue::Entity::update_many()
        .col_expr(issue::Column::Updated, Expr::value(millis(now)))
        .col_expr(issue::Column::LastNote, Expr::value(text.to_owned()))
        .filter(issue::Column::Id.eq(id.get()))
        .exec(conn)
        .await?;
    Ok(line.id)
}

async fn touch(conn: &impl ConnectionTrait, id: TaskId, now: Timestamp) -> Result<(), DbError> {
    issue::Entity::update_many()
        .col_expr(issue::Column::Updated, Expr::value(millis(now)))
        .filter(issue::Column::Id.eq(id.get()))
        .exec(conn)
        .await?;
    Ok(())
}

async fn area_pid(conn: &impl ConnectionTrait, id: AreaId) -> Result<Option<ProjectId>, DbError> {
    Ok(area::Entity::find_by_id(id.get())
        .one(conn)
        .await?
        .map(|row| ProjectId::new(row.pid)))
}

/// The parent a task may go under: any task but a finding, as long as the tree stays a tree
/// and no deeper than [`MAX_DEPTH`] (a finding hangs off its scan, at any depth).
async fn parent_of(
    conn: &impl ConnectionTrait,
    parent: Option<TaskId>,
    me: Option<TaskId>,
    finding: bool,
) -> Result<Option<Task>, TaskStoreError> {
    let Some(parent) = parent.filter(|id| id.get() != 0) else {
        return Ok(None);
    };
    let p = find(conn, parent)
        .await?
        .ok_or(TaskRefusal::NoTask(parent))?;
    if p.finding {
        return Err(TaskRefusal::UnderAFinding(parent).into());
    }
    let tree = family(conn).await?;
    let ancestors = tree.ancestors(parent);
    if let Some(me) = me {
        if me == parent {
            return Err(TaskRefusal::OwnParent(parent).into());
        }
        if ancestors.contains(&me) {
            return Err(TaskRefusal::Loop { task: me, parent }.into());
        }
    }
    let depth = ancestors.len() + 2 + me.map_or(0, |me| tree.height(me));
    if depth > MAX_DEPTH && !finding {
        return Err(TaskRefusal::TooDeep.into());
    }
    Ok(Some(p))
}

#[expect(
    clippy::too_many_lines,
    reason = "a new task's rules and its row, in order"
)]
async fn create(
    txn: &impl ConnectionTrait,
    mut new: NewTask,
    by: Who,
    now: Timestamp,
) -> Result<Task, TaskStoreError> {
    let title = new.title.trim().to_owned();
    if title.is_empty() {
        return Err(TaskRefusal::NoTitle.into());
    }
    if new.finding && new.parent.is_none_or(|p| p.get() == 0) {
        return Err(TaskRefusal::FindingWithoutScan.into());
    }
    if !new.finding && (new.landed.is_some() || new.fixed_by.is_some()) {
        return Err(TaskRefusal::FindingFieldsOnTask.into());
    }
    if let Some(fix) = new.fixed_by
        && find(txn, fix).await?.is_none()
    {
        return Err(TaskRefusal::NoFix(fix).into());
    }
    let parent = parent_of(txn, new.parent, None, new.finding).await?;
    if let Some(p) = &parent {
        new.owner = Some(if new.finding {
            Owner::Assistant
        } else {
            new.owner.unwrap_or(p.owner)
        });
        new.pid = new.pid.or(p.pid);
        new.goal = new.goal.or(p.goal);
        new.area = p.area;
        new.tracked = new.tracked || p.tracked;
        // A subtask or a finding is made in its parent's room: an agent's task's are the
        // agent's too, never the owner's.
        if new.room.is_none() {
            new.room = issue::Entity::find_by_id(p.id.get())
                .one(txn)
                .await?
                .and_then(|row| row.room)
                .map(RoomId::new);
        }
    } else if let Some(area) = new.area.filter(|a| a.get() != 0) {
        let pid = area_pid(txn, area)
            .await?
            .ok_or(TaskRefusal::NoArea(area))?;
        if new
            .pid
            .is_some_and(|given| given.get() != 0 && given != pid)
        {
            return Err(TaskRefusal::AreaElsewhere(area).into());
        }
        new.pid = Some(pid);
    }
    let landed = new.finding.then(|| new.landed.unwrap_or(Landed::Unchecked));
    let status = landed.map_or(Status::Todo, Landed::status);
    let row = issue::ActiveModel {
        title: Set(title),
        body: Set(new.body),
        owner: Set(new.owner.unwrap_or(Owner::Person).as_str().to_owned()),
        status: Set(status.as_str().to_owned()),
        goal: Set(new.goal.map(GoalId::get)),
        project: Set(new.project),
        pid: Set(new.pid.map(ProjectId::get)),
        hard: Set(new.hard),
        size: Set(new.size.map(|s| s.as_str().to_owned())),
        priority: Set(new.priority.unwrap_or(3)),
        due: Set(new.due.map(|d| d.to_string())),
        today: Set(new.today),
        parked_until: Set(new.parked_until),
        decide_by: Set(None),
        decision: Set(None),
        action: Set(None),
        created: Set(millis(now)),
        updated: Set(millis(now)),
        done_at: Set((status == Status::Done).then(|| millis(now))),
        last_note: Set(None),
        parent: Set(parent.as_ref().map(|p| p.id.get())),
        progress: Set(parent.as_ref().and(new.progress)),
        total: Set(parent.as_ref().and(new.total)),
        count_unit: Set(parent.as_ref().and(new.unit)),
        tracked: Set(new.tracked || new.finding),
        eta: Set(None),
        ultra: Set(false),
        ultra_parallel: Set(None),
        ultra_budget: Set(None),
        model: Set(None),
        area: Set(new.area.filter(|a| a.get() != 0).map(AreaId::get)),
        kind: Set(new.finding.then(|| FINDING.to_owned())),
        landed: Set(landed.map(|l| l.as_str().to_owned())),
        fixed_by: Set(new.fixed_by.map(TaskId::get)),
        runner: Set(None),
        next: Set(None),
        unit: Set(None),
        session: Set(None),
        requested: Set(if by == Who::Person && !new.finding {
            new.requested
        } else {
            None
        }),
        room: Set(new.room.map(RoomId::get)),
        requested_by: Set(new.requested_by),
        ..Default::default()
    };
    let id = TaskId::new(row.insert(txn).await?.id);
    let line = match (&parent, landed) {
        (Some(p), Some(landed)) => {
            format!("Filed as a finding of {}: {}", p.id.key(), landed.words())
        }
        _ if by == Who::Person => "Created".to_owned(),
        _ => "Created by the assistant".to_owned(),
    };
    note(txn, id, by, &line, now).await?;
    if let Some(p) = &parent
        && !new.finding
    {
        settle(txn, p.id, now, 0).await?;
    }
    Ok(find(txn, id).await?.ok_or(TaskRefusal::NoTask(id))?)
}

fn put<T>(field: Patch<T>, column: &mut sea_orm::ActiveValue<Option<T>>)
where
    T: Into<sea_orm::Value>,
    Option<T>: Into<sea_orm::Value>,
{
    if let Some(value) = field {
        *column = Set(value);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "a change and everything that follows from it, in order"
)]
fn update<'a, C: ConnectionTrait>(
    txn: &'a C,
    id: TaskId,
    change: TaskChange,
    by: Who,
    said: Option<&'a str>,
    now: Timestamp,
    depth: usize,
) -> Boxed<'a, Task> {
    Box::pin(async move {
        let before = find(txn, id).await?.ok_or(TaskRefusal::NoTask(id))?;
        if let Some(Some(fix)) = change.fixed_by
            && (fix == id || find(txn, fix).await?.is_none())
        {
            return Err(TaskRefusal::NoFix(fix).into());
        }
        let new_parent = match change.parent {
            Some(parent) => parent_of(txn, parent, Some(id), before.finding).await?,
            None => None,
        };
        if let (Some(area), None, Some(parent)) = (change.area, change.parent, before.parent)
            && area != before.area
        {
            return Err(TaskRefusal::SubtaskArea { task: id, parent }.into());
        }
        if let Some(Some(area)) = change.area
            && area_pid(txn, area).await?.is_none()
        {
            return Err(TaskRefusal::NoArea(area).into());
        }
        let change = settle_change(&before, change)?;
        let row = issue::Entity::find_by_id(id.get())
            .one(txn)
            .await?
            .ok_or(TaskRefusal::NoTask(id))?;
        let mut active = row.into_active_model();
        if let Some(title) = change.title.clone() {
            active.title = Set(title);
        }
        put(change.body.clone(), &mut active.body);
        if let Some(owner) = change.owner {
            active.owner = Set(owner.as_str().to_owned());
        }
        if let Some(status) = change.status {
            active.status = Set(status.as_str().to_owned());
        }
        put(change.goal.map(|g| g.map(GoalId::get)), &mut active.goal);
        if let Some(priority) = change.priority {
            active.priority = Set(priority);
        }
        put(
            change.due.map(|d| d.map(|d| d.to_string())),
            &mut active.due,
        );
        if let Some(today) = change.today {
            active.today = Set(today);
        }
        put(change.parked_until.clone(), &mut active.parked_until);
        put(change.pid.map(|p| p.map(ProjectId::get)), &mut active.pid);
        if let Some(hard) = change.hard {
            active.hard = Set(hard);
        }
        put(
            change.size.map(|s| s.map(|s| s.as_str().to_owned())),
            &mut active.size,
        );
        put(
            change.parent.map(|p| p.map(TaskId::get)),
            &mut active.parent,
        );
        if let Some(progress) = change.progress {
            active.progress = Set(Some(progress));
        }
        if let Some(total) = change.total {
            active.total = Set(Some(total));
        }
        if let Some(unit) = change.unit.clone() {
            active.count_unit = Set(Some(unit));
        }
        if let Some(tracked) = change.tracked {
            active.tracked = Set(tracked);
        }
        put(change.eta.clone(), &mut active.eta);
        put(change.action.clone(), &mut active.action);
        put(change.runner.clone(), &mut active.runner);
        put(
            change.decide_by.map(|d| d.map(|d| d.to_string())),
            &mut active.decide_by,
        );
        if let Some(ultra) = change.ultra {
            active.ultra = Set(ultra);
        }
        put(change.model.clone(), &mut active.model);
        put(change.area.map(|a| a.map(AreaId::get)), &mut active.area);
        if let Some(landed) = change.landed {
            active.landed = Set(Some(landed.as_str().to_owned()));
        }
        put(
            change.fixed_by.map(|f| f.map(TaskId::get)),
            &mut active.fixed_by,
        );
        let moved = change.status.filter(|status| *status != before.status);
        if let Some(status) = moved {
            active.done_at = Set((status == Status::Done).then(|| millis(now)));
            if status == Status::Done {
                active.today = Set(false);
            }
            if status.is_closed() {
                active.eta = Set(None);
            }
            if status != Status::Waiting {
                active.decision = Set(None);
                active.action = Set(None);
                active.decide_by = Set(None);
            }
        }
        active.update(txn).await?;
        if let Some(status) = moved {
            note(txn, id, by, said.unwrap_or(status.note()), now).await?;
            if status == Status::Waiting {
                issue::Entity::update_many()
                    .col_expr(
                        issue::Column::ParkedUntil,
                        Expr::value(Option::<String>::None),
                    )
                    .filter(issue::Column::Id.eq(id.get()))
                    .filter(issue::Column::ParkedUntil.eq(PARKED_BY_ARCHIVE))
                    .exec(txn)
                    .await?;
            }
        }
        let tree = family(txn).await?;
        let below = tree.descendants(id);
        if let Some(tracked) = change.tracked {
            issue::Entity::update_many()
                .col_expr(issue::Column::Tracked, Expr::value(tracked))
                .filter(issue::Column::Id.is_in(below.iter().map(|d| d.get())))
                .filter(issue::Column::Kind.is_null())
                .exec(txn)
                .await?;
        }
        if before.finding
            && let Some(landed) = change.landed
            && Some(landed) != before.landed
        {
            let fixed = match change.fixed_by {
                Some(Some(fix)) => format!(" (fixed by {})", fix.key()),
                _ => String::new(),
            };
            note(txn, id, by, &format!("Now {}{fixed}", landed.words()), now).await?;
        }
        let subtree: Vec<i64> = std::iter::once(id).chain(below).map(TaskId::get).collect();
        let area_given = change.area.is_some();
        if new_parent.is_some()
            || (area_given && before.parent.is_none())
            || (matches!(change.parent, Some(None)) && area_given)
        {
            let area = match &new_parent {
                Some(p) => p.area,
                None => change.area.flatten().filter(|a| a.get() != 0),
            };
            let pid = match (&new_parent, area) {
                (Some(p), _) => p.pid,
                (None, Some(area)) => area_pid(txn, area).await?,
                (None, None) => None,
            };
            let mut query = issue::Entity::update_many()
                .col_expr(issue::Column::Area, Expr::value(area.map(AreaId::get)));
            if let Some(pid) = pid {
                query = query.col_expr(issue::Column::Pid, Expr::value(pid.get()));
            }
            query
                .filter(issue::Column::Id.is_in(subtree.clone()))
                .exec(txn)
                .await?;
        } else if let Some(pid) = change.pid
            && pid != before.pid
        {
            let keep = match before.area {
                Some(area) => area_pid(txn, area).await? == pid,
                None => true,
            };
            let mut query = issue::Entity::update_many()
                .col_expr(issue::Column::Pid, Expr::value(pid.map(ProjectId::get)));
            if !keep {
                query = query.col_expr(issue::Column::Area, Expr::value(Option::<i64>::None));
            }
            query
                .filter(issue::Column::Id.is_in(subtree.clone()))
                .exec(txn)
                .await?;
        }
        let after = find(txn, id).await?.ok_or(TaskRefusal::NoTask(id))?;
        let mut fix = issue::Entity::update_many();
        let mut fixing = false;
        if after.decide_by.is_some() && after.status != Status::Waiting {
            fix = fix.col_expr(issue::Column::DecideBy, Expr::value(Option::<String>::None));
            fixing = true;
        }
        let brief = match &change.body {
            Some(Some(body)) => body.trim(),
            _ => "",
        };
        if !brief.is_empty()
            && brief != before.body.as_deref().unwrap_or_default().trim()
            && after.session.is_some()
            && after.owner == Owner::Assistant
            && !after.tracked
        {
            let line = format!("Your brief changed. New brief: {brief}");
            let next = match after.next.as_deref() {
                Some(was) if !was.is_empty() => format!("{was}\n\n{line}"),
                _ => line,
            };
            fix = fix.col_expr(issue::Column::Next, Expr::value(next));
            fixing = true;
        }
        if fixing {
            fix.filter(issue::Column::Id.eq(id.get())).exec(txn).await?;
        }
        touch(txn, id, now).await?;
        if depth < 50 {
            let mut parents: Vec<TaskId> = before.parent.into_iter().chain(after.parent).collect();
            parents.dedup();
            for parent in parents {
                settle(txn, parent, now, depth + 1).await?;
            }
            if before.status == Status::Waiting && after.status != Status::Waiting {
                let key = id.key();
                let pointing = issue::Entity::find()
                    .filter(issue::Column::Status.eq(Status::Waiting.as_str()))
                    .filter(issue::Column::Decision.is_null())
                    .filter(issue::Column::Action.contains(&key))
                    .all(txn)
                    .await?;
                for row in pointing {
                    if names_key(row.action.as_deref().unwrap_or_default(), &key) {
                        let clear = TaskChange {
                            action: Some(None),
                            ..TaskChange::default()
                        };
                        update(txn, TaskId::new(row.id), clear, by, None, now, depth + 1).await?;
                    }
                }
            }
        }
        Ok(find(txn, id).await?.ok_or(TaskRefusal::NoTask(id))?)
    })
}

/// Whether `text` names the task `key` as a whole word (task 12's key, not task 123's).
fn names_key(text: &str, key: &str) -> bool {
    text.match_indices(key).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + key.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

/// A parent is done when every subtask is done or canceled, and open again when one reopens.
fn settle<C: ConnectionTrait>(
    txn: &C,
    parent: TaskId,
    now: Timestamp,
    depth: usize,
) -> Boxed<'_, ()> {
    Box::pin(async move {
        let Some(p) = find(txn, parent).await? else {
            return Ok(());
        };
        let kids = under(txn, parent, false).await?;
        if kids.is_empty() {
            touch(txn, parent, now).await?;
            return Ok(());
        }
        let finished = kids.iter().all(|kid| kid.status.is_closed());
        if finished && p.status.is_open() {
            note(
                txn,
                parent,
                Who::Assistant,
                "All its subtasks are done",
                now,
            )
            .await?;
            update(
                txn,
                parent,
                TaskChange::status(Status::Done),
                Who::Assistant,
                None,
                now,
                depth,
            )
            .await?;
        } else if !finished && p.status == Status::Done {
            update(
                txn,
                parent,
                TaskChange::status(Status::Todo),
                Who::Assistant,
                None,
                now,
                depth,
            )
            .await?;
        } else {
            touch(txn, parent, now).await?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use emberhut_core::task::wire::KEY_PREFIX;

    use super::*;

    /// Task `n`'s key, as the tasks spell it.
    fn key(n: u32) -> String {
        format!("{KEY_PREFIX}{n}")
    }

    #[test]
    fn a_key_is_named_only_as_a_whole_word() {
        assert!(names_key(
            &format!("One yes on {} covers it", key(12)),
            &key(12)
        ));
        assert!(names_key(&key(12), &key(12)));
        assert!(!names_key(&format!("see {}", key(123)), &key(12)));
        assert!(!names_key(&format!("x{}", key(12)), &key(12)));
    }
}
