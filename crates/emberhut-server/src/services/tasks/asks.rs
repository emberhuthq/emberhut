//! Asks: the one decision a task needs from the person, their answer from the app, and their
//! reply on a task's page.

use emberhut_api as api;
use emberhut_core::task::ask::{
    Card, OptionIn, action_from, answer_line, cancels, choices_of, plain_text,
};
use emberhut_core::task::{AskId, Owner, Status, TaskChange, TaskId, TaskRefusal, Who, date_only};
use jiff::SignedDuration;

use super::{TaskError, TaskService};
use crate::convert::timestamp_to_api;

/// An ask, as the assistant or a worker puts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AskInput {
    /// The question: the advice and what the cards don't say.
    pub question: String,
    /// The answers.
    pub options: Vec<OptionIn>,
    /// What the person should do, one short line (none: made from the question).
    pub action: Option<String>,
    /// Why now, a line.
    pub context: Option<String>,
    /// `approve`, `choose` or `do`.
    pub kind: Option<String>,
    /// A few bullets of what the person needs to know.
    pub brief: Option<Vec<String>>,
    /// What each plain answer does.
    pub notes: Option<Vec<String>>,
    /// The pick: an index into the options.
    pub recommend: Option<usize>,
    /// A message for the person to send, word for word.
    pub draft: Option<String>,
    /// Where the draft goes.
    pub chat: Option<String>,
    /// The ask the person already has, re-checked: it keeps its asked time.
    pub same: bool,
    /// The day to decide by.
    pub decide_by: Option<String>,
}

/// A file the person attached to a reply, as the upload route answered it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageFile {
    /// Its kind (`image`, `file`, ...).
    pub kind: Option<String>,
    /// Where it is kept.
    pub path: Option<String>,
}

