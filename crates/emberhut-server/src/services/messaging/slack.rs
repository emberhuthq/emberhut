//! The owner's Slack, in one table, from up to three sources:
//!
//! - Slack's notifications on the owner's Mac (DMs, mentions, the owner's threads; never the
//!   owner's own replies), collected every 2 minutes through the Mac helper. Every workspace
//!   starts here.
//! - The owner's own messages from the Slack app on the Mac: its saved state, read by the Mac
//!   helper without Slack's API, as of Slack's last save (`slackSaved`, per workspace).
//! - In a workspace where the owner installed a Slack app with a user token (the secret
//!   `slack-<workspace>-user`, given on a one-time secret page), Slack's Web API: every 2
//!   minutes a search for the owner's own messages (`from:me`), so the assistant sees what was
//!   answered already; and each conversation read fresh when it is read.
//!
//! Voice clips are transcribed (`voice_notes`); any other file reads
//! `[file: name.ext, kind; slack_file F…]`, and `slack_file` fetches it.
//!
//! Reading never posts. The one thing that posts is [`SlackService::send`], `slack_send`: one
//! message as the owner (`chat:write` on the owner's user token), each only after the owner's
//! OK on that exact text ([`super::guard`]).
//!
//! [`SlackApi`] is the seam: [`HttpSlack`] is Slack's Web API, [`NoSlackApi`] stands in when it
//! isn't set up, and tests swap in a fake.

use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use base64::Engine as _;
use emberhut_core::RoomId;
use emberhut_core::messaging::OWNER;
use emberhut_core::messaging::slack::{
    self as rules, SlackMessage, chat_id, is_file_id, parse_chat, ts_instant, user_token_secret,
    workspace_of_secret,
};
use emberhut_core::messaging::text::{first_chars, js_trim, same, slack_formatting};
use emberhut_core::messaging::voice::{SeenNote, VoiceSource};
use emberhut_db::{DbError, SettingKey, SlackStore};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;
use serde_json::{Value, json};

use super::context::Context;
use super::guard::{
    self, CANCEL, Card, CardAnswer, Decision, Draft, Drafts, Go, Pending, Prepared, Reading,
    SEND_IT, Sent,
};
use super::hooks::{Ask, TaskId};
use super::mac::{MacMessages, encode};
use super::voice_notes::{VoiceNotes, truthy};
use super::web::{WebClient, WebRequest};

/// How long a call to Slack may take.
const CALL_WAIT: Duration = Duration::from_secs(20);
/// How long a file's download may take.
const DOWNLOAD_WAIT: Duration = Duration::from_secs(120);
/// How long after a failed connect it is tried again.
const RECONNECT: SignedDuration = SignedDuration::from_mins(10);
/// The biggest file `slack_file` fetches.
const FILE_LIMIT: u64 = 100 << 20;
/// The biggest picture `slack_file` hands back to look at.
const IMAGE_LIMIT: usize = 5 << 20;
/// How long a read waits for a voice clip just seen.
const READ_SETTLE: Duration = Duration::from_secs(30);

/// When the Slack app on the Mac last saved each workspace's messages.
const SLACK_SAVED: SettingKey<BTreeMap<String, Option<String>>> = SettingKey::new("slackSaved");
/// When the Mac helper last answered.
const MAC_AT: SettingKey<String> = SettingKey::new("macAt");
/// `slack_send`'s waiting and sent messages.
const DRAFTS: Drafts = Drafts::new("slackPending", "slackSent");

/// Why Slack didn't answer as asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SlackError {
    /// No Slack API is set up on this server.
    #[error("Slack isn't set up on this server")]
    NotSetUp,
    /// Slack didn't answer.
    #[error("{0}")]
    Unreachable(String),
    /// Slack answered `ok: false`.
    #[error("{method}: {error}")]
    Refused {
        /// The method called.
        method: String,
        /// Slack's error.
        error: String,
    },
}

/// A Web API call's answer: Slack's JSON (with `ok: true`), and the token's scopes when Slack
/// told them.
#[derive(Clone, Debug, PartialEq)]
pub struct SlackAnswer {
    /// The JSON.
    pub json: Value,
    /// `x-oauth-scopes`.
    pub scopes: Option<String>,
}

/// A file fetched with a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// Its status.
    pub status: u16,
    /// Its `content-type`.
    pub content_type: String,
    /// Its bytes.
    pub body: Bytes,
}

/// Slack's Web API.
pub trait SlackApi: fmt::Debug + Send + Sync {
    /// Calls `method` with `token` and the form `args`. An answer with `ok: false` is
    /// [`SlackError::Refused`].
    fn call<'a>(
        &'a self,
        token: &'a str,
        method: &'a str,
        args: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<SlackAnswer, SlackError>>;

    /// Fetches `url` (a file's `url_private`) with `token`.
    fn download<'a>(
        &'a self,
        url: &'a str,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Download, SlackError>>;
}

/// No Slack API set up.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSlackApi;

impl SlackApi for NoSlackApi {
    fn call<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<SlackAnswer, SlackError>> {
        Box::pin(async { Err(SlackError::NotSetUp) })
    }

    fn download<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Download, SlackError>> {
        Box::pin(async { Err(SlackError::NotSetUp) })
    }
}

/// Slack's Web API over HTTPS.
#[derive(Clone, Debug)]
pub struct HttpSlack {
    web: WebClient,
    base: String,
}

impl HttpSlack {
    /// The API at `base` (`https://slack.com/api`).
    #[must_use]
    pub fn new(web: WebClient, base: &str) -> Self {
        Self {
            web,
            base: base.trim_end_matches('/').to_owned(),
        }
    }
}

impl SlackApi for HttpSlack {
    fn call<'a>(
        &'a self,
        token: &'a str,
        method: &'a str,
        args: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<SlackAnswer, SlackError>> {
        Box::pin(async move {
            let fields: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let request = WebRequest::form(format!("{}/{method}", self.base), &fields, CALL_WAIT)
                .bearer(token)
                .limit(16 << 20);
            let answer = self
                .web
                .send(request)
                .await
                .map_err(|error| SlackError::Unreachable(error.to_string()))?;
            let json = answer.json();
            if !truthy(&json["ok"]) {
                return Err(SlackError::Refused {
                    method: method.to_owned(),
                    error: text_of(&json["error"]),
                });
            }
            Ok(SlackAnswer {
                scopes: answer.header("x-oauth-scopes").map(str::to_owned),
                json,
            })
        })
    }

    fn download<'a>(
        &'a self,
        url: &'a str,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Download, SlackError>> {
        Box::pin(async move {
            let request = WebRequest::get(url, DOWNLOAD_WAIT)
                .bearer(token)
                .limit(usize::try_from(FILE_LIMIT).unwrap_or(usize::MAX));
            let answer = self
                .web
                .send(request)
                .await
                .map_err(|error| SlackError::Unreachable(error.to_string()))?;
            Ok(Download {
                status: answer.status.as_u16(),
                content_type: answer.header("content-type").unwrap_or_default().to_owned(),
                body: answer.body,
            })
        })
    }
}

