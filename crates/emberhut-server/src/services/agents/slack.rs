//! An agent's own Slack bot, with the bot token of the agent's channel: posting its answer
//! (`chat.postMessage`, or its files uploaded with the words as their comment) and reading a
//! thread it is in (`conversations.replies`).
//!
//! A trait with a real implementation over the server's HTTP client ([`HttpAgentSlack`]) and a
//! stand-in for a server without one ([`NoAgentSlack`]); tests swap in fakes.

use std::fmt;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::services::approvals::channels::{ChannelError, DmFile};
use crate::services::messaging::web::{WebClient, WebRequest};

/// How long a call to Slack may take.
const CALL_WAIT: Duration = Duration::from_secs(20);
/// How long a file's upload may take.
const UPLOAD_WAIT: Duration = Duration::from_secs(300);

/// Where an agent posts: a channel, and the thread in it (none: the channel itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place<'a> {
    /// The channel's id.
    pub channel: &'a str,
    /// The thread's timestamp, if it answers in one.
    pub thread: Option<&'a str>,
}

/// One message of a thread, as Slack gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadMessage {
    /// Its timestamp.
    pub ts: String,
    /// Who wrote it (a user id); none for a bot's.
    pub user: Option<String>,
    /// A bot wrote it.
    pub bot: bool,
    /// Its words.
    pub text: String,
    /// The files shared with it: their ids and names.
    pub files: Vec<(String, String)>,
}

/// An agent's Slack bot.
pub trait AgentSlack: fmt::Debug + Send + Sync {
    /// Posts `text` at `place` with the bot `token`; the message's timestamp.
    fn post<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
    ) -> BoxFuture<'a, Result<String, ChannelError>>;

    /// Uploads `files` at `place` as one message, `text` its comment.
    fn upload<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
        files: &'a [DmFile],
    ) -> BoxFuture<'a, Result<(), ChannelError>>;

    /// The last `limit` messages of the thread at `place`, oldest first.
    fn replies<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<ThreadMessage>, ChannelError>>;
}

/// No Slack on this server.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAgentSlack;

impl AgentSlack for NoAgentSlack {
    fn post<'a>(
        &'a self,
        _: &'a str,
        _: Place<'a>,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }

    fn upload<'a>(
        &'a self,
        _: &'a str,
        _: Place<'a>,
        _: &'a str,
        _: &'a [DmFile],
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }

    fn replies<'a>(
        &'a self,
        _: &'a str,
        _: Place<'a>,
        _: u32,
    ) -> BoxFuture<'a, Result<Vec<ThreadMessage>, ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }
}

/// Slack's Web API over the server's HTTP client.
#[derive(Clone, Debug)]
pub struct HttpAgentSlack {
    web: WebClient,
    base: String,
}

impl HttpAgentSlack {
    /// The API at `base` (`https://slack.com/api`), reached with `web`.
    #[must_use]
    pub fn new(web: WebClient, base: &str) -> Self {
        Self {
            web,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// Calls `method` with `token` and the form `fields`: its JSON, which said `ok: true`.
    async fn call(
        &self,
        token: &str,
        method: &str,
        fields: &[(&str, &str)],
    ) -> Result<Value, ChannelError> {
        let request = WebRequest::form(format!("{}/{method}", self.base), fields, CALL_WAIT)
            .bearer(token)
            .limit(4 << 20);
        let answer = self
            .web
            .send(request)
            .await
            .map_err(|error| ChannelError::Unreachable(error.to_string()))?;
        let json = answer.json();
        if json["ok"] == Value::Bool(true) {
            Ok(json)
        } else {
            let error = json["error"].as_str().unwrap_or("no answer").to_owned();
            Err(ChannelError::Refused(format!("{method}: {error}")))
        }
    }

    /// Uploads `file`'s bytes to a new upload slot: the file's id.
    async fn upload_one(&self, token: &str, file: &DmFile) -> Result<String, ChannelError> {
        let length = file.bytes.len().to_string();
        let slot = self
            .call(
                token,
                "files.getUploadURLExternal",
                &[("filename", &file.name), ("length", &length)],
            )
            .await?;
        let (Some(url), Some(id)) = (slot["upload_url"].as_str(), slot["file_id"].as_str()) else {
            return Err(ChannelError::Refused(
                "files.getUploadURLExternal: no upload address".to_owned(),
            ));
        };
        let sent = self
            .web
            .send(
                WebRequest::post(
                    url,
                    "application/octet-stream",
                    file.bytes.clone(),
                    UPLOAD_WAIT,
                )
                .limit(64 << 10),
            )
            .await
            .map_err(|error| ChannelError::Unreachable(error.to_string()))?;
        if !sent.ok() {
            return Err(ChannelError::Refused(format!(
                "file upload: HTTP {}",
                sent.status
            )));
        }
        Ok(id.to_owned())
    }
}

impl AgentSlack for HttpAgentSlack {
    fn post<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
    ) -> BoxFuture<'a, Result<String, ChannelError>> {
        Box::pin(async move {
            // Plain text: no mrkdwn formatting, no previews made from it.
            let mut fields = vec![
                ("channel", place.channel),
                ("text", text),
                ("mrkdwn", "false"),
                ("unfurl_links", "false"),
                ("unfurl_media", "false"),
            ];
            if let Some(thread) = place.thread {
                fields.push(("thread_ts", thread));
            }
            let posted = self.call(token, "chat.postMessage", &fields).await?;
            posted["ts"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| ChannelError::Refused("chat.postMessage: no ts".to_owned()))
        })
    }

