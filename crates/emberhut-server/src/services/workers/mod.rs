//! The assistant's workers: an assistant-owned task with a brief gets its own session of the
//! agent CLI ([`crate::services::claude::worker`]), with a tool set that only reaches its own
//! task (`POST /mcp/worker`): notes into its history, one question to the owner, done. The job
//! is done end to end; outward steps (a push, a merge, a message as the owner, a payment) are
//! prepared and asked, never done without the owner's OK ([`crate::services::guard`]). An
//! answer from the owner resumes the same session with it.
//!
//! A worker runs as a child of the server, in a process group of its own: its stream (each
//! line the CLI writes) goes to a turn file in the task's folder (`<work>/<task key>/turn-<ms>
//! .json`), which is where its steps are read back from for the app ([`steps`]) and where an
//! ultra run's sub-agents are seen ([`ultra`]). A server that stops ends its workers
//! ([`WorkerService::close_all`]: each CLI's whole process group, killed once the grace is
//! over); at the next start each task they had is picked up again by the tick.
//!
//! One task has one worker at most: a start takes the task's slot (check and reserve under one
//! lock) before it reads or spawns anything, and gives it back on every way out.
//!
//! Every 30 seconds the tick wraps up the workers that ended (their run closed; one that stopped
//! without finishing asks the owner whether to try again; the assistant hears what came of the
//! task), resumes the tasks the owner answered, and starts every assistant task with a brief
//! that has no worker yet.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use emberhut_core::event::NewEvent;
use emberhut_core::task::ask::OptionIn;
use emberhut_core::task::{Owner, Status, Task, TaskChange, TaskId, Who};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{Database, DbError, EventStore, TaskStore, WorkerStore};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::mcp::worker_tokens::WorkerTokens;
use crate::services::activity::ActivityLog;
use crate::services::agents::scope::TaskOwner;
use crate::services::claude::worker::{AgentWorker, WorkerConfig, WorkerLaunch, WorkerSession};
use crate::services::claude::{CliSpawner, Said};
use crate::services::guard::Guard;
use crate::services::machines::MachineService;
use crate::services::runners::RunnerService;
use crate::services::tasks::{AskInput, TaskService, TaskWorkers};
use crate::services::workers_view::WorkerSessions;

pub mod steps;
pub mod ultra;
pub mod working;

/// How often the workers' tick runs.
pub const TICK: Duration = Duration::from_secs(30);

/// How long an assistant task with a brief may sit in to do with no worker before the
/// assistant hears of it.
const UNSTARTED: SignedDuration = SignedDuration::from_mins(3);

/// How often [`WorkerService::close_all`] looks whether every worker has ended.
const CLOSE_POLL: Duration = Duration::from_millis(50);

/// The newest lines of a task's history a fresh worker's brief carries, in bytes.
const HISTORY_BYTES: usize = 48 * 1024;

/// How a server runs workers: the CLI and how it is started.
#[derive(Clone, Debug)]
pub struct WorkerRuntime {
    /// What starts the CLI.
    pub spawner: Arc<dyn CliSpawner>,
    /// How the CLI runs.
    pub config: WorkerConfig,
}

/// What the workers are built from.
#[derive(Clone, Debug)]
pub struct WorkerParts {
    /// The database.
    pub db: Database,
    /// The tasks.
    pub tasks: TaskService,
    /// The guard rails.
    pub guard: Guard,
    /// The runners (an ultra run's lanes).
    pub runners: RunnerService,
    /// The machines (a lane's machine has to have room).
    pub machines: MachineService,
    /// The activity log.
    pub activity: ActivityLog,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The workers' tokens.
    pub tokens: WorkerTokens,
    /// Where the tasks' folders are.
    pub work_dir: PathBuf,
    /// The CLI; none: this server runs no workers.
    pub runtime: Option<WorkerRuntime>,
    /// The ultra runs' caps.
    pub limits: emberhut_core::ultra::Limits,
    /// The runner machine an ultra run's lanes start on; none: the first runner configured.
    pub lane_machine: Option<String>,
    /// The most workers at once; none: no ceiling.
    pub max_working: Option<usize>,
}

/// A worker running now.
#[derive(Debug, Clone)]
struct Running {
    stop: CancellationToken,
    unit: String,
    session: Option<String>,
    started: Timestamp,
}

