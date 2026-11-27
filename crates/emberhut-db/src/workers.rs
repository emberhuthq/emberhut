//! What the workers and ultra runs keep: each run's sub-agents (`agents`), how far each
//! worker's stream has been read (`streams`), and an ultra task's run and its worker's
//! current stream file (columns of `issues`).

use emberhut_core::task::TaskId;
use emberhut_core::ultra;
use jiff::Timestamp;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};

use crate::DbError;
use crate::entity::{agent, issue, stream};
use crate::time::{instant, millis};

/// One sub-agent of a task's run.
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    /// Its row.
    pub id: i64,
    /// Its task.
    pub task: TaskId,
    /// The session that started it.
    pub run: String,
    /// The CLI's own id for it (a workflow's agent: `<task id>/<agent id>`).
    pub key: String,
    /// The tool call that started it.
    pub tool_use: Option<String>,
    /// Where it was seen: `worker` or `runner`.
    pub source: Option<String>,
    /// Its kind, from its name.
    pub kind: ultra::AgentKind,
    /// Its name.
    pub name: Option<String>,
    /// Its part.
    pub part: Option<f64>,
    /// `running`, `queued`, `done`, `failed` or `stopped`.
    pub state: String,
    /// Its latest line.
    pub last: Option<String>,
    /// Its tokens so far.
    pub tokens: Option<i64>,
    /// Its tool calls so far.
    pub tools: Option<i64>,
    /// The account it runs on.
    pub account: Option<String>,
    /// The machine it runs on.
    pub machine: Option<String>,
    /// When it started.
    pub started: Option<Timestamp>,
    /// When it ended.
    pub ended: Option<Timestamp>,
}

impl Agent {
    /// A workflow's own agent (its row sits under the workflow's).
    #[must_use]
    pub fn in_workflow(&self) -> bool {
        self.key.contains('/')
    }
}

/// What is seen of a sub-agent: its start, its progress or its end. What isn't given keeps
/// what the row has.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSeen {
    /// Its task.
    pub task: TaskId,
    /// The session that started it.
    pub run: String,
    /// The CLI's own id for it.
    pub key: String,
    /// Where it was seen.
    pub source: String,
    /// Its name.
    pub name: Option<String>,
    /// The tool call that started it.
    pub tool_use: Option<String>,
    /// Its state.
    pub state: Option<String>,
    /// Its latest line.
    pub last: Option<String>,
    /// Its tokens so far.
    pub tokens: Option<i64>,
    /// Its tool calls so far.
    pub tools: Option<i64>,
    /// The account it runs on.
    pub account: Option<String>,
    /// The machine it runs on.
    pub machine: Option<String>,
    /// When it was seen; now when none.
    pub at: Option<Timestamp>,
}

impl AgentSeen {
    /// Run `run`'s sub-agent `key` of `task`, seen in `source`, with nothing else said.
    #[must_use]
    pub fn new(
        task: TaskId,
        run: impl Into<String>,
        key: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            task,
            run: run.into(),
            key: key.into(),
            source: source.into(),
            name: None,
            tool_use: None,
            state: None,
            last: None,
            tokens: None,
            tools: None,
            account: None,
            machine: None,
            at: None,
        }
    }
}

/// How far a stream has been read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamPlace {
    /// The stream's file.
    pub file: String,
    /// The bytes read so far.
    pub offset: u64,
    /// Its task.
    pub task: Option<TaskId>,
    /// What its lead was doing at the last line read.
    pub doing: Option<String>,
    /// The lead's tokens so far.
    pub tokens: f64,
}

/// A task whose worker runs: its unit, its session and its stream file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingUnit {
    /// The task.
    pub task: TaskId,
    /// Its worker's unit.
    pub unit: String,
    /// Its worker's session.
    pub session: Option<String>,
    /// Its current stream file.
    pub turn_file: Option<String>,
}

/// Reads and writes the workers' and ultra runs' tables.
#[derive(Clone, Debug)]
pub struct WorkerStore {
    conn: DatabaseConnection,
}

/// The longest name and line a sub-agent keeps.
const NAME: usize = 120;
const LAST: usize = 300;

