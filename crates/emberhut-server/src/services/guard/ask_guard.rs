//! The ask guard: before a task's ask (the assistant's or a worker's) reaches the owner, the
//! owner's words are read (their request for the task, their messages since in every room,
//! their lines on the task, their standing rules and the facts they set) and a judge model
//! says whether they already cover the step ([`emberhut_core::guard::ask_guard`]'s rules check its
//! answer). Covered, the ask goes back as "covered, do it", with the owner's own words quoted,
//! and it is noted on the task. With no judge, or a judge that fails, the ask reaches them.
//! Each check is logged for the weekly review (`askGuard`, the last 300).

use std::fmt;

use emberhut_core::guard::ask_guard::{self as rules, Ask, JudgeAnswer, Line, Verdict};
use emberhut_core::message::Author;
use emberhut_core::task::{TaskId, Who};
use emberhut_core::{MessageId, RoomId};
use emberhut_db::SettingKey;
use futures_util::future::BoxFuture;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Guard, GuardError};

/// The judge model: one turn, no tools, its instructions and the prompt in, its text out.
pub trait Judge: fmt::Debug + Send + Sync {
    /// Whether there is a judge to ask: without one the guard is off.
    fn on(&self) -> bool {
        true
    }

    /// The judge's answer to `prompt` under `rules`; none when it couldn't answer.
    fn judge<'a>(&'a self, rules: &'a str, prompt: &'a str) -> BoxFuture<'a, Option<String>>;
}

/// No judge: the guard is off, and every ask reaches the owner.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoJudge;

impl Judge for NoJudge {
    fn on(&self) -> bool {
        false
    }

    fn judge<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
}

/// One check, as the log keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// When.
    pub at: String,
    /// `task_ask` or `reply`.
    pub via: String,
    /// The task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<i64>,
    /// The room.
    pub room: i64,
    /// What it asked, short.
    pub ask: String,
    /// `covered`, `reserved`, `not_covered`, `no_verdict` or `overridden`.
    pub outcome: String,
    /// The owner's words that covered it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// Why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

pub(crate) const LOG: SettingKey<Vec<Entry>> = SettingKey::new("askGuard");

/// The characters of the owner's messages the judge reads, newest first.
const HIS_BUDGET: usize = 60_000;
/// The characters of facts it reads.
const FACTS_BUDGET: usize = 50_000;

/// A task's ask, as the ask guard reads it (`task_ask`'s arguments).
#[derive(Clone, Debug, Default)]
pub struct TaskAsk {
    /// What the owner should do.
    pub action: Option<String>,
    /// Why now.
    pub context: Option<String>,
    /// The question.
    pub question: String,
    /// The brief's lines.
    pub brief: Vec<String>,
    /// The answers' labels.
    pub options: Vec<String>,
    /// A message to send as the owner.
    pub draft: Option<String>,
    /// Where it goes.
    pub chat: Option<String>,
    /// `approve`, `choose` or `do`.
    pub kind: Option<String>,
    /// The ask the owner already has, re-checked.
    pub same: bool,
    /// The asker says it isn't covered after it came back: why.
    pub not_covered: Option<String>,
}

impl Guard {
    /// Whether the guard is on: a judge is there to ask.
    fn guard_on(&self) -> bool {
        self.inner.judge.on()
    }