/// What a worker's lead did last, as its stream says.
#[derive(Debug, Clone, Default)]
pub(crate) struct LeadSeen {
    pub(crate) doing: Option<String>,
    pub(crate) tokens: f64,
}

#[derive(Debug, Default)]
struct Live {
    running: HashMap<i64, Running>,
    /// Tasks whose worker is being started: their slot is taken until it runs or fails.
    starting: HashSet<i64>,
    /// The server is stopping: no worker starts any more.
    closed: bool,
    /// Workers stopped on purpose: their end is the stop's, nothing to wrap up.
    stopping: HashSet<i64>,
    /// The unstarted warnings given, by task and when it last changed.
    warned: HashSet<(i64, i64)>,
    leads: HashMap<i64, LeadSeen>,
}

/// The assistant's workers. Cheap to clone.
#[derive(Clone, Debug)]
pub struct WorkerService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    parts: WorkerParts,
    store: TaskStore,
    db: WorkerStore,
    events: EventStore,
    live: Mutex<Live>,
}

/// Why a worker didn't start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerError {
    /// A rule says no; the words are for the assistant.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<crate::services::tasks::TaskError> for WorkerError {
    fn from(error: crate::services::tasks::TaskError) -> Self {
        Self::Refused(error.to_string())
    }
}

impl From<crate::services::guard::GuardError> for WorkerError {
    fn from(error: crate::services::guard::GuardError) -> Self {
        Self::Refused(error.to_string())
    }
}

