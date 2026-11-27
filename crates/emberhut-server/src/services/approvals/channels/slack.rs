//! An approver's Slack DM: opened with the agent's bot token in the approver's workspace, the
//! request's files uploaded into it, then the request posted word for word with Approve and
//! Reject buttons. Slack calls the interactivity route when one is pressed; the message is then
//! updated to say what was decided, without its buttons.

use std::fmt;
use std::time::Duration;

use emberhut_core::approvals::SlackPost;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::ChannelError;
use crate::services::messaging::web::{WebClient, WebRequest};

/// How long a call to Slack may take.
const CALL_WAIT: Duration = Duration::from_secs(20);
/// How long a file's upload may take.
const UPLOAD_WAIT: Duration = Duration::from_secs(120);
/// The button that approves: its `action_id`.
pub(crate) const APPROVE_ACTION: &str = "approve";
/// The button that rejects: its `action_id`.
pub(crate) const REJECT_ACTION: &str = "reject";

/// A file to upload into the DM, read by the server and checked against its hash.
#[derive(Clone, PartialEq, Eq)]
pub struct DmFile {
    /// Its name.
    pub name: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for DmFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmFile")
            .field("name", &self.name)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A request as its approver's DM shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmMessage {
    /// The approver's Slack user id.
    pub user: String,
    /// One line: who asks, for what kind of action, where.
    pub heading: String,
    /// The agent's one line.
    pub summary: String,
    /// The exact words, shown as they are.
    pub text: Option<String>,
    /// One line per file: its name, size and SHA-256.
    pub file_lines: Vec<String>,
    /// The files to upload (those still matching their hash).
    pub files: Vec<DmFile>,
    /// The payload hash, as the approver may compare it.
    pub payload_hash: String,
    /// The buttons' value: the request id and the payload hash.
    pub button_value: String,
}

/// Posts approval DMs and updates them.
pub trait SlackDm: fmt::Debug + Send + Sync {
    /// Opens a DM with `message.user` with the bot `token`, uploads the files into it and posts
    /// the request with its buttons. Where the message is.
    fn post<'a>(
        &'a self,
        token: &'a str,
        message: &'a DmMessage,
    ) -> BoxFuture<'a, Result<SlackPost, ChannelError>>;

    /// Replaces the message at `post` with `outcome`, without its buttons.
    fn update<'a>(
        &'a self,
        token: &'a str,
        post: &'a SlackPost,
        outcome: &'a str,
    ) -> BoxFuture<'a, Result<(), ChannelError>>;
}

/// No Slack on this server.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSlackDm;

impl SlackDm for NoSlackDm {
    fn post<'a>(
        &'a self,
        _: &'a str,
        _: &'a DmMessage,
    ) -> BoxFuture<'a, Result<SlackPost, ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }

    fn update<'a>(
        &'a self,
        _: &'a str,
        _: &'a SlackPost,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }
}

/// Slack's Web API over the server's HTTP client.
#[derive(Clone, Debug)]
pub struct HttpSlackDm {
    web: WebClient,
    base: String,
}

impl HttpSlackDm {
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
            .limit(1 << 20);
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

    /// Uploads `file` and shares it into `channel`.
    async fn upload(&self, token: &str, channel: &str, file: &DmFile) -> Result<(), ChannelError> {
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
        let files = json!([{ "id": id, "title": file.name }]).to_string();
        self.call(
            token,
            "files.completeUploadExternal",
            &[("files", &files), ("channel_id", channel)],
        )
        .await
        .map(drop)
    }
}