fn card(c: &Card) -> api::choices::Choice {
    api::choices::Choice {
        label: c.label.clone(),
        title: c.title.clone(),
        summary: c.summary.clone(),
        badge: c.badge.clone(),
        then: c.then.clone(),
        more: c.more.clone(),
        fields: c.fields.as_ref().map(|fields| {
            fields
                .iter()
                .map(|(k, v)| api::choices::ChoiceField {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect()
        }),
        why: c.why.clone(),
        media: c.media.as_ref().map(|media| {
            media
                .iter()
                .map(|m| api::ids::ArtifactId::new(m.get()))
                .collect()
        }),
    }
}

/// The answer to a reply on a task's page that went to the assistant.
fn to_assistant() -> api::task_message::TaskMessageReply {
    api::task_message::TaskMessageReply {
        ok: true,
        to: api::task_message::MessageTo::Assistant,
        session: None,
        queued: None,
    }
}

fn decision_of(task: &emberhut_core::task::Task) -> Option<api::tasks::Decision> {
    task.decision
        .as_deref()
        .and_then(|d| serde_json::from_str(d).ok())
}

impl TaskService {
    /// Puts the decision a task needs in front of the person: it waits, with its question, its
    /// answers (plain ones, or cards to compare), what each does, the pick, an action (made
    /// from the question when none is given) and a day to decide by (the one it had, else a
    /// week out when it has no deadline to go by). A re-check (`same`) keeps when it was first
    /// asked, and the cards it had when it sends the same labels alone.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for answers the rules refuse, a bad day or no such task.
    pub async fn ask(
        &self,
        id: TaskId,
        input: AskInput,
        by: Who,
    ) -> Result<api::tasks::Issue, TaskError> {
        Ok(self.ask_with_id(id, input, by).await?.0)
    }

    /// [`Self::ask`], with the ask's id: the person's answers name it, and any later ask on the
    /// task (a re-check too) has another.
    ///
    /// # Errors
    ///
    /// As [`Self::ask`].
    pub async fn ask_with_id(
        &self,
        id: TaskId,
        input: AskInput,
        by: Who,
    ) -> Result<(api::tasks::Issue, AskId), TaskError> {
        let choices = choices_of(&input.options, input.notes.clone(), input.recommend)?;
        let task = self.task(id).await?;
        let was = decision_of(&task);
        let now = self.now();
        let asked = match (&was, input.same) {
            (Some(was), true) => was
                .asked
                .clone()
                .unwrap_or_else(|| timestamp_to_api(task.updated)),
            _ => timestamp_to_api(now),
        };
        let mut cards: Option<Vec<api::choices::Choice>> = choices
            .cards
            .as_ref()
            .map(|cards| cards.iter().map(card).collect());
        let mut context = input.context.clone();
        let mut recommend = choices.recommend;
        if let Some(was) = &was
            && input.same
            && cards.is_none()
            && was.choices.is_some()
            && was.options == choices.options
        {
            cards.clone_from(&was.choices);
            context = context.or_else(|| was.context.clone());
            recommend = recommend.or_else(|| was.recommend.and_then(|r| usize::try_from(r).ok()));
        }
        let decide_by = match input.decide_by.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(text) => Some(date_only(text, &self.zone).map_err(TaskError::Refused)?),
            None => match task.decide_by {
                Some(day) => Some(day),
                None if self.deadline_of(&task).await?.is_none() => Some(
                    (now + SignedDuration::from_hours(24 * 6))
                        .to_zoned_datetime(self.zone.clone())
                        .date(),
                ),
                None => None,
            },
        };
        let decision = api::tasks::Decision {
            question: input.question.clone(),
            options: choices.options.clone(),
            draft: input
                .draft
                .as_deref()
                .filter(|d| !d.trim().is_empty())
                .map(plain_text),
            kind: input.kind.as_deref().map(api::tasks::AskKind::from),
            brief: input.brief.clone(),
            notes: choices.notes.clone(),
            recommend: recommend.and_then(|r| i32::try_from(r).ok()),
            context,
            asked: Some(asked),
            checked: Some(timestamp_to_api(now)),
            choices: cards,
            chat: input.chat.clone(),
        };
        let action = input
            .action
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map_or_else(
                || action_from(&input.question, &choices.options),
                str::to_owned,
            );
        let line = if input.same && was.is_some() {
            "Re-checked: still needs you".to_owned()
        } else {
            input.question.clone()
        };
        let json = serde_json::to_string(&decision)
            .map_err(|error| TaskError::refused(format!("the ask could not be kept: {error}")))?;
        let (_, ask) = self
            .tasks
            .ask(id, json, action, decide_by, &line, by, now)
            .await?;
        let mut named = input.question.clone();
        for line in input.brief.iter().flatten() {
            named.push('\n');
            named.push_str(line);
        }
        self.scan(id, &named, true).await?;
        let issue = self.issue(id).await?.ok_or(TaskRefusal::NoTask(id))?;
        Ok((issue, ask))
    }

    /// The task's latest ask; none when it was never asked anything.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn last_ask(&self, id: TaskId) -> Result<Option<AskId>, TaskError> {
        Ok(self.tasks.last_ask(id).await?)
    }

    /// The person's answers to the ask `ask` of the task, oldest first: each answer names the
    /// ask that was the task's latest when it was given.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn answers_to(
        &self,
        id: TaskId,
        ask: AskId,
    ) -> Result<Vec<emberhut_db::HistoryLine>, TaskError> {
        Ok(self.tasks.answers_to(id, ask).await?)
    }

    /// Whether the task's ask approves something to go out (`kind: approve`): only a signed-in
    /// device may answer it, whatever the auth mode.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] when there is no such task.
    pub async fn asks_approval(&self, id: TaskId) -> Result<bool, TaskError> {
        let task = self.task(id).await?;
        Ok(decision_of(&task).is_some_and(|d| d.kind == Some(api::tasks::AskKind::Approve)))
    }

    /// The person's answer `line` to the task's latest ask goes to its history, naming that ask
    /// (a key or token in it masked, as in every history line).
    async fn note_answer(&self, id: TaskId, line: &str) -> Result<(), TaskError> {
        let line = &emberhut_core::guard::key_mask::mask(line, &[], None);
        if !self.tasks.answer(id, Who::Person, line, self.now()).await? {
            return Err(TaskRefusal::NoTask(id).into());
        }
        Ok(())
    }

    /// A task's deadline as it counts: its own, else its project's.
    async fn deadline_of(
        &self,
        task: &emberhut_core::task::Task,
    ) -> Result<Option<emberhut_core::task::Deadline>, TaskError> {
        if let Some(own) = task.own_deadline() {
            return Ok(Some(own));
        }
        let Some(pid) = task.pid else { return Ok(None) };
        Ok(self.projects.get(pid).await?.and_then(|p| p.deadline()))
    }

    /// The person's answer to an ask (`taskDecide`): `Leave it`, `Not now` and `No` put the
    /// task away; otherwise it carries on: the person's own task goes back to their list and
    /// the assistant is told, the assistant's task that a worker does is queued for its worker
    /// with the answer.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] when there is no such task.
    pub async fn decide(&self, id: TaskId, choice: &str) -> Result<(), TaskError> {
        let task = self.task(id).await?;
        let decision = decision_of(&task);
        let question = decision.as_ref().map_or_default(|d| d.question.clone());
        let said = if question.is_empty() {
            format!(
                "The person says they did it (\"{}\"): {choice}.",
                task.action.as_deref().unwrap_or(&task.title)
            )
        } else {
            let options = decision.as_ref().map_or_default(|d| d.options.clone());
            let cards: Option<Vec<Card>> =
                decision
                    .as_ref()
                    .and_then(|d| d.choices.as_ref())
                    .map(|cs| {
                        cs.iter()
                            .map(|c| Card {
                                label: c.label.clone(),
                                title: c.title.clone(),
                                ..Card::default()
                            })
                            .collect()
                    });
            format!(
                "The person answered \"{question}\": {}.",
                answer_line(&options, cards.as_deref(), choice)
            )
        };
        let line = if question.is_empty() {
            format!("{choice}: {}", task.action.as_deref().unwrap_or("did it"))
        } else {
            format!("Chose: {choice}")
        };
        self.note_answer(id, &line).await?;
        if self.answer_taken(id, choice).await {
            return Ok(());
        }
        if cancels(choice) {
            self.update(id, TaskChange::status(Status::Canceled), Who::Person, None)
                .await?;
            return Ok(());
        }
        if task.owner != Owner::Assistant || task.tracked {
            let status = if task.owner == Owner::Assistant {
                Status::Working
            } else {
                Status::Todo
            };
            self.update(id, TaskChange::status(status), Who::Person, None)
                .await?;
            self.tell_about(
                &task,
                "plan",
                format!(
                    "{} \"{}\": {said} Carry it on (update the task).",
                    task.key(),
                    task.title
                ),
                "was answered by the owner",
                None,
            )
            .await?;
            return Ok(());
        }
        self.tasks.queue_next(id, &said, true, self.now()).await?;
        Ok(())
    }

    /// The person's reply on a task's page: it is the task's, never Main's. It goes where an
    /// answer goes and clears the ask: to the task's worker (its next run carries on with it;
    /// queued while one runs), else to the assistant, told to answer on the task.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] for an empty reply or no such task.
    pub async fn message(
        &self,
        id: TaskId,
        text: &str,
        files: &[MessageFile],
    ) -> Result<api::task_message::TaskMessageReply, TaskError> {
        let task = self.task(id).await?;
        let text = text.trim();
        if text.is_empty() && files.is_empty() {
            return Err(TaskError::refused("nothing to send"));
        }
        let question = if task.status == Status::Waiting {
            decision_of(&task).map_or_default(|d| d.question)
        } else {
            String::new()
        };
        let pics = match files {
            [] => String::new(),
            [one] => format!(
                "{}(1 {} attached)",
                if text.is_empty() { "" } else { " " },
                if one.kind.as_deref() == Some("image") {
                    "photo"
                } else {
                    "file"
                }
            ),
            many => format!(
                "{}({} files attached)",
                if text.is_empty() { "" } else { " " },
                many.len()
            ),
        };
        self.note(id, Who::Person, &format!("{text}{pics}"), false)
            .await?;
        if self.reply_taken(id, text).await {
            return Ok(to_assistant());
        }
        let on = if question.is_empty() {
            task.key()
        } else {
            format!("your question (\"{question}\")")
        };
        let mut said = format!(
            "The person wrote on {on}: {}",
            if text.is_empty() {
                "(see the attached)"
            } else {
                text
            }
        );
        let paths: Vec<&str> = files.iter().filter_map(|f| f.path.as_deref()).collect();
        if !paths.is_empty() {
            said = format!("{said}\nTheir files: {}", paths.join(", "));
        }
        if task.owner == Owner::Assistant
            && !task.tracked
            && task.body.as_deref().is_some_and(|b| !b.trim().is_empty())
        {
            let running = task.worker.is_some();
            self.tasks
                .queue_next(id, &said, !running, self.now())
                .await?;
            return Ok(api::task_message::TaskMessageReply {
                ok: true,
                to: api::task_message::MessageTo::Worker,
                session: None,
                queued: Some(running),
            });
        }
        let status = if task.owner == Owner::Assistant {
            Status::Working
        } else {
            Status::Todo
        };
        self.update(
            id,
            TaskChange {
                status: Some(status),
                action: Some(None),
                ..TaskChange::default()
            },
            Who::Person,
            None,
        )
        .await?;
        self.tell_about(
            &task,
            "owner",
            format!(
                "{} \"{}\": {said}\nThey wrote it on the task's page, so answer them there (task_note with your reply, or task_ask if it needs them again) and carry the task on; don't answer in the chat.",
                task.key(),
                task.title
            ),
            "has a message from the owner on its page",
            None,
        )
        .await?;
        Ok(to_assistant())
    }

    /// A worker's or the assistant's task is finished and needs nothing from the person: the
    /// summary goes to its history and it is done.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn done(
        &self,
        id: TaskId,
        summary: &str,
        by: Who,
    ) -> Result<api::tasks::Issue, TaskError> {
        self.note(id, by, summary, true).await?;
        self.update(id, TaskChange::status(Status::Done), by, None)
            .await?;
        self.issue(id)
            .await?
            .ok_or_else(|| TaskRefusal::NoTask(id).into())
    }

    /// The task in full, as the tools answer it: its fields and its history.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn task_json(&self, id: TaskId) -> Result<serde_json::Value, TaskError> {
        let detail = self.detail(id).await?.ok_or(TaskRefusal::NoTask(id))?;
        let mut value =
            serde_json::to_value(&detail).map_err(|error| TaskError::refused(error.to_string()))?;
        if let serde_json::Value::Object(object) = &mut value {
            object.remove("sessions");
            object.remove("ultraRun");
        }
        Ok(value)
    }
}

