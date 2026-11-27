//! The owner's OK for the final steps of a job in the browser or on their Mac, on the one grant
//! model ([`super::grants`]). A final step goes through on:
//!
//! - the job's grant: the owner's request for the task, or one OK of theirs on it;
//! - for a message or a post, the text written on the page being one they OK'd (once, any
//!   channel);
//! - else the step's own Go ahead card (one for the whole job when it's planned with
//!   [`Guard::plan_job`]); their Go ahead is the job's grant from then on.
//!
//! Payments, purchases, passwords, codes and account security ask on the step every time, and
//! only the owner's tap on that card clears them. The step waiting on its card and the job's
//! plan are kept in the settings (`browserPending`, `browserPlans`, `browserByRequest`), so a
//! restart doesn't lose the owner's answer.
//!
//! A browser's or the Mac's final-step gate is an adapter over [`Guard::step_gate`]: its
//! answer's [`GateResult::By`] lets the click through (with the line the log says), its
//! [`GateResult::Text`] holds it and is what the agent is told.

use std::collections::BTreeMap;

use emberhut_core::RoomId;
use emberhut_core::guard::go_ahead::{self as rules, CARRY_HOURS, PLAN_HOURS};
use emberhut_core::guard::grants::{self as grant_rules, StepKind};
use emberhut_core::guard::{chat_ok as chat_rules, go};
use emberhut_core::message::Author;
use emberhut_core::task::ask::OptionIn;
use emberhut_core::task::{Status, TaskId, Who};
use emberhut_db::{DbError, SettingKey};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use super::{Guard, GuardError};
use crate::services::tasks::AskInput;

/// The step waiting on its card.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// The step's label.
    pub what: String,
    /// When it was asked.
    pub at: String,
    /// The card's question.
    pub question: String,
    /// Money or account security.
    pub hard: bool,
    /// The page it is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// The text it would send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A planned job's final steps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// The job.
    pub job: String,
    /// Its final steps.
    pub steps: Vec<String>,
    /// When it was asked.
    pub asked: String,
    /// The card's question.
    pub question: Option<String>,
    /// When the owner OK'd it.
    pub ok: Option<String>,
    /// How.
    pub how: Option<String>,
}

/// The steps logged in a job's window on its grant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// When the window opened.
    pub from: String,
    /// The steps logged in it.
    pub steps: Vec<String>,
}

/// The owner's request a room's work carries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Carried {
    /// When.
    pub at: String,
    /// The owner's message.
    pub msg: Option<i64>,
    /// The room it was said in.
    pub room: Option<i64>,
    /// The room that handed it over.
    pub from: i64,
}

const PENDING: SettingKey<BTreeMap<String, Pending>> = SettingKey::new("browserPending");
const PLANS: SettingKey<BTreeMap<String, Plan>> = SettingKey::new("browserPlans");
const BY_REQUEST: SettingKey<BTreeMap<String, Window>> = SettingKey::new("browserByRequest");
const CARRIED: SettingKey<BTreeMap<String, Carried>> = SettingKey::new("roomRequests");

/// What a final step's card asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateCard {
    /// The card's question.
    pub question: String,
    /// Its action line.
    pub action: String,
    /// Its brief.
    pub brief: Vec<String>,
}

/// A final step's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateResult {
    /// The owner OK'd it: it goes through, and this is the line the log says.
    By(String),
    /// It is held: what the agent is told (the owner was asked, or still is).
    Text(String),
}

/// The owner's request a task carries: when, their message, the room it was said in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// When.
    pub at: Timestamp,
    /// Their message.
    pub msg: Option<i64>,
    /// The room.
    pub room: RoomId,
}

