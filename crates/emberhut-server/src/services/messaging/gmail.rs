//! The owner's email, read only: each Gmail account the owner signed in, through the server's
//! own Google OAuth client (scope `gmail.readonly`).
//!
//! - Secrets: `gmail-client-id` and `gmail-client-secret` (the client), and
//!   `gmail-<account>-refresh` (each account's refresh token, saved by the callback: no token
//!   passes through a chat, a log or the assistant).
//! - Signing in: `gmail_connect` gives a one-time link (`/gmail/connect/<token>`, a day) that
//!   sends the owner's browser to Google's consent screen; Google sends it back to
//!   `/gmail/callback`, which trades the code for the refresh token. The owner signs in and
//!   consents themselves.
//! - New mail: every 2 minutes, Gmail's history since the last look (no push, no public
//!   address). Each message is kept with when the server got it.
//! - A chat is one correspondent in one account: `email:<account>:<address>`. Promotions,
//!   Social and Forums, and mail that skipped the inbox, are kept for search but never wake
//!   the assistant.
//! - Reading and searching ask Gmail live (the whole mailbox), falling back to what is kept.
//!
//! Nothing here sends mail.
//!
//! [`GoogleApi`] is the seam: [`HttpGoogle`] is Google's, [`NoGoogleApi`] stands in.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_core::messaging::OWNER;
use emberhut_core::messaging::mail::{
    Address, CLIENT_ID_SECRET, CLIENT_SECRET_SECRET, MailMessage, SCOPE, account_of_secret,
    addresses, body_text, chat_id, html_text, is_account_name, is_loud, is_skipped, parse_chat,
    refresh_secret, source_of,
};
use emberhut_core::messaging::text::{first_chars, html_decode};
use emberhut_db::{DbError, MailStore, SettingKey};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::context::Context;
use super::mac::encode;
use super::web::{WebClient, WebRequest};

/// How long a call to Google may take.
const WAIT: Duration = Duration::from_secs(30);
/// How long a sign-in link works.
const LINK_FOR: SignedDuration = SignedDuration::from_hours(24);
/// What the first look reads.
const FIRST_LOOK: &str = "newer_than:7d -in:spam -in:trash -in:drafts";

/// Each account's state.
const ACCOUNTS: SettingKey<BTreeMap<String, MailAccount>> = SettingKey::new("gmail");
/// The sign-in links, by token.
const LINKS: SettingKey<BTreeMap<String, ConnectLink>> = SettingKey::new("gmailLinks");

/// An account the server reads: its address, Gmail's history id as of the last look, when
/// that was, and what went wrong.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAccount {
    /// Its address.
    #[serde(default)]
    pub email: Option<String>,
    /// Gmail's history id at the last look.
    #[serde(default)]
    pub history: Option<String>,
    /// When the last look was.
    #[serde(default)]
    pub at: Option<String>,
    /// What went wrong last.
    #[serde(default)]
    pub error: Option<String>,
}

/// A one-time sign-in link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectLink {
    /// The account it signs in.
    pub account: String,
    /// The address the owner is to sign in with.
    #[serde(default)]
    pub email: Option<String>,
    /// When it stops working, in milliseconds since the epoch.
    pub exp: i64,
}

/// An answer from Google: its status and JSON (`null` when it had none).
#[derive(Clone, Debug, PartialEq)]
pub struct GoogleAnswer {
    /// Its status.
    pub status: u16,
    /// Its JSON.
    pub json: Value,
}

impl GoogleAnswer {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Google's OAuth and Gmail APIs.
pub trait GoogleApi: fmt::Debug + Send + Sync {
    /// `POST`s `form` to the token endpoint.
    fn token<'a>(
        &'a self,
        form: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>>;

    /// `GET`s `users/me/<path>` with `access`.
    fn get<'a>(
        &'a self,
        access: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>>;

    /// The consent screen's address, without its query.
    fn consent_url(&self) -> String;
}

/// Google isn't set up.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoGoogleApi;

impl GoogleApi for NoGoogleApi {
    fn token<'a>(
        &'a self,
        _: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        Box::pin(async { Err("Google isn't set up on this server".to_owned()) })
    }

    fn get<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        Box::pin(async { Err("Google isn't set up on this server".to_owned()) })
    }

    fn consent_url(&self) -> String {
        "https://accounts.google.com/o/oauth2/v2/auth".to_owned()
    }
}

/// Google over HTTPS: its own addresses, or all three under one stand-in `base` (`<base>/token`,
/// `<base>/auth`, `<base>/gmail/v1/...`).
#[derive(Clone, Debug)]
pub struct HttpGoogle {
    web: WebClient,
    api: String,
    token: String,
    auth: String,
}

