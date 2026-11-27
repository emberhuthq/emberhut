//! `whatsapp_send`: one message as the owner, through WhatsApp Web in the server's browser
//! (no unofficial library or API; nothing read from its storage). It opens the owner's chat
//! (by the chat id the Mac helper knows, `wa:<id>`: a person by phone number, a group by its
//! exact name, never a guess), types the message and stops: sending is the owner's call, every
//! time, on the guard's terms ([`super::guard`]). Once the owner's OK for that exact text is
//! there, it presses Send, checks WhatsApp shows it sent (a tick), and logs it on the task.
//!
//! [`WhatsAppWeb`] is the browser seam: the browser itself is the browser area's. Until a
//! driver is wired in, [`NoWhatsAppWeb`] stands in and says so; the guard still holds.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use emberhut_core::RoomId;
use emberhut_core::messaging::text::{first_chars, same};
use emberhut_db::DbError;
use futures_util::future::BoxFuture;

use super::context::Context;
use super::guard::{
    self, CANCEL, Card, CardAnswer, Decision, Draft, Drafts, Go, Pending, Prepared, Reading,
    SEND_IT, Sent,
};
use super::hooks::{Ask, TaskId};
use super::mac::{MacMessages, encode};

/// `whatsapp_send`'s waiting and sent messages.
const DRAFTS: Drafts = Drafts::new("waPending", "waSent");

/// The lock every WhatsApp Web send holds: one tab, one send at a time.
const LOCK: &str = "whatsapp";

/// A chat as the owner's Mac knows it (`/wa-contact`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    /// Its id there (`wa:<id>`).
    pub chat: String,
    /// Its name.
    pub name: String,
    /// `person` or `group`.
    pub kind: String,
    /// A person's number; none for a group or a hidden number.
    pub phone: Option<String>,
    /// The owner's own "message yourself" chat.
    pub me: bool,
}

/// Why WhatsApp Web didn't type the draft.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebSendError {
    /// No browser for WhatsApp Web on this server.
    #[error("WhatsApp Web isn't available on this server (no browser is set up for it)")]
    NotAvailable,
    /// WhatsApp Web isn't linked: the owner was handed the browser to scan; the words say how.
    #[error("WhatsApp Web isn't linked yet. {0} Once it's handed back, call whatsapp_send again.")]
    NotLinked(String),
    /// The page didn't load.
    #[error(
        "WhatsApp Web didn't load (no chat list, no login page). Take a browser_snapshot to see why."
    )]
    NotLoaded,
    /// More or fewer than one chat by that name.
    #[error("{0}")]
    NotOne(String),
    /// The chat didn't open.
    #[error("{0}")]
    NotOpened(String),
    /// The text read back differs from what was typed: nothing kept.
    #[error(
        "WhatsApp Web changed the text while typing it (it reads back different), so it wasn't kept. Tell the owner; don't send."
    )]
    Changed,
    /// Something else failed; its words.
    #[error("{0}")]
    Failed(String),
}

/// How a sent message shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Two ticks.
    Delivered,
    /// One tick.
    Sent,
    /// No tick yet.
    Unconfirmed,
}

impl Tick {
    fn said(self) -> &'static str {
        match self {
            Self::Delivered => "delivered (✓✓)",
            Self::Sent => "sent (✓)",
            Self::Unconfirmed => "not confirmed yet (no tick after 30 s)",
        }
    }
}

/// WhatsApp Web in the server's browser.
pub trait WhatsAppWeb: fmt::Debug + Send + Sync {
    /// Opens `contact`'s chat, types `text` as a draft (lines with Shift+Enter) and reads it
    /// back; the chat's name as its header shows it. Nothing is sent.
    fn type_draft<'a>(
        &'a self,
        contact: &'a Contact,
        text: &'a str,
    ) -> BoxFuture<'a, Result<String, WebSendError>>;

    /// Presses Send on the draft just typed, and waits up to 30 s for its tick. `Ok` only once
    /// the press itself went through (the tick may still be missing); an error means the
    /// message didn't go out.
    fn send<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<Tick, WebSendError>>;
}

/// No browser for WhatsApp Web.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWhatsAppWeb;

impl WhatsAppWeb for NoWhatsAppWeb {
    fn type_draft<'a>(
        &'a self,
        _: &'a Contact,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, WebSendError>> {
        Box::pin(async { Err(WebSendError::NotAvailable) })
    }

    fn send<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Tick, WebSendError>> {
        Box::pin(async { Err(WebSendError::NotAvailable) })
    }
}

