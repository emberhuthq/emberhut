//! News from the owner's chats: after each round of collecting, the chats with new messages
//! (iMessage, SMS and WhatsApp from the Mac's `/changes`, Slack, email) go to Main as one
//! event, with the assistant's card on each person and the open loops each chat may answer.
//! The rules (what can't wait, how long the rest gather, the event's text) are
//! [`emberhut_core::messaging::news`].
//!
//! Two marks say how far the chats were read: `context.at` for the Mac's chats and Slack's
//! (they wait together while the Mac doesn't answer), and `mailAt` for email, its own, so a
//! sleeping Mac doesn't hold the owner's mail back. A mark moves only once what it covers was
//! passed on, so messages that gather for the batch are read again, with what came since, at
//! the next look. The `newsWakes` setting (on unless set to `false`) turns it all off.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use emberhut_core::RoomId;
use emberhut_core::messaging::OWNER;
use emberhut_core::messaging::mail::is_mail;
use emberhut_core::messaging::news::{
    self, BATCH, FIRST_LOOK, MAIL_CHARS, MAX_CHATS, MAX_MESSAGES, MESSAGE_CHARS, VOICE_CHARS,
};
use emberhut_core::messaging::slack::{CHAT_PREFIX, chat_id};
use emberhut_core::messaging::text::first_chars;
use emberhut_db::SettingKey;
use jiff::Timestamp;
use serde_json::Value;

use super::context::Context;
use super::gmail::{GmailService, line};
use super::mac::{MacMessages, encode};
use super::slack::SlackService;
use super::voice_notes::{VoiceNotes, truthy};
use crate::convert::timestamp_to_api;
use crate::services::context::CONTEXT;
use crate::services::facts::FactService;
use crate::services::mac::MacError;

/// Whether new messages wake the assistant; on unless set to `false`.
pub const NEWS_WAKES: SettingKey<bool> = SettingKey::new("newsWakes");
/// How far email was read.
const MAIL_AT: SettingKey<String> = SettingKey::new("mailAt");
/// How long the Mac may take, voice notes being transcribed included.
const WAIT: Duration = Duration::from_secs(20);

/// A chat with new messages.
#[derive(Clone, Debug)]
struct Change {
    id: String,
    name: String,
    source: String,
    group: bool,
    messages: u64,
}

/// A new message, as one line.
#[derive(Clone, Debug)]
struct Line {
    at: Timestamp,
    from_me: bool,
    text: String,
}

/// What one look found.
#[derive(Debug, Default)]
struct Found {
    /// Each chat's part, together.
    text: String,
    /// How many chats.
    chats: usize,
    /// Why they can't wait, one per chat that can't.
    why: Vec<String>,
    /// Why the Mac didn't answer; none: it did, or there is none.
    mac_down: Option<String>,
}

/// A loop's change from one look: touched now, and maybe answered at the owner's message.
type Touch = (String, Option<String>);

/// The look for news.
#[derive(Clone, Debug)]
pub struct News {
    ctx: Context,
    mac: Arc<dyn MacMessages>,
    slack: SlackService,
    gmail: GmailService,
    voice: VoiceNotes,
    facts: Option<FactService>,
    owner: Option<String>,
}

/// What [`News`] is made of.
#[derive(Clone, Debug)]
pub(super) struct NewsParts {
    pub(super) ctx: Context,
    pub(super) mac: Arc<dyn MacMessages>,
    pub(super) slack: SlackService,
    pub(super) gmail: GmailService,
    pub(super) voice: VoiceNotes,
    pub(super) facts: Option<FactService>,
    pub(super) owner: Option<String>,
}

impl News {
    /// The look over `parts`.
    pub(super) fn new(parts: NewsParts) -> Self {
        let NewsParts {
            ctx,
            mac,
            slack,
            gmail,
            voice,
            facts,
            owner,
        } = parts;
        Self {
            ctx,
            mac,
            slack,
            gmail,
            voice,
            facts,
            owner: owner.filter(|owner| !owner.trim().is_empty()),
        }
    }