impl WorkerService {
    /// The workers over `parts`.
    #[must_use]
    pub fn new(parts: WorkerParts) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: parts.db.tasks(),
                db: parts.db.workers(),
                events: parts.db.events(),
                parts,
                live: Mutex::default(),
            }),
        }
    }

    fn live(&self) -> MutexGuard<'_, Live> {
        self.inner
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> Timestamp {
        self.inner.parts.clock.now()
    }

    pub(crate) fn tasks(&self) -> &TaskService {
        &self.inner.parts.tasks
    }

    pub(crate) fn guard(&self) -> &Guard {
        &self.inner.parts.guard
    }

    /// What the workers keep: the sub-agents, the streams read, the ultra runs.
    #[must_use]
    pub fn store(&self) -> &WorkerStore {
        &self.inner.db
    }

    /// The task's folder: its scratch space and its turn files.
    #[must_use]
    pub fn folder(&self, task: TaskId) -> PathBuf {
        self.inner.parts.work_dir.join(task.key())
    }

    async fn task(&self, id: TaskId) -> Result<Task, WorkerError> {
        self.inner
            .store
            .get(id)
            .await?
            .ok_or_else(|| WorkerError::Refused(format!("no task {}", id.key())))
    }

    /// Whether the task's worker runs now (or is being started).
    #[must_use]
    pub fn is_running(&self, task: TaskId) -> bool {
        let live = self.live();
        live.running.contains_key(&task.get()) || live.starting.contains(&task.get())
    }

    /// Takes the task's slot for a start: none when its worker runs or is being started
    /// already, or the server is stopping. Checked and taken under one lock, so two starts
    /// can't both pass.
    fn reserve(&self, task: TaskId) -> Option<Slot> {
        let mut live = self.live();
        if live.closed
            || live.running.contains_key(&task.get())
            || !live.starting.insert(task.get())
        {
            return None;
        }
        Some(Slot {
            workers: self.clone(),
            task: task.get(),
            held: true,
        })
    }

    /// The ids of `tasks` a worker may start on: the owner's and enabled agents'; never a
    /// disabled agent's or no one's.
    async fn startable(&self, tasks: impl Iterator<Item = &Task>) -> Result<HashSet<i64>, DbError> {
        let mut startable = HashSet::new();
        for t in tasks {
            let may = match self.guard().task_owner(t.id).await? {
                TaskOwner::Owner => true,
                TaskOwner::Agent(agent) => agent.enabled,
                TaskOwner::Orphaned => false,
            };
            if may {
                startable.insert(t.id.get());
            }
        }
        Ok(startable)
    }

    /// Tells the assistant about `task`: in Main for the owner's task; in its agent's room for a
    /// configured agent's (`agent_text`, the line that agent reads), never in Main, so nothing
    /// its worker wrote reaches the owner's session; nobody for a task no one owns.
    async fn tell(
        &self,
        task: TaskId,
        kind: &str,
        text: String,
        agent_text: impl FnOnce() -> String,
        key: Option<String>,
    ) -> Result<(), DbError> {
        let (room, text) = match self.guard().task_owner(task).await? {
            TaskOwner::Owner => (RoomId::MAIN, text),
            TaskOwner::Agent(agent) => (agent.room, agent_text()),
            TaskOwner::Orphaned => {
                tracing::info!(task = %task.key(), "a task no one owns: nobody is told");
                return Ok(());
            }
        };
        let mut event = NewEvent::new(room, kind, text);
        event.key = key;
        self.inner.events.add(event, self.now()).await?;
        Ok(())
    }

    /// The task's history as a fresh brief carries it: its newest lines within
    /// [`HISTORY_BYTES`], each line at most 2,000 characters.
    async fn history(&self, task: TaskId) -> Result<String, WorkerError> {
        let lines: Vec<String> = self
            .tasks()
            .history_of(task)
            .await?
            .into_iter()
            .map(|h| {
                format!(
                    "- {} {}: {}",
                    crate::convert::timestamp_to_api(h.at).as_str(),
                    h.who.as_str(),
                    h.text.chars().take(2000).collect::<String>()
                )
            })
            .collect();
        let mut kept = Vec::new();
        let mut bytes = 0;
        for (i, line) in lines.iter().enumerate().rev() {
            bytes += line.len() + 1;
            if bytes > HISTORY_BYTES {
                kept.push(format!("- ({} older lines left out)", i + 1));
                break;
            }
            kept.push(line.clone());
        }
        kept.reverse();
        Ok(kept.join("\n"))
    }

    /// A worker's brief: the task, its history and how a worker works; or, resumed, what it
    /// carries on with.
    async fn brief(
        &self,
        t: &Task,
        resume_with: Option<&str>,
        resume: bool,
    ) -> Result<String, WorkerError> {
        let key = t.key();
        if resume && let Some(with) = resume_with {
            let lead = if t.ultra {
                " You're still the lead of its ultra run (the plan, the gate and the caps as before)."
            } else {
                ""
            };
            return Ok(format!(
                "{with}\n\nCarry on with {key}. Report with task_note, finish with task_done or task_ask.{lead}"
            ));
        }
        let folder = self.folder(t.id);
        let body = t
            .body
            .as_deref()
            .map_or(String::new(), |b| format!("Brief: {b}\n"));
        let with = resume_with.map_or(String::new(), |w| format!("{w}\n"));
        let project = t
            .project
            .as_deref()
            .map_or(String::new(), |p| format!("Project: {p}\n"));
        let history = self.history(t.id).await?;
        let ultra = if t.ultra {
            let (parallel, budget) = self.caps(t.id).await?;
            emberhut_core::ultra::lead_brief(&self.inner.parts.limits, parallel, budget)
        } else {
            String::new()
        };
        Ok(format!(
            "You are one of the owner's assistant's workers: you own exactly one task and work on it alone.

Task {key}: {title}
{body}{with}{project}
History so far:
{history}

How you work:
- Do the work now: research, reading the repositories and docs, analysis, drafts, code on a new branch (a git worktree under {folder} if you change code). Your scratch folder is {folder}.
- After each meaningful step, call task_note with one plain line the owner can read.
- The owner's request is their go: do the task end to end, never stop to ask \"should I go ahead?\". Payments, passwords, codes and account security ask on the step, by themselves. Only these are never yours to do without them: messages as the owner (Slack, WhatsApp, email, PR comments), public posts, pushing to main or merging, and deleting their data outside the job. When the task needs one, prepare it completely (the branch, the exact text) and call task_ask with the exact question and short options, then stop.
- A message the owner will send to a person is plain text in their voice, no markdown. Put it in task_ask's draft, with the chat it goes to and a \"Send it\" option: their tap then sends that exact draft there as them.
- Anything you start that keeps running after you (a long render, a server): run it detached and call task_runner with its unit right away, so the owner sees it running and the assistant hears when it ends.
- A scan, review or audit files every finding it reports with task_findings, so none drops between your report and the fix.
- Finish with task_done(summary) when nothing is needed from the owner, or task_ask when it is. Plain words for a phone.
- task_ask's action is the line the owner's app shows in bold: what they should do. Ask only when they really have to act or decide: never for an OK on work the task already asks for. The server checks each ask against their words first: one their request or their messages already cover comes back \"Not asked: covered, do it\" with their quote; then do it.{ultra}",
            title = t.title,
            folder = folder.display(),
        ))
    }

    /// An ultra task's sub-agents at a time and spending cap.
    pub(crate) async fn caps(&self, task: TaskId) -> Result<(u32, f64), DbError> {
        let limits = self.inner.parts.limits;
        let (parallel, budget) = self.inner.db.ultra_caps(task).await?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a token cap stays far below 2^52"
        )]
        let budget = budget.map_or(limits.budget, |b| b as f64);
        Ok((
            parallel
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(limits.parallel),
            budget,
        ))
    }

    /// Starts (or resumes) the worker on a task: with `resume_with` (the owner's answer, a
    /// new brief), the same session carries on with it.
    ///
    /// # Errors
    ///
    /// [`WorkerError::Refused`] when the task can't have a worker (tracked work, no brief, no
    /// CLI here, it couldn't start).
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn start(&self, id: TaskId, resume_with: Option<String>) -> Result<(), WorkerError> {
        let t = self.task(id).await?;
        if t.tracked {
            return Err(WorkerError::Refused(format!(
                "{} is tracked work (done elsewhere), not a worker's job",
                t.key()
            )));
        }
        if t.body.as_deref().is_none_or(|b| b.trim().is_empty()) {
            return Err(WorkerError::Refused(format!(
                "{} has no brief: give it a body first",
                t.key()
            )));
        }
        let Some(runtime) = self.inner.parts.runtime.clone() else {
            return Err(WorkerError::Refused(
                "this server runs no workers".to_owned(),
            ));
        };
        // Read before anything is reserved or begun: a failure here changes nothing.
        let provider = runtime.config.session_provider().await?;
        let Some(slot) = self.reserve(id) else {
            return Ok(());
        };
        let resume_with = resume_with.filter(|r| !r.trim().is_empty());
        let resume = t.session.is_some() && resume_with.is_some();
        let brief = self.brief(&t, resume_with.as_deref(), resume).await?;
        let token = self
            .inner
            .parts
            .tokens
            .for_task(id)
            .map_err(|error| WorkerError::Refused(error.to_string()))?;
        let agent = match self.guard().task_owner(id).await? {
            TaskOwner::Owner => None,
            TaskOwner::Agent(agent) if agent.enabled => Some(AgentWorker {
                shell: agent.worker_shell,
            }),
            TaskOwner::Agent(_) => {
                return Err(WorkerError::Refused(format!(
                    "{}'s agent is disabled: its tasks take no worker",
                    t.key()
                )));
            }
            TaskOwner::Orphaned => {
                return Err(WorkerError::Refused(format!(
                    "{}'s agent is gone: the task is no one's",
                    t.key()
                )));
            }
        };
        let deny_outward = self.guard().covered_by(id).await?.is_none();
        let folder = self.folder(id);
        if let Err(error) = tokio::fs::create_dir_all(&folder).await {
            return Err(WorkerError::Refused(format!(
                "its folder couldn't be made: {error}"
            )));
        }
        let ms = self.now().as_millisecond();
        let file = folder.join(format!("turn-{ms}.json"));
        let unit = format!("worker-{}-{ms}", id.get());
        let account = runtime.config.accounts.first().map(|a| a.id.clone());
        let session = if resume { t.session.clone() } else { None };
        if t.ultra {
            self.begin_run(id, session.clone(), account.clone(), resume)
                .await?;
        }
        let launch = runtime.config.launch(
            &WorkerLaunch {
                brief: brief.clone(),
                resume: session.clone(),
                ultra: t.ultra,
                deny_outward,
                model: t.model.clone(),
                folder: folder.clone(),
                account,
                agent,
            },
            token.as_str(),
            provider.as_ref(),
        );
        let cli = WorkerSession::start(runtime.spawner.as_ref(), &launch, &brief)
            .map_err(|error| WorkerError::Refused(error.to_string()))?;
        let change = TaskChange {
            status: Some(Status::Working),
            action: Some(None),
            decide_by: Some(None),
            ..TaskChange::default()
        };
        self.tasks()
            .update(id, change, Who::Assistant, None)
            .await?;
        let asked = resume_with
            .clone()
            .or_else(|| t.body.clone())
            .unwrap_or_else(|| t.title.clone());
        self.tasks().worker_started(id, &unit, &asked).await?;
        self.inner
            .store
            .set_worker(id, Some(unit.clone()), session.clone())
            .await
            .map_err(|error| WorkerError::Refused(error.to_string()))?;
        self.inner
            .db
            .set_turn_file(id, Some(&file.display().to_string()))
            .await?;
        let line = match resume_with.as_deref() {
            None => "A worker started on it",
            Some(with) if with.starts_with("The person") || with.starts_with("The owner") => {
                "Picked up again with the owner's answer"
            }
            Some(_) => "Picked up again with its new brief",
        };
        self.tasks().note(id, Who::Assistant, line, false).await?;
        self.inner
            .parts
            .activity
            .note(format!("Working on {}: {}", t.key(), t.title))
            .await?;
        let stop = CancellationToken::new();
        slot.run(Running {
            stop: stop.clone(),
            unit: unit.clone(),
            session,
            started: self.now(),
        });
        let me = self.clone();
        tokio::spawn(async move {
            let said = me.run(id, cli, file, stop).await;
            if let Err(error) = me.ended(id, &unit, &said).await {
                tracing::warn!(%error, task = id.get(), "the worker's end wasn't wrapped up");
            }
        });
        Ok(())
    }

    /// Reads a worker's stream until its result (or its stop): each line goes to its turn
    /// file, its session's id is kept, an ultra run's sub-agents are seen. Its answer.
    async fn run(
        &self,
        id: TaskId,
        mut cli: WorkerSession,
        file: PathBuf,
        stop: CancellationToken,
    ) -> String {
        let mut out = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .await
        {
            Ok(out) => Some(out),
            Err(error) => {
                tracing::warn!(%error, "a worker's turn file couldn't be written");
                None
            }
        };
        let mut said = String::new();
        loop {
            let line = tokio::select! {
                () = stop.cancelled() => break,
                line = cli.line() => line,
            };
            let Some(line) = line else { break };
            if let Some(out) = out.as_mut()
                && let Err(error) = out.write_all(format!("{line}\n").as_bytes()).await
            {
                tracing::warn!(%error, "a worker's line wasn't kept");
            }
            if let Err(error) = self.scan_line(id, &line).await {
                tracing::debug!(%error, "a worker's line wasn't read");
            }
            match Said::read(&line) {
                Said::Init { session_id } if !session_id.is_empty() => {
                    let unit = {
                        let mut held = self.live();
                        held.running.get_mut(&id.get()).map(|r| {
                            r.session = Some(session_id.clone());
                            r.unit.clone()
                        })
                    };
                    if let Some(unit) = unit
                        && let Err(error) = self
                            .inner
                            .store
                            .set_worker(id, Some(unit), Some(session_id.clone()))
                            .await
                    {
                        tracing::warn!(%error, "a worker's session wasn't kept");
                    }
                    if let Err(error) = self.lead_session(id, &session_id).await {
                        tracing::debug!(%error, "an ultra run's session wasn't kept");
                    }
                }
                Said::Result { text, .. } => {
                    said = text;
                    break;
                }
                _ => {}
            }
        }
        if let Some(mut out) = out
            && let Err(error) = out.flush().await
        {
            tracing::debug!(%error, "a worker's turn file wasn't flushed");
        }
        cli.close().await;
        said
    }

    /// A worker ended: its run closes, an ultra run's sub-agents still running are stopped,
    /// and the task goes on from what it was left at.
    async fn ended(&self, id: TaskId, unit: &str, said: &str) -> Result<(), WorkerError> {
        let ended = self.wrap_up(id, unit, said).await;
        let mut live = self.live();
        live.running.remove(&id.get());
        live.stopping.remove(&id.get());
        ended
    }

    async fn wrap_up(&self, id: TaskId, unit: &str, said: &str) -> Result<(), WorkerError> {
        let (was, stopped) = {
            let live = self.live();
            (
                live.running.get(&id.get()).cloned(),
                live.stopping.contains(&id.get()),
            )
        };
        self.tasks().worker_ended(unit).await?;
        if let Some(session) = was.as_ref().and_then(|w| w.session.clone()) {
            self.end_run(id, &session).await?;
        }
        if stopped {
            return Ok(());
        }
        let t = self.task(id).await?;
        if t.next.as_deref().is_some_and(|n| !n.trim().is_empty()) && t.status != Status::Canceled {
            // The owner wrote while it ran: the same session carries on with it at the tick.
            return Ok(());
        }
        if t.status == Status::Working {
            let said = said.replace('\0', "");
            let said = said.trim();
            let words: String = said.chars().take(600).collect();
            self.tasks()
                .note(
                    id,
                    Who::Worker,
                    &format!(
                        "Stopped without finishing: {}",
                        if words.is_empty() {
                            "no answer"
                        } else {
                            &words
                        }
                    ),
                    false,
                )
                .await?;
            self.tasks()
                .ask(
                    id,
                    AskInput {
                        question: "The worker stopped before finishing. Try again?".to_owned(),
                        options: vec![
                            OptionIn::Plain("Try again".to_owned()),
                            OptionIn::Plain("Leave it".to_owned()),
                        ],
                        action: Some("Say if the assistant should try again".to_owned()),
                        ..AskInput::default()
                    },
                    Who::Assistant,
                )
                .await?;
        }
        let t = self.task(id).await?;
        let last = t.last.clone().unwrap_or_else(|| "null".to_owned());
        let (kind, text) = if t.status == Status::Waiting {
            let question = t
                .decision
                .as_deref()
                .and_then(|d| serde_json::from_str::<emberhut_api::tasks::Decision>(d).ok())
                .map_or_else(|| last.clone(), |d| d.question);
            (
                "ask",
                format!(
                    "{} \"{}\" needs the owner: {}. {question}. Tell them in one line (the app shows the action and the buttons).",
                    t.key(),
                    t.title,
                    t.action.as_deref().unwrap_or("null")
                ),
            )
        } else if t.status == Status::Done && t.priority == 1 {
            (
                "ask",
                format!(
                    "{} \"{}\" is ready: {last}. Send it to the owner now, short.",
                    t.key(),
                    t.title
                ),
            )
        } else {
            (
                "work",
                format!(
                    "{} \"{}\" is {}: {last}. Mention it in the next brief if it matters.",
                    t.key(),
                    t.title,
                    t.status.as_str()
                ),
            )
        };
        self.tell(id, kind, text, || agent_news(&t, &last), None)
            .await?;
        Ok(())
    }

    /// Stops the worker on a task: its session ends (one gone already is no matter). The
    /// task's side (its run, its status) is the tasks' own.
    pub fn stop(&self, id: TaskId) {
        let mut live = self.live();
        if let Some(running) = live.running.get(&id.get()) {
            running.stop.cancel();
            live.stopping.insert(id.get());
        }
    }

    /// The tick's work: the tasks the owner answered (or gave a new brief) resume first, with
    /// it; then every assistant task with a brief and no worker starts; one that sat unstarted
    /// in to do for three minutes with room for it is the assistant's to look at (once). None
    /// of it while the agent has nothing to think with
    /// ([`crate::services::claude::worker::WorkerConfig::thinks`]).
    ///
    /// # Errors
    ///
    /// [`WorkerError::Db`].
    pub async fn tick(&self) -> Result<(), WorkerError> {
        if let Err(error) = self.sweep().await {
            tracing::warn!(%error, "the sub-agents' sweep failed");
        }
        if !self.may_start().await? {
            return Ok(());
        }
        let now = self.now();
        let open = self.inner.store.open().await?;
        let mut free = self.inner.parts.max_working.map_or(usize::MAX, |max| {
            max.saturating_sub(self.live().running.len())
        });
        let candidate = |t: &&Task| {
            t.owner == Owner::Assistant
                && t.status == Status::Todo
                && !t.tracked
                && !t.finding
                && t.worker.is_none()
                && t.body.as_deref().is_some_and(|b| !b.trim().is_empty())
                && !self.is_running(t.id)
        };
        // A disabled agent's task, or no one's, takes no worker: it is left out before its
        // queued message is taken, so the message waits for the agent to be enabled again.
        let startable = self.startable(open.iter().filter(candidate)).await?;
        let ours = |t: &&Task| startable.contains(&t.id.get()) && !self.is_running(t.id);
        let mut resumed = Vec::new();
        for t in open.iter().filter(ours).filter(|t| t.next.is_some()) {
            if free == 0 {
                break;
            }
            let next = self.inner.db.take_next(t.id).await?;
            match self.start(t.id, next).await {
                Ok(()) => free -= 1,
                Err(error) => tracing::info!(%error, task = t.id.get(), "a worker didn't start"),
            }
            resumed.push(t.id);
        }
        let parked = |t: &Task| {
            t.parked_until.as_deref().is_some_and(|p| {
                p.parse::<Timestamp>().map_or_else(
                    |_| {
                        p.parse::<jiff::civil::Date>().is_ok_and(|d| {
                            d > now.to_zoned_datetime(self.tasks().zone().clone()).date()
                        })
                    },
                    |at| at > now,
                )
            })
        };
        let mut fresh: Vec<&Task> = open
            .iter()
            .filter(ours)
            .filter(|t| t.session.is_none() && !resumed.contains(&t.id) && !parked(t))
            .collect();
        fresh.sort_by_key(|t| (!t.today, t.priority, t.due, t.id));
        for t in fresh {
            if free == 0 {
                break;
            }
            match self.start(t.id, None).await {
                Ok(()) => free -= 1,
                Err(error) => tracing::info!(%error, task = t.id.get(), "a worker didn't start"),
            }
        }
        if free > 0 {
            for t in open.iter().filter(ours).filter(|t| !parked(t)) {
                if now.duration_since(t.updated) < UNSTARTED || self.is_running(t.id) {
                    continue;
                }
                let key = (t.id.get(), t.updated.as_millisecond());
                if !self.live().warned.insert(key) {
                    continue;
                }
                let before = if t.session.is_some() {
                    " (it had a worker before)"
                } else {
                    ""
                };
                self.tell(
                    t.id,
                    "urgent",
                    format!(
                        "{} \"{}\" has a brief and has sat in to do with a worker slot free, and no worker started on it{before}. Start it (task_work), give it a new brief (task_update body), park it, or hand it to the owner.",
                        t.key(),
                        t.title
                    ),
                    || {
                        format!(
                            "Your task {} \"{}\" has not started: start it again with task_send, \
                             or cancel it.",
                            t.key(),
                            t.title
                        )
                    },
                    Some(format!("unstarted:{}:{}", t.id.get(), key.1)),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Whether the tick may start workers: a runtime is configured and the agent has something
    /// to think with (otherwise the tasks wait in to do for the first tick it can).
    async fn may_start(&self) -> Result<bool, WorkerError> {
        Ok(match &self.inner.parts.runtime {
            Some(runtime) => runtime.config.thinks().await?,
            None => false,
        })
    }

    /// Runs [`Self::tick`] every `every` until `stop`; a tick in flight is dropped at `stop`.
    /// On `stop`, the workers running are told to end; [`Self::close_all`] waits for them.
    pub async fn keep_ticking(self, every: Duration, stop: CancellationToken) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                _ = ticks.tick() => {}
            }
            tokio::select! {
                () = stop.cancelled() => break,
                ticked = self.tick() => {
                    if let Err(error) = ticked {
                        tracing::warn!(%error, "the workers' tick failed");
                    }
                }
            }
        }
        self.stop_all();
    }

    /// Tells every worker running to end, and starts no more. Its CLI's process group gets
    /// `SIGTERM`, and `SIGKILL` after the spawner's grace.
    fn stop_all(&self) {
        let mut live = self.live();
        live.closed = true;
        for running in live.running.values() {
            running.stop.cancel();
        }
    }

    /// Ends every worker as the server stops: none starts any more, each one running is
    /// stopped (its CLI's whole process group gets `SIGTERM`, and `SIGKILL` once the spawner's
    /// grace is over), and this waits up to `grace` for each to exit and be wrapped up. A
    /// start under way when it is called is waited for and stopped too. Give `grace` more than
    /// the spawner's ([`crate::services::claude::STOP_GRACE`]), so the kill lands before
    /// the server goes.
    pub async fn close_all(&self, grace: Duration) {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            self.stop_all();
            let left = {
                let live = self.live();
                live.running.len() + live.starting.len()
            };
            if left == 0 {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(left, ?grace, "workers still running after the grace period");
                return;
            }
            tokio::time::sleep(CLOSE_POLL).await;
        }
    }

    /// The worker session `w-<task>`'s head, as the app shows it.
    async fn session_head(&self, task: i64) -> Option<Map<String, Value>> {
        let id = TaskId::new(task);
        let t = self.inner.store.get(id).await.ok()??;
        let running = self.live().running.get(&task).cloned();
        let lead = self.live().leads.get(&task).cloned().unwrap_or_default();
        let last_run = self
            .lead_of(id, true)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let status = if running.is_some() {
            "running"
        } else {
            match last_run.get("state").and_then(Value::as_str) {
                Some("failed") => "failed",
                Some("stopped") => "interrupted",
                _ => "completed",
            }
        };
        let since = running.as_ref().map(|r| {
            crate::convert::timestamp_to_api(r.started)
                .as_str()
                .to_owned()
        });
        let head = json!({
            "id": format!("w-{task}"),
            "title": format!("{} {}", t.key(), t.title),
            "status": status,
            "running": running.is_some(),
            "doing": if running.is_some() { lead.doing } else { None },
            "since": since.or_else(|| last_run.get("started").and_then(Value::as_str).map(str::to_owned)),
            "place": "this server",
            "tasks": [task],
            "project": t.project,
            "cwd": Value::Null,
        });
        match head {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }
}