/// A JSON value as JavaScript's `String(v)` writes it inside a template.
fn text_of(value: &Value) -> String {
    match value {
        Value::Null => "undefined".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The first of `keys` in `object` that is truthy, as text.
fn first_of(object: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .map(|key| &object[*key])
        .find(|value| truthy(value))
        .map(text_of)
}

/// A workspace read with the owner's token: its name, the owner's user id there, the token,
/// and the workspace's short name in the secret's name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Live {
    team: String,
    me: String,
    user: String,
    short: String,
}

/// What the service remembers between calls.
#[derive(Debug, Default)]
struct Memory {
    /// The workspaces read with the owner's token, by their name in lower case.
    live: HashMap<String, Live>,
    /// The workspaces' short names tried, and when a failed one is tried again.
    tried: HashMap<String, Option<Timestamp>>,
    /// People's names by their id.
    people: HashMap<String, String>,
    /// Conversations' names by their id.
    rooms: HashMap<String, String>,
}

/// `messages_chats`' Slack row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackChat {
    /// The chat id.
    pub id: String,
    /// The conversation's name.
    pub name: String,
    /// `Slack <workspace>`.
    pub source: String,
    /// When its newest message was sent.
    pub last_at: String,
    /// Its messages in the stretch.
    pub messages: u64,
}

/// `messages_search`' Slack hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackHit {
    /// The chat id.
    pub chat: String,
    /// `Slack <workspace>`.
    pub source: String,
    /// When it was sent.
    pub at: String,
    /// Who wrote it.
    pub sender: String,
    /// What it says.
    pub text: String,
}

/// `messages_read`' answer for a Slack chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SlackRead {
    /// `<name> (Slack <workspace>)`.
    pub conversation: String,
    /// Where the messages come from.
    pub note: String,
    /// `[at] sender: text`, oldest first.
    pub messages: Vec<String>,
}

/// One message as `replied` reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlackLine {
    /// When it was sent.
    pub at: Timestamp,
    /// The owner wrote it.
    pub from_me: bool,
    /// Who wrote it.
    pub sender: String,
    /// What it says.
    pub text: String,
}

/// `slack_file`'s file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlackFile {
    /// The workspace it is in.
    pub workspace: String,
    /// Its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its kind.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Its type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,
    /// Its size.
    pub bytes: usize,
    /// Where it was saved.
    pub path: String,
    /// Its length, for a clip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// A picture to look at: its bytes in base64, and its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlackImage {
    /// Base64.
    pub data: String,
    /// `image/png`...
    pub mime_type: String,
}

/// Why `slack_file` fetched nothing.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SlackFileError {
    /// Not a file id, not found, too big, refused...: the words say.
    #[error("{0}")]
    Refused(String),
    /// The file couldn't be saved.
    #[error("the file could not be saved")]
    Save(#[source] std::io::Error),
}

/// What the Slack service is made of besides the context.
#[derive(Clone, Debug)]
pub struct SlackParts {
    /// The messages.
    pub store: SlackStore,
    /// Slack's Web API.
    pub api: Arc<dyn SlackApi>,
    /// The Mac helper.
    pub mac: Arc<dyn MacMessages>,
    /// Voice notes.
    pub voice: VoiceNotes,
    /// Where `slack_file` saves files.
    pub files: PathBuf,
}

/// The owner's Slack.
#[derive(Clone, Debug)]
pub struct SlackService {
    ctx: Context,
    parts: SlackParts,
    memory: Arc<Mutex<Memory>>,
}

impl SlackService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(ctx: Context, parts: SlackParts) -> Self {
        Self {
            ctx,
            parts,
            memory: Arc::default(),
        }
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The workspaces read with the owner's token, by name.
    #[must_use]
    pub fn live_teams(&self) -> Vec<String> {
        let mut teams: Vec<String> = self
            .memory()
            .live
            .values()
            .map(|l| l.team.clone())
            .collect();
        teams.sort();
        teams
    }

    fn live(&self, workspace: &str) -> Option<Live> {
        self.memory().live.get(&workspace.to_lowercase()).cloned()
    }

    /// Connects each workspace the secrets hold a user token for that isn't yet (or whose last
    /// try is due again), and takes a new token for one connected already.
    pub async fn connect_all(&self) {
        let now = self.ctx.now();
        for name in self.ctx.secrets.names().await {
            let Some(short) = workspace_of_secret(&name).map(str::to_owned) else {
                continue;
            };
            let due = match self.memory().tried.get(&short) {
                None => true,
                Some(Some(retry)) => *retry <= now,
                Some(None) => false,
            };
            if due {
                self.connect(&short).await;
                continue;
            }
            let token = self.ctx.secrets.read(&name).await;
            let replaced = {
                let mut memory = self.memory();
                memory
                    .live
                    .values_mut()
                    .find(|l| l.short == short && !token.is_empty() && l.user != token)
                    .map(|l| l.user.clone_from(&token))
                    .is_some()
            };
            if replaced && let Err(error) = self.parts.voice.retry_slack().await {
                tracing::warn!(%error, "Slack's clips weren't tried again");
            }
        }
    }

    /// Connects workspace `short` with its user token: who the owner is there, then their own
    /// messages. A failure is tried again in 10 minutes.
    async fn connect(&self, short: &str) {
        let token = self.ctx.secrets.read(&user_token_secret(short)).await;
        if token.is_empty() {
            return;
        }
        let connected = async {
            let me = self.parts.api.call(&token, "auth.test", &[]).await?.json;
            let live = Live {
                team: text_of(&me["team"]),
                me: text_of(&me["user_id"]),
                user: token.clone(),
                short: short.to_owned(),
            };
            self.memory()
                .live
                .insert(live.team.to_lowercase(), live.clone());
            self.mine(&live).await?;
            Ok::<Live, SlackError>(live)
        };
        match connected.await {
            Ok(live) => {
                self.memory().tried.insert(short.to_owned(), None);
                self.ctx
                    .activity(format!(
                        "Slack {}: connected, the owner's own messages included",
                        live.team
                    ))
                    .await;
            }
            Err(error) => {
                self.memory()
                    .tried
                    .insert(short.to_owned(), Some(self.ctx.now() + RECONNECT));
                self.ctx
                    .activity(format!(
                        "Slack {short}: can't connect ({error}); trying again in 10 minutes"
                    ))
                    .await;
            }
        }
    }

    /// The owner's own messages in every connected workspace (each workspace's error logged,
    /// never thrown).
    pub async fn collect_mine(&self) {
        let live: Vec<Live> = self.memory().live.values().cloned().collect();
        for l in live {
            if let Err(error) = self.mine(&l).await {
                tracing::warn!(team = l.team, %error, "Slack: the owner's messages weren't read");
            }
        }
    }

