//! The guard on sending as the owner, shared by `slack_send` and `whatsapp_send`.
//!
//! The first call for a message sends nothing: the owner is asked on the task (Send it /
//! Cancel) with the chat and the exact text, and the message waits (`Pending`, kept in the
//! settings, so a restart doesn't lose the answer). Only the owner's Send it on that very ask
//! (its id, not its question: another card can say the same words), for that chat and that
//! text, sends it, once. A changed text is a new ask; any later ask on the task replaces the
//! waiting message; a card that waited a day, or a Send it older than an hour, is asked again;
//! and the same text to the same chat again after it went out is refused (no resend). The
//! waiting and sent messages are changed in one step each, so two tasks writing at once can't
//! bring back a message that was taken. The other ways the owner OKs a message (a tap on the
//! card, a go in the chat after seeing the text, an OK of the exact text on another card, a
//! standing OK for a 1:1 chat) come through [`SendOks`](super::hooks::SendOks).

use std::collections::BTreeMap;

use emberhut_core::RoomId;
use emberhut_db::{DbError, SettingKey};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use super::context::Context;
use super::hooks::{AskId, ChatOk, TaskId, TextOk};

/// The answer that sends.
pub(crate) const SEND_IT: &str = "Send it";
/// The answer that doesn't.
pub(crate) const CANCEL: &str = "Cancel";

/// How long a card waits for the owner: after that it is asked again.
pub const CARD_LASTS: SignedDuration = SignedDuration::from_hours(24);
/// How long the owner's Send it counts: after that it is asked again.
pub const SEND_IT_LASTS: SignedDuration = SignedDuration::from_hours(1);

/// A message waiting for the owner's Send it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pending {
    /// The task it was asked on.
    pub(crate) task: TaskId,
    /// The chat, as the channel knows it.
    pub(crate) chat: String,
    /// The exact text.
    pub(crate) text: String,
    /// Who it is for, as the card names them.
    pub(crate) to: String,
    /// The card's question.
    pub(crate) question: String,
    /// The ask that showed it (its id on the task); none: no answer counts for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ask: Option<i64>,
    /// When it was asked, in milliseconds since the epoch.
    pub(crate) at: i64,
    /// The room whose assistant asked.
    pub(crate) room: Option<i64>,
    /// Slack's channel id, found when it was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) channel: Option<String>,
    /// The chat id the assistant gave, when the channel knows the chat by another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arg: Option<String>,
}

impl Pending {
    /// When it was asked.
    pub(crate) fn asked(&self) -> Timestamp {
        Timestamp::from_millisecond(self.at).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    /// The ask that showed it.
    pub(crate) fn ask_id(&self) -> Option<AskId> {
        self.ask.map(AskId::new)
    }
}

/// A message that went out, the last one per task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sent {
    /// The chat.
    pub(crate) chat: String,
    /// The text.
    pub(crate) text: String,
    /// When, in milliseconds since the epoch.
    pub(crate) at: i64,
}

impl Sent {
    /// When it went out.
    pub(crate) fn when(&self) -> Timestamp {
        Timestamp::from_millisecond(self.at).unwrap_or(Timestamp::UNIX_EPOCH)
    }
}

/// How many tasks' last sent messages are kept.
const SENT_KEPT: usize = 200;

/// One channel's waiting and sent messages, in the settings.
#[derive(Clone, Debug)]
pub(crate) struct Drafts {
    pending: SettingKey<BTreeMap<String, Pending>>,
    sent: SettingKey<BTreeMap<String, Sent>>,
}

impl Drafts {
    /// The drafts kept under these two settings.
    pub(crate) const fn new(pending: &'static str, sent: &'static str) -> Self {
        Self {
            pending: SettingKey::new(pending),
            sent: SettingKey::new(sent),
        }
    }

    /// Every waiting message, by task.
    pub(crate) async fn all_pending(
        &self,
        ctx: &Context,
    ) -> Result<BTreeMap<String, Pending>, DbError> {
        ctx.settings.get_or(&self.pending, BTreeMap::new()).await
    }

    /// The message waiting on `task`.
    pub(crate) async fn pending(
        &self,
        ctx: &Context,
        task: TaskId,
    ) -> Result<Option<Pending>, DbError> {
        Ok(self.all_pending(ctx).await?.remove(&task.to_string()))
    }

    /// Keeps `pending` as `task`'s, in place of the one it had, in one step (no other write
    /// to the settings comes between the read and the write).
    pub(crate) async fn set_pending(
        &self,
        ctx: &Context,
        task: TaskId,
        pending: Pending,
    ) -> Result<(), DbError> {
        ctx.settings
            .update(&self.pending, BTreeMap::new(), |all| {
                all.insert(task.to_string(), pending);
            })
            .await
    }