impl TaskWorkers for WorkerService {
    fn start(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            Self::start(self, task, None)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn stop(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Self::stop(self, task);
        Box::pin(async { Ok(()) })
    }
}

impl WorkerSessions for WorkerService {
    fn head(&self, task: i64) -> BoxFuture<'_, Option<Map<String, Value>>> {
        Box::pin(self.session_head(task))
    }

    fn steps<'a>(
        &'a self,
        task: i64,
        after: u64,
        agent: Option<&'a str>,
    ) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let dir = self.folder(TaskId::new(task));
            let agent = agent.map(str::to_owned);
            tokio::task::spawn_blocking(move || steps::more(task, &dir, after, agent.as_deref()))
                .await
                .unwrap_or_default()
        })
    }

    fn message(
        &self,
        task: i64,
        text: String,
    ) -> BoxFuture<'_, Result<Map<String, Value>, String>> {
        Box::pin(async move {
            let reply = self
                .tasks()
                .message(TaskId::new(task), &text, &[])
                .await
                .map_err(|e| e.to_string())?;
            match serde_json::to_value(reply) {
                Ok(Value::Object(mut map)) => {
                    map.remove("ok");
                    Ok(map)
                }
                _ => Ok(Map::new()),
            }
        })
    }

    fn stop(&self, task: i64) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(error) = self.tasks().stop_worker(TaskId::new(task)).await {
                tracing::warn!(%error, task, "the worker didn't stop");
            }
        })
    }
}

