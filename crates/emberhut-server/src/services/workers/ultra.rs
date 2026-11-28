//! Ultra runs: one task's run splits a big job across many sub-agents in parallel, a verify
//! agent per part ([`emberhut_core::ultra`]'s rules). The task's worker is the lead:
//!
//! - it plans first (`ultra_plan`): parts with sizes, agents, tokens, accounts. The plan goes
//!   on the task; a big one (over the caps that ask) waits for the owner's OK, unless the
//!   owner's own OK already covers the run;
//! - every sub-agent is tracked from the CLI's own messages about its background tasks
//!   (`task_started`, `task_progress`, `task_notification`, `task_updated`) in the lead's
//!   stream (`agents`): its name, part, state, last line and tokens;
//! - a gate before every new sub-agent and shell command (the lead's hook, `POST /ultra/gate`)
//!   holds the plan, the parallel cap and the spending cap, and the outward rule; past the cap
//!   no new sub-agent starts and the owner is asked;
//! - parts can run as their own sessions on a runner, on other accounts with room (lanes,
//!   `ultra_lane`), and the lead waits for them (`ultra_wait`).
//!
//! The run is kept on the task (`issues.ultra_run`).

use std::collections::BTreeMap;
use std::time::Duration;

use emberhut_api as api;
use emberhut_core::guard::go;
use emberhut_core::task::ask::OptionIn;
use emberhut_core::task::{TaskId, Who};
use emberhut_core::ultra::{self, AgentKind, Part, Seen, Size};
use emberhut_db::{Agent, AgentSeen, DbError, StreamPlace};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{WorkerError, WorkerService};
use crate::convert::timestamp_to_api;
use crate::services::guard::agents::AgentCheck;
use crate::services::runners::WorkSession;
use crate::services::tasks::AskInput;

/// One part of a plan, as the run keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPart {
    /// Its number.
    pub n: f64,
    /// A short name.
    pub name: String,
    /// `S`, `M` or `L`.
    pub size: String,
    /// `false`: it needs no verify agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<bool>,
}

/// An ultra run's plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Its parts.
    pub parts: Vec<PlanPart>,
    /// Its sub-agents.
    pub agents: f64,
    /// Its tokens.
    pub tokens: f64,
    /// The accounts it runs on.
    pub accounts: Vec<String>,
    /// A note.
    pub note: Option<String>,
    /// When it was posted.
    pub at: String,
}

/// What the lead has done so far: `planning` (no plan yet), `asking` (a big plan waits for the
/// owner), `running`, `capped` (the spending cap stopped new sub-agents; the owner is asked).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    /// Where the run is.
    pub state: String,
    /// Its plan.
    pub plan: Option<Plan>,
    /// Its spending cap.
    pub budget: f64,
    /// Sub-agents at a time.
    pub parallel: u32,
    /// The lead's session.
    pub session: Option<String>,
    /// The lead's account.
    pub account: Option<String>,
    /// The sub-agents the gate let start, by their tool call.
    #[serde(default)]
    pub admitted: Vec<String>,
    /// What the lead's turns cost.
    pub cost: Option<f64>,
    /// The lead's own tokens.
    pub lead_tokens: Option<f64>,
    /// Each turn file's cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub costs: Option<BTreeMap<String, f64>>,
    /// The owner's OK for the run, relayed by the assistant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<String>,
    /// The owner's OK on this run's plan: a re-plan within the cap starts by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared: Option<String>,
    /// The outward command the owner was asked about for the lead, while that card is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outward_ask: Option<String>,
}

/// `ultra_plan`'s input.
#[derive(Clone, Debug, Default)]
pub struct PlanInput {
    /// The parts.
    pub parts: Vec<PlanPart>,
    /// Sub-agents, when the lead counts them.
    pub agents: Option<f64>,
    /// Tokens, when the lead estimates them.
    pub tokens: Option<f64>,
    /// The accounts.
    pub accounts: Option<Vec<String>>,
    /// A note.
    pub note: Option<String>,
}

/// What became of a plan: cleared, or put to the owner (`big` or `over` its cap).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanResult {
    /// It is cleared to start.
    pub ok: bool,
    /// The plan.
    pub plan: Plan,
    /// Why it waits: `big` or `over`.
    pub why: Option<&'static str>,
}

/// The ultra card's question openings: the plan's, the cap's, the outward step's.
const PLAN_CARD: &str = "This ultra run plans";
const CAP_CARD: &str = "The ultra run used";
const OUTWARD_CARD: &str = "The ultra run's lead wants to push, merge or deploy:";
/// Why a lane didn't start while the task's outward steps aren't covered.
const LANE_UNGATED: &str = "Not started: a lane runs on a runner without the gate, so it starts only once the owner's words cover this task's push, merge and deploy steps. Run this part yourself (a sub-agent in your own session).";
/// The outward step's card for a worker that doesn't lead an ultra run.
const WORKER_OUTWARD_CARD: &str = "The worker wants to push, merge or deploy:";
/// How long the gate waits for the owner's outward card to be put up before it answers the
/// worker anyway (the card goes up in the background): its hook has a time limit.
const CARD_WAIT: Duration = Duration::from_secs(8);

/// Who an outward card is about: the ultra run's lead, or a plain worker.
const fn held_back(ultra: bool) -> (&'static str, &'static str) {
    if ultra {
        (OUTWARD_CARD, "the ultra run's lead")
    } else {
        (WORKER_OUTWARD_CARD, "the worker")
    }
}