/// `whatsapp_send`.
#[derive(Clone, Debug)]
pub struct WhatsAppSend {
    ctx: Context,
    mac: Arc<dyn MacMessages>,
    web: Arc<dyn WhatsAppWeb>,
}

impl WhatsAppSend {
    /// The sender over the Mac helper (who a chat is) and WhatsApp Web.
    #[must_use]
    pub fn new(ctx: Context, mac: Arc<dyn MacMessages>, web: Arc<dyn WhatsAppWeb>) -> Self {
        Self { ctx, mac, web }
    }

    /// Who `chat` is, by the owner's Mac: `Err` is the line for the assistant.
    async fn contact(&self, chat: &str) -> Result<Contact, String> {
        let path = format!("/wa-contact?chat={}", encode(chat));
        let answer = match self.mac.get(&path, Duration::from_secs(15)).await {
            Ok(answer) if answer.ok() => answer,
            Ok(answer) if answer.status == 404 => {
                return Err(format!(
                    "No WhatsApp chat {chat}. Use an id from messages_chats or messages_search (wa:12)."
                ));
            }
            _ => {
                return Err(
                    "The owner's Mac didn't answer (it knows the WhatsApp chats). Try again later."
                        .to_owned(),
                );
            }
        };
        let n = answer.json();
        let text = |key: &str| n[key].as_str().unwrap_or_default().to_owned();
        Ok(Contact {
            chat: text("chat"),
            name: text("name"),
            kind: text("kind"),
            phone: n["phone"].as_str().map(str::to_owned),
            me: n["me"].as_bool() == Some(true) || n["me"].as_i64() == Some(1),
        })
    }

    /// Keeps `draft` as `task`'s send card, for the guard rails' send-on-tap: the chat as the
    /// owner's Mac knows it. The owner's tap on its send answer types and sends it
    /// ([`Self::answer_card`]).
    ///
    /// # Errors
    ///
    /// Why the draft can't be a card, in words.
    pub async fn prepare(&self, task: TaskId, draft: &Draft) -> Result<Prepared, String> {
        let text = same(&draft.text);
        let who = self.contact(&draft.chat).await?;
        let to = if who.me {
            "Message yourself".to_owned()
        } else {
            who.name.clone()
        };
        let pending = Pending {
            task,
            chat: who.chat.clone(),
            text: text.clone(),
            to: to.clone(),
            question: draft.question.clone(),
            ask: Some(draft.ask.get()),
            at: self.ctx.now().as_millisecond(),
            room: guard::room_id(draft.room),
            channel: None,
            arg: Some(draft.chat.clone()),
        };
        DRAFTS
            .set_pending(&self.ctx, task, pending)
            .await
            .map_err(|e| e.to_string())?;
        let chats = vec![draft.chat.clone(), who.chat.clone()];
        let free = self
            .ctx
            .oks
            .standing_ok(&chats, who.kind == "person" && !who.me)
            .await;
        if free.is_none() {
            self.ctx
                .note(
                    task,
                    format!(
                        "WhatsApp message to {to} ({}) waiting for the owner's {}: «{text}»",
                        who.chat, draft.send
                    ),
                )
                .await;
        }
        Ok(Prepared {
            line: format!("{to} on WhatsApp ({})", who.chat),
            to,
            free,
        })
    }

