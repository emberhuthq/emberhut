//! The runners, as the assistant drives them: `thread_start` (a session for a piece of work,
//! followed), `session_read`, `session_send`, `session_send_file` and `session_stop` on a
//! runner session (`r-…`), `machines` (what each runner machine has left) and the account tool
//! ([`emberhut_api::wire::mcp::ACCOUNT_TOOL`]: the account the assistant runs on, and switching it
//! when the user asks).
//!
//! A session's id from a tool's arguments goes to a runner only as a
//! [`crate::services::runners::SessionId`], so it can't name another of the runner's
//! endpoints.
//!
//! `thread_start` takes the folder and machine it is given, names the task in the session's
//! meta and follows the session; `session_send`'s `task_id` is ignored. On a server that keeps
//! plans, every session works for a task: `thread_start` with no `task_id` works for its parent
//! session's tasks (`parent`, else the calling session), else for a task made for it.

use emberhut_api::wire::session_meta;
use emberhut_core::task::session as task_session;
use emberhut_core::task::{Owner, TaskId};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::runner_api::by;
use crate::services::runners::{SessionId, WorkSession};
use crate::services::tasks::SessionTask;
use crate::services::transfer::mb;

/// What each runner machine has left.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Machines;

/// `machines`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct MachinesArgs {
    /// Only this machine.
    #[garde(length(max = 64))]
    machine: Option<String>,
    /// Its last 24 h too, in 5-minute points.
    #[serde(default)]
    #[garde(skip)]
    history: bool,
}

impl Tool for Machines {
    type Args = MachinesArgs;

    fn name(&self) -> &'static str {
        "machines"
    }

    fn description(&self) -> &'static str {
        "How much power is left on each of the user's runner machines, live from their runners \
         every 20 s: state (ok, warn, low: no new sessions placed there, stale, offline, \
         unknown: a runner that sends no samples), CPU load and cores, memory available (and \
         pressure), each GPU's use and memory and which processes hold it, free disk, battery, \
         the runner's sessions, agent processes and memory cap, its warnings, and the sessions \
         on it with their account's use. thread_start skips a machine that's offline or low \
         and says why. machine: only that one; history: true adds its last 24 h in 5-minute \
         points."
    }

    async fn call(&self, ctx: &ToolContext, args: MachinesArgs) -> Result<ToolOutput, ToolError> {
        let machines = ctx.app.machines();
        let only = args
            .machine
            .as_deref()
            .filter(|machine| !machine.is_empty());
        let view = machines
            .view(args.history)
            .await
            .map_err(ToolError::failed)?;
        let listed: Vec<_> = view
            .machines
            .into_iter()
            .filter(|machine| only.is_none_or(|only| machine.id.as_str() == only))
            .collect();
        if let Some(only) = only
            && listed.is_empty()
        {
            return Ok(ToolOutput::text(format!(
                "No runner machine {only} (there are {}).",
                ctx.app.runners().machines().join(", ")
            )));
        }
        let summary = machines.summaries(only).await.map_err(ToolError::failed)?;
        ToolOutput::json(&json!({
            "summary": summary,
            "machines": listed,
            "thresholds": view.thresholds,
        }))
    }
}

/// The account the assistant runs on, and switching it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AssistantAccount;

/// The account tool's arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AccountArgs {
    /// An account id, or auto.
    #[garde(length(max = 128))]
    account: Option<String>,
}

impl Tool for AssistantAccount {
    type Args = AccountArgs;