    /// The owner's newest messages in `l`, by Slack's search: those after the newest kept.
    async fn mine(&self, l: &Live) -> Result<(), SlackError> {
        let last = self
            .parts
            .store
            .last_mine(&l.team)
            .await
            .map_err(|e| SlackError::Unreachable(e.to_string()))?;
        let args = [
            ("query", "from:me".to_owned()),
            ("sort", "timestamp".to_owned()),
            ("sort_dir", "desc".to_owned()),
            ("count", "40".to_owned()),
        ];
        let found = self
            .parts
            .api
            .call(&l.user, "search.messages", &args)
            .await?
            .json;
        let matches = found["messages"]["matches"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for mut m in matches {
            let Some(at) = m["ts"].as_str().and_then(ts_instant) else {
                continue;
            };
            if last.is_some_and(|last| at <= last) {
                continue;
            }
            m["user"] = Value::String(l.me.clone());
            let channel = text_of(&m["channel"]["id"]);
            if let Err(error) = self.keep(l, &channel, &m).await {
                tracing::warn!(%error, "Slack: a message of the owner's wasn't kept");
            }
        }
        Ok(())
    }

    /// Slack's notifications from the Mac helper, since the newest kept (or 3 days).
    ///
    /// # Errors
    ///
    /// The helper's or the database's error, in words.
    pub async fn collect_notifications(&self) -> Result<(), String> {
        let now = self.ctx.now();
        let since = self
            .parts
            .store
            .last_notification()
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or(now - SignedDuration::from_hours(72));
        let path = format!("/notifications?since={}", encode(&since.to_string()));
        let answer = self
            .parts
            .mac
            .get(&path, CALL_WAIT)
            .await
            .map_err(|e| e.to_string())?;
        if !answer.ok() {
            return Err(format!("the Mac said {}", answer.status));
        }
        let Value::Array(notifications) = answer.json() else {
            return Err("the Mac's notifications aren't a list".to_owned());
        };
        for n in notifications {
            let Some(at) = n["at"].as_str().and_then(|at| at.parse::<Timestamp>().ok()) else {
                continue;
            };
            let message = SlackMessage {
                id: text_of(&n["id"]),
                workspace: text_of(&n["title"]),
                who: text_of(&n["subtitle"]),
                body: text_of(&n["body"]),
                at,
                from_me: false,
                sender: None,
                channel: None,
            };
            self.parts
                .store
                .add(&message)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.ctx
            .settings
            .set(&MAC_AT, &now.to_string())
            .await
            .map_err(|e| e.to_string())
    }

    /// The owner's own messages from the Slack app's saved state, through the Mac helper.
    ///
    /// # Errors
    ///
    /// The helper's or the database's error, in words.
    pub async fn collect_own(&self) -> Result<(), String> {
        let since = self.ctx.now() - SignedDuration::from_hours(72);
        let path = format!("/slack-mine?since={}", encode(&since.to_string()));
        let answer = self
            .parts
            .mac
            .get(&path, Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;
        if !answer.ok() {
            return Err(format!("the Mac said {}", answer.status));
        }
        let j = answer.json();
        for m in j["messages"].as_array().cloned().unwrap_or_default() {
            let Some(at) = m["at"].as_str().and_then(|at| at.parse::<Timestamp>().ok()) else {
                continue;
            };
            let message = SlackMessage {
                id: text_of(&m["id"]),
                workspace: text_of(&m["workspace"]),
                who: text_of(&m["who"]),
                body: text_of(&m["body"]),
                at,
                from_me: true,
                sender: Some(OWNER.to_owned()),
                channel: m["channel"].as_str().map(str::to_owned),
            };
            self.parts
                .store
                .add(&message)
                .await
                .map_err(|e| e.to_string())?;
        }
        let saved: BTreeMap<String, Option<String>> =
            serde_json::from_value(j["saved"].clone()).unwrap_or_default();
        self.ctx
            .settings
            .set(&SLACK_SAVED, &saved)
            .await
            .map_err(|e| e.to_string())
    }

    /// When the Slack app on the Mac last saved workspace `w`'s messages.
    async fn saved(&self, w: &str) -> Option<String> {
        self.ctx
            .settings
            .get(&SLACK_SAVED)
            .await
            .ok()
            .flatten()
            .and_then(|saved| saved.get(w).cloned().flatten())
    }

    async fn person(&self, l: &Live, id: &str) -> Result<String, SlackError> {
        if let Some(name) = self.memory().people.get(id) {
            return Ok(name.clone());
        }
        let user = self
            .parts
            .api
            .call(&l.user, "users.info", &[("user", id.to_owned())])
            .await?
            .json;
        let user = &user["user"];
        let name = first_of(&user["profile"], &["display_name"])
            .or_else(|| first_of(user, &["real_name", "name"]))
            .unwrap_or_else(|| id.to_owned());
        self.memory().people.insert(id.to_owned(), name.clone());
        Ok(name)
    }

    /// A conversation as Slack's notifications name it: the person for a DM, `#name` for a
    /// channel, the others for a group DM.
    async fn room(&self, l: &Live, id: &str, hint: Option<&str>) -> Result<String, SlackError> {
        if let Some(name) = self.memory().rooms.get(id) {
            return Ok(name.clone());
        }
        let channel = match self
            .parts
            .api
            .call(&l.user, "conversations.info", &[("channel", id.to_owned())])
            .await
        {
            Ok(answer) => answer.json["channel"].clone(),
            Err(_) if hint.is_some_and(|h| !h.is_empty()) => json!({ "name": hint }),
            Err(error) => return Err(error),
        };
        let name = if truthy(&channel["is_im"]) {
            self.person(l, &text_of(&channel["user"])).await?
        } else if truthy(&channel["is_mpim"]) {
            let members = self
                .parts
                .api
                .call(
                    &l.user,
                    "conversations.members",
                    &[("channel", id.to_owned())],
                )
                .await?
                .json;
            let mut names = Vec::new();
            for member in members["members"].as_array().cloned().unwrap_or_default() {
                let member = text_of(&member);
                if member != l.me {
                    names.push(self.person(l, &member).await?);
                }
            }
            names.join(", ")
        } else {
            format!("#{}", text_of(&channel["name"]))
        };
        self.memory().rooms.insert(id.to_owned(), name.clone());
        Ok(name)
    }

    /// A message's file in full: a live stub (`file_access: check_file_info`, no name) gets
    /// the rest from `files.info`.
    async fn full(&self, l: &Live, file: &Value) -> Value {
        let stub = !truthy(&file["name"]) || file["file_access"] == "check_file_info";
        let gone = file["mode"] == "tombstone" || file["mode"] == "hidden_by_limit";
        if !truthy(&file["id"]) || !stub || gone {
            return file.clone();
        }
        match self
            .parts
            .api
            .call(&l.user, "files.info", &[("file", text_of(&file["id"]))])
            .await
        {
            Ok(answer) if truthy(&answer.json["file"]) => answer.json["file"].clone(),
            _ => file.clone(),
        }
    }

    /// Keeps one message from Slack (a search hit or a history read), in `channel`.
    #[expect(
        clippy::too_many_lines,
        reason = "one message's parts in the order it reads: mentions, shares, files, clips"
    )]
    async fn keep(&self, l: &Live, channel: &str, m: &Value) -> Result<(), SlackError> {
        let subtype = m["subtype"].as_str().unwrap_or_default();
        if truthy(&m["subtype"])
            && ![
                "thread_broadcast",
                "file_share",
                "bot_message",
                "me_message",
            ]
            .contains(&subtype)
        {
            return Ok(());
        }
        let mut body = m["text"].as_str().unwrap_or_default().to_owned();
        for mention in mentions(&body) {
            let name = self
                .person(l, &mention)
                .await
                .unwrap_or_else(|_| mention.clone());
            body = body.replace(&format!("<@{mention}>"), &format!("@{name}"));
        }
        for a in m["attachments"].as_array().cloned().unwrap_or_default() {
            let share = truthy(&a["is_share"]) || truthy(&a["is_msg_unfurl"]);
            if !share && !js_trim(&body).is_empty() {
                continue;
            }
            let what = first_chars(
                js_trim(&first_of(&a, &["text", "fallback", "title"]).unwrap_or_default()),
                1000,
            );
            let url = first_of(&a, &["from_url", "title_link", "original_url"]);
            if share {
                let from = first_of(&a, &["author_name"])
                    .map_or_else(String::new, |author| format!(" from {author}"));
                let _written = write!(body, " [forwarded{from}: {what}]");
            } else if !what.is_empty() || url.is_some() {
                let parts: Vec<&str> = [Some(what.as_str()), url.as_deref()]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.is_empty())
                    .collect();
                let _written = write!(body, " [link: {}]", parts.join(" "));
            }
        }
        let mut files = Vec::new();
        for file in m["files"].as_array().cloned().unwrap_or_default() {
            files.push(self.full(l, &file).await);
        }
        let is_audio = |f: &Value| {
            truthy(&f["id"])
                && (f["subtype"] == "slack_audio"
                    || f["mimetype"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("audio/")))
        };
        for file in files.iter().filter(|f| !is_audio(f)) {
            body.push(' ');
            body.push_str(&file_shown(file));
        }
        let mut clips = Vec::new();
        for file in files.iter().filter(|f| is_audio(f)) {
            let note = format!("slack:{}", text_of(&file["id"]));
            let name = text_of(&file["name"]);
            let shown = self
                .parts
                .voice
                .shown(&note, &name)
                .await
                .map_err(|e| SlackError::Unreachable(e.to_string()))?;
            body.push(' ');
            body.push_str(&shown);
            clips.push((note, name, shown));
        }
        let ts = text_of(&m["ts"]);
        let at = ts_instant(&ts)
            .ok_or_else(|| SlackError::Unreachable("Invalid time value".to_owned()))?;
        let id = format!("{}:{channel}:{ts}", l.team);
        let who = self.room(l, channel, m["channel"]["name"].as_str()).await?;
        let sender = if truthy(&m["user"]) {
            self.person(l, &text_of(&m["user"])).await?
        } else {
            m["username"].as_str().unwrap_or("a bot").to_owned()
        };
        let from_me = m["user"].as_str() == Some(l.me.as_str());
        let message = SlackMessage {
            id: id.clone(),
            workspace: l.team.clone(),
            who: who.clone(),
            body: js_trim(&body).to_owned(),
            at,
            from_me,
            sender: Some(sender.clone()),
            channel: Some(channel.to_owned()),
        };
        self.parts
            .store
            .keep(&message)
            .await
            .map_err(|e| SlackError::Unreachable(e.to_string()))?;
        // A note that got its new line while this body was being put together had it written
        // into the row this one just replaced: it is put back.
        for (note, name, shown) in &clips {
            let now = self
                .parts
                .voice
                .shown(note, name)
                .await
                .map_err(|e| SlackError::Unreachable(e.to_string()))?;
            if now != *shown {
                self.parts
                    .store
                    .replace_in_body(&id, shown, &now)
                    .await
                    .map_err(|e| SlackError::Unreachable(e.to_string()))?;
            }
        }
        for file in files.iter().filter(|f| is_audio(f)) {
            let url = file["url_private_download"]
                .as_str()
                .or_else(|| file["url_private"].as_str())
                .unwrap_or("undefined");
            let seen = SeenNote {
                id: format!("slack:{}", text_of(&file["id"])),
                source: VoiceSource::Slack,
                reference: format!("{}|{url}", l.short),
                name: file["name"].as_str().map(str::to_owned),
                seconds: file["duration_ms"]
                    .as_f64()
                    .filter(|ms| *ms != 0.0)
                    .map(|ms| ms / 1000.0),
                chat: Some(chat_id(&l.team, &who)),
                chat_name: Some(who.clone()),
                sender: Some(if from_me {
                    OWNER.to_owned()
                } else {
                    sender.clone()
                }),
                msg_at: Some(at),
                msg: Some(id.clone()),
            };
            self.parts
                .voice
                .seen(&seen)
                .await
                .map_err(|e| SlackError::Unreachable(e.to_string()))?;
        }
        Ok(())
    }

