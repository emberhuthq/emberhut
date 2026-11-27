//! One job, one OK: the job's grant (the owner's request for the task, or one approval of
//! theirs on it; their Stop takes it back until their next OK), the OK of an exact text on a
//! task for any channel (used once), a post's OK, the cards about one step only, and the
//! standing OK for the people the owner lets the assistant message without asking. Kept in
//! the settings under the keys `jobGrants`, `textOks`, `postOks` and `stepCards`.

use std::collections::BTreeMap;

use emberhut_api as api;
use emberhut_core::RoomId;
use emberhut_core::guard::grants::{self as rules, StepKind, Tap};
use emberhut_core::guard::{chat_ok as chat_rules, go};
use emberhut_core::task::{Status, TaskId, Who};
use emberhut_db::{DbError, SettingKey};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::Guard;

/// The job's grant on a task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    /// When it was given.
    #[serde(default)]
    pub at: String,
    /// The line a step says for it.
    #[serde(default)]
    pub by: String,
    /// When the owner's Stop took it back.
    #[serde(default)]
    pub stopped: Option<String>,
}

/// The owner's OK of an exact text on a task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextOk {
    /// The text as OKs are matched ([`rules::text_key`]).
    pub key: String,
    /// The text.
    pub text: String,
    /// How the owner OK'd it.
    pub how: String,
    /// When.
    pub at: String,
    /// Where it was for.
    pub to: Option<String>,
    /// When it went out (its OK is used).
    pub spent: Option<String>,
}

/// A post's OK for the job.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostOk {
    /// When.
    pub at: String,
    /// How.
    pub how: String,
}

/// A card about one step only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepCard {
    /// Its question.
    pub q: String,
    /// `hard` or `said`.
    pub kind: String,
}

pub(crate) const JOB_GRANTS: SettingKey<BTreeMap<String, Grant>> = SettingKey::new("jobGrants");
pub(crate) const TEXT_OKS: SettingKey<BTreeMap<String, Vec<TextOk>>> = SettingKey::new("textOks");
pub(crate) const POST_OKS: SettingKey<BTreeMap<String, PostOk>> = SettingKey::new("postOks");
pub(crate) const STEP_CARDS: SettingKey<BTreeMap<String, Vec<StepCard>>> =
    SettingKey::new("stepCards");

/// The person's request for a task, as the tasks keep it (`requested`).
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Requested {
    /// When.
    #[serde(default)]
    pub(crate) at: Option<String>,
    /// The person's message it came from.
    #[serde(default)]
    pub(crate) msg: Option<i64>,
    /// The room it was said in.
    #[serde(default)]
    #[expect(dead_code, reason = "the stored shape, read whole")]
    pub(crate) room: Option<i64>,
}

/// A message or post step's OK: the line the step says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OkBy(pub String);

impl Guard {
    /// The task's request by the person, when it has one.
    pub(crate) async fn requested(&self, task: TaskId) -> Result<Option<Requested>, DbError> {
        Ok(self
            .task(task)
            .await?
            .and_then(|t| t.requested)
            .and_then(|r| serde_json::from_str(&r).ok()))
    }