    fn name(&self) -> &'static str {
        emberhut_api::wire::mcp::ACCOUNT_TOOL
    }

    fn description(&self) -> &'static str {
        "The account you yourself run on (the main room and the others), and switching it when \
         the user asks. account: an account id pins you there until they set auto again; \
         \"auto\" goes back to the account with the most room. Leave it out to only look. You \
         move between turns: say in one line where you'll be."
    }

    async fn call(&self, ctx: &ToolContext, args: AccountArgs) -> Result<ToolOutput, ToolError> {
        let accounts = ctx.app.accounts();
        let Some(account) = args.account else {
            let mine = accounts.assistant().await.map_err(ToolError::failed)?;
            let usage = accounts.usage_card().await.map_err(ToolError::failed)?;
            let mut answer = serde_json::to_value(mine).map_err(ToolError::failed)?;
            answer["accounts"] = serde_json::to_value(usage).map_err(ToolError::failed)?;
            return ToolOutput::json(&answer);
        };
        match accounts.use_account(Some(&account)).await {
            Ok(mine) => ToolOutput::json(&mine),
            Err(crate::services::accounts::AccountError::Refused(why)) => {
                Ok(ToolOutput::text(why.to_string()))
            }
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// A runner session's last steps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionRead;

/// `session_read`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SessionReadArgs {
    /// The session's id (`r-…`).
    #[garde(length(min = 1, max = 128))]
    thread: String,
    /// How many steps (default 60).
    #[garde(range(min = 5, max = 400))]
    #[schema(minimum = 5, maximum = 400)]
    limit: Option<u32>,
}

impl Tool for SessionRead {
    type Args = SessionReadArgs;

    fn name(&self) -> &'static str {
        "session_read"
    }

    fn description(&self) -> &'static str {
        "Read a session on one of your runners (thread id r-…), yours or one the user opened \
         themselves (direct): what it was asked, said and ran, its last steps first cut. \
         limit: how many steps (default 60)."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: SessionReadArgs,
    ) -> Result<ToolOutput, ToolError> {
        let limit = args.limit.map_or(60, |limit| limit as usize);
        let read = async {
            let id = SessionId::parse(&args.thread)?;
            ctx.app.runners().read(&id, limit).await
        };
        Ok(match read.await {
            Ok(text) => ToolOutput::text(text),
            Err(error) => ToolOutput::text(format!("Couldn't read it: {error}")),
        })
    }
}

/// Writes into a runner session, as its next turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionSend;

/// `session_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SessionSendArgs {
    /// The session's id (`r-…`).
    #[garde(length(min = 1, max = 128))]
    thread: String,
    /// What to write.
    #[garde(length(min = 1))]
    message: String,
}

impl Tool for SessionSend {
    type Args = SessionSendArgs;

    fn name(&self) -> &'static str {
        "session_send"
    }

    fn description(&self) -> &'static str {
        "Write into a session on one of your runners (thread id r-…), above all one the user \
         opened themselves (direct in threads): to pass on what they asked you to tell it, \
         answer its question for them when they told you the answer, or relay news it needs. \
         It's its next turn (queued behind a running one), shown in their session as from you."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: SessionSendArgs,
    ) -> Result<ToolOutput, ToolError> {
        let runners = ctx.app.runners();
        let sent = async {
            let id = SessionId::parse(&args.thread)?;
            let machine = runners.machine_of(&id).await?;
            let direct = runners
                .session(id.as_str())
                .await?
                .is_some_and(|thread| thread.direct);
            let answer = runners
                .send_turn(&machine, &id, &args.message, by::ASSISTANT, &[])
                .await?;
            Ok::<_, crate::services::runners::SessionError>((direct, answer))
        }
        .await;
        Ok(match sent {
            Ok((direct, answer)) => {
                let whose = if direct {
                    "the user's own session"
                } else {
                    "runner session"
                };
                let title = runners
                    .session(&args.thread)
                    .await
                    .ok()
                    .flatten()
                    .map_or_else(|| args.thread.clone(), |thread| thread.title);
                if let Err(error) = ctx
                    .app
                    .activity()
                    .note(format!("Wrote in {whose} {title}"))
                    .await
                {
                    tracing::warn!(%error, "an activity line could not be written");
                }
                let queued = if truthy(answer.get("queued")) {
                    " (queued behind the running turn)"
                } else {
                    ""
                };
                ToolOutput::text(format!("Sent to {whose} {}{queued}.", args.thread))
            }
            Err(error) => ToolOutput::text(format!(
                "Not sent: {error}. Check the session's id with the threads tool."
            )),
        })
    }
}

/// Puts a file into a runner session's inbox.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionSendFile;

/// `session_send_file`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SessionSendFileArgs {
    /// The session's id (`r-…`).
    #[garde(length(min = 1, max = 128))]
    thread: String,
    /// The file on this machine.
    #[garde(length(min = 1, max = 4096))]
    path: String,
}

impl Tool for SessionSendFile {
    type Args = SessionSendFileArgs;