    /// Takes `pending` off `task`, in one step, if it is still the one waiting there: `false`
    /// when another took it or it changed since, and then nothing may be sent for it.
    pub(crate) async fn take_pending(
        &self,
        ctx: &Context,
        task: TaskId,
        pending: &Pending,
    ) -> Result<bool, DbError> {
        ctx.settings
            .update(&self.pending, BTreeMap::new(), |all| {
                let key = task.to_string();
                let same = all.get(&key) == Some(pending);
                if same {
                    all.remove(&key);
                }
                same
            })
            .await
    }

    /// The last message sent on `task`.
    pub(crate) async fn sent(&self, ctx: &Context, task: TaskId) -> Result<Option<Sent>, DbError> {
        Ok(ctx
            .settings
            .get_or(&self.sent, BTreeMap::new())
            .await?
            .remove(&task.to_string()))
    }

    /// Records `sent` as `task`'s last, in one step, keeping the newest tasks' only.
    pub(crate) async fn set_sent(
        &self,
        ctx: &Context,
        task: TaskId,
        sent: Sent,
    ) -> Result<(), DbError> {
        ctx.settings
            .update(&self.sent, BTreeMap::new(), |all| {
                all.insert(task.to_string(), sent);
                if all.len() > SENT_KEPT {
                    let mut tasks: Vec<i64> = all.keys().filter_map(|k| k.parse().ok()).collect();
                    tasks.sort_unstable();
                    for old in tasks.iter().take(all.len() - SENT_KEPT) {
                        all.remove(&old.to_string());
                    }
                }
            })
            .await
    }

    /// The message waiting on `task`, read against the owner's answers: one the owner can no
    /// longer answer (see [`Stale`]) is taken off, and then none waits.
    pub(crate) async fn read(&self, ctx: &Context, task: TaskId) -> Result<Reading, DbError> {
        let Some(pending) = self.pending(ctx, task).await? else {
            return Ok(Reading::default());
        };
        match answer(ctx, &pending).await {
            Answer::Stale(stale) => {
                self.take_pending(ctx, task, &pending).await?;
                Ok(Reading {
                    pending: None,
                    decided: None,
                    stale: Some(stale),
                })
            }
            Answer::Waiting => Ok(Reading {
                pending: Some(pending),
                decided: None,
                stale: None,
            }),
            Answer::Send => Ok(Reading {
                pending: Some(pending),
                decided: Some(Decision::Send),
                stale: None,
            }),
            Answer::Cancel => Ok(Reading {
                pending: Some(pending),
                decided: Some(Decision::Cancel),
                stale: None,
            }),
        }
    }
}

/// A task's waiting message as a send finds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reading {
    /// The message waiting, when one still can be answered.
    pub(crate) pending: Option<Pending>,
    /// The owner's answer to its card, when they gave one that counts.
    pub(crate) decided: Option<Decision>,
    /// Why the one that waited was taken off.
    pub(crate) stale: Option<Stale>,
}

/// What the owner said to the card asked for `pending`: Send it or Cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Send it.
    Send,
    /// Cancel.
    Cancel,
}

/// Why a waiting message can no longer be answered: it is asked again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stale {
    /// Another ask on the task came after its card (or it was never tied to one).
    Replaced,
    /// Its card waited longer than [`CARD_LASTS`].
    Expired,
    /// The owner's Send it is older than [`SEND_IT_LASTS`].
    SendItExpired,
}

impl Stale {
    /// Why, for the assistant.
    pub(crate) fn said(self) -> &'static str {
        match self {
            Self::Replaced => "another question was asked on the task after its card",
            Self::Expired => "its card waited more than 24 hours",
            Self::SendItExpired => "the owner's Send it is more than an hour old",
        }
    }
}

/// The owner's answer to `pending`'s card, read now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// No answer yet.
    Waiting,
    /// Send it, given within the hour.
    Send,
    /// Cancel.
    Cancel,
    /// It can't be answered any more.
    Stale(Stale),
}

/// The owner's answer to `pending`'s card: the first Send it or Cancel that names its very ask,
/// while that ask is still the task's latest and the card is less than a day old; a Send it
/// counts for an hour.
pub(crate) async fn answer(ctx: &Context, pending: &Pending) -> Answer {
    let now = ctx.now();
    if now.duration_since(pending.asked()) > CARD_LASTS {
        return Answer::Stale(Stale::Expired);
    }
    let Some(ask) = pending.ask_id() else {
        return Answer::Stale(Stale::Replaced);
    };
    let answers = match ctx.tasks.answers(pending.task, ask).await {
        Ok(answers) => answers,
        Err(error) => {
            tracing::warn!(%error, task = pending.task, "the task's answers can't be read");
            return Answer::Waiting;
        }
    };
    if answers.latest != Some(ask) {
        return Answer::Stale(Stale::Replaced);
    }
    match answers
        .chosen
        .into_iter()
        .find(|c| c.choice == SEND_IT || c.choice == CANCEL)
    {
        None => Answer::Waiting,
        Some(c) if c.choice == CANCEL => Answer::Cancel,
        Some(c) if now.duration_since(c.at) > SEND_IT_LASTS => Answer::Stale(Stale::SendItExpired),
        Some(_) => Answer::Send,
    }
}