    /// The lines the judge reads, and the head (the task and its parents).
    #[expect(
        clippy::too_many_lines,
        reason = "one rule, kept whole to read in one go"
    )]
    async fn evidence(&self, task: Option<TaskId>) -> Result<(Vec<Line>, String), GuardError> {
        let now = self.now();
        let mut lines: Vec<Line> = Vec::new();
        let mut head: Vec<String> = Vec::new();
        let mut chain = Vec::new();
        let mut id = task;
        while let Some(at) = id {
            if chain.len() >= 6 {
                break;
            }
            let Some(t) = self.task(at).await? else { break };
            id = t.parent;
            chain.push(t);
        }
        let mut starts = Vec::new();
        for t in &chain {
            starts.push(t.created);
            if let Some(r) = self.requested(t.id).await?
                && let Some(at) = r.at.as_deref().and_then(Guard::parse)
            {
                starts.push(at);
            }
        }
        let earliest = starts.iter().min().copied().unwrap_or(now) - SignedDuration::from_hours(1);
        let since = (now - SignedDuration::from_hours(14 * 24))
            .max((now - SignedDuration::from_hours(48)).min(earliest));
        for t in &chain {
            let parent = if Some(t.id) == task {
                ""
            } else {
                " (its parent)"
            };
            head.push(format!("{}{parent}: {}", t.key(), t.title));
            if let Some(r) = self.requested(t.id).await?
                && let Some(msg) = r.msg
                && let Some(m) = self.inner.messages.get(MessageId::new(msg)).await?
                && m.author == Author::User
            {
                let room = self.room_name(m.room).await;
                lines.push(Line {
                    tag: format!("m{msg}"),
                    text: format!(
                        "The owner's request for {} ({room}, {}): {}",
                        t.key(),
                        self.hhmm(m.created_at),
                        m.text.chars().take(3000).collect::<String>()
                    ),
                    said: m.text.clone(),
                });
            }
            for h in self.inner.tasks.history_of(t.id).await? {
                if h.who != Who::Person || h.at > now {
                    continue;
                }
                let when = self.hhmm(h.at);
                lines.push(Line {
                    tag: format!("t{}@{when}", t.id.get()),
                    text: format!(
                        "The owner on {} ({when}): {}",
                        t.key(),
                        h.text.chars().take(1500).collect::<String>()
                    ),
                    said: h.text,
                });
            }
        }
        if let Some(task) = task {
            if let Some(grant) = self.job_grant(task).await? {
                head.push(format!("Its job grant now: {grant}"));
            }
            if let Some(ok) = self.covered_by(task).await? {
                head.push(format!("Its push, merge and deploy: OK'd by {ok}"));
            }
        }
        // The owner's messages in every room since then, each with the assistant's lines before it.
        let mut convo: Vec<Line> = Vec::new();
        let rooms = self.inner.rooms.list().await?;
        for room in rooms {
            let mut before: Vec<String> = Vec::new();
            for m in self.inner.messages.said_since(room.id, since, 2000).await? {
                if m.author != Author::User {
                    let flat = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !flat.is_empty() {
                        before.push(flat.chars().take(400).collect());
                    }
                    let extra = before.len().saturating_sub(2);
                    before.drain(..extra);
                    continue;
                }
                let mut prev = String::new();
                for line in before.drain(..) {
                    prev.push_str("\n   (The assistant before it: ");
                    prev.push_str(&line);
                    prev.push(')');
                }
                convo.push(Line {
                    tag: format!("m{}", m.id.get()),
                    text: format!(
                        "The owner ({}, {}): {}{prev}",
                        room.name,
                        self.hhmm(m.created_at),
                        m.text.chars().take(1500).collect::<String>()
                    ),
                    said: m.text,
                });
            }
        }
        convo.sort_by(|a, b| a.tag.cmp(&b.tag));
        let mut size = 0;
        let mut kept = Vec::new();
        for line in convo.into_iter().rev() {
            if size >= HIS_BUDGET {
                break;
            }
            size += line.text.len();
            kept.push(line);
        }
        kept.reverse();
        for line in kept {
            if !lines.iter().any(|l| l.tag == line.tag) {
                lines.push(line);
            }
        }
        lines.extend(self.standing_lines().await);
        Ok((lines, head.join("\n")))
    }

    /// The owner's standing OKs: the rules they OK'd and the facts they set.
    async fn standing_lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        if let Some(learning) = self.inner.learning.get() {
            match learning.lessons().await {
                Ok(lessons) => lines.extend(lessons.into_iter().map(|l| Line {
                    tag: format!("rule{}", l.id),
                    text: format!("Standing rule the owner OK'd: {}", l.text),
                    said: l.text,
                })),
                Err(error) => tracing::warn!(%error, "the ask guard couldn't read the rules"),
            }
        }
        if let Some(facts) = self.inner.facts.get() {
            let mut size = 0;
            for category in ["preferences", "work", "money", "projects"] {
                let Ok(list) = facts.facts(Some(category)).await else {
                    continue;
                };
                for f in list {
                    let source = if f.source.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " [source: {}]",
                            f.source.chars().take(120).collect::<String>()
                        )
                    };
                    let text = format!(
                        "Fact ({}): {}. {}{source}",
                        f.category,
                        f.title,
                        f.text.chars().take(1200).collect::<String>()
                    );
                    size += text.len();
                    if size > FACTS_BUDGET {
                        break;
                    }
                    lines.push(Line {
                        tag: format!("fact:{}", f.id),
                        text,
                        said: format!("{}. {}", f.title, f.text),
                    });
                }
            }
        }
        lines
    }

    /// The verdict for `ask` on `task` (asked in `room`), or none when there is none to act
    /// on (no judge, it failed, or its answer wasn't one). A kind that always asks comes back
    /// reserved, without asking the judge.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn check_ask(
        &self,
        task: Option<TaskId>,
        room: RoomId,
        ask: &Ask,
    ) -> Result<Option<Verdict>, GuardError> {
        if let Some(kind) = rules::reserved(ask) {
            return Ok(Some(rules::always(kind)));
        }
        if !self.guard_on() {
            return Ok(None);
        }
        let (lines, head) = self.evidence(task).await?;
        let room_name = self.room_name(room).await;
        let now = self.now();
        let when = now
            .to_zoned_datetime(self.inner.zone.clone())
            .strftime("%-d %b %Y %H:%M")
            .to_string();
        let prompt = rules::prompt(ask, &head, &room_name, &lines, &when);
        let Some(said) = self.inner.judge.judge(rules::RULES, &prompt).await else {
            return Ok(None);
        };
        let Some(answer) = read_answer(&said) else {
            tracing::info!("the ask guard's judge didn't answer in JSON");
            return Ok(None);
        };
        Ok(Some(rules::check(answer, ask, &lines)))
    }

    /// Logs one check.
    async fn record(
        &self,
        via: &str,
        task: Option<TaskId>,
        room: RoomId,
        ask: &str,
        verdict: Option<&Verdict>,
        overridden: Option<&str>,
    ) -> Result<(), GuardError> {
        let entry = Entry {
            at: Guard::iso(self.now()),
            via: via.to_owned(),
            task: task.map(TaskId::get),
            room: room.get(),
            ask: ask.trim().chars().take(200).collect(),
            outcome: rules::outcome(verdict, overridden.is_some()).to_owned(),
            quote: verdict.and_then(|v| v.quote.clone()),
            why: overridden.map(str::to_owned).or_else(|| {
                verdict.map(|v| {
                    v.reserved
                        .clone()
                        .unwrap_or_else(|| v.why.clone())
                        .chars()
                        .take(160)
                        .collect()
                })
            }),
        };
        tracing::info!(via, outcome = %entry.outcome, "ask guard");
        self.inner
            .settings
            .update(&LOG, Vec::new(), |log| {
                log.push(entry);
                let extra = log.len().saturating_sub(300);
                log.drain(..extra);
            })
            .await?;
        Ok(())
    }

    /// The checks logged, oldest first.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn ask_log(&self) -> Result<Vec<Entry>, GuardError> {
        Ok(self.inner.settings.get_or(&LOG, Vec::new()).await?)
    }

    /// `task_ask`'s guard: the text it answers instead of asking, or none (ask the owner).
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn task_ask(
        &self,
        task: TaskId,
        by: Who,
        a: &TaskAsk,
    ) -> Result<Option<String>, GuardError> {
        let mut parts: Vec<&str> = [a.action.as_deref(), a.context.as_deref(), Some(&a.question)]
            .into_iter()
            .flatten()
            .collect();
        parts.extend(a.brief.iter().map(String::as_str));
        let text = parts
            .into_iter()
            .filter(|x| !x.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if a.same || a.kind.as_deref() == Some("do") {
            return Ok(None);
        }
        let room = Guard::room_of(task);
        if let Some(why) = a.not_covered.as_deref().filter(|w| !w.is_empty()) {
            self.record("task_ask", Some(task), room, &text, None, Some(why))
                .await?;
            self.inner
                .tasks
                .note(
                    task,
                    by,
                    &format!(
                        "Asked though the ask guard sent it back: {}",
                        why.chars().take(200).collect::<String>()
                    ),
                    false,
                )
                .await?;
            return Ok(None);
        }
        let ask = Ask {
            text: text.clone(),
            options: a.options.clone(),
            draft: a.draft.clone(),
            chat: a.chat.clone(),
        };
        if rules::reserved(&ask).is_none()
            && rules::is_merge(&text)
            && let Some(named) = self.covered_by(task).await?
        {
            let verdict = Verdict {
                covered: true,
                quote: Some(named.clone()),
                place: Some("the outward rule".to_owned()),
                ..Verdict::default()
            };
            self.record("task_ask", Some(task), room, &text, Some(&verdict), None)
                .await?;
            self.inner
                .tasks
                .note(
                    task,
                    Who::Assistant,
                    &format!("Ask held by the ask guard: its merge is already OK'd by {named}. Sent back: do it"),
                    false,
                )
                .await?;
            return Ok(Some(format!(
                "Not asked: covered, do it. Its push, merge and deploy are OK'd by {named}. Don't ask the owner; run it now."
            )));
        }
        let verdict = self.check_ask(Some(task), room, &ask).await?;
        self.record("task_ask", Some(task), room, &text, verdict.as_ref(), None)
            .await?;
        let Some(verdict) = verdict.filter(|v| v.covered) else {
            return Ok(None);
        };
        let asked: String = a
            .action
            .as_deref()
            .unwrap_or(&a.question)
            .chars()
            .take(160)
            .collect();
        self.inner
            .tasks
            .note(
                task,
                Who::Assistant,
                &format!(
                    "Ask held by the ask guard: the owner's words cover it, {}. Sent back: do it. It asked: {asked}",
                    rules::covered_line(&verdict)
                ),
                false,
            )
            .await?;
        Ok(Some(rules::sent_back(&verdict)))
    }

    /// The assistant's reply in `room` before it reaches the owner: when it ends by asking
    /// their OK for a step their words cover, what the room hears instead (the reply is held).
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn reply(&self, room: RoomId, reply: &str) -> Result<Option<String>, GuardError> {
        if !rules::asking(reply) {
            return Ok(None);
        }
        let ask = Ask {
            text: reply.to_owned(),
            ..Ask::default()
        };
        let verdict = self.check_ask(None, room, &ask).await?;
        self.record("reply", None, room, reply, verdict.as_ref(), None)
            .await?;
        Ok(verdict
            .filter(|v| v.covered)
            .map(|v| rules::reply_back(&v, reply)))
    }
}

/// The judge's JSON object, from the first `{` to the last `}` of what it said.
fn read_answer(said: &str) -> Option<JudgeAnswer> {
    let start = said.find('{')?;
    let end = said.rfind('}')?;
    let value: Value = serde_json::from_str(said.get(start..=end)?).ok()?;
    let text = |key: &str| match value.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    };
    Some(JudgeAnswer {
        covered: value
            .get("covered")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reserved: text("reserved"),
        answer: text("answer"),
        quote: text("quote"),
        tag: text("tag"),
        why: text("why").unwrap_or_default(),
    })
}
