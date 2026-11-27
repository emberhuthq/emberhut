//! One set of reads over all of the owner's messages: iMessage, SMS and WhatsApp from the Mac
//! (the Mac helper answers `/chats`, `/read` and `/search`, read-only, live), Slack, the voice
//! notes' transcripts, and the owner's email. These are `messages_chats`, `messages_read`,
//! `messages_search`, and `replied`: did the owner already answer in a chat?

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use emberhut_core::messaging::text::first_chars;
use emberhut_core::messaging::{mail, slack};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use super::gmail::{GmailService, line};
use super::mac::{MacMessages, encode};
use super::slack::SlackService;
use super::voice_notes::{VoiceNotes, truthy};

/// How long the Mac helper may take for a list or a search.
const MAC_WAIT: Duration = Duration::from_secs(20);
/// How long a read waits for voice notes to be transcribed.
const READ_WAIT: Duration = Duration::from_secs(30);

/// The owner's messages, read together.
#[derive(Clone, Debug)]
pub struct Messages {
    mac: Arc<dyn MacMessages>,
    slack: SlackService,
    gmail: GmailService,
    voice: VoiceNotes,
    clock: Arc<dyn emberhut_core::Clock>,
}

/// What `replied` found.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Replied {
    /// The owner wrote in the chat since then.
    pub answered: bool,
    /// The owner's messages there since then.
    ///
    /// `his` is this field's wire name (the `replied` tool's answer).
    pub his: Vec<String>,
    /// Why the answer may be incomplete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Messages {
    /// The reads over these sources.
    #[must_use]
    pub fn new(
        mac: Arc<dyn MacMessages>,
        slack: SlackService,
        gmail: GmailService,
        voice: VoiceNotes,
        clock: Arc<dyn emberhut_core::Clock>,
    ) -> Self {
        Self {
            mac,
            slack,
            gmail,
            voice,
            clock,
        }
    }

    /// The Mac helper's answer to `path`, as text, with its voice notes as their transcripts
    /// (waiting up to `wait` for new ones).
    async fn mac_ask(&self, path: &str, wait: Duration) -> String {
        let answer = match self.mac.get(path, MAC_WAIT).await {
            Ok(answer) => answer,
            Err(error) => {
                tracing::debug!(%error, "the Mac helper didn't answer");
                return "The owner's Mac didn't answer (asleep or offline): those messages can't \
                        be read right now."
                    .to_owned();
            }
        };
        if !answer.ok() {
            return format!("The Mac answered {}.", answer.status);
        }
        let raw = answer.text();
        if !raw.contains("\"audio\"") {
            return raw;
        }
        let Value::Array(mut list) = answer.json() else {
            return raw;
        };
        match self.voice.in_mac_messages(&mut list, wait).await {
            Ok(()) => Value::Array(list).to_string(),
            Err(error) => {
                tracing::warn!(%error, "the Mac's voice notes weren't read");
                raw
            }
        }
    }

    /// The Mac's answer, then Slack's, the voice notes' and the email's.
    async fn with_all(
        &self,
        mac: String,
        slack: Value,
        voice: Option<Value>,
        mail: Option<Value>,
    ) -> String {
        let mut out = format!("{mac}\n\nSlack: {slack}");
        if let Some(voice) = voice.filter(|v| v.as_array().is_some_and(|v| !v.is_empty())) {
            let _written = write!(
                out,
                "

WhatsApp and iMessage voice notes that match (what was said): {voice}"
            );
        }
        if let Some(accounts) = self.gmail.summary().await {
            let mail = mail.unwrap_or_else(|| json!([]));
            let _written = write!(out, "\n\nEmail ({accounts}): {mail}");
        }
        out
    }

    /// `messages_chats`: the owner's recent chats everywhere, in the last `days`.
    pub async fn chats(&self, days: i64) -> String {
        let mac = self
            .mac_ask(&format!("/chats?days={days}"), Duration::ZERO)
            .await;
        let slack = self.slack.chats(days).await.unwrap_or_default();
        let mail = self.gmail.chats(days).await.unwrap_or_default();
        self.with_all(mac, json!(slack), None, Some(json!(mail)))
            .await
    }

    /// `messages_read`: one chat's latest `limit` messages, oldest first.
    pub async fn read(&self, chat: &str, limit: u64) -> String {
        if mail::is_mail(chat) {
            return match self.gmail.read(chat, limit).await {
                Ok(read) => json!(read).to_string(),
                Err(error) => error.to_string(),
            };
        }
        if chat.starts_with(slack::CHAT_PREFIX) {
            return match self.slack.read(chat, limit).await {
                Ok(read) => json!(read).to_string(),
                Err(error) => format!("Slack: {error}"),
            };
        }
        self.mac_ask(
            &format!("/read?chat={}&limit={limit}", encode(chat)),
            READ_WAIT,
        )
        .await
    }

    /// `messages_search`: the owner's messages everywhere holding `words`, newest first.
    pub async fn search(&self, words: &str, days: i64, limit: u64) -> String {
        let path = format!("/search?q={}&days={days}&limit={limit}", encode(words));
        let (mac, mail) = tokio::join!(
            self.mac_ask(&path, Duration::ZERO),
            self.gmail.search(words, days, limit)
        );
        let slack = self
            .slack
            .search(words, days, limit)
            .await
            .unwrap_or_default();
        let voice = self
            .voice
            .search(words, days, usize::try_from(limit).unwrap_or(usize::MAX))
            .await
            .unwrap_or_default();
        self.with_all(mac, json!(slack), Some(json!(voice)), Some(json!(mail)))
            .await
    }

    /// `replied`: did the owner already write in `chat` since `since` (default 3 days ago)?
    ///
    /// # Errors
    ///
    /// The reason in words: `since` isn't a time, or the messages couldn't be read.
    pub async fn replied(&self, chat: &str, since: Option<&str>) -> Result<Replied, String> {
        let since = match since.filter(|s| !s.is_empty()) {
            Some(since) => since.parse::<Timestamp>().map_err(|_| {
                format!("since: {since} isn't a time (ISO, like 2026-11-24T09:00:00Z)")
            })?,
            None => self.clock.now() - SignedDuration::from_hours(72),
        };
        if mail::is_mail(chat) {
            if let Err(error) = self.gmail.read(chat, 40).await {
                tracing::debug!(%error, "Gmail didn't answer; the kept mail instead");
            }
            let theirs: Vec<String> = self
                .gmail
                .messages(chat, 40)
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .filter(|m| m.from_me && m.at > since)
                .map(|m| format!("[{}] {}", m.at, line(m, 400)))
                .collect();
            return Ok(Replied {
                answered: !theirs.is_empty(),
                his: theirs,
                warning: None,
            });
        }
        let is_slack = chat.starts_with(slack::CHAT_PREFIX);
        let mine: Vec<(Timestamp, String)> = if is_slack {
            if let Err(error) = self.slack.read(chat, 60).await {
                tracing::debug!(%error, "Slack wasn't read fresh");
            }
            self.slack
                .messages(chat, 60)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|m| m.from_me && m.at > since)
                .map(|m| (m.at, m.text))
                .collect()
        } else {
            let answer = self
                .mac
                .get(&format!("/read?chat={}&limit=60", encode(chat)), MAC_WAIT)
                .await
                .map_err(|e| e.to_string())?;
            let Value::Array(mut list) = answer.json() else {
                return Err("the Mac's messages aren't a list".to_owned());
            };
            if let Err(error) = self
                .voice
                .in_mac_messages(&mut list, Duration::from_secs(10))
                .await
            {
                tracing::warn!(%error, "the Mac's voice notes weren't read");
            }
            list.iter()
                .filter(|m| truthy(&m["fromMe"]))
                .filter_map(|m| {
                    let at = m["at"].as_str()?.parse::<Timestamp>().ok()?;
                    (at > since).then(|| (at, m["text"].as_str().unwrap_or_default().to_owned()))
                })
                .collect()
        };
        let theirs: Vec<String> = mine
            .iter()
            .map(|(at, text)| format!("[{at}] {}", first_chars(text, 400)))
            .collect();
        let (whole, saved) = if is_slack {
            self.slack.coverage(chat).await
        } else {
            (true, None)
        };
        let warning = if whole || !theirs.is_empty() {
            None
        } else if let Some(saved) = saved {
            let unsure = if saved.parse::<Timestamp>().is_ok_and(|saved| saved < since) {
                ", so it can't be told from here: ask the owner"
            } else {
                ""
            };
            Some(format!(
                "The owner's own Slack messages are read from the Slack app on the Mac, last saved \
                 {saved}. A reply sent after that isn't seen yet{unsure}."
            ))
        } else {
            Some(
                "The owner's own replies in this Slack workspace aren't readable yet (the Mac helper \
                 hasn't sent them), so it can't be told from here. Ask the owner."
                    .to_owned(),
            )
        };
        Ok(Replied {
            answered: !theirs.is_empty(),
            his: theirs,
            warning,
        })
    }
}
