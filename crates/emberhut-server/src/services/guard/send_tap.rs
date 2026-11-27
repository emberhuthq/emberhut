//! The owner's Send it on a message's card (Slack's or WhatsApp's) sends it there and then:
//! the server sends the card's exact text to its chat itself, through the channel's own guard
//! (one message per OK, never a resend), the task shows Sent or Failed (`sendTaps`) and the
//! room that asked is told. Cancel closes the card and tells the room. A second tap on a card
//! already answered does nothing.
//!
//! [`GuardOks`] is what messaging asks for the owner's other OKs ([`SendOks`]): the tap just
//! made, their go in the chat on the exact text, an OK of the exact text on another card, the
//! standing OK for a 1:1 chat; and what went out is spent for every channel.

use std::collections::BTreeMap;
use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::event::NewEvent;
use emberhut_core::guard::send_tap as rules;
use emberhut_core::task::{AskId, TaskChange, TaskId, Who};
use emberhut_db::SettingKey;
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Guard, GuardError};
use crate::services::messaging::guard::{Card, CardAnswer};
use crate::services::messaging::hooks::{self, ChatOk, SendOks, TextOk};

/// What the owner's tap came to, as the app shows it on the task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// `sending`, `sent`, `failed` or `canceled`.
    pub state: String,
    /// When.
    pub at: String,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// `Slack` or `WhatsApp`.
    #[serde(default)]
    pub via: String,
    /// Who it was for.
    #[serde(default)]
    pub to: String,
}

pub(crate) const RESULTS: SettingKey<BTreeMap<String, SendResult>> = SettingKey::new("sendTaps");