    fn name(&self) -> &'static str {
        "session_send_file"
    }

    fn description(&self) -> &'static str {
        "Put a file into a session on one of your runners (thread id r-…): a design, a CSV, a \
         screenshot the user sent you (its path in the uploads folder), anything on this \
         machine in the home folder or /tmp (never secrets). It lands in that session's inbox \
         folder on its machine (the answer says where); the session is told on its next turn, \
         so send it one with session_send if it's idle (name the path). 200 MB at most."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: SessionSendFileArgs,
    ) -> Result<ToolOutput, ToolError> {
        let runners = ctx.app.runners();
        let Ok(id) = SessionId::parse(&args.thread) else {
            return Ok(ToolOutput::text(format!(
                "Not sent: {} is not a runner session's id",
                args.thread
            )));
        };
        let machine = match runners.session(id.as_str()).await {
            Ok(Some(thread)) => thread.machine,
            Ok(None) => {
                return Ok(ToolOutput::text(format!(
                    "Not sent: no session {} on your runners",
                    args.thread
                )));
            }
            Err(error) => return Err(ToolError::failed(error)),
        };
        Ok(
            match ctx
                .app
                .transfer()
                .push(&machine, &id, &args.path, by::ASSISTANT)
                .await
            {
                Ok(pushed) => ToolOutput::text(format!(
                    "Landed at {} ({}) on the {}, in session {}'s inbox; it's told on its next \
                     turn.",
                    pushed.path,
                    mb(pushed.size),
                    runners.place(&pushed.machine),
                    args.thread
                )),
                Err(error) => ToolOutput::text(format!("Not sent: {error}")),
            },
        )
    }
}

/// Stops a runner session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionStop;

/// `session_stop`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SessionStopArgs {
    /// The session's id (`r-…`).
    #[garde(length(min = 1, max = 128))]
    thread: String,
    /// Also drop the turns queued behind it, and close it.
    #[serde(default)]
    #[garde(skip)]
    stop: bool,
}

impl Tool for SessionStop {
    type Args = SessionStopArgs;

    fn name(&self) -> &'static str {
        "session_stop"
    }

    fn description(&self) -> &'static str {
        "Stop a session on one of your runners (thread id r-…): its running turn ends now. \
         stop: true also drops the turns queued behind it and closes it (a later session_send \
         carries it on). Only when the user asks, or it's clearly going wrong."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: SessionStopArgs,
    ) -> Result<ToolOutput, ToolError> {
        let halted = async {
            let id = SessionId::parse(&args.thread)?;
            ctx.app.runners().halt(&id, args.stop).await
        };
        Ok(match halted.await {
            Ok(answer) if truthy(answer.get("running")) => ToolOutput::text("Stopping it now."),
            Ok(_) => ToolOutput::text("Nothing was running."),
            Err(error) => ToolOutput::text(format!("Couldn't stop it: {error}")),
        })
    }
}

/// Starts a session on a runner for a piece of work, and follows it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ThreadStart;

/// `thread_start`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ThreadStartArgs {
    /// The task the work is for. Where tasks carry plans it may be left out: the session then
    /// works for its parent session's task, else for a new task made for it.
    #[garde(inner(range(min = 1)))]
    task_id: Option<i64>,
    /// The session that starts this one (`r-…`), where tasks carry plans: its task is this
    /// one's when `task_id` is left out (default: the session calling, when it names itself).
    #[garde(length(max = 64))]
    parent: Option<String>,
    /// The session's title.
    #[garde(length(min = 1, max = 200))]
    title: String,
    /// The full brief, as you'd write it to a colleague.
    #[garde(length(min = 1))]
    message: String,
    /// The runner machine (default: the first one).
    #[garde(length(max = 64))]
    machine: Option<String>,
    /// The folder on that machine.
    #[garde(length(max = 4096))]
    cwd: Option<String>,
    /// `true` for a git worktree of its own, or a branch name.
    #[garde(skip)]
    #[schema(value_type = Object)]
    worktree: Option<Value>,
    /// An account id, only to insist on one (the runner picks the one with the most room).
    #[garde(length(max = 128))]
    account: Option<String>,
    /// low, medium, high, xhigh or max (default high).
    #[garde(custom(effort_word))]
    effort: Option<String>,
    /// A model id, only when the user asks for one.
    #[garde(length(max = 128))]
    model: Option<String>,
    /// Start on this machine even though it's low or offline; only when the user asks.
    #[serde(default)]
    #[garde(skip)]
    force: bool,
}

impl Tool for ThreadStart {
    type Args = ThreadStartArgs;