impl HttpGoogle {
    /// Google's own addresses, or `base`'s.
    #[must_use]
    pub fn new(web: WebClient, base: Option<&str>) -> Self {
        match base.map(|b| b.trim_end_matches('/')) {
            Some(base) => Self {
                web,
                api: base.to_owned(),
                token: format!("{base}/token"),
                auth: format!("{base}/auth"),
            },
            None => Self {
                web,
                api: "https://gmail.googleapis.com".to_owned(),
                token: "https://oauth2.googleapis.com/token".to_owned(),
                auth: "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
            },
        }
    }
}

impl GoogleApi for HttpGoogle {
    fn token<'a>(
        &'a self,
        form: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        Box::pin(async move {
            let fields: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let answer = self
                .web
                .send(WebRequest::form(self.token.as_str(), &fields, WAIT))
                .await
                .map_err(|e| e.to_string())?;
            Ok(GoogleAnswer {
                status: answer.status.as_u16(),
                json: answer.json(),
            })
        })
    }

    fn get<'a>(
        &'a self,
        access: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        Box::pin(async move {
            let url = format!("{}/gmail/v1/users/me/{path}", self.api);
            let answer = self
                .web
                .send(WebRequest::get(url, WAIT).bearer(access).limit(32 << 20))
                .await
                .map_err(|e| e.to_string())?;
            Ok(GoogleAnswer {
                status: answer.status.as_u16(),
                json: answer.json(),
            })
        })
    }

    fn consent_url(&self) -> String {
        self.auth.clone()
    }
}

/// `messages_chats`' email row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailChat {
    /// The chat id.
    pub id: String,
    /// The correspondent.
    pub name: String,
    /// `Email`, `Email <Account>`.
    pub source: String,
    /// When the newest was sent.
    pub last_at: String,
    /// How many.
    pub messages: u64,
}

/// `messages_search`' email hit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MailHit {
    /// The chat id.
    pub chat: String,
    /// The account's source name.
    pub source: String,
    /// When it was sent.
    pub at: String,
    /// Who wrote it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// Its text (Gmail's snippet).
    pub text: String,
}

/// `messages_read`' answer for an email chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MailRead {
    /// Who, in which account.
    pub conversation: String,
    /// Where the messages come from.
    pub note: String,
    /// `[at] sender: «subject» text`, oldest first.
    pub messages: Vec<String>,
}

/// What a sign-in page shows: its status and its HTML body, or a redirect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// A page.
    Html(u16, String),
    /// On to this address.
    Redirect(String),
}

/// What the Gmail service is made of besides the context.
#[derive(Clone, Debug)]
pub struct GmailParts {
    /// The mail.
    pub store: MailStore,
    /// Google.
    pub api: Arc<dyn GoogleApi>,
    /// Where Google sends the owner back (`<public url>/gmail/callback`).
    pub callback: String,
}

/// The owner's Gmail accounts.
#[derive(Clone, Debug)]
pub struct GmailService {
    ctx: Context,
    parts: GmailParts,
    access: Arc<Mutex<HashMap<String, (String, Timestamp)>>>,
    collecting: Arc<tokio::sync::Mutex<()>>,
}

/// Why a Gmail call failed: its words.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GmailError(String);

impl From<DbError> for GmailError {
    fn from(error: DbError) -> Self {
        Self(error.to_string())
    }
}