    fn upload<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
        files: &'a [DmFile],
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            let mut uploaded = Vec::with_capacity(files.len());
            for file in files {
                let id = self.upload_one(token, file).await?;
                uploaded.push(json!({ "id": id, "title": file.name }));
            }
            let uploaded = Value::Array(uploaded).to_string();
            let mut fields = vec![("files", uploaded.as_str()), ("channel_id", place.channel)];
            if !text.is_empty() {
                fields.push(("initial_comment", text));
            }
            if let Some(thread) = place.thread {
                fields.push(("thread_ts", thread));
            }
            self.call(token, "files.completeUploadExternal", &fields)
                .await
                .map(drop)
        })
    }

    fn replies<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<ThreadMessage>, ChannelError>> {
        Box::pin(async move {
            let thread = place.thread.unwrap_or_default();
            let asked = limit.saturating_add(5).min(200).to_string();
            let answer = self
                .call(
                    token,
                    "conversations.replies",
                    &[
                        ("channel", place.channel),
                        ("ts", thread),
                        ("limit", &asked),
                    ],
                )
                .await?;
            let messages: Vec<ThreadMessage> = answer["messages"]
                .as_array()
                .map_or_default(|all| all.iter().map(thread_message).collect());
            let skip = messages
                .len()
                .saturating_sub(usize::try_from(limit).unwrap_or(usize::MAX));
            Ok(messages.into_iter().skip(skip).collect())
        })
    }
}

/// One of `conversations.replies`' messages.
fn thread_message(message: &Value) -> ThreadMessage {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    ThreadMessage {
        ts: text(&message["ts"]),
        user: message["user"].as_str().map(str::to_owned),
        bot: message["bot_id"].is_string(),
        text: text(&message["text"]),
        files: message["files"].as_array().map_or_default(|files| {
            files
                .iter()
                .map(|file| {
                    let name = file["name"]
                        .as_str()
                        .or_else(|| file["title"].as_str())
                        .unwrap_or("file");
                    (text(&file["id"]), name.to_owned())
                })
                .collect()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_message_reads_its_words_files_and_author() {
        let message = thread_message(&json!({
            "ts": "1800000000.000100", "user": "U0EXAMPLE", "text": "see the brief",
            "files": [{ "id": "F0EXAMPLE", "name": "brief.pdf" }, { "id": "F0OTHER", "title": "notes" }]
        }));
        assert_eq!(message.user.as_deref(), Some("U0EXAMPLE"));
        assert!(!message.bot);
        assert_eq!(
            message.files,
            [
                ("F0EXAMPLE".to_owned(), "brief.pdf".to_owned()),
                ("F0OTHER".to_owned(), "notes".to_owned())
            ]
        );
        let bot = thread_message(&json!({ "ts": "1", "bot_id": "B0EXAMPLE", "text": "done" }));
        assert!(bot.bot && bot.user.is_none());
    }
}