/// The OK a message goes out on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Go {
    /// The owner's Send it on its card (or a tap on it).
    Card,
    /// The owner's go in the chat.
    Chat(ChatOk),
    /// The owner's OK of this exact text on another card.
    Text(TextOk),
    /// The owner's standing OK for this chat.
    Standing(String),
}

impl Go {
    /// How the sent line names it.
    pub(crate) fn said(&self) -> &'static str {
        match self {
            Self::Chat(_) => " (you said send it in chat)",
            Self::Standing(_) => " (the owner's standing OK, no card)",
            Self::Card | Self::Text(_) => "",
        }
    }
}

/// A send card, as the guard rails' send-on-tap shows it: the message waiting on a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// `Slack` or `WhatsApp`.
    pub via: &'static str,
    /// The tool that sends it.
    pub tool: &'static str,
    /// The card's question.
    pub question: String,
    /// The ask that shows it on the task: only an answer to that ask is an answer to it.
    pub ask: Option<AskId>,
    /// Who it is for.
    pub to: String,
    /// The exact text.
    pub text: String,
    /// The room whose assistant asked.
    pub room: Option<RoomId>,
    /// When it was asked.
    pub at: Timestamp,
}

impl Card {
    /// The card of `pending`, sent through `via`'s `tool`.
    pub(crate) fn of(pending: &Pending, via: &'static str, tool: &'static str) -> Self {
        Self {
            via,
            tool,
            question: pending.question.clone(),
            ask: pending.ask_id(),
            to: pending.to.clone(),
            text: pending.text.clone(),
            room: room_of(pending),
            at: pending.asked(),
        }
    }

    /// Whether `pending` is still the message this card shows.
    pub(crate) fn shows(&self, pending: &Pending) -> bool {
        pending.ask_id() == self.ask && pending.asked() == self.at && pending.text == self.text
    }
}

/// A draft the guard rails' send-on-tap asks about on a task (its card's question and the
/// answer that sends it are the asker's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The chat id.
    pub chat: String,
    /// The text.
    pub text: String,
    /// The card's question.
    pub question: String,
    /// The ask that shows the card on the task.
    pub ask: AskId,
    /// The answer that sends it.
    pub send: String,
    /// The room whose assistant asked.
    pub room: Option<RoomId>,
}

/// A draft kept as a task's card: who it is for, the line that names the chat, and the
/// owner's standing OK for it when there is one (then it needs no card).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// Who it is for.
    pub to: String,
    /// The chat, named for the card.
    pub line: String,
    /// The standing OK that lets it go without a card.
    pub free: Option<String>,
}

/// What the owner's tap on a send card came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardAnswer {
    /// It went out, then.
    Sent(Timestamp),
    /// Nothing went out: cancelled.
    Cancelled,
    /// Nothing went out; why.
    Failed(String),
}

/// The room id a pending message keeps.
pub(crate) fn room_id(room: Option<RoomId>) -> Option<i64> {
    room.map(RoomId::get)
}

/// The room a pending message names.
pub(crate) fn room_of(pending: &Pending) -> Option<RoomId> {
    pending.room.map(RoomId::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_message_reads_its_time_and_room() {
        let pending = Pending {
            task: 3,
            chat: "slack:Acme:Ann".to_owned(),
            text: "hi".to_owned(),
            to: "Ann".to_owned(),
            question: "Send this Slack message to Ann?".to_owned(),
            ask: Some(4),
            at: 1_000,
            room: Some(2),
            channel: Some("D1".to_owned()),
            arg: None,
        };
        assert_eq!(pending.asked().as_millisecond(), 1_000);
        assert_eq!(room_of(&pending), Some(RoomId::new(2)));
        assert_eq!(pending.ask_id(), Some(AskId::new(4)));
        assert!(Card::of(&pending, "Slack", "slack_send").shows(&pending));
        let asked_again = Pending {
            ask: Some(5),
            ..pending.clone()
        };
        assert!(!Card::of(&pending, "Slack", "slack_send").shows(&asked_again));
        let json = serde_json::to_value(&pending).unwrap();
        assert!(json.get("arg").is_none());
        assert_eq!(json["channel"], "D1");
    }
}