impl Guard {
    /// What the owner's last tap on a task's send card came to.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn send_result(&self, task: TaskId) -> Result<Option<SendResult>, GuardError> {
        Ok(self
            .inner
            .settings
            .get_or(&RESULTS, BTreeMap::new())
            .await?
            .remove(&task.get().to_string()))
    }

    async fn set_result(&self, task: TaskId, result: SendResult) -> Result<(), GuardError> {
        self.inner
            .settings
            .update(&RESULTS, BTreeMap::new(), |all| {
                all.insert(task.get().to_string(), result);
                while all.len() > 200 {
                    let Some(first) = all.keys().next().cloned() else {
                        break;
                    };
                    all.remove(&first);
                }
            })
            .await?;
        Ok(())
    }

    /// The send card open on `task` that the ask `ask` shows, and which channel's. The ask's
    /// id is what ties them: another card on the task may carry the same question.
    async fn open_card(&self, task: TaskId, ask: AskId) -> Option<(Card, bool)> {
        let messaging = self.inner.messaging.get()?;
        if let Ok(Some(card)) = messaging.slack().card(task.get()).await
            && card.ask == Some(ask)
        {
            return Some((card, true));
        }
        if let Ok(Some(card)) = messaging.whatsapp_send().card(task.get()).await
            && card.ask == Some(ask)
        {
            return Some((card, false));
        }
        None
    }

    /// The owner tapped `choice` on `task`'s open ask `ask` (none: no ask is open): `true` when
    /// it was a send card's (sent or canceled now, the room told), or a second tap on one
    /// already answered.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn tap_send(
        &self,
        task: TaskId,
        choice: &str,
        ask: Option<AskId>,
    ) -> Result<bool, GuardError> {
        let card = match ask {
            Some(ask) => self.open_card(task, ask).await,
            None => None,
        };
        let Some((card, slack)) = card else {
            let answered = ask.is_none()
                && self.send_result(task).await?.is_some()
                && (choice == "Send it" || choice == "Cancel" || rules::sends_it(choice));
            return Ok(answered);
        };
        let go = choice == "Send it" || rules::sends_it(choice);
        if !go && choice != "Cancel" {
            return Ok(false);
        }
        let change = TaskChange {
            action: Some(None),
            ..TaskChange::default()
        };
        if let Err(error) = self
            .inner
            .tasks
            .update(task, change, Who::Person, None)
            .await
        {
            tracing::debug!(%error, "the send card's task didn't change");
        }
        let now = self.now();
        self.set_result(
            task,
            SendResult {
                state: if go { "sending" } else { "canceled" }.to_owned(),
                at: Guard::iso(now),
                why: None,
                via: card.via.to_owned(),
                to: card.to.clone(),
            },
        )
        .await?;
        if go {
            self.note_tap(task.get(), card.at);
        }
        let room = card.room.unwrap_or_else(|| Guard::room_of(task));
        let guard = self.clone();
        let choice = choice.to_owned();
        tokio::spawn(async move {
            if let Err(error) = guard.answer(task, &choice, card, slack, go, room).await {
                tracing::warn!(%error, task = task.get(), "the owner's tap on a send card");
            }
        });
        Ok(true)
    }

    async fn answer(
        &self,
        task: TaskId,
        choice: &str,
        card: Card,
        slack: bool,
        go: bool,
        room: RoomId,
    ) -> Result<(), GuardError> {
        let tapped = if go { "Send it" } else { "Cancel" };
        let answer = match self.inner.messaging.get() {
            Some(messaging) if slack => {
                messaging
                    .slack()
                    .answer_card(task.get(), tapped, &card)
                    .await
            }
            Some(messaging) => {
                messaging
                    .whatsapp_send()
                    .answer_card(task.get(), tapped, &card)
                    .await
            }
            None => Ok(CardAnswer::Failed("messaging isn't set up".to_owned())),
        }
        .unwrap_or_else(|error| CardAnswer::Failed(error.to_string()));
        self.take_tap(task.get(), card.at);
        let now = self.now();
        let (state, at, why) = match &answer {
            CardAnswer::Sent(at) => ("sent", *at, None),
            CardAnswer::Cancelled => ("canceled", now, None),
            CardAnswer::Failed(why) => ("failed", now, Some(why.clone())),
        };
        if state == "failed" {
            self.sent_text(task, &card.text, &format!("{} (failed)", card.via))
                .await?;
        }
        self.set_result(
            task,
            SendResult {
                state: state.to_owned(),
                at: Guard::iso(at),
                why: why.clone(),
                via: card.via.to_owned(),
                to: card.to.clone(),
            },
        )
        .await?;
        let text: String = if card.text.chars().count() > 300 {
            format!("{}…", card.text.chars().take(300).collect::<String>())
        } else {
            card.text.clone()
        };
        let what = format!("the {} message to {} («{text}»)", card.via, card.to);
        let key = task.key();
        let told = match state {
            "sent" => format!(
                "The owner tapped Send it on {key}: {what} went out at {} as them. It's sent: don't call {} for it again. Carry the task on.",
                self.hhmm(at),
                card.tool
            ),
            "canceled" => format!(
                "The owner tapped Cancel on {key}: {what} is not sent. Don't send it; carry the task on."
            ),
            _ => {
                let why = why.unwrap_or_default();
                self.inner
                    .tasks
                    .note(task, Who::Assistant, &format!("Failed: {why}"), false)
                    .await?;
                format!(
                    "The owner tapped {choice} on {key}, but {what} didn't go out: {why} Tell them in one line; if they still want it, ask again."
                )
            }
        };
        let mut event = NewEvent::new(room, "owner", told);
        event.key = Some(format!("sendtap:{}:{}", task.get(), Guard::iso(at)));
        self.inner.events.add(event, now).await?;
        Ok(())
    }
}

/// The owner's other OKs for a message, as messaging asks for them: the guard rails', and the
/// ones `also` gives (the dependencies' own, a stand-in by default), either counting.
#[derive(Clone, Debug)]
pub struct GuardOks {
    guard: Guard,
    also: Arc<dyn SendOks>,
}

impl GuardOks {
    /// The guard rails' OKs, with `also`'s.
    #[must_use]
    pub fn new(guard: Guard, also: Arc<dyn SendOks>) -> Self {
        Self { guard, also }
    }
}