/// How long an ask waits on the person after the assistant last made sure it holds, before it
/// checks again.
const RECHECK_AFTER: SignedDuration = SignedDuration::from_hours(12);

impl TaskService {
    /// The tick's work: paused projects whose day has come are back (Main hears it), and an ask
    /// that waited 12 hours since it was last checked is handed to the assistant to check
    /// again, quietly, once per check. Returns how many of each.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn tick(&self) -> Result<(usize, usize), TaskError> {
        let now = self.now();
        let today = now.to_zoned_datetime(self.zone.clone()).date();
        let resumed = self.projects.resume_due(today, now).await?;
        for p in &resumed {
            self.tell(
                "plan",
                format!(
                    "Project \"{}\" is back from its pause (its date came): its room is in the switcher again.",
                    p.name
                ),
                Some(format!("resumed:{}:{today}", p.id.get())),
            )
            .await?;
        }
        let mut rechecked = 0;
        for task in self.tasks.asking().await? {
            let Some(decision) = decision_of(&task) else {
                continue;
            };
            let checked = decision
                .checked
                .as_ref()
                .and_then(|c| c.as_str().parse::<jiff::Timestamp>().ok())
                .unwrap_or(task.updated);
            if now.duration_since(checked) < RECHECK_AFTER {
                continue;
            }
            let asked = decision.asked.as_ref().map_or_else(
                || timestamp_to_api(task.updated).as_str().to_owned(),
                |a| a.as_str().to_owned(),
            );
            let told = self.tell_about(
                &task,
                "recheck",
                format!(
                    "{} \"{}\" has waited on the person since {asked} (action: \"{}\"). Check it quietly against what is true now: its history, its threads, the machine, their chats. Already done, outdated or moot: close it (task_update to done or working, with a note on why). Still needed: task_ask again with same=true and fresh facts. Say nothing in the chat about it unless something there is urgent.",
                    task.key(),
                    task.title,
                    task.action.as_deref().unwrap_or("null")
                ),
                &format!(
                    "has waited on the owner since {asked}: it goes on when they answer \
                     (task_cancel it, saying why, if it is no longer needed)"
                ),
                Some(format!("recheck:{}:{}", task.id.get(), timestamp_to_api(checked).as_str())),
            )
            .await?;
            rechecked += usize::from(told);
        }
        Ok((resumed.len(), rechecked))
    }

    /// Runs [`Self::tick`] every `every` until `stop`.
    pub async fn keep_ticking(
        self,
        every: std::time::Duration,
        stop: tokio_util::sync::CancellationToken,
    ) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            if let Err(error) = self.tick().await {
                tracing::warn!(%error, "the tasks' tick failed");
            }
        }
    }

    /// A worker's session started on a task (the workers' area calls this): it is a run of the
    /// task, and the task shows it running.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn worker_started(
        &self,
        id: TaskId,
        unit: &str,
        brief: &str,
    ) -> Result<(), TaskError> {
        self.task(id).await?;
        self.runs
            .open(
                emberhut_core::task::live::NewRun {
                    task: id,
                    machine: emberhut_core::task::live::RunKind::Worker
                        .as_str()
                        .to_owned(),
                    thread: unit.to_owned(),
                    kind: emberhut_core::task::live::RunKind::Worker,
                    asked: Some(brief.to_owned()),
                    started: true,
                },
                self.now(),
            )
            .await?;
        self.tasks
            .set_worker(id, Some(unit.to_owned()), Some(unit.to_owned()))
            .await?;
        Ok(())
    }

    /// A worker's session ended: its run closes (stopped while its task is still marked
    /// working, completed otherwise) with the task's last line.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn worker_ended(&self, unit: &str) -> Result<(), TaskError> {
        let Some(run) = self.runs.open_worker(unit).await? else {
            return Ok(());
        };
        let task = match run.task {
            Some(id) => self.tasks.get(id).await?,
            None => None,
        };
        let outcome = if task.as_ref().is_some_and(|t| t.status == Status::Working) {
            "stopped"
        } else {
            "completed"
        };
        self.runs
            .close(
                run.id,
                outcome,
                task.as_ref().and_then(|t| t.last.as_deref()),
                self.now(),
            )
            .await?;
        if let Some(task) = task {
            self.tasks.set_worker(task.id, None, None).await?;
        }
        Ok(())
    }
}