    /// The send card waiting on `task`, for the guard rails' send-on-tap.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn card(&self, task: TaskId) -> Result<Option<Card>, DbError> {
        Ok(DRAFTS
            .pending(&self.ctx, task)
            .await?
            .map(|p| Card::of(&p, "WhatsApp", "whatsapp_send")))
    }

    /// The owner's tap on `card`: Cancel drops it; the send answer types and sends it there and
    /// then through [`Self::send`]'s own guard, unless it was asked again or changed since. The
    /// whole tap holds the one WhatsApp lock, as every `whatsapp_send` call does (one tab).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn answer_card(
        &self,
        task: TaskId,
        choice: &str,
        card: &Card,
    ) -> Result<CardAnswer, DbError> {
        let lock = self.ctx.lock(LOCK);
        let _alone = lock.lock().await;
        let pending = DRAFTS.pending(&self.ctx, task).await?;
        let gone = pending.as_ref().is_none_or(|p| !card.shows(p));
        if choice == CANCEL {
            if let Some(p) = pending.filter(|_| !gone)
                && DRAFTS.take_pending(&self.ctx, task, &p).await?
            {
                self.ctx
                    .note(
                        task,
                        format!("The owner cancelled the WhatsApp message to {}", p.to),
                    )
                    .await;
            }
            return Ok(CardAnswer::Cancelled);
        }
        let said = match &pending {
            Some(p) if !gone => {
                let chat = p.arg.clone().unwrap_or_else(|| p.chat.clone());
                Some(
                    self.send_held(&chat, &p.text, Some(task), guard::room_of(p))
                        .await,
                )
            }
            _ => None,
        };
        if let Some(sent) = DRAFTS.sent(&self.ctx, task).await?
            && sent.text == card.text
            && sent.when() >= card.at
        {
            return Ok(CardAnswer::Sent(sent.when()));
        }
        Ok(CardAnswer::Failed(said.unwrap_or_else(|| {
            "this message was asked again or changed since; nothing was sent.".to_owned()
        })))
    }

    /// `whatsapp_send`: one call at a time (one browser tab). The answer is for the assistant.
    pub async fn send(
        &self,
        chat: &str,
        text: &str,
        task: Option<TaskId>,
        room: Option<RoomId>,
    ) -> String {
        let lock = self.ctx.lock(LOCK);
        let _alone = lock.lock().await;
        self.send_held(chat, text, task, room).await
    }

    /// [`Self::send`] for a caller that holds the WhatsApp lock already.
    async fn send_held(
        &self,
        chat: &str,
        text: &str,
        task: Option<TaskId>,
        room: Option<RoomId>,
    ) -> String {
        match self.send_alone(chat, text, task, room).await {
            Ok(said) => said,
            Err(error) => {
                tracing::error!(%error, "whatsapp_send failed");
                format!("WhatsApp: {error}")
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the guard's steps in order, as one reads them; each is short"
    )]
    async fn send_alone(
        &self,
        chat: &str,
        text: &str,
        task: Option<TaskId>,
        room: Option<RoomId>,
    ) -> Result<String, DbError> {
        let Some(task) = task.filter(|t| *t != 0) else {
            return Ok(
                "whatsapp_send needs task_id: the OK to send is asked on that task.".to_owned(),
            );
        };
        let text = same(text);
        if text.is_empty() {
            return Ok("Nothing to send.".to_owned());
        }
        let who = match self.contact(chat).await {
            Ok(who) => who,
            Err(said) => return Ok(said),
        };
        let ctx = &self.ctx;
        if let Some(sent) = DRAFTS.sent(ctx, task).await?
            && sent.chat == who.chat
            && sent.text == text
        {
            return Ok(format!(
                "Already sent to {} at {}. Not sent again: one message per OK, never a resend.",
                who.name,
                sent.when()
            ));
        }
        let Reading {
            pending,
            decided,
            stale,
        } = DRAFTS.read(ctx, task).await?;
        if let (Some(Decision::Cancel), Some(p)) = (decided, &pending) {
            DRAFTS.take_pending(ctx, task, p).await?;
            ctx.note(
                task,
                format!("The owner cancelled the WhatsApp message to {}", who.name),
            )
            .await;
            return Ok("The owner said Cancel: not sent. Don't send it.".to_owned());
        }
        let asked = pending
            .as_ref()
            .filter(|p| p.chat == who.chat && p.text == text);
        let go = match asked {
            Some(p) => decided == Some(Decision::Send) || ctx.oks.tapped(task, p.asked()).await,
            None => false,
        };
        let mut ok = if go || ctx.card_oks_only {
            None
        } else {
            let since = asked.filter(|_| decided.is_none()).map(Pending::asked);
            ctx.oks.chat_ok(room, &text, &who.name, who.me, since).await
        };
        let pre = if go || ok.is_some() || ctx.card_oks_only {
            None
        } else {
            ctx.oks.text_ok(task, &text).await
        };
        let chats = vec![chat.to_owned(), who.chat.clone()];
        let one_to_one = who.kind == "person" && !who.me;
        let mut free = if go || ok.is_some() || pre.is_some() {
            None
        } else {
            ctx.oks.standing_ok(&chats, one_to_one).await
        };
        if let Some(p) = asked
            && !go
            && ok.is_none()
            && pre.is_none()
            && free.is_none()
        {
            return Ok(format!(
                "Still waiting for the owner to say Send it or Cancel (asked {}). Stop for now: \
                 their Send it sends it by itself, and you'll hear.",
                p.asked()
            ));
        }

        let header = match self.web.type_draft(&who, &text).await {
            Ok(header) => header,
            Err(error) => return Ok(error.to_string()),
        };
        let to = if who.me {
            "Message yourself".to_owned()
        } else if header.is_empty() {
            who.name.clone()
        } else {
            header
        };
        if let Some(chat_ok) = &ok
            && !ctx.oks.spend(chat_ok).await
        {
            ok = None;
        }
        if !go && ok.is_none() && pre.is_none() && free.is_none() {
            free = ctx.oks.standing_ok(&chats, one_to_one).await;
        }
        if let Some(chat_ok) = &ok {
            ctx.note(task, chat_ok.line.clone()).await;
        }
        if let Some(pre) = &pre {
            ctx.note(
                task,
                format!(
                    "WhatsApp to {to}: the exact text the owner OK'd ({}), no new card",
                    pre.how
                ),
            )
            .await;
        }
        if let Some(free) = &free {
            ctx.note(task, format!("WhatsApp to {to}: {free}, no card"))
                .await;
        }
        let how = if go {
            Go::Card
        } else if let Some(chat_ok) = ok.clone() {
            Go::Chat(chat_ok)
        } else if let Some(pre) = pre.clone() {
            Go::Text(pre)
        } else if let Some(free) = free.clone() {
            Go::Standing(free)
        } else {
            let at = ctx.now().as_millisecond();
            let question = format!("Send this WhatsApp message to {to}?");
            let ask = Ask {
                question: question.clone(),
                options: vec![SEND_IT.to_owned(), CANCEL.to_owned()],
                title: first_chars(&format!("Send WhatsApp to {to}"), 90),
                brief: vec![
                    format!("To: {to} (WhatsApp, {})", who.chat),
                    format!("Message: {text}"),
                ],
                approve: true,
            };
            let ask = match ctx.ask(task, ask).await {
                Ok(ask) => ask,
                Err(error) => {
                    return Ok(format!(
                        "Typed in WhatsApp Web, not sent, and the owner couldn't be asked: {error}."
                    ));
                }
            };
            DRAFTS
                .set_pending(
                    ctx,
                    task,
                    Pending {
                        task,
                        chat: who.chat.clone(),
                        text: text.clone(),
                        to: to.clone(),
                        question,
                        ask: Some(ask.get()),
                        at,
                        room: guard::room_id(room),
                        channel: None,
                        arg: Some(chat.to_owned()),
                    },
                )
                .await?;
            ctx.note(
                task,
                format!("Typed a WhatsApp message to {to} ({}), waiting for the owner's Send it: «{text}»", who.chat),
            )
            .await;
            let more = if text.chars().count() > 200 {
                "…"
            } else {
                ""
            };
            ctx.push(
                format!("WhatsApp to {to}?"),
                &format!(
                    "{}{more} · Send it or Cancel in the app",
                    first_chars(&text, 200)
                ),
            );
            let again = stale.map_or_else(String::new, |stale| {
                format!(" (Asked again: {}.)", stale.said())
            });
            return Ok(format!(
                "Typed in WhatsApp Web, not sent. The owner was asked (Send it / Cancel) with the chat \
                 ({to}) and the exact text. Stop; their Send it sends it by itself and you'll hear \
                 how it went (don't call whatsapp_send for it again). Only their go in the chat needs \
                 you to call it again, with the same chat and text.{again}"
            ));
        };

        if let Some(p) = asked {
            if !DRAFTS.take_pending(ctx, task, p).await? {
                return Ok(
                    "Not sent: this message was asked again or changed since. Call whatsapp_send \
                     again if it is still wanted."
                        .to_owned(),
                );
            }
            if how != Go::Card {
                ctx.oks.close_card(task, &p.question).await;
            }
        }
        // Only a press of Send that WhatsApp Web confirms is a send; any error is not.
        let tick = match self.web.send(&text).await {
            Ok(tick) => tick,
            Err(error) => {
                ctx.oks.sent(task, &text, "WhatsApp (failed)").await;
                ctx.note(
                    task,
                    format!("WhatsApp to {to} ({}) not sent: {error}", who.chat),
                )
                .await;
                return Ok(format!(
                    "Not sent: {error}. The owner's OK is spent; if they still want it, ask again."
                ));
            }
        };
        let now = ctx.now();
        // Pressed: whatever the tick says, never again.
        DRAFTS
            .set_sent(
                ctx,
                task,
                Sent {
                    chat: who.chat.clone(),
                    text: text.clone(),
                    at: now.as_millisecond(),
                },
            )
            .await?;
        ctx.oks.sent(task, &text, "WhatsApp").await;
        let status = tick.said();
        ctx.note(
            task,
            format!(
                "Sent{} on WhatsApp to {to} ({}) at {now}, as the owner: «{text}», {status}",
                how.said(),
                who.chat
            ),
        )
        .await;
        Ok(format!(
            "Sent to {to} on WhatsApp: {status}. Logged on task {task}."
        ))
    }
}