fn list(steps: &[String]) -> String {
    steps
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Guard {
    async fn pending(&self, task: TaskId) -> Result<Option<Pending>, DbError> {
        Ok(self
            .inner
            .settings
            .get_or(&PENDING, BTreeMap::new())
            .await?
            .remove(&task.get().to_string()))
    }

    async fn put_pending(&self, task: TaskId, value: Option<Pending>) -> Result<(), DbError> {
        self.inner
            .settings
            .update(&PENDING, BTreeMap::new(), |all| match value {
                Some(v) => {
                    all.insert(task.get().to_string(), v);
                }
                None => {
                    all.remove(&task.get().to_string());
                }
            })
            .await
    }

    async fn put_plan(&self, task: TaskId, value: Option<Plan>) -> Result<(), DbError> {
        self.inner
            .settings
            .update(&PLANS, BTreeMap::new(), |all| match value {
                Some(v) => {
                    all.insert(task.get().to_string(), v);
                }
                None => {
                    all.remove(&task.get().to_string());
                }
            })
            .await
    }

    /// The task's Go ahead card, moot now (archived): a later step asks afresh.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn drop_pending(&self, task: TaskId) -> Result<(), DbError> {
        self.put_pending(task, None).await
    }

    /// The owner's lines on the task after `since`.
    async fn owner_lines(
        &self,
        task: TaskId,
        since: Timestamp,
    ) -> Result<Vec<(Timestamp, String)>, GuardError> {
        Ok(self
            .inner
            .tasks
            .history_of(task)
            .await?
            .into_iter()
            .filter(|h| h.who == Who::Person && h.at > since)
            .map(|h| (h.at, h.text))
            .collect())
    }

    /// Whether the card asking `question` is the task's open one.
    async fn card_open(&self, task: TaskId, question: Option<&str>) -> Result<bool, DbError> {
        let Some(question) = question.filter(|q| !q.is_empty()) else {
            return Ok(false);
        };
        Ok(self.task(task).await?.is_some_and(|t| {
            t.status == Status::Waiting
                && t.decision
                    .as_deref()
                    .and_then(|d| serde_json::from_str::<emberhut_api::tasks::Decision>(d).ok())
                    .is_some_and(|d| d.question == question)
        }))
    }

    /// The job's plan with the owner's answer read in: OK'd (their Go ahead, or a yes on the
    /// task, after it was asked), dropped (any other answer), or still asked.
    async fn plan_of(&self, task: TaskId) -> Result<Option<Plan>, GuardError> {
        let all = self.inner.settings.get_or(&PLANS, BTreeMap::new()).await?;
        let Some(plan) = all.get(&task.get().to_string()).cloned() else {
            return Ok(None);
        };
        if plan.ok.is_some() {
            return Ok(Some(plan));
        }
        let asked = Guard::parse(&plan.asked).unwrap_or(Timestamp::UNIX_EPOCH);
        let answer = self
            .owner_lines(task, asked)
            .await?
            .into_iter()
            .find(|(_, text)| rules::is_choice(text) || go::is_yes(text));
        let Some((at, text)) = answer else {
            return Ok(Some(plan));
        };
        if rules::is_choice(&text) && !text.starts_with("Chose: Go ahead") {
            self.put_plan(task, None).await?;
            return Ok(None);
        }
        let how = if rules::is_choice(&text) {
            format!("their Go ahead at {}", self.hhmm(at))
        } else {
            format!("their yes on the task at {}", self.hhmm(at))
        };
        let done = Plan {
            ok: Some(Guard::iso(at)),
            how: Some(how),
            ..plan
        };
        self.put_plan(task, Some(done.clone())).await?;
        Ok(Some(done))
    }

    fn live(&self, plan: Option<&Plan>) -> bool {
        plan.and_then(|p| p.ok.as_deref())
            .and_then(Guard::parse)
            .is_some_and(|ok| {
                self.now().duration_since(ok) < SignedDuration::from_hours(PLAN_HOURS)
            })
    }

    /// The job's grant for these steps on the task, as the line the click says; each step new
    /// to the job's window is logged once on the task.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn by_request(
        &self,
        task: TaskId,
        steps: &[String],
        job: Option<&str>,
    ) -> Result<Option<String>, GuardError> {
        let Some(by) = self.job_grant(task).await? else {
            return Ok(None);
        };
        let now = self.now();
        let all = self
            .inner
            .settings
            .get_or(&BY_REQUEST, BTreeMap::new())
            .await?;
        let window = all
            .get(&task.get().to_string())
            .filter(|w| {
                Guard::parse(&w.from).is_some_and(|from| {
                    now.duration_since(from) < SignedDuration::from_hours(PLAN_HOURS)
                })
            })
            .cloned()
            .unwrap_or_else(|| Window {
                from: Guard::iso(now),
                steps: Vec::new(),
            });
        let added: Vec<String> = steps
            .iter()
            .filter(|x| !window.steps.iter().any(|s| rules::matches(s, x)))
            .cloned()
            .collect();
        if !added.is_empty() {
            let mut window = window;
            window.steps.extend(added.iter().cloned());
            let extra = window.steps.len().saturating_sub(40);
            window.steps.drain(..extra);
            self.inner
                .settings
                .update(&BY_REQUEST, BTreeMap::new(), |all| {
                    all.insert(task.get().to_string(), window);
                })
                .await?;
            let line = match job {
                Some(job) => format!(
                    "Browser job {by}: {job}. Final steps {}, no card",
                    list(&added)
                ),
                None => format!("Browser step {} {by}, no card", list(&added)),
            };
            self.inner
                .tasks
                .note(task, Who::Assistant, &line, false)
                .await?;
        }
        Ok(Some(by))
    }

    /// Asks the owner on the task, as the assistant: a Go ahead / Stop card.
    async fn ask_go_ahead(
        &self,
        task: TaskId,
        question: &str,
        action: &str,
        brief: Vec<String>,
        draft: Option<String>,
        notes: Option<Vec<String>>,
    ) -> Result<(), GuardError> {
        let input = AskInput {
            question: question.to_owned(),
            options: vec![
                OptionIn::Plain("Go ahead".to_owned()),
                OptionIn::Plain("Stop".to_owned()),
            ],
            action: Some(action.chars().take(90).collect()),
            kind: Some("approve".to_owned()),
            brief: Some(brief).filter(|b| !b.is_empty()),
            notes,
            draft,
            ..AskInput::default()
        };
        self.inner.tasks.ask(task, input, Who::Assistant).await?;
        Ok(())
    }

    /// A planned job: its final steps, asked once when the job has no OK yet. Hard steps stay
    /// out (they ask each time); messages and posts are decided at the click by their text.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn plan_job(
        &self,
        task: TaskId,
        job: &str,
        steps: &[String],
    ) -> Result<String, GuardError> {
        if self.task(task).await?.is_none() {
            return Ok(format!("No such task {}.", task.key()));
        }
        let mut seen: Vec<String> = Vec::new();
        for step in steps {
            let step: String = step.trim().chars().take(60).collect();
            if !step.is_empty() && !seen.contains(&step) && seen.len() < 12 {
                seen.push(step);
            }
        }
        let held: Vec<String> = seen
            .iter()
            .filter(|x| grant_rules::hard(x, ""))
            .cloned()
            .collect();
        let said = |x: &str| grant_rules::kind(x, "") == StepKind::Said;
        let later: Vec<String> = seen
            .iter()
            .filter(|x| !grant_rules::hard(x, "") && said(x))
            .cloned()
            .collect();
        let ok: Vec<String> = seen
            .iter()
            .filter(|x| !grant_rules::hard(x, "") && !said(x))
            .cloned()
            .collect();
        let still = if held.is_empty() {
            String::new()
        } else {
            format!(
                " {} still ask{} the owner on the step each time (payments, purchases, passwords, codes and account security never ride on an OK).",
                list(&held),
                if held.len() == 1 { "s" } else { "" }
            )
        };
        if ok.is_empty() && later.is_empty() {
            return Ok(format!("Nothing planned:{still}"));
        }
        let pre = if later.is_empty() {
            String::new()
        } else {
            format!(
                "{}: a message as the owner or a public post: the click goes through when the text on the page is one they OK'd, else it asks them once for that text. ",
                list(&later)
            )
        };
        if ok.is_empty() {
            return Ok(format!("{pre}{still}").trim().to_owned());
        }
        if let Some(mine) = self.by_request(task, &ok, Some(job)).await? {
            let whose = if mine.starts_with("approved by your request") {
                "the owner's own request"
            } else {
                "their OK on this job"
            };
            return Ok(format!(
                "OK'd by {whose} ({mine}): click {} when you reach them; they go through with no card while the task is open. {pre}{}",
                list(&ok),
                still.trim()
            )
            .trim()
            .to_owned());
        }
        let was = self.plan_of(task).await?;
        if self.live(was.as_ref())
            && let Some(was) = &was
            && ok
                .iter()
                .all(|x| was.steps.iter().any(|w| rules::matches(w, x)))
        {
            return Ok(format!(
                "{pre}Already OK'd ({}): {} go through on {}.{still}",
                was.how.as_deref().unwrap_or(""),
                list(&ok),
                task.key()
            ));
        }
        if let Some(how) = self.task_ok(task).await? {
            self.grant_job(task, &how, None).await?;
            let now = Guard::iso(self.now());
            self.put_plan(
                task,
                Some(Plan {
                    job: job.to_owned(),
                    steps: ok.clone(),
                    asked: now.clone(),
                    question: None,
                    ok: Some(now),
                    how: Some(how.clone()),
                }),
            )
            .await?;
            self.inner
                .tasks
                .note(
                    task,
                    Who::Assistant,
                    &format!("Browser job OK'd ({how}): {job}. Final steps {}", list(&ok)),
                    false,
                )
                .await?;
            return Ok(format!(
                "{pre}OK'd already ({how}): click {} when you reach them; they go through with no card while the task is open.{still}",
                list(&ok)
            ));
        }
        let question = format!(
            "The assistant's browser job on {}: {job}. The final steps it will click: {}. OK the whole job?",
            task.key(),
            list(&ok)
        );
        let brief = vec![
            format!("Job: {job}").chars().take(160).collect(),
            format!("Final steps: {}", list(&ok))
                .chars()
                .take(160)
                .collect(),
            "Payments, purchases, passwords and account security still ask you each time"
                .to_owned(),
        ];
        let notes = Some(vec![
            "Every step of this job goes through, in the browser and on your Mac, no card for each"
                .to_owned(),
            "Nothing is clicked".to_owned(),
        ]);
        self.ask_go_ahead(
            task,
            &question,
            &format!("OK the job: {job}"),
            brief,
            None,
            notes,
        )
        .await?;
        self.put_plan(
            task,
            Some(Plan {
                job: job.to_owned(),
                steps: ok,
                asked: Guard::iso(self.now()),
                question: Some(question),
                ok: None,
                how: None,
            }),
        )
        .await?;
        Ok(format!(
            "{pre}Asked the owner once for the whole job (Go ahead / Stop, under Needs you; their yes in the chat answers it too). Stop for now; you'll hear when they answer. After their Go ahead, every step of {}'s job goes through without asking again.{still}",
            task.key()
        ))
    }

    /// A final step about to be clicked: [`GateResult::By`] when the owner OK'd it, else what
    /// the agent is told (their card asked, or still open). `page`: the page's title and
    /// address; `composed`: for a message or a post, the text written in the box the button
    /// sends.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    pub async fn step_gate(
        &self,
        task: TaskId,
        what: &str,
        page: &str,
        card: &GateCard,
        composed: &[String],
    ) -> Result<GateResult, GuardError> {
        match self.agent_step(task, what, page, composed).await? {
            super::agents::AgentCheck::Owner => {}
            super::agents::AgentCheck::Through(by) => return Ok(GateResult::By(by)),
            super::agents::AgentCheck::Held(text) => return Ok(GateResult::Text(text)),
        }
        let kind = grant_rules::kind(what, page);
        let hard = kind == StepKind::Hard;
        let pending = self.pending(task).await?;
        let mine = pending.as_ref().filter(|p| p.what == what).cloned();
        if let Some(p) = &mine {
            let since = Guard::parse(&p.at).unwrap_or(Timestamp::UNIX_EPOCH);
            let answered = self
                .owner_lines(task, since)
                .await?
                .iter()
                .any(|(_, text)| {
                    text.starts_with("Chose: Go ahead")
                        || (!p.hard
                            && !rules::is_choice(text)
                            && if kind == StepKind::Said {
                                go::is_go(text)
                            } else {
                                go::is_yes(text)
                            })
                });
            if answered {
                self.put_pending(task, None).await?;
                if let Some(text) = &p.text {
                    self.sent_text(task, text, &format!("\"{what}\"")).await?;
                }
                return Ok(GateResult::By("their Go ahead".to_owned()));
            }
        }
        let text = composed
            .iter()
            .map(|x| emberhut_core::messaging::text::js_trim(x))
            .find(|x| !x.is_empty())
            .map(str::to_owned);
        let mut drop = false;
        let mut through = None;
        if kind == StepKind::Said {
            if let Some(ok) = self.message_ok(task, what, composed).await? {
                drop = true;
                through = Some(ok.0);
            }
        } else if kind == StepKind::Ordinary && self.open_task(task).await? {
            let plan = self.plan_of(task).await?;
            let cover = if self.live(plan.as_ref()) {
                plan.as_ref()
                    .and_then(|p| p.steps.iter().find(|x| rules::matches(x, what)))
                    .map(|_| {
                        format!(
                            "in the job they OK'd: {}",
                            plan.as_ref()
                                .and_then(|p| p.how.clone())
                                .unwrap_or_default()
                        )
                    })
            } else {
                None
            };
            let granted = self.by_request(task, &[what.to_owned()], None).await?;
            let by = match (&granted, cover) {
                (Some(g), _) => Some(g.clone()),
                (None, Some(c)) => Some(c),
                (None, None) => self.task_ok(task).await?,
            };
            if let Some(by) = by {
                if granted.is_none() {
                    self.grant_job(task, &by, None).await?;
                }
                drop = true;
                through = Some(by);
            }
        }
        if drop && let Some(p) = &mine {
            self.close_card(task, &p.question).await?;
            self.put_pending(task, None).await?;
        }
        if let Some(by) = through {
            return Ok(GateResult::By(by));
        }
        if !hard {
            let plan = self.plan_of(task).await?;
            if let Some(plan) = plan
                && plan.ok.is_none()
                && kind == StepKind::Ordinary
                && self.card_open(task, plan.question.as_deref()).await?
            {
                return Ok(GateResult::Text(format!(
                    "Paused: \"{what}\" waits for the owner's OK on the job you planned (asked {}). Stop for now; you'll hear when they answer.",
                    plan.asked
                )));
            }
        }
        if let Some(p) = &mine
            && self.card_open(task, Some(&p.question)).await?
        {
            return Ok(GateResult::Text(format!(
                "Still waiting for the owner's Go ahead on \"{what}\" (asked {}). Stop for now; you'll hear when they answer.",
                p.at
            )));
        }
        let mut brief = card.brief.clone();
        if kind == StepKind::Said && text.is_some() {
            brief.push("Your Go ahead OKs this exact text, once, in any channel".to_owned());
            brief.truncate(4);
        }
        let draft = if kind == StepKind::Said {
            text.clone()
        } else {
            None
        };
        self.ask_go_ahead(task, &card.question, &card.action, brief, draft, None)
            .await?;
        if hard || kind == StepKind::Said {
            self.step_card(task, &card.question, kind).await?;
        }
        self.put_pending(
            task,
            Some(Pending {
                what: what.to_owned(),
                at: Guard::iso(self.now()),
                question: card.question.clone(),
                hard,
                page: Some(page.to_owned()).filter(|p| !p.is_empty()),
                text: if kind == StepKind::Said {
                    text.clone()
                } else {
                    None
                },
            }),
        )
        .await?;
        Ok(GateResult::Text(match kind {
            StepKind::Hard => format!(
                "Paused: \"{what}\" is a final step with money or account security in it (payments, purchases, passwords, codes, 2FA, account changes), so the owner OKs it themselves each time; they were asked (under Needs you). When they say Go ahead, do it again. Stop for now."
            ),
            StepKind::Said => format!(
                "Paused: \"{what}\" says something as the owner{}, so they were asked once for it (under Needs you). When they say Go ahead, do it again; don't change a word. Stop for now.{}",
                if text.is_some() {
                    " and that text hasn't been OK'd"
                } else {
                    ""
                },
                if text.is_some() {
                    ""
                } else {
                    " Next time, have the exact text OK'd first (task_ask with the draft): then the send goes through with no card."
                }
            ),
            StepKind::Ordinary => format!(
                "Paused: \"{what}\" is a final step, so the owner was asked first (it's under Needs you). When they say Go ahead, do it again. Stop for now. Their one Go ahead covers the rest of this job, in the browser and on their Mac."
            ),
        }))
    }

    /// The owner's message a turn in `room` at `at` answers: their latest there, within the
    /// chat's window before, with no answer of the assistant's there since.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn asked_by(&self, room: RoomId, at: Timestamp) -> Result<Option<Request>, DbError> {
        let lines = self
            .inner
            .messages
            .said_since(room, at - chat_rules::WINDOW, 200)
            .await?;
        let Some(last) = lines
            .iter()
            .rev()
            .find(|m| m.author == Author::User && m.created_at <= at)
        else {
            return Ok(None);
        };
        let answered = lines
            .iter()
            .any(|m| m.author == Author::Assistant && m.id > last.id && m.created_at <= at);
        Ok((!answered).then(|| Request {
            at: last.created_at,
            msg: Some(last.id.get()),
            room,
        }))
    }

    /// The owner's request in `room` now: the message being answered, else their latest there
    /// within [`CARRY_HOURS`], or one another room handed over with its work, whichever is
    /// later.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn request_in(&self, room: RoomId) -> Result<Option<Request>, DbError> {
        let now = self.now();
        if let Some(asked) = self.asked_by(room, now).await? {
            return Ok(Some(asked));
        }
        let since = now - SignedDuration::from_hours(CARRY_HOURS);
        let last = self
            .inner
            .messages
            .said_since(room, since, 500)
            .await?
            .into_iter()
            .rev()
            .find(|m| m.author == Author::User);
        let hand = self
            .inner
            .settings
            .get_or(&CARRIED, BTreeMap::new())
            .await?
            .remove(&room.get().to_string())
            .and_then(|c| Guard::parse(&c.at).map(|at| (at, c)))
            .filter(|(at, _)| *at >= since && *at <= now);
        if let Some(m) = last
            && hand.as_ref().is_none_or(|(at, _)| m.created_at >= *at)
        {
            return Ok(Some(Request {
                at: m.created_at,
                msg: Some(m.id.get()),
                room,
            }));
        }
        Ok(hand.map(|(at, c)| Request {
            at,
            msg: c.msg,
            room: c.room.map_or(room, RoomId::new),
        }))
    }

    /// Work handed from room `from` to room `to` carries the owner's request with it, so the
    /// tasks made there are theirs too.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn carry_request(&self, from: RoomId, to: RoomId) -> Result<bool, DbError> {
        let Some(r) = self.request_in(from).await? else {
            return Ok(false);
        };
        let carried = Carried {
            at: Guard::iso(r.at),
            msg: r.msg,
            room: Some(r.room.get()),
            from: from.get(),
        };
        self.inner
            .settings
            .update(&CARRIED, BTreeMap::new(), |all| {
                let key = to.get().to_string();
                let newer = all
                    .get(&key)
                    .and_then(|was| Guard::parse(&was.at))
                    .is_none_or(|was| was < r.at);
                if newer {
                    all.insert(key, carried);
                }
            })
            .await?;
        Ok(true)
    }

    /// A task the assistant just made in `room`: the owner's request when it comes from their
    /// message there, now or earlier ([`Self::request_in`]), logged on the task.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn mark_requested(&self, task: TaskId, room: RoomId) -> Result<bool, GuardError> {
        let Some(r) = self.request_in(room).await? else {
            return Ok(false);
        };
        let json =
            serde_json::json!({ "at": Guard::iso(r.at), "msg": r.msg, "room": r.room.get() });
        self.inner
            .workers
            .set_requested(task, &json.to_string())
            .await?;
        let place = self.room_name(r.room).await;
        let handed = if r.room == room {
            String::new()
        } else {
            format!(", handed to {}", self.room_name(room).await)
        };
        self.inner
            .tasks
            .note(
                task,
                Who::Assistant,
                &format!(
                    "Requested by the owner: their message at {} in {place}{handed}. Their request is their OK: it's done end to end, no asking (not payments, passwords, codes or account security; a message or post as them needs its text OK'd once)",
                    self.hhmm(r.at)
                ),
                false,
            )
            .await?;
        Ok(true)
    }

    /// The owner's new message in `room`: a clear yes there answers the Go ahead card it's
    /// about, as their tap would. The tasks whose card it answers: named in the yes or the
    /// question before it, or the one open card asked since their last message, within the
    /// chat's window. Never a hard step's card; a message's card only on a clear go.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn heard(&self, room: RoomId) -> Result<Vec<TaskId>, GuardError> {
        let now = self.now();
        let lines = self
            .inner
            .messages
            .said_since(room, now - chat_rules::WINDOW - chat_rules::WINDOW, 200)
            .await?;
        let Some(m) = lines
            .iter()
            .rev()
            .find(|m| m.author == Author::User)
            .cloned()
        else {
            return Ok(Vec::new());
        };
        if !go::is_yes(&m.text) {
            return Ok(Vec::new());
        }
        let mut cards: Vec<(TaskId, Timestamp)> = Vec::new();
        for (key, p) in self
            .inner
            .settings
            .get_or(&PENDING, BTreeMap::new())
            .await?
        {
            let Ok(id) = key.parse::<i64>() else { continue };
            let task = TaskId::new(id);
            if !p.hard
                && (p.text.is_none() || go::is_go(&m.text))
                && self.card_open(task, Some(&p.question)).await?
            {
                cards.push((task, Guard::parse(&p.at).unwrap_or(now)));
            }
        }
        for (key, p) in self.inner.settings.get_or(&PLANS, BTreeMap::new()).await? {
            let Ok(id) = key.parse::<i64>() else { continue };
            let task = TaskId::new(id);
            if p.ok.is_none() && self.card_open(task, p.question.as_deref()).await? {
                cards.push((task, Guard::parse(&p.asked).unwrap_or(now)));
            }
        }
        let mine: Vec<(TaskId, Timestamp)> = cards
            .into_iter()
            .filter(|(task, _)| room == RoomId::MAIN || Guard::room_of(*task) == room)
            .collect();
        let mut named = Vec::new();
        for (task, _) in &mine {
            if self
                .task_chat_ok(*task, &[room])
                .await?
                .is_some_and(|ok| ok.id == m.id.get())
            {
                named.push(*task);
            }
        }
        if !named.is_empty() {
            return Ok(named);
        }
        let next: Vec<TaskId> = mine
            .iter()
            .filter(|(_, asked)| {
                *asked <= m.created_at
                    && m.created_at.duration_since(*asked) <= chat_rules::WINDOW
                    && !lines
                        .iter()
                        .any(|l| l.author == Author::User && l.created_at > *asked && l.id < m.id)
            })
            .map(|(task, _)| *task)
            .collect();
        Ok(if next.len() == 1 { next } else { Vec::new() })
    }
}