/// A task's worker slot, taken for a start ([`WorkerService::reserve`]): given back when it is
/// dropped, unless the worker runs ([`Slot::run`]).
#[derive(Debug)]
struct Slot {
    workers: WorkerService,
    task: i64,
    /// Still the start's: given back when dropped.
    held: bool,
}

impl Slot {
    /// The worker runs: it holds the task's slot from now on. A server that started stopping
    /// meanwhile stops it at once.
    fn run(mut self, running: Running) {
        let mut live = self.workers.live();
        if live.closed {
            running.stop.cancel();
        }
        live.starting.remove(&self.task);
        live.running.insert(self.task, running);
        self.held = false;
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.held {
            self.workers.live().starting.remove(&self.task);
        }
    }
}

/// The workers, wired into the tasks after both are built: the tasks are built first and
/// reach the workers through this.
#[derive(Clone, Debug, Default)]
pub struct LateWorkers(Arc<std::sync::OnceLock<WorkerService>>);

impl LateWorkers {
    /// Hands it the workers.
    pub fn set(&self, workers: WorkerService) {
        if self.0.set(workers).is_err() {
            tracing::warn!("the workers were wired in already");
        }
    }
}

impl TaskWorkers for LateWorkers {
    fn start(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.0.get() {
                Some(workers) => TaskWorkers::start(workers, task).await,
                None => Err("this server runs no workers".to_owned()),
            }
        })
    }

    fn stop(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.0.get() {
                Some(workers) => TaskWorkers::stop(workers, task).await,
                None => Ok(()),
            }
        })
    }
}

/// What a configured agent is told when its task's worker stops: the task, its state and last
/// line, and what to do with it (never the owner's words).
fn agent_news(t: &Task, last: &str) -> String {
    let state = if t.status == Status::Waiting {
        "asks something"
    } else {
        t.status.as_str()
    };
    format!(
        "Your task {} \"{}\" {state}: {last}. Read it (tasks with its id), then tell the person \
         who asked, in their thread, with agent_reply, or carry it on with task_send.",
        t.key(),
        t.title
    )
}