    /// A done or canceled task holds no OK of the owner's.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn open_task(&self, task: TaskId) -> Result<bool, DbError> {
        Ok(self
            .task(task)
            .await?
            .is_some_and(|t| t.status != Status::Done && t.status != Status::Canceled))
    }

    /// The owner's OK for the whole job, logged once on the task.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn grant_job(
        &self,
        task: TaskId,
        by: &str,
        at: Option<Timestamp>,
    ) -> Result<(), super::GuardError> {
        let at = Self::iso(at.unwrap_or_else(|| self.now()));
        let grant = Grant {
            at,
            by: by.to_owned(),
            stopped: None,
        };
        let was = self
            .inner
            .settings
            .update(&JOB_GRANTS, BTreeMap::new(), |all| {
                all.insert(task.get().to_string(), grant)
            })
            .await?;
        if was.is_some_and(|w| !w.at.is_empty() && w.stopped.is_none()) {
            return Ok(());
        }
        self.inner
            .tasks
            .note(
                task,
                Who::Assistant,
                &format!(
                    "Job OK'd ({by}): its steps go through with no card while the task is open, in the browser, the owner's Mac's apps and its screen (not payments, passwords, codes or account security; a message or post needs its text OK'd once)"
                ),
                false,
            )
            .await?;
        Ok(())
    }

    /// The owner's Stop on one of the task's cards: the grant is off until their next OK.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn stop_job(&self, task: TaskId) -> Result<(), DbError> {
        let now = Self::iso(self.now());
        self.inner
            .settings
            .update(&JOB_GRANTS, BTreeMap::new(), |all| {
                let entry = all.entry(task.get().to_string()).or_default();
                entry.stopped = Some(now);
            })
            .await
    }

    /// The job's grant now, as the line a step says ("approved by your request at 16:34",
    /// "your Go ahead at 16:08"), or none: the owner's request, their latest OK on the task (or
    /// its parent's), their yes in the chat about it; a later Stop of theirs wins.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn job_grant(&self, task: TaskId) -> Result<Option<String>, super::GuardError> {
        // A configured agent's task (or no one's) has no job grant: its policies decide.
        if self.not_the_owners(task).await? {
            return Ok(None);
        }
        let mut id = task;
        for _ in 0..8 {
            let Some(t) = self.task(id).await? else {
                return Ok(None);
            };
            if t.status == Status::Done || t.status == Status::Canceled {
                return Ok(None);
            }
            let grants = self
                .inner
                .settings
                .get_or(&JOB_GRANTS, BTreeMap::new())
                .await?;
            let grant = grants.get(&id.get().to_string()).cloned();
            let stopped_after = |at: &str| {
                grant
                    .as_ref()
                    .and_then(|g| g.stopped.as_deref())
                    .and_then(Self::parse)
                    .zip(Self::parse(at))
                    .is_some_and(|(stopped, at)| stopped > at)
            };
            let mut has: Vec<(String, String)> = Vec::new();
            if let Some(r) = self.requested(id).await?
                && let Some(at) = r.at
            {
                let when =
                    Self::parse(&at).map_or_else(|| "Invalid Date".to_owned(), |t| self.hhmm(t));
                has.push((at, format!("approved by your request at {when}")));
            }
            if let Some(trusted) = self.trusted_request(&t).await? {
                has.push(trusted);
            }
            if let Some(g) = grant.as_ref().filter(|g| !g.at.is_empty()) {
                has.push((g.at.clone(), g.by.clone()));
            }
            has.sort_by(|a, b| Self::parse(&b.0).cmp(&Self::parse(&a.0)));
            if let Some((at, by)) = has.first()
                && !stopped_after(at)
            {
                return Ok(Some(by.clone()));
            }
            if let Some(ok) = self
                .task_chat_ok(id, &[Self::room_of(id), RoomId::MAIN])
                .await?
            {
                let at = Self::iso(ok.at);
                if !stopped_after(&at) {
                    let line = self.ok_line(&ok, RoomId::MAIN).await;
                    self.grant_job(id, &line, Some(ok.at)).await?;
                    return Ok(Some(line));
                }
            }
            if grant.as_ref().is_some_and(|g| g.stopped.is_some()) {
                return Ok(None);
            }
            match t.parent {
                Some(parent) => id = parent,
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Notes a card about one step only (a hard one's Go ahead is that step's; a message's
    /// Stop is a no to that text).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn step_card(
        &self,
        task: TaskId,
        question: &str,
        kind: StepKind,
    ) -> Result<(), DbError> {
        let card = StepCard {
            q: question.to_owned(),
            kind: if kind == StepKind::Hard {
                "hard"
            } else {
                "said"
            }
            .to_owned(),
        };
        self.inner
            .settings
            .update(&STEP_CARDS, BTreeMap::new(), |all| {
                let list = all.entry(task.get().to_string()).or_default();
                list.retain(|c| c.q != question);
                list.push(card);
                let extra = list.len().saturating_sub(10);
                list.drain(..extra);
            })
            .await
    }

    async fn card_kind(&self, task: TaskId, question: &str) -> Result<Option<StepKind>, DbError> {
        let all = self
            .inner
            .settings
            .get_or(&STEP_CARDS, BTreeMap::new())
            .await?;
        Ok(all
            .get(&task.get().to_string())
            .and_then(|list| list.iter().find(|c| c.q == question))
            .map(|c| {
                if c.kind == "hard" {
                    StepKind::Hard
                } else {
                    StepKind::Said
                }
            }))
    }

    /// The owner OK'd exactly `text` on the task (for `to`, when it names where).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn approve_text(
        &self,
        task: TaskId,
        text: &str,
        how: &str,
        to: Option<&str>,
    ) -> Result<(), DbError> {
        let key = rules::text_key(text);
        if key.is_empty() {
            return Ok(());
        }
        let ok = TextOk {
            key: key.clone(),
            text: text.to_owned(),
            how: how.to_owned(),
            at: Self::iso(self.now()),
            to: to.map(str::to_owned),
            spent: None,
        };
        self.inner
            .settings
            .update(&TEXT_OKS, BTreeMap::new(), |all| {
                let list = all.entry(task.get().to_string()).or_default();
                list.retain(|x| x.spent.is_some() || x.key != key);
                list.push(ok);
                let extra = list.len().saturating_sub(20);
                list.drain(..extra);
            })
            .await
    }

    /// The owner's OK for exactly this text on the task, not used yet.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn text_ok(&self, task: TaskId, text: &str) -> Result<Option<TextOk>, DbError> {
        let key = rules::text_key(text);
        if key.is_empty() {
            return Ok(None);
        }
        let all = self
            .inner
            .settings
            .get_or(&TEXT_OKS, BTreeMap::new())
            .await?;
        Ok(all
            .get(&task.get().to_string())
            .and_then(|list| list.iter().find(|x| x.key == key && x.spent.is_none()))
            .cloned())
    }

    /// `text` went out on the task (any channel): its OK is used, so no other channel sends it
    /// again. The OK it used, with how it went out.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn sent_text(
        &self,
        task: TaskId,
        text: &str,
        via: &str,
    ) -> Result<Option<TextOk>, DbError> {
        let key = rules::text_key(text);
        let now = Self::iso(self.now());
        self.inner
            .settings
            .update(&TEXT_OKS, BTreeMap::new(), |all| {
                let list = all.get_mut(&task.get().to_string())?;
                let ok = list
                    .iter_mut()
                    .find(|x| x.key == key && x.spent.is_none())?;
                ok.spent = Some(now);
                let mut used = ok.clone();
                used.how = format!("{}, sent by {via}", used.how);
                Some(used)
            })
            .await
    }

    /// Whether this text went out on the task already (its OK used): the same text again is
    /// a resend, and asks.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn was_sent(&self, task: TaskId, text: &str) -> Result<bool, DbError> {
        let key = rules::text_key(text);
        let all = self
            .inner
            .settings
            .get_or(&TEXT_OKS, BTreeMap::new())
            .await?;
        Ok(all
            .get(&task.get().to_string())
            .is_some_and(|list| list.iter().any(|x| x.key == key && x.spent.is_some())))
    }

    /// The owner OK'd a post (with no text of its own) for the job.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn approve_post(&self, task: TaskId, how: &str) -> Result<(), DbError> {
        let ok = PostOk {
            at: Self::iso(self.now()),
            how: how.to_owned(),
        };
        self.inner
            .settings
            .update(&POST_OKS, BTreeMap::new(), |all| {
                all.insert(task.get().to_string(), ok);
            })
            .await
    }

    /// How the owner OK'd the job's post, if they did.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn post_ok(&self, task: TaskId) -> Result<Option<String>, DbError> {
        Ok(self
            .inner
            .settings
            .get_or(&POST_OKS, BTreeMap::new())
            .await?
            .remove(&task.get().to_string())
            .map(|ok| ok.how))
    }

    /// The owner's standing OK to message the person a 1:1 chat is with, when one of `ids`
    /// (any id the chat goes by) is on their list ([`super::GuardConfig::send_without_ok`],
    /// read now). A group never counts.
    #[must_use]
    pub fn standing_ok(&self, ids: &[String], one_to_one: bool) -> Option<String> {
        if !one_to_one {
            return None;
        }
        let path = self.inner.config.send_without_ok.as_ref()?;
        let text = std::fs::read_to_string(path).ok()?;
        let list: BTreeMap<String, Vec<String>> = serde_json::from_str(&text).ok()?;
        let norm = |s: &str| s.trim().to_lowercase();
        let mine: Vec<String> = ids
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| norm(s))
            .collect();
        list.iter()
            .find(|(_, chats)| chats.iter().any(|c| mine.contains(&norm(c))))
            .map(|(name, _)| {
                format!("the owner's standing OK to message {name} without asking (sendWithoutOk)")
            })
    }

    /// A step that says something as the owner, with `composed` the text written on the page
    /// (what the button sends): its OK when the owner OK'd that text (their tap, or their go in
    /// the chat on it), used up here; a button that only opens a box to write in; or the job's
    /// post OK for a step with no text. None: it asks.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn message_ok(
        &self,
        task: TaskId,
        what: &str,
        composed: &[String],
    ) -> Result<Option<OkBy>, super::GuardError> {
        let written: Vec<&str> = composed
            .iter()
            .map(|x| emberhut_core::messaging::text::js_trim(x))
            .filter(|x| !x.is_empty())
            .collect();
        if written.is_empty() {
            if rules::opener(what) {
                return Ok(Some(OkBy(
                    "it only opens a box to write in: nothing is sent".to_owned(),
                )));
            }
            return Ok(self
                .post_ok(task)
                .await?
                .map(|p| OkBy(format!("in the post you OK'd ({p})"))));
        }
        for text in &written {
            if let Some(ok) = self.text_ok(task, text).await? {
                self.sent_text(task, text, &format!("\"{what}\"")).await?;
                return Ok(Some(OkBy(format!("the exact text you OK'd ({})", ok.how))));
            }
        }
        let mut rooms = vec![Self::room_of(task)];
        if !rooms.contains(&RoomId::MAIN) {
            rooms.push(RoomId::MAIN);
        }
        for room in rooms {
            for text in &written {
                if self.was_sent(task, text).await? {
                    continue;
                }
                if let Some(ok) = self.find_chat_ok(room, text, "", true, None).await?
                    && self.spend(ok.id).await?
                {
                    let line = self.ok_line(&ok, room).await;
                    self.approve_text(task, text, &line, None).await?;
                    self.sent_text(task, text, &format!("\"{what}\"")).await?;
                    return Ok(Some(OkBy(line)));
                }
            }
        }
        Ok(None)
    }

    /// The owner tapped `choice` on the task's open card: their OK for the job, a text (the
    /// ask's draft) or a post; or their Stop.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn heard_tap(&self, task: TaskId, choice: &str) -> Result<(), super::GuardError> {
        let Some(t) = self.task(task).await? else {
            return Ok(());
        };
        let decision: Option<api::tasks::Decision> = t
            .decision
            .as_deref()
            .and_then(|d| serde_json::from_str(d).ok());
        let question = decision.as_ref().map(|d| d.question.clone());
        let draft = decision.as_ref().and_then(|d| d.draft.clone());
        let chat = decision.as_ref().and_then(|d| d.chat.clone());
        let kind = match &question {
            Some(q) => self.card_kind(task, q).await?,
            None => None,
        };
        let now = self.now();
        let how = format!("your {choice} at {}", self.hhmm(now));
        match rules::heard_tap(choice, question.as_deref(), draft.as_deref(), kind) {
            Tap::Nothing | Tap::NoToStep => {}
            Tap::StopJob => self.stop_job(task).await?,
            Tap::Approve { text, post, job } => {
                if let Some(text) = text {
                    self.approve_text(task, &text, &how, chat.as_deref())
                        .await?;
                }
                if post {
                    self.approve_post(task, &how).await?;
                }
                if job {
                    self.grant_job(task, &how, Some(now)).await?;
                }
            }
        }
        Ok(())
    }

    /// The owner's yes on the task's page within the chat's window, as a line; else their yes
    /// in the chat about it.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn task_ok(&self, task: TaskId) -> Result<Option<String>, super::GuardError> {
        let since = self.now() - chat_rules::WINDOW;
        let history = self.inner.tasks.history_of(task).await?;
        let last = history.iter().rfind(|h| {
            h.who == Who::Person && h.at > since && !emberhut_core::guard::go_ahead::tapped(&h.text)
        });
        if let Some(last) = last
            && go::is_yes(&last.text)
        {
            return Ok(Some(format!(
                "their yes on the task at {}: «{}»",
                self.hhmm(last.at),
                last.text.chars().take(120).collect::<String>()
            )));
        }
        match self
            .task_chat_ok(task, &[Self::room_of(task), RoomId::MAIN])
            .await?
        {
            Some(ok) => Ok(Some(self.ok_line(&ok, RoomId::MAIN).await)),
            None => Ok(None),
        }
    }

    /// The whole screen of the owner's Mac: the job's grant, or the Mac idle long enough;
    /// never right after the owner stopped it (`stopped`: seconds since).
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn screen_ok(
        &self,
        task: TaskId,
        idle: Option<f64>,
        stopped: Option<f64>,
    ) -> Result<Option<String>, super::GuardError> {
        if stopped.is_some_and(|s| s < 600.0) {
            return Ok(None);
        }
        if let Some(grant) = self.job_grant(task).await? {
            return Ok(Some(grant));
        }
        #[expect(clippy::cast_precision_loss, reason = "a few minutes")]
        let idle_for = (rules::IDLE_MINUTES * 60) as f64;
        Ok(idle
            .filter(|idle| *idle >= idle_for)
            .map(|idle| format!("your Mac idle for {} minutes", (idle / 60.0).floor())))
    }
}