    fn name(&self) -> &'static str {
        "thread_start"
    }

    fn description(&self) -> &'static str {
        "Start a session on one of your runners for a task's work (code, repos, work the user \
         may want to carry on themselves), followed, so its end reaches you at once; the user \
         reads and answers it in the app. task_id: the task it works for; where tasks carry \
         plans it may be left out, and the session then works for its parent session's task \
         (parent: the session starting it, by default the session calling) or else a new task \
         made for it. message: the full brief, as you'd write it to a \
         colleague. machine: the runner (machines tool; default the first); cwd: the folder \
         there. worktree: true for a git worktree of its own (or a branch name), when the work \
         shouldn't touch the main checkout. account: the runner picks the account with the \
         most room and moves the session when one runs out; pass an id only to insist. effort: \
         low…max (default high). model: only when the user asks for one. A machine that's \
         offline or low is skipped (force: true starts there anyway, only when the user asks)."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: ThreadStartArgs,
    ) -> Result<ToolOutput, ToolError> {
        let runners = ctx.app.runners();
        let Some(machine) = args
            .machine
            .clone()
            .filter(|machine| !machine.is_empty())
            .or_else(|| runners.machines().into_iter().next())
        else {
            return Ok(ToolOutput::text(
                "Not started: no runner is set up on this server.",
            ));
        };
        if !runners.is_runner(&machine) {
            return Ok(ToolOutput::text(format!(
                "Not started: no runner called {machine} (there are {}).",
                runners.machines().join(", ")
            )));
        }
        if !args.force
            && let Err(why) = ctx
                .app
                .machines()
                .can_place(&machine)
                .await
                .map_err(ToolError::failed)?
        {
            return Ok(ToolOutput::text(format!(
                "Not started: the {} ({machine}) was skipped: {why}. Wait and try again \
                 (machines tool), start it elsewhere with machine and cwd, or pass force: true \
                 to start there anyway.",
                runners.place(&machine)
            )));
        }
        let Some(cwd) = args.cwd.clone().filter(|cwd| !cwd.is_empty()) else {
            return Ok(ToolOutput::text(format!(
                "Not started: pass cwd, the folder on {machine} the work is in."
            )));
        };
        let worktree = match args.worktree {
            Some(Value::Bool(true)) => Some(Value::Bool(true)),
            Some(Value::String(branch)) if !branch.is_empty() => Some(json!({ "branch": branch })),
            _ => None,
        };
        let plans = ctx.app.task_plans();
        let parent = plans
            .then(|| {
                args.parent
                    .clone()
                    .filter(|parent| !parent.is_empty())
                    .or_else(|| ctx.session.clone())
            })
            .flatten();
        let work = match args.task_id {
            Some(task) => Work::Named(TaskId::new(task)),
            None if !plans => {
                return Ok(ToolOutput::text(
                    "Not started: pass task_id, the task the work is for.",
                ));
            }
            None => {
                let place = runners.place(&machine);
                work_for(ctx, parent.as_deref(), &args.title, &args.message, &place).await?
            }
        };
        let mut meta = Map::new();
        if let Work::Named(task) | Work::Made(task) = work {
            meta.insert(session_meta::TASK.to_owned(), json!(task.get()));
        }
        let why: String = args.title.chars().take(80).collect();
        let started = runners
            .start_work(
                WorkSession {
                    machine: machine.clone(),
                    cwd,
                    project: None,
                    title: args.title.clone(),
                    message: brief(ctx, work.task(), args.message),
                    account: args.account.filter(|account| !account.is_empty()),
                    model: args.model.filter(|model| !model.is_empty()),
                    effort: args.effort,
                    worktree,
                    meta,
                    parent,
                },
                &why,
            )
            .await;
        if let Work::Made(task) = work {
            made_for(ctx, task, &started).await?;
        }
        let opened = match started {
            Ok(opened) => opened,
            Err(error) => return Ok(ToolOutput::text(format!("Not started: {error}"))),
        };
        started_line(ctx, &machine, &args.title, &opened, work).await
    }
}

/// Marks `task`, made for the session `thread_start` just started, as made for it; with no
/// session started, puts it away.
async fn made_for(
    ctx: &ToolContext,
    task: TaskId,
    started: &Result<crate::services::runners::Opened, crate::services::runners::SessionError>,
) -> Result<(), ToolError> {
    match started {
        Ok(opened) => ctx
            .app
            .tasks()
            .made_for_session(task, &opened.id)
            .await
            .map_err(ToolError::failed),
        Err(error) => {
            ctx.app.runners().not_started(task, error).await;
            Ok(())
        }
    }
}