fn task_id(task: hooks::TaskId) -> TaskId {
    TaskId::new(task)
}

impl SendOks for GuardOks {
    fn tapped(&self, task: hooks::TaskId, asked: Timestamp) -> BoxFuture<'_, bool> {
        Box::pin(
            async move { self.guard.take_tap(task, asked) || self.also.tapped(task, asked).await },
        )
    }

    fn chat_ok<'a>(
        &'a self,
        room: Option<RoomId>,
        text: &'a str,
        to: &'a str,
        to_self: bool,
        asked: Option<Timestamp>,
    ) -> BoxFuture<'a, Option<ChatOk>> {
        Box::pin(async move {
            if let Some(room) = room {
                match self
                    .guard
                    .find_chat_ok(room, text, to, to_self, asked)
                    .await
                {
                    Ok(Some(ok)) => {
                        return Some(ChatOk {
                            id: ok.id.to_string(),
                            line: self.guard.ok_line(&ok, room).await,
                        });
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%error, "the chat's OK wasn't read"),
                }
            }
            self.also.chat_ok(room, text, to, to_self, asked).await
        })
    }

    fn spend<'a>(&'a self, ok: &'a ChatOk) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            match ok.id.parse::<i64>() {
                Ok(id) if ok.line.starts_with("OK from the owner in the chat") => {
                    self.guard.spend(id).await.unwrap_or(false)
                }
                _ => self.also.spend(ok).await,
            }
        })
    }

    fn text_ok<'a>(&'a self, task: hooks::TaskId, text: &'a str) -> BoxFuture<'a, Option<TextOk>> {
        Box::pin(async move {
            match self.guard.text_ok(task_id(task), text).await {
                Ok(Some(ok)) => Some(TextOk { how: ok.how }),
                _ => self.also.text_ok(task, text).await,
            }
        })
    }

    fn standing_ok<'a>(
        &'a self,
        chats: &'a [String],
        one_to_one: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            match self.guard.standing_ok(chats, one_to_one) {
                Some(ok) => Some(ok),
                None => self.also.standing_ok(chats, one_to_one).await,
            }
        })
    }

    fn sent<'a>(&'a self, task: hooks::TaskId, text: &'a str, via: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Err(error) = self.guard.sent_text(task_id(task), text, via).await {
                tracing::warn!(%error, "a sent text's OK wasn't spent");
            }
            self.also.sent(task, text, via).await;
        })
    }

    fn close_card<'a>(&'a self, task: hooks::TaskId, question: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Err(error) = self.guard.close_card(task_id(task), question).await {
                tracing::warn!(%error, "a card answered another way didn't close");
            }
            self.also.close_card(task, question).await;
        })
    }
}

/// The owner's answers on tasks, as the guard rails take them first: every tap is heard (their
/// OK for the job, a text, a post, or their Stop), and a send card's Send it or Cancel is done
/// there and then (the task's own handling stops there) when the ask answered is the one that
/// shows the card.
#[derive(Clone, Debug)]
pub struct GuardAnswers(pub Guard);

impl GuardAnswers {
    /// The ask open on `task`, the one an answer now answers; none when none is.
    async fn open_ask(&self, task: TaskId) -> Option<AskId> {
        let open = matches!(self.0.task(task).await, Ok(Some(t)) if t.decision.is_some());
        if !open {
            return None;
        }
        self.0
            .inner
            .tasks
            .last_ask(task)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the task's open ask wasn't read");
                None
            })
    }
}

impl crate::services::tasks::AnswerTaker for GuardAnswers {
    fn decide<'a>(&'a self, task: TaskId, choice: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let ask = self.open_ask(task).await;
            if let Err(error) = self.0.heard_tap(task, choice).await {
                tracing::warn!(%error, "the owner's tap wasn't heard");
            }
            self.0
                .tap_send(task, choice, ask)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "the owner's tap on a send card");
                    false
                })
        })
    }

    fn message<'a>(&'a self, _task: TaskId, _text: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
}