impl GmailService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(ctx: Context, parts: GmailParts) -> Self {
        Self {
            ctx,
            parts,
            access: Arc::default(),
            collecting: Arc::default(),
        }
    }

    /// The accounts signed in: a refresh token each, in name order.
    pub async fn accounts(&self) -> Vec<String> {
        let mut accounts: Vec<String> = self
            .ctx
            .secrets
            .names()
            .await
            .iter()
            .filter_map(|name| account_of_secret(name).map(str::to_owned))
            .collect();
        accounts.sort();
        accounts
    }

    async fn states(&self) -> BTreeMap<String, MailAccount> {
        self.ctx
            .settings
            .get_or(&ACCOUNTS, BTreeMap::new())
            .await
            .unwrap_or_default()
    }

    /// `account`'s state.
    pub async fn state(&self, account: &str) -> MailAccount {
        self.states().await.remove(account).unwrap_or_default()
    }

    async fn set_state(
        &self,
        account: &str,
        change: impl FnOnce(&mut MailAccount),
    ) -> Result<(), DbError> {
        let mut all = self.states().await;
        change(all.entry(account.to_owned()).or_default());
        self.ctx.settings.set(&ACCOUNTS, &all).await
    }

    /// An access token for `account`, from its refresh token (kept until a minute before it
    /// expires).
    async fn access_token(&self, account: &str) -> Result<String, GmailError> {
        let now = self.ctx.now();
        if let Some((token, exp)) = self.access().get(account)
            && *exp > now + SignedDuration::from_mins(1)
        {
            return Ok(token.clone());
        }
        let refresh = self.ctx.secrets.read(&refresh_secret(account)).await;
        if refresh.is_empty() {
            return Err(GmailError(format!(
                "{account} isn't signed in (gmail_connect)"
            )));
        }
        let form = [
            ("client_id", self.ctx.secrets.read(CLIENT_ID_SECRET).await),
            (
                "client_secret",
                self.ctx.secrets.read(CLIENT_SECRET_SECRET).await,
            ),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token".to_owned()),
        ];
        let answer = self.parts.api.token(&form).await.map_err(GmailError)?;
        let token = answer.json["access_token"]
            .as_str()
            .filter(|t| !t.is_empty());
        let Some(token) = token.filter(|_| answer.ok()) else {
            self.access().remove(account);
            let error = answer.json["error"].as_str().unwrap_or_default();
            return Err(GmailError(if error == "invalid_grant" {
                format!("Google no longer takes {account}'s sign-in: gmail_connect again")
            } else {
                format!("Google's token endpoint said {} {error}", answer.status)
                    .trim_end()
                    .to_owned()
            }));
        };
        self.remember(account, token, &answer.json);
        Ok(token.to_owned())
    }

    fn access(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, Timestamp)>> {
        self.access.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn remember(&self, account: &str, token: &str, json: &Value) {
        let seconds = json["expires_in"]
            .as_i64()
            .filter(|s| *s > 0)
            .unwrap_or(3600);
        let exp = self.ctx.now() + SignedDuration::from_secs(seconds);
        self.access()
            .insert(account.to_owned(), (token.to_owned(), exp));
    }

    /// `GET users/me/<path>`: its JSON, or none for a 404 (gone).
    async fn get(&self, account: &str, path: &str) -> Result<Option<Value>, GmailError> {
        let token = self.access_token(account).await?;
        let answer = self.parts.api.get(&token, path).await.map_err(GmailError)?;
        if answer.status == 404 {
            return Ok(None);
        }
        if answer.status == 401 {
            self.access().remove(account);
        }
        if !answer.ok() {
            let why = answer.json["error"]["message"].as_str().unwrap_or_default();
            return Err(GmailError(
                format!("Gmail said {} {}", answer.status, first_chars(why, 160))
                    .trim_end()
                    .to_owned(),
            ));
        }
        Ok(Some(answer.json))
    }

    /// A message as Gmail gives it, kept with `got`; false for drafts, spam and trash.
    async fn keep(&self, account: &str, m: &Value, got: Timestamp) -> Result<bool, DbError> {
        let labels: Vec<String> = m["labelIds"].as_array().map_or_else(Vec::new, |l| {
            l.iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect()
        });
        if is_skipped(&labels) {
            return Ok(false);
        }
        let me = self
            .state(account)
            .await
            .email
            .unwrap_or_default()
            .to_lowercase();
        let from = addresses(&header(m, "From").unwrap_or_default())
            .into_iter()
            .next()
            .unwrap_or_else(|| Address {
                name: String::new(),
                address: String::new(),
            });
        let from_me = labels.iter().any(|l| l == "SENT") || (!me.is_empty() && from.address == me);
        let other = if from_me {
            let to = header(m, "To").unwrap_or_default();
            let cc = header(m, "Cc").unwrap_or_default();
            addresses(&to)
                .into_iter()
                .chain(addresses(&cc))
                .find(|a| a.address != me)
                .unwrap_or_else(|| Address {
                    name: String::new(),
                    address: me.clone(),
                })
        } else {
            from.clone()
        };
        if other.address.is_empty() {
            return Ok(false);
        }
        let text = part(&m["payload"], "text/plain")
            .or_else(|| part(&m["payload"], "text/html").map(|h| html_text(&h)))
            .unwrap_or_else(|| html_decode(m["snippet"].as_str().unwrap_or_default()));
        let at = internal_date(m).unwrap_or(self.ctx.now());
        let sender = if from_me {
            OWNER.to_owned()
        } else if from.name.is_empty() {
            from.address.clone()
        } else {
            format!("{} <{}>", from.name, from.address)
        };
        let message = MailMessage {
            id: format!("{account}:{}", m["id"].as_str().unwrap_or_default()),
            account: account.to_owned(),
            chat: chat_id(account, &other.address),
            who: if other.name.is_empty() {
                other.address.clone()
            } else {
                other.name.clone()
            },
            sender,
            from_me,
            subject: header(m, "Subject").unwrap_or_default(),
            body: first_chars(&body_text(&text), 8000),
            at,
            got: got.max(at),
            loud: is_loud(&labels),
            thread: m["threadId"].as_str().map(str::to_owned),
        };
        self.parts.store.add(&message).await?;
        Ok(true)
    }

    /// Fetches these messages (`format=full`), the ones not kept yet, into the table, each
    /// with the `got` its JSON gives; gone ones skipped. How many were kept.
    async fn fetch(
        &self,
        account: &str,
        ids: &[String],
        got: impl Fn(&Value) -> Timestamp,
    ) -> Result<usize, GmailError> {
        let mut kept = 0;
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id.clone()) || self.parts.store.has(&format!("{account}:{id}")).await? {
                continue;
            }
            if let Some(m) = self
                .get(account, &format!("messages/{id}?format=full"))
                .await?
                && self.keep(account, &m, got(&m)).await?
            {
                kept += 1;
            }
        }
        Ok(kept)
    }

    /// New mail in every account signed in (one look at a time: a second waits for the
    /// first).
    pub async fn collect(&self) {
        let _alone = self.collecting.lock().await;
        for account in self.accounts().await {
            let result = self.collect_one(&account).await;
            let was = self.state(&account).await.error;
            let error = result.err().map(|e| e.0);
            if error != was {
                if let Some(error) = &error {
                    self.ctx
                        .activity(format!("Email ({account}): {}", first_chars(error, 200)))
                        .await;
                }
                if let Err(e) = self
                    .set_state(&account, |s| s.error.clone_from(&error))
                    .await
                {
                    tracing::warn!(%e, "an account's state wasn't kept");
                }
            }
        }
    }

    async fn collect_one(&self, account: &str) -> Result<(), GmailError> {
        let state = self.state(account).await;
        let (Some(start), Some(_)) = (state.history.clone(), state.email.clone()) else {
            return self.first_look(account).await;
        };
        let mut ids = Vec::new();
        let mut history = start.clone();
        let mut page: Option<String> = None;
        loop {
            let token = page
                .as_ref()
                .map_or_else(String::new, |p| format!("&pageToken={p}"));
            let path = format!(
                "history?startHistoryId={start}&historyTypes=messageAdded&maxResults=500{token}"
            );
            let Some(j) = self.get(account, &path).await? else {
                return self.first_look(account).await;
            };
            for h in j["history"].as_array().cloned().unwrap_or_default() {
                for added in h["messagesAdded"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = added["message"]["id"].as_str() {
                        ids.push(id.to_owned());
                    }
                }
            }
            if let Some(next) = j["historyId"].as_str() {
                next.clone_into(&mut history);
            }
            page = j["nextPageToken"].as_str().map(str::to_owned);
            if page.is_none() {
                break;
            }
        }
        let now = self.ctx.now();
        let kept = self.fetch(account, &ids, |_| now).await?;
        self.set_state(account, |s| {
            s.history = Some(history);
            s.at = Some(now.to_string());
        })
        .await?;
        if kept > 0 {
            self.ctx
                .activity(format!("Email ({account}): {kept} new"))
                .await;
        }
        Ok(())
    }

    /// Signed in, or Gmail lost its place: who the owner is there and where its history is
    /// now, then the last week's mail for reading and search. What came into the inbox in
    /// the last 12 hours (at most 10) is new: it gets the next look's news.
    async fn first_look(&self, account: &str) -> Result<(), GmailError> {
        let profile = self
            .get(account, "profile")
            .await?
            .ok_or_else(|| GmailError("Gmail has no profile for this sign-in".to_owned()))?;
        let history = profile["historyId"].as_str().map(str::to_owned);
        let email = profile["emailAddress"].as_str().map(str::to_owned);
        self.set_state(account, |s| {
            s.email.clone_from(&email);
            s.history = None;
        })
        .await?;
        let listed = self
            .get(
                account,
                &format!("messages?maxResults=100&q={}", encode(FIRST_LOOK)),
            )
            .await?
            .unwrap_or(Value::Null);
        let ids = message_ids(&listed);
        let now = self.ctx.now();
        let fresh = now - SignedDuration::from_hours(12);
        let shown = std::sync::atomic::AtomicUsize::new(0);
        let kept = self
            .fetch(account, &ids, |m| {
                let at = internal_date(m).unwrap_or(Timestamp::UNIX_EPOCH);
                let inbox = m["labelIds"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|l| l == "INBOX"));
                if at > fresh && inbox && shown.fetch_add(1, Ordering::SeqCst) < 10 {
                    now
                } else {
                    at
                }
            })
            .await?;
        self.set_state(account, |s| {
            s.history = history;
            s.at = Some(now.to_string());
        })
        .await?;
        self.ctx
            .activity(format!(
                "Email ({account}, {}): read the last week ({kept} messages); new mail comes in \
                 every 2 minutes",
                email.as_deref().unwrap_or("?")
            ))
            .await;
        Ok(())
    }

    /// The correspondents with mail in the last `days`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn chats(&self, days: i64) -> Result<Vec<MailChat>, DbError> {
        let since = self.ctx.now() - SignedDuration::from_hours(24 * days);
        Ok(self
            .parts
            .store
            .conversations(since)
            .await?
            .into_iter()
            .map(|c| MailChat {
                id: c.chat,
                name: c.who,
                source: source_of(&c.account),
                last_at: c.last_at.to_string(),
                messages: c.messages,
            })
            .collect())
    }

    /// Each account, for `messages_chats`' header: its source name, its address, and what's
    /// wrong with it.
    pub async fn summary(&self) -> Option<String> {
        let accounts = self.accounts().await;
        if accounts.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        for account in &accounts {
            let state = self.state(account).await;
            let error = state
                .error
                .map_or_else(String::new, |e| format!(", not read now: {e}"));
            parts.push(format!(
                "{}: {}{error}",
                source_of(account),
                state.email.unwrap_or_else(|| account.clone())
            ));
        }
        Some(parts.join(", "))
    }

    /// The correspondents with loud mail the server got after `since`, each with its source
    /// (`Email Work, me@example.com`).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn changes(
        &self,
        since: Timestamp,
    ) -> Result<Vec<(emberhut_core::messaging::mail::MailConversation, String)>, DbError> {
        let mut changes = Vec::new();
        for change in self.parts.store.changes(since).await? {
            let state = self.state(&change.account).await;
            let source = format!(
                "{}, {}",
                source_of(&change.account),
                state.email.unwrap_or_else(|| change.account.clone())
            );
            changes.push((change, source));
        }
        Ok(changes)
    }

    /// `chat`'s newest `limit` messages the server got after `since`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn new_since(
        &self,
        chat: &str,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<MailMessage>, DbError> {
        self.parts.store.new_since(chat, since, limit).await
    }

    /// A chat's newest `limit` messages, oldest first, each cut to `chars`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn messages(&self, chat: &str, limit: u64) -> Result<Vec<MailMessage>, DbError> {
        self.parts.store.messages(chat, limit).await
    }

    /// `messages_read` on an email chat: that correspondent's mail, asked from Gmail first
    /// (the whole mailbox), then from the table.
    ///
    /// # Errors
    ///
    /// [`GmailError`] for a chat id that isn't an email one, or the database's.
    pub async fn read(&self, chat: &str, limit: u64) -> Result<MailRead, GmailError> {
        let (account, address) = parse_chat(chat).ok_or_else(|| {
            GmailError(format!(
                "{chat} isn't an email chat id (email:home:name@example.com)"
            ))
        })?;
        let note = if self.accounts().await.contains(&account) {
            let q = format!(
                "(from:{address} OR to:{address} OR cc:{address}) -in:spam -in:trash -in:drafts"
            );
            let fetched = async {
                let listed = self
                    .get(
                        &account,
                        &format!("messages?maxResults={}&q={}", limit.min(50), encode(&q)),
                    )
                    .await?
                    .unwrap_or(Value::Null);
                self.fetch(&account, &message_ids(&listed), |m| {
                    internal_date(m).unwrap_or(Timestamp::UNIX_EPOCH)
                })
                .await
            };
            fetched
                .await
                .err()
                .map(|e| format!("Gmail didn't answer ({e}): this is what was kept already."))
        } else {
            Some(format!(
                "{account} isn't signed in: this is what was kept already."
            ))
        };
        let messages = self.parts.store.messages(chat, limit).await?;
        let who = messages
            .iter()
            .rev()
            .find(|m| !m.from_me)
            .map_or_else(|| address.clone(), |m| m.sender.clone());
        let email = self.state(&account).await.email.unwrap_or_default();
        Ok(MailRead {
            conversation: format!("{who} ({}, {email})", source_of(&account)),
            note: note.unwrap_or_else(|| {
                "Every email with this address in the mailbox (quoted earlier mail cut), oldest \
                 first."
                    .to_owned()
            }),
            messages: messages
                .iter()
                .map(|m| format!("[{}] {}", m.at, line(m, 3000)))
                .collect(),
        })
    }

    /// `messages_search` in the mail: Gmail's own search on each account (its words work:
    /// `from:ann`), its snippet as the text; what is kept when Gmail doesn't answer.
    pub async fn search(&self, words: &str, days: i64, limit: u64) -> Vec<MailHit> {
        let mut hits = Vec::new();
        for account in self.accounts().await {
            match self.search_one(&account, words, days, limit).await {
                Ok(found) => hits.extend(found),
                Err(error) => {
                    tracing::debug!(%error, account, "Gmail's search failed; the kept mail instead");
                    let since = self.ctx.now() - SignedDuration::from_hours(24 * days);
                    let kept = self
                        .parts
                        .store
                        .search(&account, words, since, limit)
                        .await
                        .unwrap_or_default();
                    hits.extend(kept.into_iter().map(|m| MailHit {
                        chat: m.chat,
                        source: source_of(&account),
                        at: m.at.to_string(),
                        sender: m.sender,
                        subject: m.subject,
                        text: first_chars(&m.body, 300),
                    }));
                }
            }
        }
        hits.sort_by(|a, b| b.at.cmp(&a.at));
        hits.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        hits
    }

    async fn search_one(
        &self,
        account: &str,
        words: &str,
        days: i64,
        limit: u64,
    ) -> Result<Vec<MailHit>, GmailError> {
        let q = format!("{words} newer_than:{days}d -in:spam -in:trash -in:drafts");
        let listed = self
            .get(
                account,
                &format!("messages?maxResults={}&q={}", limit.min(25), encode(&q)),
            )
            .await?
            .unwrap_or(Value::Null);
        let me = self
            .state(account)
            .await
            .email
            .unwrap_or_default()
            .to_lowercase();
        let mut hits = Vec::new();
        for id in message_ids(&listed) {
            let path = format!(
                "messages/{id}?format=metadata&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Subject"
            );
            let Some(m) = self.get(account, &path).await? else {
                continue;
            };
            let from = addresses(&header(&m, "From").unwrap_or_default())
                .into_iter()
                .next()
                .unwrap_or_else(|| Address {
                    name: String::new(),
                    address: String::new(),
                });
            let mine = m["labelIds"]
                .as_array()
                .is_some_and(|l| l.iter().any(|l| l == "SENT"))
                || from.address == me;
            let other = if mine {
                addresses(&header(&m, "To").unwrap_or_default())
                    .into_iter()
                    .find(|a| a.address != me)
                    .unwrap_or_else(|| from.clone())
            } else {
                from.clone()
            };
            hits.push(MailHit {
                chat: chat_id(account, &other.address),
                source: source_of(account),
                at: internal_date(&m)
                    .unwrap_or(Timestamp::UNIX_EPOCH)
                    .to_string(),
                sender: if mine {
                    OWNER.to_owned()
                } else if from.name.is_empty() {
                    from.address.clone()
                } else {
                    format!("{} <{}>", from.name, from.address)
                },
                subject: header(&m, "Subject").unwrap_or_default(),
                text: html_decode(m["snippet"].as_str().unwrap_or_default()),
            });
        }
        Ok(hits)
    }

    // ── Signing in ──

    /// `gmail_connect`: a one-time link (a day) that signs `account` in; `email`: the address
    /// the owner is to sign in with.
    ///
    /// # Errors
    ///
    /// [`GmailError`] for a bad account name, no OAuth client saved yet, or no randomness.
    pub async fn connect_link(
        &self,
        account: &str,
        email: Option<&str>,
    ) -> Result<String, GmailError> {
        if !is_account_name(account) {
            return Err(GmailError(
                "account: a short lowercase name (home, work)".to_owned(),
            ));
        }
        if self.ctx.secrets.read(CLIENT_ID_SECRET).await.is_empty()
            || self.ctx.secrets.read(CLIENT_SECRET_SECRET).await.is_empty()
        {
            return Err(GmailError(format!(
                "the server's Google OAuth client isn't saved yet (secrets {CLIENT_ID_SECRET} and \
                 {CLIENT_SECRET_SECRET})"
            )));
        }
        let token = self.ctx.token(24).map_err(|e| GmailError(e.to_string()))?;
        let now = self.ctx.now();
        let mut links = self.ctx.settings.get_or(&LINKS, BTreeMap::new()).await?;
        links.retain(|_, link| link.exp >= now.as_millisecond());
        links.insert(
            token.clone(),
            ConnectLink {
                account: account.to_owned(),
                email: email.map(str::to_owned).filter(|e| !e.is_empty()),
                exp: (now + LINK_FOR).as_millisecond(),
            },
        );
        self.ctx.settings.set(&LINKS, &links).await?;
        Ok(format!(
            "{}/gmail/connect/{token}",
            origin(&self.parts.callback)
        ))
    }

    async fn link(&self, token: &str) -> Option<ConnectLink> {
        let now = self.ctx.now().as_millisecond();
        self.ctx
            .settings
            .get_or(&LINKS, BTreeMap::new())
            .await
            .ok()?
            .remove(token)
            .filter(|link| link.exp >= now)
    }

    /// Takes the link `token` out of the kept ones, in one step (so two callbacks with it
    /// can't both get it): it, unless it has expired.
    async fn take_link(&self, token: &str) -> Option<ConnectLink> {
        let now = self.ctx.now().as_millisecond();
        match self
            .ctx
            .settings
            .update(&LINKS, BTreeMap::new(), |links| links.remove(token))
            .await
        {
            Ok(link) => link.filter(|link| link.exp >= now),
            Err(error) => {
                tracing::warn!(%error, "the sign-in links can't be read");
                None
            }
        }
    }

    /// `GET /gmail/connect/<token>`: on to Google's consent screen for that account.
    pub async fn connect_page(&self, token: &str) -> Page {
        let Some(link) = self.link(token).await else {
            return Page::Html(
                404,
                "<p>This link has expired or was already used. Ask for a new one.</p>".to_owned(),
            );
        };
        let client = self.ctx.secrets.read(CLIENT_ID_SECRET).await;
        let mut query = form_urlencoded::Serializer::new(String::new());
        for (key, value) in [
            ("client_id", Some(client.as_str())),
            ("redirect_uri", Some(self.parts.callback.as_str())),
            ("response_type", Some("code")),
            ("scope", Some(SCOPE)),
            ("access_type", Some("offline")),
            ("prompt", Some("consent")),
            ("include_granted_scopes", Some("false")),
            ("login_hint", link.email.as_deref()),
            ("state", Some(token)),
        ] {
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                query.append_pair(key, value);
            }
        }
        Page::Redirect(format!(
            "{}?{}",
            self.parts.api.consent_url(),
            query.finish()
        ))
    }

    /// `GET /gmail/callback?code&state`: Google's code for a refresh token, saved as the
    /// account's secret. The link is used up whatever happens.
    pub async fn callback(
        &self,
        state: Option<&str>,
        code: Option<&str>,
        error: Option<&str>,
    ) -> Page {
        let token = state.unwrap_or_default();
        let link = if token.is_empty() {
            None
        } else {
            self.take_link(token).await
        };
        let Some(link) = link else {
            return Page::Html(
                404,
                "<p>This sign-in link has expired or was already used. Ask for a new one.</p>"
                    .to_owned(),
            );
        };
        match self.sign_in(&link, code, error).await {
            Ok(email) => Page::Html(
                200,
                format!(
                    "<p>Now readable here: {} (read-only). You can close this tab.</p>",
                    escape(&email)
                ),
            ),
            Err(page) => page,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the callback's checks in order, each refusing with its own page"
    )]
    async fn sign_in(
        &self,
        link: &ConnectLink,
        code: Option<&str>,
        error: Option<&str>,
    ) -> Result<String, Page> {
        let account = link.account.as_str();
        if let Some(error) = error.filter(|e| !e.is_empty()) {
            return Err(Page::Html(
                400,
                format!(
                    "<p>Google said {}: nothing was saved. Ask for a new link.</p>",
                    escape(error)
                ),
            ));
        }
        let form = [
            ("code", code.unwrap_or_default().to_owned()),
            ("client_id", self.ctx.secrets.read(CLIENT_ID_SECRET).await),
            (
                "client_secret",
                self.ctx.secrets.read(CLIENT_SECRET_SECRET).await,
            ),
            ("redirect_uri", self.parts.callback.clone()),
            ("grant_type", "authorization_code".to_owned()),
        ];
        let answer = self.parts.api.token(&form).await.map_err(|e| {
            Page::Html(
                400,
                format!(
                    "<p>Google didn't answer ({}). Nothing was saved.</p>",
                    escape(&e)
                ),
            )
        })?;
        let refresh = answer.json["refresh_token"]
            .as_str()
            .filter(|r| !r.is_empty());
        let Some(refresh) = refresh.filter(|_| answer.ok()) else {
            let why = answer.json["error"]
                .as_str()
                .map_or_else(|| answer.status.to_string(), str::to_owned);
            self.ctx
                .activity(format!(
                    "Email ({account}): Google didn't give a refresh token ({why})"
                ))
                .await;
            return Err(Page::Html(
                400,
                format!(
                    "<p>Google didn't give a sign-in to keep ({}). Nothing was saved.</p>",
                    escape(&why)
                ),
            ));
        };
        let scopes = answer.json["scope"].as_str().unwrap_or(SCOPE);
        if !scopes.split(' ').any(|s| s == SCOPE) {
            return Err(Page::Html(
                400,
                "<p>Reading Gmail wasn't allowed on Google's screen, so nothing was saved. Ask for a \
                 new link and tick it.</p>"
                    .to_owned(),
            ));
        }
        let access = answer.json["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let profile = self.parts.api.get(&access, "profile").await.map_err(|e| {
            Page::Html(
                400,
                format!(
                    "<p>Gmail didn't answer ({}). Nothing was saved.</p>",
                    escape(&e)
                ),
            )
        })?;
        let Some(email) = profile.json["emailAddress"]
            .as_str()
            .filter(|_| profile.ok())
        else {
            return Err(Page::Html(
                400,
                format!(
                    "<p>Gmail didn't answer ({}). Nothing was saved.</p>",
                    profile.status
                ),
            ));
        };
        if let Some(want) = &link.email
            && !want.eq_ignore_ascii_case(email)
        {
            return Err(Page::Html(
                400,
                format!(
                    "<p>That was {}, not {}: nothing was saved. Ask for a new link and pick {}.</p>",
                    escape(email),
                    escape(want),
                    escape(want)
                ),
            ));
        }
        if let Err(e) = self
            .ctx
            .secrets
            .write(&refresh_secret(account), refresh)
            .await
        {
            tracing::error!(%e, "a Gmail sign-in wasn't saved");
            return Err(Page::Html(
                500,
                "<p>The sign-in couldn't be saved here. Nothing was kept.</p>".to_owned(),
            ));
        }
        self.remember(account, &access, &answer.json);
        let email = email.to_owned();
        let all = async {
            let mut states = self.states().await;
            states.insert(
                account.to_owned(),
                MailAccount {
                    email: Some(email.clone()),
                    ..MailAccount::default()
                },
            );
            self.ctx.settings.set(&ACCOUNTS, &states).await
        };
        if let Err(e) = all.await {
            tracing::warn!(%e, "a Gmail account's state wasn't kept");
        }
        self.ctx
            .event(
                emberhut_core::RoomId::MAIN,
                "secret",
                format!(
                    "The owner signed in {email} to be read here ({}, read-only): its new mail comes \
                     with the messages from now on.",
                    source_of(account)
                ),
                None,
            )
            .await;
        let gmail = self.clone();
        drop(tokio::spawn(async move { gmail.collect().await }));
        Ok(email)
    }
}