impl SlackDm for HttpSlackDm {
    fn post<'a>(
        &'a self,
        token: &'a str,
        message: &'a DmMessage,
    ) -> BoxFuture<'a, Result<SlackPost, ChannelError>> {
        Box::pin(async move {
            let opened = self
                .call(token, "conversations.open", &[("users", &message.user)])
                .await?;
            let channel = opened["channel"]["id"]
                .as_str()
                .ok_or_else(|| ChannelError::Refused("conversations.open: no channel".to_owned()))?
                .to_owned();
            for file in &message.files {
                self.upload(token, &channel, file).await?;
            }
            let blocks = request_blocks(message).to_string();
            let posted = self
                .call(
                    token,
                    "chat.postMessage",
                    &[
                        ("channel", &channel),
                        ("text", &message.heading),
                        ("blocks", &blocks),
                        ("unfurl_links", "false"),
                    ],
                )
                .await?;
            let ts = posted["ts"]
                .as_str()
                .ok_or_else(|| ChannelError::Refused("chat.postMessage: no ts".to_owned()))?;
            Ok(SlackPost {
                channel,
                ts: ts.to_owned(),
            })
        })
    }

    fn update<'a>(
        &'a self,
        token: &'a str,
        post: &'a SlackPost,
        outcome: &'a str,
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            let blocks = json!([plain_section(outcome)]).to_string();
            self.call(
                token,
                "chat.update",
                &[
                    ("channel", &post.channel),
                    ("ts", &post.ts),
                    ("text", outcome),
                    ("blocks", &blocks),
                ],
            )
            .await
            .map(drop)
        })
    }
}

/// Plain text Slack shows as it is: no mrkdwn, no mentions, no links made from it.
fn plain(text: &str) -> Value {
    json!({ "type": "plain_text", "text": text, "emoji": false })
}

/// A section of plain text.
fn plain_section(text: &str) -> Value {
    json!({ "type": "section", "text": plain(text) })
}

/// The request's message: its heading and summary, the exact words in a preformatted block,
/// its files and hash, the link, and the two buttons carrying the request id and hash.
fn request_blocks(message: &DmMessage) -> Value {
    let mut blocks = vec![
        plain_section(&message.heading),
        plain_section(&message.summary),
    ];
    if let Some(text) = &message.text {
        blocks.push(json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_preformatted",
                "elements": [{ "type": "text", "text": text }],
            }],
        }));
    }
    let mut context: Vec<Value> = message.file_lines.iter().map(|line| plain(line)).collect();
    context.push(plain(&format!("Payload hash: {}", message.payload_hash)));
    blocks.push(json!({ "type": "context", "elements": context }));
    blocks.push(json!({
        "type": "actions",
        "elements": [
            {
                "type": "button",
                "action_id": APPROVE_ACTION,
                "style": "primary",
                "text": plain("Approve"),
                "value": message.button_value,
            },
            {
                "type": "button",
                "action_id": REJECT_ACTION,
                "style": "danger",
                "text": plain("Reject"),
                "value": message.button_value,
            },
        ],
    }));
    Value::Array(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_carries_the_words_as_they_are_and_the_buttons_the_hash() {
        let message = DmMessage {
            user: "U0APPROVER".to_owned(),
            heading: "Example helper asks to post to example blog".to_owned(),
            summary: "The launch post".to_owned(),
            text: Some("We *shipped* <it> & more".to_owned()),
            file_lines: vec!["cover.png, 9 bytes, sha256 ab".to_owned()],
            files: Vec::new(),
            payload_hash: "ab".repeat(32),
            button_value: format!("7.{}", "ab".repeat(32)),
        };
        let blocks = request_blocks(&message);
        assert_eq!(
            blocks[2]["elements"][0]["elements"][0]["text"],
            "We *shipped* <it> & more"
        );
        let buttons = &blocks[4]["elements"];
        assert_eq!(buttons[0]["action_id"], APPROVE_ACTION);
        assert_eq!(buttons[1]["action_id"], REJECT_ACTION);
        assert_eq!(buttons[0]["value"], message.button_value);
        let context = blocks[3]["elements"].to_string();
        assert!(context.contains("cover.png"));
        assert!(
            !blocks.to_string().contains("/approve/"),
            "the bot's DM never carries a link"
        );
    }
}