    /// One look: the chats with messages since the marks go to Main as one event, `important`
    /// at once when one can't wait, `messages` once the oldest mark is [`BATCH`] old. Nothing
    /// when `newsWakes` is off. A failure is logged, never thrown, and the marks then stay.
    pub async fn tick(&self) {
        match self.ctx.settings.get_or(&NEWS_WAKES, true).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(%error, "news: the switch couldn't be read");
                return;
            }
        }
        let lock = self.ctx.lock("news");
        let _one = lock.lock().await;
        let now = self.ctx.now();
        let since = self.since(now).await;
        let mail_since = match self.ctx.settings.get(&MAIL_AT).await {
            Ok(at) => at.and_then(|at| at.parse().ok()).unwrap_or(since),
            Err(error) => {
                tracing::warn!(%error, "news: the mail mark couldn't be read");
                return;
            }
        };
        let found = match self.look(since, mail_since).await {
            Ok(found) => found,
            Err(why) => {
                self.down(since, now, &why).await;
                return;
            }
        };
        if found.chats > 0 {
            let oldest = if found.mac_down.is_none() {
                since.min(mail_since)
            } else {
                mail_since
            };
            // They gather for the batch, unless one can't wait.
            if found.why.is_empty() && now.duration_since(oldest) < BATCH {
                return;
            }
            let label = if found.mac_down.is_none() {
                since
            } else {
                mail_since
            };
            let (kind, text) = news::event(
                &found.why,
                timestamp_to_api(label).as_str(),
                &news::cut(found.text),
            );
            self.ctx.event(RoomId::MAIN, kind, text, None).await;
            let plural = if found.chats == 1 { "" } else { "s" };
            let soon = if found.why.is_empty() { "" } else { " at once" };
            self.ctx
                .activity(format!(
                    "New messages in {} chat{plural}: passed to the assistant{soon}",
                    found.chats
                ))
                .await;
        }
        match &found.mac_down {
            None => self.seen(now).await,
            Some(why) => self.down(since, now, why).await,
        }
        if let Err(error) = self
            .ctx
            .settings
            .set(&MAIL_AT, &timestamp_to_api(now).as_str().to_owned())
            .await
        {
            tracing::warn!(%error, "news: the mail mark wasn't kept");
        }
    }

    /// How far the Mac's chats and Slack's were read: the mark, or [`FIRST_LOOK`] ago.
    async fn since(&self, now: Timestamp) -> Timestamp {
        let first = now.checked_sub(FIRST_LOOK).unwrap_or(now);
        match self.ctx.settings.get(&CONTEXT).await {
            Ok(context) => context
                .and_then(|context| context.at)
                .and_then(|at| at.parse().ok())
                .unwrap_or(first),
            Err(error) => {
                tracing::warn!(%error, "news: the mark couldn't be read");
                first
            }
        }
    }

    /// The Mac's chats and Slack's were read up to `at`.
    async fn seen(&self, at: Timestamp) {
        let kept = async {
            let mut context = self.ctx.settings.get(&CONTEXT).await?.unwrap_or_default();
            context.at = Some(timestamp_to_api(at).as_str().to_owned());
            self.ctx.settings.set(&CONTEXT, &context).await
        };
        if let Err(error) = kept.await {
            tracing::warn!(%error, "news: the mark wasn't kept");
        }
    }

    /// The Mac didn't answer: said in the activity log once its chats have waited a batch.
    async fn down(&self, since: Timestamp, now: Timestamp, why: &str) {
        tracing::debug!(why, "news: the Mac didn't answer");
        if now.duration_since(since) > BATCH {
            self.ctx
                .activity(format!("Messages: the Mac didn't answer ({why})"))
                .await;
        }
    }

    /// The chats with messages after the marks: the Mac's and Slack's after `since` (none, and
    /// why in `found`, while the Mac doesn't answer), then email got after `mail_since`.
    async fn changes(
        &self,
        since: Timestamp,
        mail_since: Timestamp,
        found: &mut Found,
    ) -> Result<Vec<Change>, String> {
        let mut changes = match self.mac_changes(since).await {
            Ok(mut changes) => {
                let slack = self.slack.changes(since).await.map_err(|e| e.to_string())?;
                changes.extend(slack.into_iter().map(|c| Change {
                    id: chat_id(&c.workspace, &c.who),
                    source: format!("Slack {}", c.workspace),
                    group: news::is_group_name(&c.who),
                    messages: c.messages,
                    name: c.who,
                }));
                changes
            }
            // The Mac's chats and Slack's wait for it, with their mark.
            Err(why) => {
                found.mac_down = Some(why);
                Vec::new()
            }
        };
        let mail = self
            .gmail
            .changes(mail_since)
            .await
            .map_err(|e| e.to_string())?;
        changes.extend(mail.into_iter().map(|(c, source)| Change {
            id: c.chat,
            name: c.who,
            source,
            group: false,
            messages: c.messages,
        }));
        Ok(changes)
    }

    /// The chats with messages after the marks, each with its new messages. An error (in
    /// words) when a chat that was listed couldn't be read.
    async fn look(&self, since: Timestamp, mail_since: Timestamp) -> Result<Found, String> {
        let mut found = Found::default();
        let changes = self.changes(since, mail_since, &mut found).await?;
        // Voice notes being transcribed now get a little time, all together.
        let until = tokio::time::Instant::now() + WAIT;
        self.voice.settle(WAIT, None).await;
        let mut awareness = self
            .ctx
            .settings
            .get(&CONTEXT)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let mut touched: HashMap<i64, Touch> = HashMap::new();
        let now = timestamp_to_api(self.ctx.now()).as_str().to_owned();
        let mut parts = Vec::new();
        for change in changes.into_iter().take(MAX_CHATS) {
            let limit = (change.messages + 3).min(MAX_MESSAGES);
            let left = until.saturating_duration_since(tokio::time::Instant::now());
            let lines = self.lines(&change, since, mail_since, limit, left).await?;
            if lines.is_empty() {
                continue;
            }
            let card = match &self.facts {
                Some(facts) => facts.person_for(&change.id).await.unwrap_or_else(|error| {
                    tracing::warn!(%error, "news: a person's card couldn't be read");
                    None
                }),
                None => None,
            };
            let theirs: Vec<&str> = lines
                .iter()
                .filter(|l| !l.from_me)
                .map(|l| l.text.as_str())
                .collect();
            let important = card.as_ref().is_some_and(|c| c.important == Some(true));
            if let Some(why) = news::why_now(
                &change.name,
                &theirs,
                important,
                change.group,
                self.owner.as_deref(),
            ) {
                found.why.push(why);
            }
            let mine = lines.iter().find(|l| l.from_me).map(|l| l.at);
            let answered = touch_loops(&mut awareness.loops, &change, mine, &now, &mut touched);
            let group = if change.group { ", group" } else { "" };
            let mut part = format!(
                "{} ({}{group}, chat {})\n",
                change.name, change.source, change.id
            );
            if !answered.is_empty() {
                let _written = writeln!(
                    part,
                    "The owner wrote here after {} opened: maybe answered. Read their reply; \
                     close_loop if it's done.",
                    answered.join(", ")
                );
            }
            match &card {
                Some(card) => {
                    let now = card
                        .now
                        .as_deref()
                        .filter(|now| !now.is_empty())
                        .map_or_else(String::new, |now| format!(" Now: {now}"));
                    let _written = writeln!(part, "Your card {}: {}{now}", card.id, card.text);
                }
                None => part.push_str("No card yet.\n"),
            }
            part.push_str(
                &lines
                    .iter()
                    .map(|l| format!("[{}] {}", timestamp_to_api(l.at).as_str(), l.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            parts.push(part);
        }
        if !touched.is_empty() {
            self.touch(&touched).await;
        }
        found.chats = parts.len();
        found.text = parts.join("\n\n");
        Ok(found)
    }

    /// The Mac's chats with messages after `since`; none when there is no Mac helper. An
    /// error (in words) when it didn't answer.
    async fn mac_changes(&self, since: Timestamp) -> Result<Vec<Change>, String> {
        // The helper reads whole-second times only.
        let path = format!("/changes?since={}", since.strftime("%Y-%m-%dT%H:%M:%SZ"));
        let answer = match self.mac.get(&path, WAIT).await {
            Ok(answer) => answer,
            Err(MacError::NotSetUp) => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        if !answer.ok() {
            return Err(format!("the Mac said {}", answer.status));
        }
        let Value::Array(list) = answer.json() else {
            return Err("the Mac's changes aren't a list".to_owned());
        };
        Ok(list
            .iter()
            .filter_map(|c| {
                let id = c["id"].as_str().filter(|id| !id.is_empty())?.to_owned();
                let count = |key: &str| c[key].as_f64().unwrap_or(0.0).max(0.0);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a count of messages, never negative, capped where it is used"
                )]
                let messages = (count("incoming") + count("outgoing")) as u64;
                Some(Change {
                    name: c["name"].as_str().unwrap_or(&id).to_owned(),
                    source: c["source"].as_str().unwrap_or_default().to_owned(),
                    group: truthy(&c["group"]),
                    messages,
                    id,
                })
            })
            .collect())
    }

    /// `change`'s new messages, at most `limit`, oldest first.
    async fn lines(
        &self,
        change: &Change,
        since: Timestamp,
        mail_since: Timestamp,
        limit: u64,
        left: Duration,
    ) -> Result<Vec<Line>, String> {
        if is_mail(&change.id) {
            let mail = self
                .gmail
                .new_since(&change.id, mail_since, limit)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(mail
                .iter()
                .map(|m| Line {
                    at: m.at,
                    from_me: m.from_me,
                    text: line(m, MAIL_CHARS),
                })
                .collect());
        }
        if change.id.starts_with(CHAT_PREFIX) {
            let slack = self
                .slack
                .messages(&change.id, limit)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(slack
                .into_iter()
                .filter(|m| m.at > since)
                .map(|m| Line {
                    at: m.at,
                    from_me: m.from_me,
                    text: format!("{}: {}", m.sender, m.text),
                })
                .collect());
        }
        let path = format!("/read?chat={}&limit={limit}", encode(&change.id));
        let answer = self.mac.get(&path, WAIT).await.map_err(|e| e.to_string())?;
        if !answer.ok() {
            return Err(format!("the Mac said {}", answer.status));
        }
        let Value::Array(list) = answer.json() else {
            return Err("the Mac's messages aren't a list".to_owned());
        };
        let mut new: Vec<Value> = list
            .into_iter()
            .filter(|m| {
                m["at"]
                    .as_str()
                    .and_then(|at| at.parse::<Timestamp>().ok())
                    .is_some_and(|at| at > since)
            })
            .collect();
        if let Err(error) = self.voice.in_mac_messages(&mut new, left).await {
            tracing::warn!(%error, "news: the Mac's voice notes weren't read");
        }
        Ok(new
            .iter()
            .filter_map(|m| {
                let at = m["at"].as_str()?.parse::<Timestamp>().ok()?;
                let from_me = truthy(&m["fromMe"]);
                let who = if from_me {
                    OWNER
                } else {
                    m["sender"].as_str().unwrap_or(&change.name)
                };
                let said = m["text"].as_str().unwrap_or_default();
                let chars = if said.starts_with("[voice") {
                    VOICE_CHARS
                } else {
                    MESSAGE_CHARS
                };
                Some(Line {
                    at,
                    from_me,
                    text: format!("{who}: {}", first_chars(said, chars)),
                })
            })
            .collect())
    }

    /// Keeps what this look changed on the loops, on the loops as they are now (one may have
    /// been set or closed meanwhile).
    async fn touch(&self, touched: &HashMap<i64, Touch>) {
        let kept = async {
            let mut context = self.ctx.settings.get(&CONTEXT).await?.unwrap_or_default();
            for open in &mut context.loops {
                let Some((at, maybe)) = open
                    .get("id")
                    .and_then(Value::as_i64)
                    .and_then(|id| touched.get(&id))
                else {
                    continue;
                };
                open.insert("touched".to_owned(), Value::from(at.clone()));
                if let Some(maybe) = maybe {
                    open.insert("maybe".to_owned(), Value::from(maybe.clone()));
                }
            }
            self.ctx.settings.set(&CONTEXT, &context).await
        };
        if let Err(error) = kept.await {
            tracing::warn!(%error, "news: the loops weren't kept");
        }
    }
}

/// Marks the loops in `change`'s chat touched at `now`, and maybe answered when the owner wrote
/// there (`mine`, their first new message) after the loop opened; each change goes into
/// `touched`. The loops that may be answered, in words.
fn touch_loops(
    loops: &mut [serde_json::Map<String, Value>],
    change: &Change,
    mine: Option<Timestamp>,
    now: &str,
    touched: &mut HashMap<i64, Touch>,
) -> Vec<String> {
    let mut answered = Vec::new();
    for open in loops {
        if !in_chat(open, &change.id, &change.name) {
            continue;
        }
        let Some(id) = open.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let opened = text(open, "since").parse::<Timestamp>().ok();
        let mut maybe = Some(text(open, "maybe")).filter(|m| !m.is_empty());
        if maybe.is_none()
            && let (Some(mine), Some(opened)) = (mine, opened)
            && mine > opened
        {
            maybe = Some(timestamp_to_api(mine).as_str().to_owned());
        }
        open.insert("touched".to_owned(), Value::from(now.to_owned()));
        if let Some(maybe) = &maybe {
            open.insert("maybe".to_owned(), Value::from(maybe.clone()));
            answered.push(format!(
                "loop #{id} ({}: {})",
                text(open, "who"),
                first_chars(&text(open, "what"), 80)
            ));
        }
        touched.insert(id, (now.to_owned(), maybe));
    }
    answered
}

/// `open`'s field `key` as text ("" when it has none).
fn text(open: &serde_json::Map<String, Value>, key: &str) -> String {
    open.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Whether loop `open` lives in chat `chat` (named `name`): its own chat, or, when it has none,
/// one whose name its source or person mentions (`WhatsApp: Garden club`).
fn in_chat(open: &serde_json::Map<String, Value>, chat: &str, name: &str) -> bool {
    let own = text(open, "chat");
    if !own.is_empty() {
        return own == chat;
    }
    name.chars().count() >= 3
        && format!("{} {}", text(open, "source"), text(open, "who"))
            .to_lowercase()
            .contains(&name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn open(value: Value) -> serde_json::Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        }
    }

    #[test]
    fn a_loop_is_in_its_own_chat_or_one_it_names() {
        let own = open(json!({ "chat": "wa:1", "who": "Ann" }));
        assert!(in_chat(&own, "wa:1", "Bob"));
        assert!(!in_chat(&own, "wa:2", "Ann"));
        let named = open(json!({ "source": "WhatsApp: Garden club", "who": "Ann" }));
        assert!(in_chat(&named, "wa:9", "garden CLUB"));
        assert!(!in_chat(&named, "wa:9", "An"));
    }
}