/// A message's header.
fn header(m: &Value, name: &str) -> Option<String> {
    m["payload"]["headers"]
        .as_array()?
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str().map(str::to_owned))
}

/// The first part of `kind`, depth first, decoded.
fn part(p: &Value, kind: &str) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mime = p["mimeType"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if mime.starts_with(kind)
        && let Some(data) = p["body"]["data"].as_str().filter(|d| !d.is_empty())
    {
        return Some(base64url(data));
    }
    p["parts"]
        .as_array()?
        .iter()
        .find_map(|child| part(child, kind))
}

/// Gmail's base64url, decoded as UTF-8 (empty when it isn't).
fn base64url(data: &str) -> String {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .unwrap_or_default();
    String::from_utf8(bytes).unwrap_or_default()
}

/// A message's `internalDate` (milliseconds, as text).
fn internal_date(m: &Value) -> Option<Timestamp> {
    m["internalDate"]
        .as_str()
        .and_then(|ms| ms.parse::<i64>().ok())
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
}

/// The message ids of a list.
fn message_ids(listed: &Value) -> Vec<String> {
    listed["messages"].as_array().map_or_else(Vec::new, |all| {
        all.iter()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect()
    })
}

/// A message as one line: `sender: «subject» text`, the text cut to `chars`.
#[must_use]
pub fn line(m: &MailMessage, chars: usize) -> String {
    let subject = if m.subject.is_empty() {
        String::new()
    } else {
        format!("«{}» ", m.subject)
    };
    let more = if m.body.chars().count() > chars {
        " …"
    } else {
        ""
    };
    format!(
        "{}: {subject}{}{more}",
        m.sender,
        first_chars(&m.body, chars)
    )
}