impl WorkerStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Notes what was seen of a sub-agent: a new one is added with its kind and part from its
    /// name; one known already takes the new state, line and counts (a finished one stays
    /// finished: a late progress line doesn't bring it back).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn upsert(&self, seen: AgentSeen, now: Timestamp) -> Result<(), DbError> {
        let at = millis(seen.at.unwrap_or(now));
        let ended = seen.state.as_deref().is_some_and(|s| !ultra::running(s));
        let last = seen
            .last
            .as_deref()
            .filter(|l| !l.is_empty())
            .map(|l| ultra::clip(l, LAST));
        let was = agent::Entity::find()
            .filter(agent::Column::Run.eq(&seen.run))
            .filter(agent::Column::Key.eq(&seen.key))
            .one(&self.conn)
            .await?;
        let Some(was) = was else {
            let name = ultra::clip(
                seen.name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or("Sub-agent"),
                NAME,
            );
            let (kind, part) = ultra::kind_of(&name);
            agent::ActiveModel {
                task: Set(seen.task.get()),
                run: Set(seen.run),
                key: Set(seen.key),
                tool_use: Set(seen.tool_use),
                source: Set(Some(seen.source)),
                kind: Set(Some(kind.as_str().to_owned())),
                name: Set(Some(name)),
                part: Set(part),
                state: Set(seen.state.unwrap_or_else(|| "running".to_owned())),
                last: Set(last),
                tokens: Set(seen.tokens),
                tools: Set(seen.tools),
                account: Set(seen.account),
                machine: Set(seen.machine),
                started: Set(Some(at)),
                ended: Set(ended.then_some(at)),
                ..Default::default()
            }
            .insert(&self.conn)
            .await?;
            return Ok(());
        };
        let finished = !ultra::running(&was.state);
        let mut row = was.clone().into_active_model();
        if !finished && let Some(state) = seen.state {
            row.state = Set(state);
        }
        if last.is_some() {
            row.last = Set(last);
        }
        if seen.tokens.is_some() {
            row.tokens = Set(seen.tokens);
        }
        if seen.tools.is_some() {
            row.tools = Set(seen.tools);
        }
        if was.tool_use.is_none() && seen.tool_use.is_some() {
            row.tool_use = Set(seen.tool_use);
        }
        if was.ended.is_none() && !finished && ended {
            row.ended = Set(Some(at));
        }
        row.update(&self.conn).await?;
        Ok(())
    }

    /// Whether the run `run` has a sub-agent `key`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn known(&self, run: &str, key: &str) -> Result<bool, DbError> {
        Ok(agent::Entity::find()
            .filter(agent::Column::Run.eq(run))
            .filter(agent::Column::Key.eq(key))
            .one(&self.conn)
            .await?
            .is_some())
    }

    /// A task's sub-agents, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn of_task(&self, task: TaskId) -> Result<Vec<Agent>, DbError> {
        agent::Entity::find()
            .filter(agent::Column::Task.eq(task.get()))
            .order_by_asc(agent::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// The sub-agent of run `run` keyed `key`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, task: TaskId, run: &str) -> Result<Option<Agent>, DbError> {
        agent::Entity::find()
            .filter(agent::Column::Task.eq(task.get()))
            .filter(agent::Column::Run.eq(run))
            .order_by_desc(agent::Column::Id)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Every sub-agent still marked running or queued.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn live(&self) -> Result<Vec<Agent>, DbError> {
        agent::Entity::find()
            .filter(agent::Column::State.is_in(["running", "queued"]))
            .order_by_asc(agent::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// What a task's sub-agents used so far: each one's latest count, a workflow's own agents
    /// left out (the workflow's row holds their total).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn spent(&self, task: TaskId) -> Result<f64, DbError> {
        let all = self.of_task(task).await?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "token counts stay far below 2^52"
        )]
        let total = all
            .iter()
            .filter(|a| !a.in_workflow())
            .filter_map(|a| a.tokens)
            .sum::<i64>() as f64;
        Ok(total)
    }

    /// How many of a task's sub-agents run now (a workflow's own agents left out): with `run`,
    /// only that session's and the lanes.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn running(&self, task: TaskId, run: Option<&str>) -> Result<usize, DbError> {
        Ok(self
            .of_task(task)
            .await?
            .iter()
            .filter(|a| ultra::running(&a.state) && !a.in_workflow())
            .filter(|a| run.is_none_or(|run| a.run == run || a.kind == ultra::AgentKind::Lane))
            .count())
    }

    /// Whether a sub-agent of the task was started by the tool call `tool_use`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn started_by(&self, task: TaskId, tool_use: &str) -> Result<bool, DbError> {
        Ok(agent::Entity::find()
            .filter(agent::Column::Task.eq(task.get()))
            .filter(agent::Column::ToolUse.eq(tool_use))
            .one(&self.conn)
            .await?
            .is_some())
    }

    /// The run `run` of the task ended: what still says running there is stopped (its lanes,
    /// sessions of their own, are left).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn end_run(&self, task: TaskId, run: &str, now: Timestamp) -> Result<u64, DbError> {
        let done = agent::Entity::update_many()
            .col_expr(agent::Column::State, Expr::value("stopped"))
            .col_expr(agent::Column::Ended, Expr::value(millis(now)))
            .filter(agent::Column::Task.eq(task.get()))
            .filter(agent::Column::Run.eq(run))
            .filter(agent::Column::State.is_in(["running", "queued"]))
            .filter(
                Condition::any()
                    .add(agent::Column::Kind.ne("lane"))
                    .add(agent::Column::Kind.is_null()),
            )
            .exec(&self.conn)
            .await?;
        Ok(done.rows_affected)
    }

    /// Stops the sub-agent `id`: its run ended without saying so.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn stop(&self, id: i64, now: Timestamp) -> Result<(), DbError> {
        agent::Entity::update_many()
            .col_expr(agent::Column::State, Expr::value("stopped"))
            .col_expr(agent::Column::Ended, Expr::value(millis(now)))
            .filter(agent::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The tasks with a sub-agent running or queued.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tasks_running(&self) -> Result<Vec<TaskId>, DbError> {
        let mut tasks: Vec<TaskId> = self.live().await?.iter().map(|a| a.task).collect();
        tasks.dedup();
        Ok(tasks)
    }

    /// How far `file` has been read.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn place(&self, file: &str) -> Result<Option<StreamPlace>, DbError> {
        Ok(stream::Entity::find_by_id(file.to_owned())
            .one(&self.conn)
            .await?
            .map(place_of))
    }

    /// Every stream's place.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn places(&self) -> Result<Vec<StreamPlace>, DbError> {
        Ok(stream::Entity::find()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(place_of)
            .collect())
    }

    /// Keeps how far a stream has been read.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_place(&self, place: &StreamPlace) -> Result<(), DbError> {
        let row = stream::ActiveModel {
            file: Set(place.file.clone()),
            offset: Set(i64::try_from(place.offset).unwrap_or(i64::MAX)),
            task: Set(place.task.map(TaskId::get)),
            doing: Set(place.doing.clone()),
            tokens: Set(Some(place.tokens).filter(|t| t.is_finite())),
        };
        stream::Entity::insert(row)
            .on_conflict(
                OnConflict::column(stream::Column::File)
                    .update_columns([
                        stream::Column::Offset,
                        stream::Column::Task,
                        stream::Column::Doing,
                        stream::Column::Tokens,
                    ])
                    .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Forgets the places of every stream but `keep`'s (the running workers').
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn prune_places(&self, keep: &[String]) -> Result<(), DbError> {
        stream::Entity::delete_many()
            .filter(stream::Column::File.is_not_in(keep.iter().cloned()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// An ultra task's run, as kept (JSON).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn run_of(&self, task: TaskId) -> Result<Option<String>, DbError> {
        let run: Option<Option<String>> = issue::Entity::find_by_id(task.get())
            .select_only()
            .column(issue::Column::UltraRun)
            .into_tuple()
            .one(&self.conn)
            .await?;
        Ok(run.flatten().filter(|r| !r.is_empty()))
    }

    /// Keeps an ultra task's run; the task changed now.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_run(&self, task: TaskId, run: &str, now: Timestamp) -> Result<(), DbError> {
        issue::Entity::update_many()
            .col_expr(issue::Column::UltraRun, Expr::value(run))
            .col_expr(issue::Column::Updated, Expr::value(millis(now)))
            .filter(issue::Column::Id.eq(task.get()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Keeps the stream file of the task's worker's current turn.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_turn_file(&self, task: TaskId, file: Option<&str>) -> Result<(), DbError> {
        issue::Entity::update_many()
            .col_expr(
                issue::Column::TurnFile,
                Expr::value(file.map(str::to_owned)),
            )
            .filter(issue::Column::Id.eq(task.get()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Keeps the owner's request for a task (`requested`, as JSON: `{ at, msg, room }`), when
    /// the assistant made the task from their message.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_requested(&self, task: TaskId, requested: &str) -> Result<(), DbError> {
        issue::Entity::update_many()
            .col_expr(issue::Column::Requested, Expr::value(requested))
            .filter(issue::Column::Id.eq(task.get()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Takes what the task's worker carries on with (`next`): it is cleared as it is read.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn take_next(&self, task: TaskId) -> Result<Option<String>, DbError> {
        let next: Option<Option<String>> = issue::Entity::find_by_id(task.get())
            .select_only()
            .column(issue::Column::Next)
            .into_tuple()
            .one(&self.conn)
            .await?;
        issue::Entity::update_many()
            .col_expr(issue::Column::Next, Expr::value(Option::<String>::None))
            .filter(issue::Column::Id.eq(task.get()))
            .exec(&self.conn)
            .await?;
        Ok(next.flatten().filter(|n| !n.trim().is_empty()))
    }

    /// An ultra task's own caps: sub-agents at a time and its spending cap, when set.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn ultra_caps(&self, task: TaskId) -> Result<(Option<i32>, Option<i64>), DbError> {
        let caps: Option<(Option<i32>, Option<i64>)> = issue::Entity::find_by_id(task.get())
            .select_only()
            .column(issue::Column::UltraParallel)
            .column(issue::Column::UltraBudget)
            .into_tuple()
            .one(&self.conn)
            .await?;
        Ok(caps.unwrap_or((None, None)))
    }

    /// The tasks whose worker runs now, with its unit, session and stream file.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn working(&self) -> Result<Vec<WorkingUnit>, DbError> {
        let rows: Vec<WorkingRow> = issue::Entity::find()
            .select_only()
            .column(issue::Column::Id)
            .column(issue::Column::Unit)
            .column(issue::Column::Session)
            .column(issue::Column::TurnFile)
            .filter(issue::Column::Unit.is_not_null())
            .order_by_asc(issue::Column::Id)
            .into_tuple()
            .all(&self.conn)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, unit, session, turn_file)| {
                Some(WorkingUnit {
                    task: TaskId::new(id),
                    unit: unit?,
                    session,
                    turn_file,
                })
            })
            .collect())
    }

    /// The stream file of the task's worker's current turn.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn turn_file(&self, task: TaskId) -> Result<Option<String>, DbError> {
        let file: Option<Option<String>> = issue::Entity::find_by_id(task.get())
            .select_only()
            .column(issue::Column::TurnFile)
            .into_tuple()
            .one(&self.conn)
            .await?;
        Ok(file.flatten())
    }
}

/// A working task's columns: its id, unit, session and stream file.
type WorkingRow = (i64, Option<String>, Option<String>, Option<String>);

fn place_of(row: stream::Model) -> StreamPlace {
    StreamPlace {
        file: row.file,
        offset: u64::try_from(row.offset).unwrap_or(0),
        task: row.task.map(TaskId::new),
        doing: row.doing,
        tokens: row.tokens.unwrap_or(0.0),
    }
}

fn from_row(row: agent::Model) -> Result<Agent, DbError> {
    Ok(Agent {
        id: row.id,
        task: TaskId::new(row.task),
        run: row.run,
        key: row.key,
        tool_use: row.tool_use,
        source: row.source,
        kind: ultra::AgentKind::parse(row.kind.as_deref().unwrap_or("other")),
        name: row.name,
        part: row.part,
        state: row.state,
        last: row.last,
        tokens: row.tokens,
        tools: row.tools,
        account: row.account,
        machine: row.machine,
        started: row.started.map(instant).transpose()?,
        ended: row.ended.map(instant).transpose()?,
    })
}