    /// The conversations with messages in the last `days`, the newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn chats(&self, days: i64) -> Result<Vec<SlackChat>, DbError> {
        let since = self.ctx.now() - SignedDuration::from_hours(24 * days);
        Ok(self
            .parts
            .store
            .conversations(since)
            .await?
            .into_iter()
            .map(|c| SlackChat {
                id: chat_id(&c.workspace, &c.who),
                source: format!("Slack {}", c.workspace),
                last_at: c.last_at.to_string(),
                messages: c.messages,
                name: c.who,
            })
            .collect())
    }

    /// A chat's newest `limit` messages, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn messages(&self, id: &str, limit: u64) -> Result<Vec<SlackLine>, DbError> {
        let Some((w, who)) = parse_chat(id) else {
            return Ok(Vec::new());
        };
        Ok(self
            .parts
            .store
            .messages(&w, &who, limit)
            .await?
            .into_iter()
            .map(|m| SlackLine {
                at: m.at,
                sender: if m.from_me {
                    OWNER.to_owned()
                } else {
                    m.sender.unwrap_or_else(|| who.clone())
                },
                from_me: m.from_me,
                text: m.body,
            })
            .collect())
    }

    /// `messages_read` on a Slack chat: in a connected workspace the conversation is read
    /// fresh from Slack first (anything missed), then the kept messages.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn read(&self, id: &str, limit: u64) -> Result<SlackRead, DbError> {
        let (w, who) = parse_chat(id).unwrap_or_default();
        let live = self.live(&w);
        let mut clips = Vec::new();
        if let Some(l) = &live
            && let Some(channel) = self
                .parts
                .store
                .channels(&w, &who)
                .await?
                .into_iter()
                .next()
        {
            let history = self
                .parts
                .api
                .call(
                    &l.user,
                    "conversations.history",
                    &[("channel", channel.clone()), ("limit", limit.to_string())],
                )
                .await
                .map_or_else(
                    |_| Vec::new(),
                    |answer| {
                        answer.json["messages"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                    },
                );
            for m in history {
                if let Err(error) = self.keep(l, &channel, &m).await {
                    tracing::debug!(%error, "Slack: a message read fresh wasn't kept");
                }
                for file in m["files"].as_array().cloned().unwrap_or_default() {
                    if truthy(&file["id"]) {
                        clips.push(format!("slack:{}", text_of(&file["id"])));
                    }
                }
            }
        }
        self.parts.voice.settle(READ_SETTLE, Some(&clips)).await;
        let note = if live.is_some() {
            "The whole conversation, the owner's own messages included.".to_owned()
        } else {
            let saved = self.saved(&w).await;
            format!(
                "What Slack notified on the owner's Mac (DMs, mentions, the owner's threads) and \
                 the owner's own messages{}.",
                match saved {
                    Some(saved) => format!(", as of Slack's last save at {saved}"),
                    None => " (none read from the Mac yet)".to_owned(),
                }
            )
        };
        let messages = self
            .messages(id, limit)
            .await?
            .into_iter()
            .map(|m| format!("[{}] {}: {}", m.at, m.sender, m.text))
            .collect();
        Ok(SlackRead {
            conversation: format!("{who} (Slack {w})"),
            note,
            messages,
        })
    }

    /// Whether the owner's own messages in chat `id`'s workspace are read whole (it is
    /// connected), and if not, when the Slack app on the Mac last saved them.
    pub async fn coverage(&self, id: &str) -> (bool, Option<String>) {
        let (w, _) = parse_chat(id).unwrap_or_default();
        if self.live(&w).is_some() {
            return (true, None);
        }
        (false, self.saved(&w).await)
    }

    /// The messages in the last `days` holding `words`, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn search(
        &self,
        words: &str,
        days: i64,
        limit: u64,
    ) -> Result<Vec<SlackHit>, DbError> {
        let since = self.ctx.now() - SignedDuration::from_hours(24 * days);
        Ok(self
            .parts
            .store
            .search(words, since, limit)
            .await?
            .into_iter()
            .map(|m| SlackHit {
                chat: chat_id(&m.workspace, &m.who),
                source: format!("Slack {}", m.workspace),
                at: m.at.to_string(),
                sender: if m.from_me {
                    OWNER.to_owned()
                } else {
                    m.sender.unwrap_or_else(|| m.who.clone())
                },
                text: m.body,
            })
            .collect())
    }

    /// `slack_file`: a file a message shows as `[file: …; slack_file F…]`, fetched with the
    /// owner's token (`files:read`) into the files folder; a small picture comes back to look
    /// at too.
    ///
    /// # Errors
    ///
    /// [`SlackFileError`].
    pub async fn file(&self, id: &str) -> Result<(SlackFile, Option<SlackImage>), SlackFileError> {
        if !is_file_id(id) {
            return Err(SlackFileError::Refused(format!(
                "{id} isn't a Slack file id (F…, from \"[file: …; slack_file F…]\")"
            )));
        }
        let live: Vec<Live> = self.memory().live.values().cloned().collect();
        for l in live {
            let Ok(answer) = self
                .parts
                .api
                .call(&l.user, "files.info", &[("file", id.to_owned())])
                .await
            else {
                continue;
            };
            let f = &answer.json["file"];
            if !truthy(f) {
                continue;
            }
            let name = f["name"].as_str().map(str::to_owned);
            let Some(url) = f["url_private_download"]
                .as_str()
                .or_else(|| f["url_private"].as_str())
            else {
                return Err(SlackFileError::Refused(format!(
                    "Slack has no download for {} ({})",
                    name.as_deref().unwrap_or(id),
                    f["mode"].as_str().unwrap_or("no file")
                )));
            };
            let size = f["size"].as_f64().unwrap_or(0.0);
            #[expect(
                clippy::cast_precision_loss,
                reason = "the limit is a small power of two, exact as a float"
            )]
            if size > FILE_LIMIT as f64 {
                return Err(SlackFileError::Refused(format!(
                    "{} is {} MB: too big to fetch here",
                    text_of(&f["name"]),
                    (size / f64::from(1 << 20)).round()
                )));
            }
            let got = self
                .parts
                .api
                .download(url, &l.user)
                .await
                .map_err(|error| {
                    SlackFileError::Refused(format!("Slack didn't answer ({error})"))
                })?;
            if !(200..300).contains(&got.status) || got.content_type.contains("text/html") {
                return Err(SlackFileError::Refused(format!(
                    "Slack wouldn't hand over {} ({}; files:read?)",
                    text_of(&f["name"]),
                    got.status
                )));
            }
            tokio::fs::create_dir_all(&self.parts.files)
                .await
                .map_err(SlackFileError::Save)?;
            let safe = safe_name(name.as_deref().unwrap_or("file"));
            let path = self.parts.files.join(format!("{id}-{safe}"));
            tokio::fs::write(&path, &got.body)
                .await
                .map_err(SlackFileError::Save)?;
            let mimetype = f["mimetype"].as_str().map(str::to_owned);
            let picture = mimetype.as_deref().is_some_and(|t| {
                ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&t)
            }) && got.body.len() < IMAGE_LIMIT;
            let image = picture.then(|| SlackImage {
                data: base64::engine::general_purpose::STANDARD.encode(&got.body),
                mime_type: mimetype.clone().unwrap_or_default(),
            });
            let file = SlackFile {
                workspace: l.team.clone(),
                name,
                kind: f["pretty_type"]
                    .as_str()
                    .or_else(|| f["filetype"].as_str())
                    .map(str::to_owned),
                mimetype,
                bytes: got.body.len(),
                path: path.display().to_string(),
                seconds: f["duration_ms"]
                    .as_f64()
                    .filter(|ms| *ms != 0.0)
                    .map(|ms| (ms / 1000.0).round()),
            };
            return Ok((file, image));
        }
        Err(SlackFileError::Refused(format!(
            "None of the owner's Slack workspaces read here has file {id}"
        )))
    }

    // ── Sending as the owner ──

    /// `slack_send`: one message as the owner, to `chat`, on `task`, asked for by `room`'s
    /// assistant. The first call posts nothing: the owner is asked (Send it / Cancel) with the
    /// chat and the exact text. Only the owner's OK for that exact text posts it, once. One
    /// call at a time per task. The answer is for the assistant.
    pub async fn send(
        &self,
        chat: &str,
        text: &str,
        task: Option<TaskId>,
        room: Option<RoomId>,
    ) -> String {
        let key = format!(
            "slack:{}",
            task.map_or_else(|| "none".to_owned(), |t| t.to_string())
        );
        let lock = self.ctx.lock(&key);
        let _alone = lock.lock().await;
        self.send_held(chat, text, task, room).await
    }

    /// [`Self::send`] for a caller that holds the task's lock already.
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
                tracing::error!(%error, "slack_send failed");
                format!("Slack: {error}")
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
                "slack_send needs task_id: the OK to send is asked on that task.".to_owned(),
            );
        };
        let text = same(text);
        if text.is_empty() {
            return Ok("Nothing to send.".to_owned());
        }
        if let Some(bad) = slack_formatting(&text) {
            return Ok(format!(
                "Not plain text ({bad}): Slack would show it as formatting. Write it the way the \
                 owner types, plain text, and ask again."
            ));
        }
        let Some((w, who)) = parse_chat(chat) else {
            return Ok(format!(
                "{chat} isn't a Slack chat id. Use one from messages_chats or messages_search \
                 (slack:Workspace:Name)."
            ));
        };
        let ctx = &self.ctx;
        if let Some(sent) = DRAFTS.sent(ctx, task).await?
            && sent.chat == chat
            && sent.text == text
        {
            return Ok(format!(
                "Already sent to {who} at {}. Not sent again: one message per OK, never a resend.",
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
                format!("The owner cancelled the Slack message to {}", p.to),
            )
            .await;
            return Ok("The owner said Cancel: not sent. Don't send it.".to_owned());
        }
        let asked = pending
            .as_ref()
            .filter(|p| p.chat == chat && p.text == text);
        let go = match asked {
            Some(p) => decided == Some(Decision::Send) || ctx.oks.tapped(task, p.asked()).await,
            None => false,
        };
        let mut ok = if go || ctx.card_oks_only {
            None
        } else {
            let since = asked.filter(|_| decided.is_none()).map(Pending::asked);
            ctx.oks.chat_ok(room, &text, &who, false, since).await
        };
        let pre = if go || ok.is_some() || ctx.card_oks_only {
            None
        } else {
            ctx.oks.text_ok(task, &text).await
        };
        let standing = |channel: &str| {
            let chats = vec![chat.to_owned(), format!("slack:{w}:{channel}")];
            let dm = channel.starts_with('D');
            async move { ctx.oks.standing_ok(&chats, dm).await }
        };
        let mut free = match asked {
            Some(p) if !go && ok.is_none() && pre.is_none() => {
                standing(p.channel.as_deref().unwrap_or_default()).await
            }
            _ => None,
        };
        if let Some(p) = asked
            && !go
            && ok.is_none()
            && pre.is_none()
            && free.is_none()
        {
            return Ok(format!(
                "Still waiting for the owner to say Send it or Cancel (asked {}). Stop for now: \
                 the owner's Send it posts it by itself, and you'll hear.",
                p.asked()
            ));
        }

        let sender = match self.sender(&w).await {
            Ok(sender) => sender,
            Err(said) => return Ok(said),
        };
        let channel = match asked.and_then(|p| p.channel.clone()) {
            Some(channel) => channel,
            None => match self.channel(&sender.token, &w, &who).await {
                Ok(channel) => channel,
                Err(said) => return Ok(said),
            },
        };
        if let Some(chat_ok) = &ok
            && !ctx.oks.spend(chat_ok).await
        {
            ok = None;
        }
        if !go && ok.is_none() && pre.is_none() && free.is_none() {
            free = standing(&channel).await;
        }
        if let Some(chat_ok) = &ok {
            ctx.note(task, chat_ok.line.clone()).await;
        }
        if let Some(pre) = &pre {
            ctx.note(
                task,
                format!(
                    "Slack to {who}: the exact text the owner OK'd ({}), no new card",
                    pre.how
                ),
            )
            .await;
        }
        if let Some(free) = &free {
            ctx.note(task, format!("Slack to {who}: {free}, no card"))
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
            let said = self
                .ask(task, chat, &text, &who, &sender.team, channel, room)
                .await?;
            return Ok(match stale {
                Some(stale) => format!("{said} (Asked again: {}.)", stale.said()),
                None => said,
            });
        };

        if let Some(p) = asked {
            if !DRAFTS.take_pending(ctx, task, p).await? {
                return Ok(
                    "Not sent: this message was asked again or changed since. Call slack_send \
                     again if it is still wanted."
                        .to_owned(),
                );
            }
            if how != Go::Card {
                ctx.oks.close_card(task, &p.question).await;
            }
        }
        let post = [
            ("channel", channel.clone()),
            ("text", rules::escape(&text)),
            ("mrkdwn", "false".to_owned()),
        ];
        if let Err(error) = self
            .parts
            .api
            .call(&sender.token, "chat.postMessage", &post)
            .await
        {
            let error = error.to_string();
            let why = if error.contains("missing_scope") {
                format!(
                    "the owner's token has no chat:write scope; add it to the Slack app, \
                     reinstall it and save the new token (secret_link {})",
                    user_token_secret(&w.to_lowercase())
                )
            } else {
                format!(
                    "Slack said {}",
                    error.trim_start_matches("chat.postMessage: ")
                )
            };
            ctx.note(
                task,
                format!("Slack message to {who} ({chat}) not sent: {why}"),
            )
            .await;
            ctx.oks.sent(task, &text, "Slack (failed)").await;
            return Ok(format!(
                "Not sent: {why}. The owner's OK is spent; if they still want it, ask again (call \
                 slack_send again)."
            ));
        }
        let now = ctx.now();
        DRAFTS
            .set_sent(
                ctx,
                task,
                Sent {
                    chat: chat.to_owned(),
                    text: text.clone(),
                    at: now.as_millisecond(),
                },
            )
            .await?;
        ctx.oks.sent(task, &text, "Slack").await;
        ctx.note(
            task,
            format!(
                "Sent{} on Slack to {who} ({chat}) at {now}, as the owner: «{text}»",
                how.said()
            ),
        )
        .await;
        Ok(format!(
            "Sent to {who} on Slack ({}), as the owner. Logged on task {task}.",
            sender.team
        ))
    }

    /// Keeps `draft` as `task`'s send card, for the guard rails' send-on-tap: the same checks
    /// as `slack_send`'s (plain text, the owner's token for that workspace with `chat:write`,
    /// the one conversation by that name). The owner's tap on its send answer posts it
    /// ([`Self::answer_card`]). `Err` is why not, in words.
    ///
    /// # Errors
    ///
    /// Why the draft can't be a card, in words.
    pub async fn prepare(&self, task: TaskId, draft: &Draft) -> Result<Prepared, String> {
        let lock = self.ctx.lock(&format!("slack:{task}"));
        let _alone = lock.lock().await;
        let text = same(&draft.text);
        if let Some(bad) = slack_formatting(&text) {
            return Err(format!(
                "Not plain text ({bad}): Slack would show it as formatting. Write the draft the way \
                 the owner types, plain text, and ask again."
            ));
        }
        let Some((w, who)) = parse_chat(&draft.chat) else {
            return Err(format!(
                "{} isn't a Slack chat id (slack:Workspace:Name).",
                draft.chat
            ));
        };
        let sender = self.sender(&w).await?;
        let channel = self.channel(&sender.token, &w, &who).await?;
        let pending = Pending {
            task,
            chat: draft.chat.clone(),
            text: text.clone(),
            to: who.clone(),
            question: draft.question.clone(),
            ask: Some(draft.ask.get()),
            at: self.ctx.now().as_millisecond(),
            room: guard::room_id(draft.room),
            channel: Some(channel.clone()),
            arg: None,
        };
        DRAFTS
            .set_pending(&self.ctx, task, pending)
            .await
            .map_err(|e| e.to_string())?;
        let chats = vec![draft.chat.clone(), format!("slack:{w}:{channel}")];
        let free = self
            .ctx
            .oks
            .standing_ok(&chats, channel.starts_with('D'))
            .await;
        if free.is_none() {
            self.ctx
                .note(
                    task,
                    format!(
                        "Slack message to {who} ({}) waiting for the owner's {}: «{text}»",
                        draft.chat, draft.send
                    ),
                )
                .await;
        }
        Ok(Prepared {
            line: format!("{who} on Slack ({}, {})", sender.team, draft.chat),
            to: who,
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
            .map(|p| Card::of(&p, "Slack", "slack_send")))
    }

    /// The owner's tap on `card` (asked at its time, with its text): `choice` is the answer
    /// tapped. Its message goes out there and then through [`Self::send`]'s own guard (the
    /// guard rails record the tap first), unless it was asked again or changed since. The
    /// whole tap runs under the task's lock, so no `slack_send` call on the task comes between
    /// reading the card and answering it.
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
        let lock = self.ctx.lock(&format!("slack:{task}"));
        let _alone = lock.lock().await;
        let pending = DRAFTS.pending(&self.ctx, task).await?;
        let gone = pending.as_ref().is_none_or(|p| !card.shows(p));
        let said = match &pending {
            Some(p) if !gone => {
                self.send_held(&p.chat, &p.text, Some(task), guard::room_of(p))
                    .await
            }
            _ => String::new(),
        };
        if let Some(sent) = DRAFTS.sent(&self.ctx, task).await?
            && sent.text == card.text
            && sent.when() >= card.at
        {
            return Ok(CardAnswer::Sent(sent.when()));
        }
        if choice == CANCEL && (gone || said.starts_with("The owner said Cancel")) {
            return Ok(CardAnswer::Cancelled);
        }
        Ok(CardAnswer::Failed(if gone {
            "this message was asked again or changed since; nothing was sent.".to_owned()
        } else {
            let why = said.trim_start_matches("Not sent: ");
            why.split_once(" The owner's OK is spent")
                .map_or(why, |(why, _)| why)
                .to_owned()
        }))
    }

    /// Asks the owner to OK the message, and keeps it waiting.
    #[expect(
        clippy::too_many_arguments,
        reason = "the card's parts, each named where it is asked"
    )]
    async fn ask(
        &self,
        task: TaskId,
        chat: &str,
        text: &str,
        who: &str,
        team: &str,
        channel: String,
        room: Option<RoomId>,
    ) -> Result<String, DbError> {
        let ctx = &self.ctx;
        let at = ctx.now().as_millisecond();
        let question = format!("Send this Slack message to {who}?");
        let ask = Ask {
            question: question.clone(),
            options: vec![SEND_IT.to_owned(), CANCEL.to_owned()],
            title: first_chars(&format!("Send Slack to {who}"), 90),
            brief: vec![
                format!("To: {who} (Slack {team}, {chat})"),
                "From: the owner, as themselves".to_owned(),
                format!("Message: {text}"),
            ],
            approve: true,
        };
        let ask = match ctx.ask(task, ask).await {
            Ok(ask) => ask,
            Err(error) => {
                return Ok(format!(
                    "Not sent, and the owner couldn't be asked: {error}."
                ));
            }
        };
        DRAFTS
            .set_pending(
                ctx,
                task,
                Pending {
                    task,
                    chat: chat.to_owned(),
                    text: text.to_owned(),
                    to: who.to_owned(),
                    question,
                    ask: Some(ask.get()),
                    at,
                    room: guard::room_id(room),
                    channel: Some(channel),
                    arg: None,
                },
            )
            .await?;
        ctx.note(
            task,
            format!("Slack message to {who} ({chat}) waiting for the owner's Send it: «{text}»"),
        )
        .await;
        let more = if text.chars().count() > 200 {
            "…"
        } else {
            ""
        };
        ctx.push(
            format!("Slack to {who}?"),
            &format!(
                "{}{more} · Send it or Cancel in the app",
                first_chars(text, 200)
            ),
        );
        Ok(format!(
            "Not sent. The owner was asked (Send it / Cancel) with the chat ({who}, Slack {team}) \
             and the exact text. Stop; their Send it posts it by itself and you'll hear how it went \
             (don't call slack_send for it again). Only their go in the chat needs you to call it \
             again, with the same chat and text."
        ))
    }

    /// The owner's token for workspace `w`, checked: the right workspace, allowed to post.
    async fn sender(&self, w: &str) -> Result<Sender, String> {
        let live = self.live(w);
        let short = live
            .as_ref()
            .map_or_else(|| w.to_lowercase(), |l| l.short.clone());
        let name = user_token_secret(&short);
        let token = self.ctx.secrets.read(&name).await;
        if token.is_empty() {
            return Err(format!(
                "No Slack token for {w}: it is read only from the Mac's notifications here, and \
                 can't be posted to."
            ));
        }
        let me = match self.parts.api.call(&token, "auth.test", &[]).await {
            Ok(me) => me,
            Err(SlackError::Refused { error, .. }) => {
                return Err(format!(
                    "Slack refused the owner's {w} token ({error}). The Slack app may need \
                     reinstalling, and its new token saving (secret_link {name})."
                ));
            }
            Err(error) => return Err(format!("Slack didn't answer ({error}). Try again later.")),
        };
        let team = text_of(&me.json["team"]);
        if team.to_lowercase() != w.to_lowercase() {
            return Err(format!(
                "The {name} token is for {team}, not {w}: not sent."
            ));
        }
        if let Some(scopes) = &me.scopes
            && !scopes
                .split(',')
                .any(|scope| js_trim(scope) == "chat:write")
        {
            return Err(format!(
                "Can't post in {w}: the owner's Slack token has no chat:write scope (it has: \
                 {scopes}). Add chat:write to the Slack app's user scopes, reinstall it to {w}, and \
                 save the new user token (secret_link {name}). Nothing was asked or sent."
            ));
        }
        if let Some(l) = live
            && let Some(kept) = self.memory().live.get_mut(&l.team.to_lowercase())
        {
            kept.user.clone_from(&token);
        }
        Ok(Sender { token, team })
    }

    /// The conversation behind a chat: the channel id its messages came with, else by name (a
    /// channel the owner is in, a person the owner has a DM with). More than one: never a
    /// guess.
    async fn channel(&self, token: &str, w: &str, who: &str) -> Result<String, String> {
        let known = self
            .parts
            .store
            .channels(w, who)
            .await
            .map_err(|e| format!("Slack: {e}"))?;
        match known.as_slice() {
            [one] => return Ok(one.clone()),
            [] => {}
            many => {
                return Err(format!(
                    "More than one Slack conversation is called \"{who}\" in {w} ({}): ask the \
                     owner which; never guess.",
                    many.join(", ")
                ));
            }
        }
        let unreachable =
            |error: SlackError| format!("Slack didn't answer ({error}). Try again later.");
        if who.starts_with('#') {
            let channels = self
                .pages(
                    token,
                    "conversations.list",
                    "public_channel,private_channel",
                )
                .await
                .map_err(unreachable)?;
            let hits: Vec<&Value> = channels
                .iter()
                .filter(|c| format!("#{}", text_of(&c["name"])) == who && truthy(&c["is_member"]))
                .collect();
            return match hits.as_slice() {
                [one] => Ok(text_of(&one["id"])),
                [] => Err(format!(
                    "No channel the owner is in named {who} in {w}: ask the owner; never guess."
                )),
                _ => Err(format!(
                    "More than one channel named {who} in {w}: ask the owner; never guess."
                )),
            };
        }
        if who.contains(',') {
            return Err(format!(
                "No group DM \"{who}\" with a known id in {w}: ask the owner to post there once, or \
                 pick another chat."
            ));
        }
        let people = self
            .pages(token, "users.list", "")
            .await
            .map_err(unreachable)?;
        let lower = who.to_lowercase();
        let named = |u: &Value| {
            [
                &u["profile"]["display_name"],
                &u["real_name"],
                &u["profile"]["real_name"],
                &u["name"],
            ]
            .iter()
            .any(|n| n.as_str().is_some_and(|n| n.to_lowercase() == lower))
        };
        let matches: Vec<&Value> = people
            .iter()
            .filter(|u| !truthy(&u["deleted"]) && named(u))
            .collect();
        let [person] = matches.as_slice() else {
            let many = if matches.is_empty() {
                "No one".to_owned()
            } else {
                format!("{} people", matches.len())
            };
            return Err(format!(
                "{many} called \"{who}\" in {w}'s Slack: ask the owner which; never guess."
            ));
        };
        let ims = self
            .pages(token, "conversations.list", "im")
            .await
            .map_err(unreachable)?;
        ims.iter()
            .find(|c| c["user"].as_str().is_some() && c["user"] == person["id"])
            .map(|im| text_of(&im["id"]))
            .ok_or_else(|| {
                format!("The owner has no DM with {who} in {w} yet: they should start it.")
            })
    }

    /// Up to 10 pages of a list (`conversations.list` of `types`, or `users.list`), following
    /// Slack's cursor.
    async fn pages(
        &self,
        token: &str,
        method: &str,
        types: &str,
    ) -> Result<Vec<Value>, SlackError> {
        let field = if method == "users.list" {
            "members"
        } else {
            "channels"
        };
        let mut all = Vec::new();
        let mut cursor = String::new();
        for _ in 0..10 {
            let mut args = vec![("limit", "1000".to_owned())];
            if !types.is_empty() {
                args.push(("types", types.to_owned()));
                args.push(("exclude_archived", "true".to_owned()));
            }
            if !cursor.is_empty() {
                args.push(("cursor", cursor.clone()));
            }
            let page = self.parts.api.call(token, method, &args).await?.json;
            all.extend(page[field].as_array().cloned().unwrap_or_default());
            page["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or_default()
                .clone_into(&mut cursor);
            if cursor.is_empty() {
                break;
            }
        }
        Ok(all)
    }
}