/// `url`'s scheme and authority.
fn origin(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.trim_end_matches('/').to_owned();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{scheme}://{host}")
}

/// `text` safe in HTML.
pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_message_s_parts_and_headers_are_read() {
        let m = json!({
            "payload": {
                "headers": [{ "name": "subject", "value": "Hi" }],
                "mimeType": "multipart/alternative",
                "parts": [
                    { "mimeType": "text/html", "body": { "data": "PGI-aGk8L2I-" } },
                    { "mimeType": "text/plain", "body": { "data": "aGk" } }
                ]
            },
            "internalDate": "1800000000000"
        });
        assert_eq!(header(&m, "Subject").as_deref(), Some("Hi"));
        assert_eq!(part(&m["payload"], "text/plain").as_deref(), Some("hi"));
        assert_eq!(
            part(&m["payload"], "text/html").as_deref(),
            Some("<b>hi</b>")
        );
        assert_eq!(
            internal_date(&m).map(Timestamp::as_millisecond),
            Some(1_800_000_000_000)
        );
    }

    #[test]
    fn the_link_goes_on_the_callback_s_origin() {
        assert_eq!(
            origin("https://box.example:8443/gmail/callback"),
            "https://box.example:8443"
        );
        assert_eq!(
            origin("http://127.0.0.1:18990/gmail/callback?x"),
            "http://127.0.0.1:18990"
        );
    }

    #[test]
    fn a_line_shows_the_subject_and_cuts_the_text() {
        let m = MailMessage {
            id: "home:1".to_owned(),
            account: "home".to_owned(),
            chat: "email:home:a@example.com".to_owned(),
            who: "Ann".to_owned(),
            sender: "Ann <a@example.com>".to_owned(),
            from_me: false,
            subject: "Lunch".to_owned(),
            body: "Friday works for me".to_owned(),
            at: Timestamp::UNIX_EPOCH,
            got: Timestamp::UNIX_EPOCH,
            loud: true,
            thread: None,
        };
        assert_eq!(line(&m, 6), "Ann <a@example.com>: «Lunch» Friday …");
    }
}