/// `words` with a capital first letter.
fn capital(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn size_of(text: &str) -> Size {
    Size::parse(text).unwrap_or(Size::M)
}

fn parts_of(plan: &[PlanPart]) -> Vec<Part> {
    plan.iter()
        .map(|p| Part {
            n: p.n,
            name: p.name.clone(),
            size: size_of(&p.size),
            verify: p.verify != Some(false),
        })
        .collect()
}

fn fmt(n: f64) -> String {
    ultra::tokens(n)
}

fn js(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

impl WorkerService {
    /// The task's ultra run, when it has one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn run_of(&self, task: TaskId) -> Result<Option<RunState>, DbError> {
        Ok(self
            .store()
            .run_of(task)
            .await?
            .and_then(|run| serde_json::from_str(&run).ok()))
    }

    async fn save_run(&self, task: TaskId, run: &RunState) -> Result<(), DbError> {
        let text = serde_json::to_string(run).unwrap_or_default();
        self.store().set_run(task, &text, self.now()).await
    }

    async fn fresh(
        &self,
        task: TaskId,
        session: Option<String>,
        account: Option<String>,
    ) -> Result<RunState, DbError> {
        let (parallel, budget) = self.caps(task).await?;
        Ok(RunState {
            state: "planning".to_owned(),
            plan: None,
            budget,
            parallel,
            session,
            account,
            admitted: Vec::new(),
            cost: None,
            lead_tokens: None,
            costs: None,
            ok: None,
            cleared: None,
            outward_ask: None,
        })
    }

    /// A new run of the task (its worker starts): planning, its agents counted afresh; a
    /// resumed session carries on its run.
    pub(crate) async fn begin_run(
        &self,
        task: TaskId,
        session: Option<String>,
        account: Option<String>,
        resumed: bool,
    ) -> Result<(), DbError> {
        let was = self.run_of(task).await?;
        let run = match was {
            Some(mut was) if resumed && was.session == session => {
                if account.is_some() {
                    was.account = account;
                }
                was
            }
            was => {
                let mut fresh = self.fresh(task, session, account).await?;
                fresh.ok = was.and_then(|w| w.ok);
                fresh
            }
        };
        self.save_run(task, &run).await
    }

    /// The lead's session started: its run knows it.
    pub(crate) async fn lead_session(&self, task: TaskId, session: &str) -> Result<(), DbError> {
        if let Some(mut run) = self.run_of(task).await?
            && run.session.as_deref() != Some(session)
        {
            run.session = Some(session.to_owned());
            self.save_run(task, &run).await?;
        }
        Ok(())
    }

    /// What already OKs the task's ultra run in the owner's own words: their OK relayed by the
    /// assistant, or the job's grant.
    async fn run_covered(
        &self,
        task: TaskId,
        run: &RunState,
    ) -> Result<Option<String>, WorkerError> {
        if let Some(ok) = &run.ok {
            return Ok(Some(ok.clone()));
        }
        Ok(self.guard().job_grant(task).await?)
    }

    /// The lead's plan: put on the task, and either cleared to start or put to the owner
    /// first (too big, or over the cap).
    ///
    /// # Errors
    ///
    /// [`WorkerError::Refused`] for a task that isn't an ultra one, or a plan with no parts.
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn set_plan(
        &self,
        task: TaskId,
        input: PlanInput,
        by: Who,
    ) -> Result<PlanResult, WorkerError> {
        let t = self.task(task).await?;
        if !t.ultra {
            return Err(WorkerError::Refused(format!(
                "{} isn't an ultra task",
                t.key()
            )));
        }
        if input.parts.is_empty() {
            return Err(WorkerError::Refused(
                "a plan needs at least one part".to_owned(),
            ));
        }
        let est = ultra::estimate(&parts_of(&input.parts));
        let mut run = match self.run_of(task).await? {
            Some(run) => run,
            None => self.fresh(task, None, None).await?,
        };
        let plan = Plan {
            agents: input.agents.unwrap_or(0.0).max(f64::from(est.agents)),
            tokens: input.tokens.filter(|t| *t > 0.0).unwrap_or(est.tokens),
            accounts: input
                .accounts
                .filter(|a| !a.is_empty())
                .or_else(|| run.account.clone().map(|a| vec![a]))
                .unwrap_or_default(),
            note: input.note,
            at: timestamp_to_api(self.now()).as_str().to_owned(),
            parts: input.parts,
        };
        let limits = self.inner.parts.limits;
        let big = ultra::big(plan.agents, plan.tokens, &limits);
        let mut over = plan.tokens > run.budget;
        let before = run.plan.clone();
        let cleared = run.cleared.clone().or_else(|| {
            (run.state == "running"
                && before
                    .as_ref()
                    .is_some_and(|b| ultra::big(b.agents, b.tokens, &limits)))
            .then(|| "their OK on this run's plan".to_owned())
        });
        if over
            && run.state == "running"
            && before.as_ref().is_some_and(|b| plan.tokens <= b.tokens)
        {
            over = false;
        }
        let ok_by = if over {
            None
        } else if cleared.is_some() {
            cleared.clone()
        } else if big {
            self.run_covered(task, &run).await?
        } else {
            None
        };
        let on = if plan.accounts.is_empty() {
            "the lead's account".to_owned()
        } else {
            plan.accounts.join(" and ")
        };
        let parts: Vec<String> = plan
            .parts
            .iter()
            .map(|p| format!("{}. {} {}", js(p.n), p.name, p.size))
            .collect();
        let note = plan
            .note
            .as_deref()
            .map_or(String::new(), |n| format!(" {n}"));
        self.tasks()
            .note(
                task,
                by,
                &format!(
                    "Ultra plan: {} part{} ({}), {} sub-agents, about {} tokens, up to {} at a time, on {on}. Spending cap {}.{note}",
                    plan.parts.len(),
                    if plan.parts.len() == 1 { "" } else { "s" },
                    parts.join("; "),
                    js(plan.agents),
                    fmt(plan.tokens),
                    run.parallel,
                    fmt(run.budget)
                ),
                false,
            )
            .await?;
        if (big || over) && ok_by.is_none() {
            run.plan = Some(plan.clone());
            "asking".clone_into(&mut run.state);
            self.save_run(task, &run).await?;
            let cap = if over {
                format!(", more than its spending cap of {}", fmt(run.budget))
            } else {
                String::new()
            };
            let question = format!(
                "{PLAN_CARD} {} sub-agents over {} parts, about {} tokens{cap}. Start it?",
                js(plan.agents),
                plan.parts.len(),
                fmt(plan.tokens)
            );
            let brief = vec![
                format!(
                    "{} parts, {} sub-agents, {} at a time",
                    plan.parts.len(),
                    js(plan.agents),
                    run.parallel
                ),
                format!("About {} tokens on {on}", fmt(plan.tokens)),
                if over {
                    format!(
                        "Over its cap ({}): Go ahead raises the cap to fit",
                        fmt(run.budget)
                    )
                } else {
                    format!("Under its cap ({})", fmt(run.budget))
                },
            ];
            self.ask(
                task,
                &question,
                &["Go ahead", "Make it smaller", "Leave it"],
                "Approve the ultra run's plan",
                brief,
                vec![
                    "Starts the sub-agents now".to_owned(),
                    "The lead plans it again, smaller".to_owned(),
                    "Stops the task".to_owned(),
                ],
                0,
            )
            .await?;
            return Ok(PlanResult {
                ok: false,
                plan,
                why: Some(if big { "big" } else { "over" }),
            });
        }
        run.plan = Some(plan.clone());
        "running".clone_into(&mut run.state);
        if ok_by.is_some() && (big || cleared.is_some()) {
            run.cleared.clone_from(&ok_by);
        }
        self.save_run(task, &run).await?;
        if let (Some(cleared), Some(_)) = (&cleared, &before) {
            self.tasks()
                .note(
                    task,
                    Who::Assistant,
                    &format!(
                        "Ultra re-plan cleared by itself ({} sub-agents, about {} tokens): {cleared}, and it fits its spending cap of {}",
                        js(plan.agents),
                        fmt(plan.tokens),
                        fmt(run.budget)
                    ),
                    false,
                )
                .await?;
        } else if let Some(ok) = ok_by.as_ref().filter(|_| big) {
            self.tasks()
                .note(
                    task,
                    Who::Assistant,
                    &format!(
                        "Ultra plan cleared without a card: {ok}, and it fits its spending cap of {}",
                        fmt(run.budget)
                    ),
                    false,
                )
                .await?;
        }
        Ok(PlanResult {
            ok: true,
            plan,
            why: None,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "an ask's parts, each named where it is asked"
    )]
    async fn ask(
        &self,
        task: TaskId,
        question: &str,
        options: &[&str],
        action: &str,
        brief: Vec<String>,
        notes: Vec<String>,
        recommend: usize,
    ) -> Result<(), WorkerError> {
        let input = AskInput {
            question: question.to_owned(),
            options: options
                .iter()
                .map(|o| OptionIn::Plain((*o).to_owned()))
                .collect(),
            action: Some(action.to_owned()),
            kind: Some("approve".to_owned()),
            brief: Some(brief),
            notes: Some(notes),
            recommend: Some(recommend),
            ..AskInput::default()
        };
        self.tasks().ask(task, input, Who::Assistant).await?;
        Ok(())
    }

    /// The ultra card the owner has open on the task now (the plan's, the cap's, the outward
    /// step's): its question.
    async fn open_card(&self, task: TaskId) -> Result<Option<String>, WorkerError> {
        let Some(run) = self.run_of(task).await? else {
            return Ok(None);
        };
        if run.state != "asking" && run.state != "capped" && run.outward_ask.is_none() {
            return Ok(None);
        }
        let t = self.task(task).await?;
        if t.status != emberhut_core::task::Status::Waiting {
            return Ok(None);
        }
        let question = t
            .decision
            .as_deref()
            .and_then(|d| serde_json::from_str::<api::tasks::Decision>(d).ok())
            .map(|d| d.question)
            .filter(|q| {
                [PLAN_CARD, CAP_CARD, OUTWARD_CARD, WORKER_OUTWARD_CARD]
                    .iter()
                    .any(|p| q.starts_with(p))
            });
        Ok(question)
    }

    /// The owner answered an ask the ultra run put to them (its plan, its cap, an outward
    /// step): the run is set for their answer. `true` when it was one. `cap`: the cap their OK
    /// covers; without one their Go ahead raises the cap to fit, as the card says.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn answered(
        &self,
        task: TaskId,
        choice: &str,
        cap: Option<f64>,
    ) -> Result<bool, WorkerError> {
        let Some(mut run) = self.run_of(task).await? else {
            return Ok(false);
        };
        let question = self
            .task(task)
            .await?
            .decision
            .as_deref()
            .and_then(|d| serde_json::from_str::<api::tasks::Decision>(d).ok())
            .map_or_default(|d| d.question);
        let lower = choice.to_lowercase();
        if let Some(asked) = run.outward_ask.clone()
            && (question.starts_with(OUTWARD_CARD) || question.starts_with(WORKER_OUTWARD_CARD))
        {
            let (_, who) = held_back(question.starts_with(OUTWARD_CARD));
            run.outward_ask = None;
            self.save_run(task, &run).await?;
            if lower.starts_with("go ahead") {
                let how = format!("their Go ahead at {}", self.hhmm());
                self.guard().outward_ok(task, &how).await?;
                self.tasks()
                    .note(
                        task,
                        Who::Assistant,
                        &format!(
                            "Outward steps OK'd (push, merge, deploy) for {who}; it was asking for: {}",
                            asked.chars().take(200).collect::<String>()
                        ),
                        false,
                    )
                    .await?;
            }
            return Ok(true);
        }
        if run.state != "asking" && run.state != "capped" {
            return Ok(false);
        }
        let (_, base) = self.caps(task).await?;
        if lower.starts_with("go ahead") || lower.starts_with("raise") {
            let need = if run.state == "asking" {
                (run.plan.as_ref().map_or(0.0, |p| p.tokens) * 1.25).ceil()
            } else {
                self.store().spent(task).await? + base
            };
            let budget = cap.unwrap_or_else(|| run.budget.max(need));
            if run.state == "asking" {
                run.cleared = Some(format!("their Go ahead at {}", self.hhmm()));
            }
            let raised = budget > run.budget;
            "running".clone_into(&mut run.state);
            run.budget = budget;
            self.save_run(task, &run).await?;
            let line = if raised {
                format!("Ultra run cleared; spending cap raised to {}", fmt(budget))
            } else {
                format!("Ultra run cleared (spending cap {})", fmt(budget))
            };
            self.tasks()
                .note(task, Who::Assistant, &line, false)
                .await?;
        } else if lower.starts_with("make it smaller") {
            "planning".clone_into(&mut run.state);
            run.plan = None;
            self.save_run(task, &run).await?;
        } else {
            let next = if run.state == "asking" {
                "planning"
            } else {
                "capped"
            };
            next.clone_into(&mut run.state);
            self.save_run(task, &run).await?;
        }
        Ok(true)
    }

    fn hhmm(&self) -> String {
        self.now()
            .to_zoned_datetime(self.tasks().zone().clone())
            .strftime("%H:%M")
            .to_string()
    }

    /// The gate before each shell command of any worker, and each new sub-agent of an ultra
    /// run's lead: none lets it through; otherwise why not, for the worker. A shell command's
    /// outward step waits for the owner's OK ([`crate::services::guard::Guard::hold_outward`]);
    /// a sub-agent waits for the plan and the caps.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn gate(&self, task: TaskId, hook: &Value) -> Result<Option<String>, WorkerError> {
        let t = self.task(task).await?;
        let tool = hook
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = hook.get("tool_input").cloned().unwrap_or(Value::Null);
        let at = hook.get("cwd").and_then(Value::as_str).unwrap_or("/");
        match self.guard().agent_tool(task, tool, &input, at).await? {
            AgentCheck::Owner => {}
            AgentCheck::Through(_) => return Ok(None),
            AgentCheck::Held(why) => return Ok(Some(why)),
        }
        if tool == "Bash" {
            let command = hook
                .get("tool_input")
                .and_then(|i| i.get("command"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let cwd = hook
                .get("cwd")
                .and_then(Value::as_str)
                .map_or_else(|| "/".to_owned(), str::to_owned);
            match self.guard().agent_outward(task, command, &cwd).await? {
                AgentCheck::Owner => {}
                AgentCheck::Through(_) => return Ok(None),
                AgentCheck::Held(why) => return Ok(Some(why)),
            }
            let Some(waits) = self.guard().hold_outward(task, command, &cwd).await? else {
                return Ok(None);
            };
            self.outward_card(task, command).await?;
            return Ok(Some(waits.to_owned()));
        }
        if !t.ultra || !matches!(tool, "Agent" | "Task" | "Workflow") {
            return Ok(None);
        }
        let Some(mut run) = self.run_of(task).await? else {
            return Ok(Some(
                "Post your plan first with ultra_plan (parts with sizes, agents, tokens, accounts); no sub-agent starts before it.".to_owned(),
            ));
        };
        match run.state.as_str() {
            "planning" => {
                return Ok(Some(
                    "Post your plan first with ultra_plan (parts with sizes, agents, tokens, accounts); no sub-agent starts before it.".to_owned(),
                ));
            }
            "asking" => {
                return Ok(Some(
                    "The owner hasn't OKed this run's plan yet. Don't start sub-agents: stop now (no task_done); you're resumed with their answer.".to_owned(),
                ));
            }
            _ => {}
        }
        let used = self.store().spent(task).await?;
        if run.state == "capped" || used >= run.budget {
            if run.state != "capped" {
                self.capped(task, &mut run, used).await?;
            }
            return Ok(Some(format!(
                "This run's spending cap is reached ({} of {} tokens); the owner was asked. Start no more sub-agents. Let the running ones finish, note what's done and what's left (task_note), then stop without task_done: you're resumed with their answer.",
                fmt(used),
                fmt(run.budget)
            )));
        }
        let id = hook
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let now = self.store().running(task, run.session.as_deref()).await?;
        let mut pending = 0;
        for admitted in &run.admitted {
            if !self.store().started_by(task, admitted).await? {
                pending += 1;
            }
        }
        if now + pending >= usize::try_from(run.parallel).unwrap_or(usize::MAX) {
            return Ok(Some(format!(
                "{} sub-agents are running and this run takes {} at a time: this one didn't start. Wait until one finishes (you're told when it does), then start it again.",
                now + pending,
                run.parallel
            )));
        }
        if !id.is_empty() {
            run.admitted.push(id.to_owned());
            let extra = run.admitted.len().saturating_sub(51);
            run.admitted.drain(..extra);
            self.save_run(task, &run).await?;
        }
        Ok(None)
    }

    /// Puts the owner's outward card up for `command` ([`Self::ask_outward`]), in the
    /// background when it takes longer than [`CARD_WAIT`] (a busy database): the worker hears
    /// it waits either way, well inside its hook's time limit.
    async fn outward_card(&self, task: TaskId, command: &str) -> Result<(), WorkerError> {
        let this = self.clone();
        let held = command.to_owned();
        let card = tokio::spawn(async move { this.ask_outward(task, &held).await });
        match tokio::time::timeout(CARD_WAIT, card).await {
            Ok(Ok(asked)) => {
                asked?;
            }
            Ok(Err(error)) => tracing::warn!(%error, "the outward card failed"),
            Err(_) => {
                tracing::info!(task = %task.key(), "the outward card goes up in the background");
            }
        }
        Ok(())
    }

    async fn capped(&self, task: TaskId, run: &mut RunState, used: f64) -> Result<(), WorkerError> {
        "capped".clone_into(&mut run.state);
        self.save_run(task, run).await?;
        let (_, base) = self.caps(task).await?;
        let (done, total) = self.done(task).await?;
        self.ask(
            task,
            &format!(
                "{CAP_CARD} {} tokens, its cap is {}. No new sub-agents start until you say. Raise the cap by {}?",
                fmt(used),
                fmt(run.budget),
                fmt(base)
            ),
            &["Raise the cap", "Stop here"],
            "Raise the ultra run's spending cap or stop it",
            vec![
                format!("{done} of {total} parts done"),
                format!("{} of {} tokens used", fmt(used), fmt(run.budget)),
            ],
            vec![
                "The lead carries on with the parts left".to_owned(),
                "The lead wraps up with what's done".to_owned(),
            ],
            1,
        )
        .await
    }

    /// The lead's gate held back an outward command the owner's words don't cover: the
    /// server asks the owner itself, once while that card is open. `true` when their words
    /// cover it already.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn ask_outward(&self, task: TaskId, command: &str) -> Result<bool, WorkerError> {
        if self.guard().covered_by(task).await?.is_some() {
            return Ok(true);
        }
        let mut run = match self.run_of(task).await? {
            Some(run) => run,
            None => self.fresh(task, None, None).await?,
        };
        if run.outward_ask.is_some() && self.open_card(task).await?.is_some() {
            return Ok(false);
        }
        let t = self.task(task).await?;
        let (card, who) = held_back(t.ultra);
        let short: String = command.chars().take(300).collect();
        if t.status == emberhut_core::task::Status::Waiting && t.decision.is_some() {
            self.tasks()
                .note(
                    task,
                    Who::Assistant,
                    &format!(
                        "{} is held back from an outward step (asked after the open card): {short}",
                        capital(who)
                    ),
                    false,
                )
                .await?;
            return Ok(false);
        }
        run.outward_ask = Some(command.to_owned());
        self.save_run(task, &run).await?;
        self.ask(
            task,
            &format!(
                "{card} {short}. The owner's request for this task doesn't name it. Go ahead?"
            ),
            &["Go ahead", "Not yet"],
            &format!("OK {who}'s push, merge or deploy"),
            vec![
                command.chars().take(160).collect(),
                "Your Go ahead covers this task's push, merge and deploy steps".to_owned(),
            ],
            vec![
                format!("{} runs it, and the next ones of this task", capital(who)),
                format!(
                    "{} keeps it ready and carries on with the rest",
                    capital(who)
                ),
            ],
            0,
        )
        .await?;
        Ok(false)
    }

    /// One line of a lead's stream: a sub-agent seen (the CLI's word on its background tasks),
    /// the lead's step and tokens, its result's cost.
    pub(crate) async fn scan_line(&self, task: TaskId, line: &str) -> Result<(), WorkerError> {
        if !matters(line) {
            return Ok(());
        }
        let Ok(m) = serde_json::from_str::<Value>(line) else {
            return Ok(());
        };
        let run = self.run_of(task).await?;
        let session = run
            .as_ref()
            .and_then(|r| r.session.clone())
            .or_else(|| {
                self.live()
                    .running
                    .get(&task.get())
                    .and_then(|r| r.session.clone())
            })
            .unwrap_or_default();
        let account = run.as_ref().and_then(|r| r.account.clone());
        if self
            .seen(task, &session, &m, "worker", account, None)
            .await?
        {
            return Ok(());
        }
        let kind = m.get("type").and_then(Value::as_str).unwrap_or_default();
        let top = m.get("parent_tool_use_id").is_none_or(Value::is_null);
        let mut lead = 0.0;
        let mut step = None;
        if kind == "assistant" && top {
            let message = m.get("message");
            if let Some(usage) = message.and_then(|msg| msg.get("usage")) {
                let n = |k: &str| usage.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                lead = n("input_tokens") + n("cache_creation_input_tokens") + n("output_tokens");
            }
            for c in message
                .and_then(|msg| msg.get("content"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                match c.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("undefined");
                        let input = c.get("input").cloned().unwrap_or(Value::Null);
                        step = Some(super::steps::doing(name, &input));
                    }
                    Some("text") => step = Some("Writing".to_owned()),
                    _ => {}
                }
            }
        }
        let cost = (kind == "result")
            .then(|| m.get("total_cost_usd").and_then(Value::as_f64))
            .flatten();
        let seen = {
            let mut held = self.live();
            let was = held.leads.entry(task.get()).or_default();
            if step.is_some() {
                was.doing = step;
            }
            was.tokens += lead;
            was.clone()
        };
        if let Some(file) = self.store().turn_file(task).await? {
            let place = StreamPlace {
                file: file.clone(),
                offset: 0,
                task: Some(task),
                doing: seen.doing.clone(),
                tokens: seen.tokens,
            };
            self.store().set_place(&place).await?;
            if let Some(mut run) = run
                && (lead != 0.0 || cost.is_some())
            {
                run.lead_tokens = Some(run.lead_tokens.unwrap_or(0.0) + lead);
                if let Some(cost) = cost {
                    let costs = run.costs.get_or_insert_with(BTreeMap::new);
                    costs.insert(file, cost);
                    run.cost = Some(costs.values().sum());
                }
                self.save_run(task, &run).await?;
            }
        }
        Ok(())
    }

    /// One of the CLI's own messages about its background tasks (sub-agents, workflows), from
    /// a lead's stream or a runner's event: `true` when it was one.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn seen(
        &self,
        task: TaskId,
        run: &str,
        m: &Value,
        source: &str,
        account: Option<String>,
        machine: Option<String>,
    ) -> Result<bool, WorkerError> {
        let subtype = m.get("subtype").and_then(Value::as_str).unwrap_or_default();
        if m.get("type").and_then(Value::as_str) != Some("system")
            || !matches!(
                subtype,
                "task_started" | "task_progress" | "task_notification" | "task_updated"
            )
        {
            return Ok(false);
        }
        let Some(key) = m
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
        else {
            return Ok(false);
        };
        if let Some(kind) = m
            .get("task_type")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
            && !kind.contains("agent")
            && !kind.contains("workflow")
        {
            // A background shell command: not a sub-agent.
            return Ok(true);
        }
        let now = self.now();
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
        #[expect(clippy::cast_possible_truncation, reason = "a token count")]
        let int = |v: Option<&Value>| v.and_then(Value::as_f64).map(|n| n as i64);
        let mut base = AgentSeen::new(task, run, key, source);
        base.tool_use = text(m.get("tool_use_id"));
        base.account = account;
        base.machine = machine;
        let known = self.store().known(run, key).await?;
        let usage = m.get("usage");
        match subtype {
            "task_started" => {
                let mut seen = base.clone();
                seen.name = text(m.get("description"));
                seen.state = Some("running".to_owned());
                if m.get("task_type").and_then(Value::as_str) == Some("local_workflow") {
                    seen.last = Some(
                        format!(
                            "Workflow {}",
                            text(m.get("workflow_name")).unwrap_or_default()
                        )
                        .trim()
                        .to_owned(),
                    );
                }
                self.store().upsert(seen, now).await?;
            }
            _ if !known => return Ok(true),
            "task_progress" => {
                let mut seen = base.clone();
                seen.name = text(m.get("description"));
                seen.last = text(m.get("description"));
                seen.tokens = int(usage.and_then(|u| u.get("total_tokens")));
                seen.tools = int(usage.and_then(|u| u.get("tool_uses")));
                self.store().upsert(seen, now).await?;
            }
            "task_notification" => {
                let mut seen = base.clone();
                seen.state = Some(
                    m.get("status")
                        .and_then(Value::as_str)
                        .and_then(ultra::state_of)
                        .unwrap_or("done")
                        .to_owned(),
                );
                seen.last = text(m.get("summary")).filter(|s| !s.is_empty());
                seen.tokens = int(usage.and_then(|u| u.get("total_tokens")));
                seen.tools = int(usage.and_then(|u| u.get("tool_uses")));
                self.store().upsert(seen, now).await?;
            }
            _ => {
                if let Some(state) = m
                    .get("patch")
                    .and_then(|p| p.get("status"))
                    .and_then(Value::as_str)
                {
                    let mut seen = base.clone();
                    seen.state = ultra::state_of(state).map(str::to_owned);
                    self.store().upsert(seen, now).await?;
                }
            }
        }
        for w in m
            .get("workflow_progress")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if w.get("type").and_then(Value::as_str) != Some("workflow_agent") {
                continue;
            }
            let Some(agent) = w
                .get("agentId")
                .and_then(|a| {
                    a.as_str()
                        .map(str::to_owned)
                        .or_else(|| Some(a.to_string()))
                })
                .filter(|a| a != "null")
            else {
                continue;
            };
            let mut seen = base.clone();
            seen.key = format!("{key}/{agent}");
            seen.tool_use = None;
            seen.name = text(w.get("label")).or_else(|| text(w.get("promptPreview")));
            seen.state = Some(
                w.get("state")
                    .and_then(Value::as_str)
                    .and_then(ultra::state_of)
                    .unwrap_or("running")
                    .to_owned(),
            );
            seen.last = text(w.get("resultPreview")).or_else(|| text(w.get("promptPreview")));
            seen.tokens = int(w.get("tokens"));
            seen.tools = int(w.get("toolCalls"));
            self.store().upsert(seen, now).await?;
        }
        Ok(true)
    }

    /// The lead's run ended: what still says running there was cut off with it.
    pub(crate) async fn end_run(&self, task: TaskId, run: &str) -> Result<(), DbError> {
        self.store()
            .end_run(task, run, self.now())
            .await
            .map(|_| ())
    }

    /// A sub-agent still marked running whose run isn't, is closed (a minute's grace for one
    /// just started): a worker's run is live while its worker runs that session; a runner's
    /// while its session runs. How many it closed.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn sweep(&self) -> Result<usize, WorkerError> {
        let now = self.now();
        let mut closed = 0;
        for agent in self.store().live().await? {
            if agent
                .started
                .is_some_and(|s| now.duration_since(s) < jiff::SignedDuration::from_mins(1))
            {
                continue;
            }
            let live = if agent.source.as_deref() == Some("worker") {
                self.live()
                    .running
                    .get(&agent.task.get())
                    .is_some_and(|r| r.session.as_deref() == Some(agent.run.as_str()))
            } else {
                let id = if agent.kind == AgentKind::Lane {
                    &agent.key
                } else {
                    &agent.run
                };
                self.inner
                    .parts
                    .runners
                    .session(id)
                    .await?
                    .is_some_and(|s| matches!(s.status.as_str(), "running" | "queued"))
            };
            if !live {
                self.store().stop(agent.id, now).await?;
                closed += 1;
            }
        }
        Ok(closed)
    }

    /// Parts done out of all.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn done(&self, task: TaskId) -> Result<(usize, usize), DbError> {
        let run = self.run_of(task).await?;
        let agents = self.store().of_task(task).await?;
        let seen: Vec<Seen> = agents
            .iter()
            .map(|a| Seen {
                kind: a.kind,
                part: a.part,
                done: a.state == "done",
            })
            .collect();
        let plan = run.and_then(|r| r.plan).map(|p| parts_of(&p.parts));
        Ok(ultra::done(plan.as_deref(), &seen))
    }

    /// A task's sub-agents as the app shows them: in part order, each part's check right after
    /// it (a run with a workflow keeps its start order).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn agents_of(&self, task: TaskId) -> Result<Vec<Value>, DbError> {
        let mut agents = self.store().of_task(task).await?;
        if !agents.iter().any(Agent::in_workflow) {
            let order = |k: AgentKind| match k {
                AgentKind::Part | AgentKind::Lane => 0,
                AgentKind::Verify => 1,
                AgentKind::Other => 2,
            };
            agents.sort_by(|a, b| {
                a.part
                    .unwrap_or(1e9)
                    .total_cmp(&b.part.unwrap_or(1e9))
                    .then(order(a.kind).cmp(&order(b.kind)))
                    .then(a.id.cmp(&b.id))
            });
        }
        Ok(agents
            .into_iter()
            .map(|a| {
                let workflow = a.in_workflow();
                let viewer = if workflow {
                    None
                } else if a.kind == AgentKind::Lane {
                    Some(a.key.clone())
                } else if a.source.as_deref() == Some("worker") {
                    Some(format!("w-{}", task.get()))
                } else {
                    Some(a.run.clone())
                };
                json!({
                    "id": a.id, "name": a.name, "kind": a.kind.as_str(), "part": a.part,
                    "state": a.state, "last": a.last, "tokens": a.tokens, "tools": a.tools,
                    "account": a.account, "accountName": a.account, "machine": a.machine,
                    "run": a.run,
                    "started": a.started.map(|t| timestamp_to_api(t).as_str().to_owned()),
                    "ended": a.ended.map(|t| timestamp_to_api(t).as_str().to_owned()),
                    "workflow": workflow, "viewer": viewer,
                    "parent": if workflow || a.kind == AgentKind::Lane { None } else { a.tool_use },
                })
            })
            .collect())
    }

    /// A task's latest run, its worker or the runner session it runs in: what it does this
    /// moment, since when, and what it used. `worker`: its worker's run only.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn lead_of(
        &self,
        task: TaskId,
        worker: bool,
    ) -> Result<Option<serde_json::Map<String, Value>>, DbError> {
        let Some(t) = self.inner.store.get(task).await? else {
            return Ok(None);
        };
        let Ok(Some(detail)) = self.tasks().detail(task).await else {
            return Ok(None);
        };
        let run = detail
            .runs
            .iter()
            .flatten()
            .find(|r| r.kind == "worker" || (!worker && r.thread.starts_with("r-")))
            .cloned();
        let Some(run) = run else {
            return Ok(None);
        };
        let is_worker = run.kind == "worker";
        let running = if is_worker {
            t.worker.as_deref() == Some(run.thread.as_str()) && self.is_running(task)
        } else {
            run.ended.is_none()
        };
        let state = if running {
            "running"
        } else {
            match run.outcome.as_deref() {
                Some("completed") => "done",
                Some("failed") => "failed",
                Some("interrupted" | "cancelled" | "stopped") => "stopped",
                _ if run.ended.is_some() => "done",
                _ => "stopped",
            }
        };
        let lead = self
            .live()
            .leads
            .get(&task.get())
            .cloned()
            .unwrap_or_default();
        let ultra_run = self.run_of(task).await?;
        let tokens = ultra_run
            .as_ref()
            .and_then(|r| r.lead_tokens)
            .or_else(|| (is_worker && lead.tokens != 0.0).then_some(lead.tokens));
        let started = run.started.as_ref().map(|t| t.as_str().to_owned());
        let ended = run.ended.as_ref().map(|t| t.as_str().to_owned());
        let head = json!({
            "viewer": if is_worker { format!("w-{}", task.get()) } else { run.thread.clone() },
            "name": if t.ultra { "Lead" } else if is_worker { "Worker" } else { "Session" },
            "place": if is_worker { "this server".to_owned() } else { self.inner.parts.runners.place(run.machine.as_str()) },
            "state": state,
            "running": running,
            "doing": if running && is_worker { lead.doing.clone() } else { None },
            "started": started,
            "ended": if running { None } else { ended },
            "tokens": tokens,
            "cost": ultra_run.as_ref().and_then(|r| r.cost),
            "machine": if is_worker { "this server".to_owned() } else { run.machine.as_str().to_owned() },
            "account": ultra_run.and_then(|r| r.account),
        });
        Ok(match head {
            Value::Object(map) => Some(map),
            _ => None,
        })
    }

    /// The task page's ultra run, when it has one (or sub-agents, or a worker to watch).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn view(&self, task: TaskId) -> Result<Option<Value>, DbError> {
        let Some(t) = self.inner.store.get(task).await? else {
            return Ok(None);
        };
        let agents = self.agents_of(task).await?;
        let lead = self.lead_of(task, false).await?;
        let watched = lead
            .as_ref()
            .and_then(|l| l.get("viewer"))
            .and_then(Value::as_str)
            .is_some_and(|v| v.starts_with("w-"));
        if !t.ultra && agents.is_empty() && !watched {
            return Ok(None);
        }
        let run = self.run_of(task).await?;
        let (parallel, budget) = self.caps(task).await?;
        let (done, total) = self.done(task).await?;
        let running = agents
            .iter()
            .filter(|a| {
                a.get("state")
                    .and_then(Value::as_str)
                    .is_some_and(ultra::running)
            })
            .count();
        Ok(Some(json!({
            "on": t.ultra,
            "state": run.as_ref().map(|r| r.state.clone()),
            "parallel": parallel,
            "budget": run.as_ref().map_or(budget, |r| r.budget),
            "spent": self.store().spent(task).await?,
            "cost": run.as_ref().and_then(|r| r.cost),
            "leadTokens": run.as_ref().and_then(|r| r.lead_tokens),
            "account": run.as_ref().and_then(|r| r.account.clone()),
            "plan": run.as_ref().and_then(|r| r.plan.clone()),
            "done": done,
            "total": total,
            "running": running,
            "agents": agents,
            "lead": lead,
        })))
    }

    /// Every agent running now across the tasks (the state's `agentsNow`): each running lead,
    /// then its sub-agents; 40 at most.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn agents_now(&self) -> Result<Vec<api::runner::agents::AgentNow>, DbError> {
        let mut tasks: Vec<TaskId> = self
            .live()
            .running
            .keys()
            .map(|k| TaskId::new(*k))
            .collect();
        for task in self.store().tasks_running().await? {
            if !tasks.contains(&task) {
                tasks.push(task);
            }
        }
        tasks.sort();
        let mut rows = Vec::new();
        for task in tasks {
            let Some(t) = self.inner.store.get(task).await? else {
                continue;
            };
            let lead = self.lead_of(task, false).await?;
            let subs: Vec<Agent> = self
                .store()
                .of_task(task)
                .await?
                .into_iter()
                .filter(|a| ultra::running(&a.state))
                .collect();
            let lead_running = lead
                .as_ref()
                .and_then(|l| l.get("running"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !lead_running && subs.is_empty() {
                continue;
            }
            let text = |l: &serde_json::Map<String, Value>, k: &str| {
                l.get(k).and_then(Value::as_str).map(str::to_owned)
            };
            let time = |at: Option<String>| {
                at.and_then(|a| a.parse::<jiff::Timestamp>().ok())
                    .map(timestamp_to_api)
            };
            let viewer = lead.as_ref().and_then(|l| text(l, "viewer"));
            let kind = if t.ultra {
                "lead"
            } else if viewer.as_deref().is_some_and(|v| v.starts_with("w-")) {
                "worker"
            } else {
                "session"
            };
            let machine = lead.as_ref().and_then(|l| text(l, "machine"));
            let account = lead.as_ref().and_then(|l| text(l, "account"));
            if let Some(lead) = lead.as_ref().filter(|_| lead_running) {
                rows.push(api::runner::agents::AgentNow {
                    task: api::ids::TaskId::from(task.get()),
                    key: task.key(),
                    title: Some(t.title.clone()),
                    id: format!("lead-{}", task.get()),
                    name: text(lead, "name"),
                    line: text(lead, "doing"),
                    started: time(text(lead, "started")),
                    tokens: lead.get("tokens").and_then(Value::as_f64),
                    viewer: viewer.clone().map(api::ids::SessionId::new),
                    parent: None,
                    lead: true,
                    kind: Some(api::runner::agents::AgentWho::from(kind)),
                    machine: machine.clone().map(api::ids::MachineId::new),
                    place: text(lead, "place"),
                    account: account.clone(),
                    account_name: account.clone(),
                });
            }
            for a in subs {
                let workflow = a.in_workflow();
                let viewer = if workflow {
                    None
                } else if a.kind == AgentKind::Lane {
                    Some(a.key.clone())
                } else if a.source.as_deref() == Some("worker") {
                    Some(format!("w-{}", task.get()))
                } else {
                    Some(a.run.clone())
                };
                let machine = a.machine.clone().or_else(|| machine.clone());
                let account = a.account.clone().or_else(|| account.clone());
                #[expect(clippy::cast_precision_loss, reason = "a token count")]
                let tokens = a.tokens.map(|n| n as f64);
                rows.push(api::runner::agents::AgentNow {
                    task: api::ids::TaskId::from(task.get()),
                    key: task.key(),
                    title: Some(t.title.clone()),
                    id: format!("agent-{}", a.id),
                    name: a.name.clone(),
                    line: a.last.clone(),
                    started: a.started.map(timestamp_to_api),
                    tokens,
                    viewer: viewer.map(api::ids::SessionId::new),
                    parent: if workflow || a.kind == AgentKind::Lane {
                        None
                    } else {
                        a.tool_use.clone().map(api::ids::ToolCallId::new)
                    },
                    lead: false,
                    kind: Some(api::runner::agents::AgentWho::from("helper")),
                    place: machine.as_ref().map(|m| self.inner.parts.runners.place(m)),
                    machine: machine.map(api::ids::MachineId::new),
                    account_name: account.clone(),
                    account,
                });
            }
        }
        rows.truncate(40);
        Ok(rows)
    }

    /// Runs one part of the ultra run as its own session on a runner, on another account with
    /// room (a lane). It counts as a sub-agent; [`Self::wait`] waits for it.
    ///
    /// A runner's session has no gate before its shell commands and no deny rules, so a lane
    /// starts only when the owner's words cover the task's outward steps
    /// ([`crate::services::guard::Guard::covered_by`]); otherwise the lead runs the part as a
    /// sub-agent of its own, behind the gate.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn lane(
        &self,
        task: TaskId,
        part: u32,
        name: &str,
        brief: &str,
        cwd: &str,
        account: Option<String>,
    ) -> Result<String, WorkerError> {
        let t = self.task(task).await?;
        let Some(run) = self.run_of(task).await?.filter(|r| r.state == "running") else {
            return Ok(
                "Not started: post the plan first (ultra_plan) and wait until it's cleared."
                    .to_owned(),
            );
        };
        if self.guard().covered_by(task).await?.is_none() {
            return Ok(LANE_UNGATED.to_owned());
        }
        let machine = self
            .inner
            .parts
            .lane_machine
            .clone()
            .or_else(|| self.inner.parts.runners.machines().into_iter().next());
        let Some(machine) = machine else {
            return Ok("Not started: this server has no runner to run a lane on. Run this part yourself (a sub-agent in your own session).".to_owned());
        };
        if let Err(why) = self.inner.parts.machines.can_place(&machine).await? {
            return Ok(format!(
                "Not started: the {} ({machine}) was skipped: {why}. Run this part yourself (a sub-agent in your own session) or try again later.",
                self.inner.parts.runners.place(&machine)
            ));
        }
        let lane = format!("lane-{part}-{}", self.now().as_millisecond());
        if let Some(why) = self
            .gate(task, &json!({ "tool_name": "Agent", "tool_use_id": lane }))
            .await?
        {
            return Ok(format!("Not started: {why}"));
        }
        let mut meta = serde_json::Map::new();
        meta.insert("task".to_owned(), json!(task.get()));
        meta.insert("ultra".to_owned(), json!({ "lane": part }));
        let ask = WorkSession {
            machine: machine.clone(),
            cwd: cwd.to_owned(),
            project: t.project.clone(),
            title: format!("{} lane {part}: {name}", t.key()),
            message: format!(
                "{brief}\n\nYou are one part (part {part}) of an ultra run on {}, as your own session. No push, merge, PR, release, deploy, message, post, payment or deleting anything; prepare it and say so in your answer instead.{}",
                t.key(),
                if self.tasks().keeps_plans() {
                    format!(" {}", crate::services::tasks::lane_rule(&t.key()))
                } else {
                    String::new()
                }
            ),
            account: account.or_else(|| {
                run.account.as_ref().and_then(|mine| {
                    run.plan
                        .as_ref()?
                        .accounts
                        .iter()
                        .find(|a| *a != mine)
                        .cloned()
                })
            }),
            model: t.model.clone(),
            effort: None,
            worktree: None,
            meta,
        };
        let opened = match self
            .inner
            .parts
            .runners
            .start_work(ask, &format!("{} lane {part}", t.key()))
            .await
        {
            Ok(opened) => opened,
            Err(error) => return Ok(format!("Not started: {error}")),
        };
        let mut seen = AgentSeen::new(task, opened.id.clone(), opened.id.clone(), "runner");
        seen.tool_use = Some(lane);
        seen.name = Some(format!("Lane {part}: {name}"));
        seen.state = Some("running".to_owned());
        seen.account.clone_from(&opened.account);
        seen.machine = Some(machine.clone());
        self.store().upsert(seen, self.now()).await?;
        let on = opened.account.clone().unwrap_or_else(|| "null".to_owned());
        self.tasks()
            .note(
                task,
                Who::Worker,
                &format!("Ultra lane {part} ({name}) started on {on}"),
                false,
            )
            .await?;
        Ok(format!(
            "Lane {} started on {on}. Wait for it with ultra_wait.",
            opened.id
        ))
    }

    /// Waits for the run's lanes (their session ids) to finish, up to `minutes`: each one's
    /// state and its answer. A session that isn't one of this task's lanes is named as such and
    /// never read.
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn wait(
        &self,
        task: TaskId,
        sessions: &[String],
        minutes: f64,
    ) -> Result<String, WorkerError> {
        let until =
            tokio::time::Instant::now() + Duration::from_secs_f64(minutes.clamp(0.0, 9.0) * 60.0);
        loop {
            let mut looks = Vec::new();
            for sid in sessions {
                let agent = self.store().get(task, sid).await?;
                if agent.is_none() {
                    // Not one of this run's lanes: another session isn't this run's to read.
                    looks.push((sid.clone(), None, None));
                    continue;
                }
                let thread = self.inner.parts.runners.session(sid).await?;
                if let (Some(agent), Some(thread)) = (&agent, &thread)
                    && ultra::running(&agent.state)
                    && !matches!(thread.status.as_str(), "running" | "queued")
                {
                    let state = match thread.status.as_str() {
                        "completed" | "done" => "done",
                        "failed" => "failed",
                        _ => "stopped",
                    };
                    let mut seen = AgentSeen::new(task, sid.clone(), sid.clone(), "runner");
                    seen.state = Some(state.to_owned());
                    seen.last.clone_from(&thread.last);
                    self.store().upsert(seen, self.now()).await?;
                }
                let agent = self.store().get(task, sid).await?;
                looks.push((sid.clone(), agent, thread));
            }
            let waiting = looks
                .iter()
                .any(|(_, a, _)| a.as_ref().is_some_and(|a| ultra::running(&a.state)));
            if !waiting || tokio::time::Instant::now() >= until {
                return Ok(looks
                    .into_iter()
                    .map(|(sid, a, t)| {
                        let Some(a) = a else {
                            return format!("{sid}: not one of this run's lanes");
                        };
                        let state = a.state;
                        let last = t.and_then(|t| t.last).filter(|l| !l.is_empty());
                        match last {
                            Some(last) => format!("{sid}: {state}: {last}"),
                            None => format!("{sid}: {state}"),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"));
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// What the gate's caps and the owner's OK say about the task's run, for a lead on a
    /// runner (whose gate runs there).
    ///
    /// # Errors
    ///
    /// [`WorkerError`].
    pub async fn runner_limits(&self, task: TaskId) -> Result<Value, WorkerError> {
        let run = self.run_of(task).await?;
        let (parallel, budget) = self.caps(task).await?;
        let limits = self.inner.parts.limits;
        let ok = match &run {
            Some(run) if run.state != "capped" => self.run_covered(task, run).await?.is_some(),
            Some(_) => false,
            None => self.guard().job_grant(task).await?.is_some(),
        };
        Ok(json!({
            "parallel": parallel,
            "budget": run.as_ref().map_or(budget, |r| r.budget),
            "approved": run.as_ref().is_some_and(|r| r.state == "running"),
            "askTokens": limits.ask_tokens,
            "askAgents": limits.ask_agents,
            "ok": ok,
            "outward": self.guard().covered_by(task).await?.is_some(),
        }))
    }
}

/// Whether a stream line can say anything to the scan: a sub-agent's line, the lead's turns,
/// the result. A line that doesn't start with its type is read, to be safe.
fn matters(line: &str) -> bool {
    if !line.starts_with('{') {
        return false;
    }
    if !line.starts_with("{\"type\":\"") {
        return true;
    }
    line.starts_with("{\"type\":\"assistant\"")
        || line.starts_with("{\"type\":\"result\"")
        || (line.starts_with("{\"type\":\"system\"") && line.contains("\"subtype\":\"task_"))
}

/// The owner's answers on tasks, as the ultra runs take them: an answer to the run's plan,
/// cap or outward card sets the run for it (the task's own handling goes on, the lead resumed
/// with the answer); their yes written on the task's page with such a card open is their Go
/// ahead.
#[derive(Clone, Debug)]
pub struct UltraAnswers(pub WorkerService);

impl crate::services::tasks::AnswerTaker for UltraAnswers {
    fn decide<'a>(
        &'a self,
        task: TaskId,
        choice: &'a str,
    ) -> futures_util::future::BoxFuture<'a, bool> {
        Box::pin(async move {
            if let Err(error) = self.0.answered(task, choice, None).await {
                tracing::warn!(%error, "the ultra run didn't take the owner's answer");
            }
            false
        })
    }

    fn message<'a>(
        &'a self,
        task: TaskId,
        text: &'a str,
    ) -> futures_util::future::BoxFuture<'a, bool> {
        Box::pin(async move {
            if !go::is_yes(text) || !matches!(self.0.open_card(task).await, Ok(Some(_))) {
                return false;
            }
            match self.0.tasks().decide(task, "Go ahead").await {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "the owner's yes didn't answer the ultra card");
                    false
                }
            }
        })
    }
}