/// The owner's checked token for a workspace.
struct Sender {
    token: String,
    team: String,
}

/// The user ids mentioned in a message (`<@U123>`), in order.
fn mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<@U") {
        let tail = &rest[at + 2..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(tail.len());
        if tail[end..].starts_with('>') && end > 1 {
            found.push(tail[..end].to_owned());
        }
        rest = &tail[end..];
    }
    found
}

/// How a message shows a file that isn't a voice clip.
fn file_shown(f: &Value) -> String {
    match f["mode"].as_str() {
        Some("tombstone") => "[file: deleted]".to_owned(),
        Some("hidden_by_limit") => "[file: hidden by Slack's plan limit]".to_owned(),
        _ => {
            let name = first_of(f, &["name", "title"]).unwrap_or_else(|| "unnamed".to_owned());
            let kind = first_of(f, &["pretty_type", "filetype", "mimetype"])
                .unwrap_or_else(|| "unknown type".to_owned());
            let fetch = if truthy(&f["id"]) {
                format!("; slack_file {}", text_of(&f["id"]))
            } else {
                String::new()
            };
            format!("[file: {name}, {kind}{fetch}]")
        }
    }
}

/// A file name safe on disk: runs of anything but word characters, `.` and `-` as `_`, at
/// most its last 120 characters.
fn safe_name(name: &str) -> String {
    let mut safe = String::new();
    let mut run = false;
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
            safe.push(c);
            run = false;
        } else if !run {
            safe.push('_');
            run = true;
        }
    }
    let count = safe.chars().count();
    safe.chars().skip(count.saturating_sub(120)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_found_in_order() {
        assert_eq!(
            mentions("hi <@U12> and <@U3_x>, not <@W1> or <@U9"),
            ["U12", "U3_x"]
        );
    }

    #[test]
    fn files_show_their_name_kind_and_how_to_fetch_them() {
        assert_eq!(
            file_shown(&json!({ "id": "F1", "name": "plan.pdf", "pretty_type": "PDF" })),
            "[file: plan.pdf, PDF; slack_file F1]"
        );
        assert_eq!(
            file_shown(&json!({ "mode": "tombstone" })),
            "[file: deleted]"
        );
        assert_eq!(file_shown(&json!({})), "[file: unnamed, unknown type]");
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(safe_name("my plan (v2).pdf"), "my_plan_v2_.pdf");
        assert_eq!(safe_name(&"a".repeat(130)).len(), 120);
    }
}