/// `thread_start`'s answer for the session `opened` on `machine`, titled `title`, working for
/// `work`; the activity log says it started.
async fn started_line(
    ctx: &ToolContext,
    machine: &str,
    title: &str,
    opened: &crate::services::runners::Opened,
    work: Work,
) -> Result<ToolOutput, ToolError> {
    let runners = ctx.app.runners();
    let account = runners
        .account_name(machine, opened.account.as_deref())
        .await
        .map_err(ToolError::failed)?
        .unwrap_or_else(|| "null".to_owned());
    let model = opened
        .model
        .as_deref()
        .map_or_default(|model| format!(", {model}"));
    let branch = opened
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .map_or_default(|branch| format!(" on branch {branch}"));
    let place = runners.place(machine);
    if let Err(error) = ctx
        .app
        .activity()
        .note(format!("Started a session on the {place}: \"{title}\""))
        .await
    {
        tracing::warn!(%error, "an activity line could not be written");
    }
    let task = match work {
        Work::Named(_) => String::new(),
        Work::Inherited(task) => format!(" It works for {}, its parent's task.", task.key()),
        Work::Made(task) => format!(" It works for {}, a task made for it.", task.key()),
    };
    Ok(ToolOutput::text(format!(
        "Session {} started on the runner on the {place} ({account}{model}) in {}{branch}, \
         followed. The user can read it and reply in the app.{task}",
        opened.id,
        opened.cwd.as_deref().unwrap_or_default()
    )))
}

/// The task a session `thread_start` starts works for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Work {
    /// The one the call named.
    Named(TaskId),
    /// Its parent session's (the first, when it has several: the session is linked to each
    /// when its runner first reports it).
    Inherited(TaskId),
    /// One made for it now.
    Made(TaskId),
}

impl Work {
    /// The task, as its brief names it.
    fn task(self) -> TaskId {
        match self {
            Self::Named(task) | Self::Inherited(task) | Self::Made(task) => task,
        }
    }
}

/// The task a session started with no task named works for: its parent session's, else a new
/// one made for it, the assistant's, titled from its title (else its brief; else after
/// `place`, where it runs).
async fn work_for(
    ctx: &ToolContext,
    parent: Option<&str>,
    title: &str,
    message: &str,
    place: &str,
) -> Result<Work, ToolError> {
    let tasks = ctx.app.tasks();
    if let Some(parent) = parent
        && let Some(task) = tasks
            .session_tasks(parent)
            .await
            .map_err(ToolError::failed)?
            .first()
    {
        return Ok(Work::Inherited(*task));
    }
    let title = Some(title.trim())
        .filter(|title| !title.is_empty() && title.chars().count() <= task_session::TITLE_MAX)
        .map(str::to_owned)
        .or_else(|| task_session::title_from(title))
        .or_else(|| task_session::title_from(message))
        .unwrap_or_else(|| task_session::session_title("", place));
    let made = tasks
        .make_session_task(SessionTask {
            title,
            owner: Owner::Assistant,
            why: task_session::MADE_FOR_SESSION.to_owned(),
            session: None,
        })
        .await
        .map_err(ToolError::failed)?;
    Ok(Work::Made(made.id))
}

/// A work session's brief: `message`, and on a server that keeps plans, what the session is
/// asked about its task's plan.
fn brief(ctx: &ToolContext, task: TaskId, message: String) -> String {
    if ctx.app.task_plans() {
        format!(
            "{message}\n\n{}",
            crate::services::tasks::session_rule(&task.key())
        )
    } else {
        message
    }
}

/// The efforts a session takes.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `effort` is one of [`EFFORTS`], when given.
#[expect(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "garde's signature for a custom rule"
)]
fn effort_word(effort: &Option<String>, _context: &()) -> garde::Result {
    match effort {
        Some(word) if !EFFORTS.contains(&word.as_str()) => {
            Err(garde::Error::new("not low, medium, high, xhigh or max"))
        }
        _ => Ok(()),
    }
}

/// Whether a JSON value is truthy, as JavaScript reads it.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(on)) => *on,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}
