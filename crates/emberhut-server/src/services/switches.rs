//! Settings → MCP servers and Skills: what the assistant's own session has, each with its live
//! state and the person's switch.
//!
//! Off is the agent's own: a deny list of servers and skill overrides in the settings every
//! session the assistant starts gets ([`SwitchService::session_settings`]), kept here (the
//! `prefs` table). The assistant never toggles a live session: a room restarts with the new
//! settings once its turn is over, and the runners take them at a session's next start (they
//! are pushed to every runner, `POST /off`, whenever they change). The server's own MCP server
//! has no switch: without it the assistant can't act. A server or skill that is off stays
//! listed (seen before), so it can be turned back on.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use emberhut_api as api;
use emberhut_core::Clock;
use emberhut_db::{DbError, PrefKey, PrefStore};
use serde_json::{Map, Value, json};

use crate::convert::timestamp_to_api;
use crate::services::accounts::AccountService;
use crate::services::activity::ActivityLog;
use crate::services::runner_api::Verb;
use crate::services::runners::{RunnerService, text};
use crate::services::threads::ThreadService;
use crate::services::workers_view::AssistantTasks;

/// `POST /api/switches`'s body, read leniently: a missing or unusable value is refused by
/// [`SwitchService::act`], saying what.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct SwitchRequest {
    /// `mcp`, `skill` or `reconnect`.
    #[serde(default)]
    pub kind: Option<String>,
    /// The server's or the skill's name.
    #[serde(default)]
    pub name: Option<String>,
    /// With `mcp` or `skill`: on or off.
    #[serde(default)]
    pub on: Option<bool>,
}

/// A server's row as the page is put together.
#[derive(Clone, Debug)]
struct McpRow {
    name: String,
    state: String,
    error: Option<String>,
    source: Option<String>,
    kind: Option<String>,
    on: bool,
    fixed: bool,
    note: Option<String>,
}

impl McpRow {
    fn into_api(self) -> api::switches::McpServer {
        api::switches::McpServer {
            name: self.name,
            state: api::switches::McpState::from(self.state),
            error: self.error,
            source: self.source,
            kind: self.kind,
            on: self.on,
            fixed: self.fixed,
            note: self.note,
        }
    }
}

/// A skill's row as the page is put together.
#[derive(Clone, Debug)]
struct SkillRow {
    name: String,
    description: String,
    source: String,
    on: bool,
}

impl SkillRow {
    fn into_api(self) -> api::switches::Skill {
        api::switches::Skill {
            name: self.name,
            description: self.description,
            source: api::switches::SkillSource::from(self.source),
            on: self.on,
        }
    }
}

/// The servers that have no switch: the server's own.
const FIXED: [&str; 1] = [api::mcp::SERVER_NAME];

/// Why a switch wasn't changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SwitchError {
    /// Not a change that can be made, in words.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The switches.
#[derive(Clone, Debug)]
pub struct SwitchService {
    prefs: PrefStore,
    runners: RunnerService,
    threads: ThreadService,
    accounts: AccountService,
    session: Arc<dyn AssistantTasks>,
    activity: ActivityLog,
    clock: Arc<dyn Clock>,
}

/// The parts [`SwitchService`] is built from.
#[derive(Clone, Debug)]
pub struct SwitchParts {
    /// Where the switches are kept.
    pub prefs: PrefStore,
    /// The runners, told what is off.
    pub runners: RunnerService,
    /// The coding-thread app, whose machines' errors explain a failed server.
    pub threads: ThreadService,
    /// The account the assistant runs on.
    pub accounts: AccountService,
    /// The assistant's session.
    pub session: Arc<dyn AssistantTasks>,
    /// The activity log.
    pub activity: ActivityLog,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
}

impl SwitchService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: SwitchParts) -> Self {
        let SwitchParts {
            prefs,
            runners,
            threads,
            accounts,
            session,
            activity,
            clock,
        } = parts;
        Self {
            prefs,
            runners,
            threads,
            accounts,
            session,
            activity,
            clock,
        }
    }

    async fn names(&self, key: PrefKey) -> Result<BTreeSet<String>, DbError> {
        Ok(self
            .prefs
            .get(key)
            .await?
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default())
    }

    async fn seen(&self, key: PrefKey) -> Result<BTreeMap<String, Map<String, Value>>, DbError> {
        Ok(self
            .prefs
            .get(key)
            .await?
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default())
    }

    async fn keep(&self, key: PrefKey, value: &impl serde::Serialize) -> Result<(), DbError> {
        let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
        self.prefs.set(key, Some(&text)).await
    }

    /// The assistant's session started (its init: `{ mcp_servers: [{ name, status, source }],
    /// skills: [name] }`): what it has is remembered, so a server or skill stays listed.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn observe(&self, init: &Value) -> Result<(), DbError> {
        let now = timestamp_to_api(self.clock.now());
        let mut mcp = self.seen(PrefKey::McpSeen).await?;
        for server in init
            .get("mcp_servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = text(server, "name").filter(|name| !name.is_empty()) {
                let row = json!({ "source": text(server, "source"), "status": text(server, "status"), "at": now });
                mcp.insert(name, row.as_object().cloned().unwrap_or_default());
            }
        }
        self.keep(PrefKey::McpSeen, &mcp).await?;
        let mut skills = self.seen(PrefKey::SkillsSeen).await?;
        for skill in init
            .get("skills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = skill.as_str() {
                skills.entry(name.to_owned()).or_insert_with(|| {
                    json!({ "description": "" })
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                });
            }
        }
        self.keep(PrefKey::SkillsSeen, &skills).await
    }

    /// The settings every session the assistant starts gets: the agent's own deny list
    /// (`deniedMcpServers`) and skill overrides (`skillOverrides`), empty when all are on.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn session_settings(&self) -> Result<Map<String, Value>, DbError> {
        let mut settings = Map::new();
        let mcp = self.names(PrefKey::McpOff).await?;
        if !mcp.is_empty() {
            settings.insert(
                "deniedMcpServers".to_owned(),
                mcp.iter()
                    .map(|name| json!({ "serverName": name }))
                    .collect(),
            );
        }
        let skills = self.names(PrefKey::SkillsOff).await?;
        if !skills.is_empty() {
            settings.insert(
                "skillOverrides".to_owned(),
                skills
                    .iter()
                    .map(|name| (name.clone(), json!("off")))
                    .collect::<Map<_, _>>()
                    .into(),
            );
        }
        Ok(settings)
    }

    /// The page: every MCP server the session has or had and every skill, with their state and
    /// switch.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    #[expect(
        clippy::too_many_lines,
        reason = "the servers' rows, then the skills', each from what was seen and what is off"
    )]
    pub async fn page(&self) -> Result<api::switches::SwitchPage, DbError> {
        let off = self.names(PrefKey::McpOff).await?;
        let live = self.session.mcp_status().await;
        let mut rows: BTreeMap<String, McpRow> = BTreeMap::new();
        for server in live.iter().flatten() {
            let Some(name) = text(server, "name").filter(|name| !name.is_empty()) else {
                continue;
            };
            let config = server.get("config").unwrap_or(&Value::Null);
            let kind = match text(config, "type").as_deref() {
                Some("claudeai-proxy") => Some("claude.ai connector".to_owned()),
                Some(kind) => Some(kind.to_owned()),
                None => config.get("command").map(|_| "stdio".to_owned()),
            };
            rows.insert(
                name.clone(),
                row(
                    name,
                    text(server, "status").unwrap_or_else(|| "unknown".to_owned()),
                )
                .with(
                    text(server, "error"),
                    text(server, "scope").or_else(|| text(server, "source")),
                    kind,
                ),
            );
        }
        for (name, seen) in self.seen(PrefKey::McpSeen).await? {
            if !rows.contains_key(&name) {
                let state = if live.is_none() {
                    seen.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned()
                } else {
                    "unknown".to_owned()
                };
                let source = seen
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                rows.insert(name.clone(), row(name, state).with(None, source, None));
            }
        }
        for name in &off {
            rows.entry(name.clone())
                .or_insert_with(|| row(name.clone(), "unknown".to_owned()));
        }
        // The coding-thread app's own poll says why a machine's server can't be reached.
        let errors = self.threads.machine_errors();
        let mut mcp: Vec<McpRow> = rows
            .into_values()
            .map(|mut row| {
                row.on = !off.contains(&row.name);
                row.fixed = FIXED.contains(&row.name.as_str());
                if !row.on {
                    "off".clone_into(&mut row.state);
                    row.error = None;
                }
                if row.on
                    && let Some(Some(error)) = errors.get(&row.name)
                    && !row
                        .error
                        .as_deref()
                        .unwrap_or_default()
                        .contains(error.as_str())
                {
                    row.error = Some(
                        match row
                            .error
                            .as_deref()
                            .map(str::trim)
                            .filter(|e| !e.is_empty())
                        {
                            Some(had) => format!("{} ({error})", had.trim_end_matches(':')),
                            None => error.clone(),
                        },
                    );
                }
                row.note = if row.fixed {
                    Some("The server's own tools: always on".to_owned())
                } else if row.state == "needs-auth" {
                    Some("Needs a sign-in in the agent's own MCP settings".to_owned())
                } else if row.state == "disabled" {
                    Some("Turned off in the agent's own MCP settings".to_owned())
                } else {
                    None
                };
                row
            })
            .collect();
        let rank = |state: &str| match state {
            "connected" => 0,
            "failed" | "needs-auth" => 1,
            "pending" => 2,
            "off" => 4,
            _ => 3,
        };
        mcp.sort_by(|a, b| {
            b.fixed
                .cmp(&a.fixed)
                .then_with(|| rank(&a.state).cmp(&rank(&b.state)))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let skills_off = self.names(PrefKey::SkillsOff).await?;
        let mut seen = self.seen(PrefKey::SkillsSeen).await?;
        for name in &skills_off {
            seen.entry(name.clone()).or_default();
        }
        let source_rank = |source: &str| match source {
            "yours" => 0,
            "plugin" => 1,
            _ => 2,
        };
        let mut skills: Vec<SkillRow> = seen
            .into_iter()
            .map(|(name, seen)| SkillRow {
                source: if name.contains(':') {
                    "plugin".to_owned()
                } else {
                    seen.get("source")
                        .and_then(Value::as_str)
                        .unwrap_or("built in")
                        .to_owned()
                },
                description: seen
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                on: !skills_off.contains(&name),
                name,
            })
            .collect();
        skills.sort_by(|a, b| {
            source_rank(&a.source)
                .cmp(&source_rank(&b.source))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(api::switches::SwitchPage {
            at: timestamp_to_api(self.clock.now()),
            account: self.accounts.assistant().await?.on,
            live: live.is_some(),
            note: live.is_none().then(|| {
                "The assistant's session isn't open right now: this is what it had last.".to_owned()
            }),
            mcp: mcp.into_iter().map(McpRow::into_api).collect(),
            skills: skills.into_iter().map(SkillRow::into_api).collect(),
        })
    }

    /// The person's change: a server or skill on or off, or a server reconnected. The page
    /// after it.
    ///
    /// # Errors
    ///
    /// [`SwitchError::Refused`] for a change that can't be made; [`SwitchError::Database`].
    pub async fn act(
        &self,
        action: &SwitchRequest,
    ) -> Result<api::switches::SwitchPage, SwitchError> {
        let kind = action.kind.as_deref().unwrap_or_default();
        let name = action.name.as_deref().unwrap_or_default().trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(SwitchError::Refused("name: the server or skill".to_owned()));
        }
        match kind {
            "mcp" | "skill" => {
                let Some(on) = action.on else {
                    return Err(SwitchError::Refused("on: true or false".to_owned()));
                };
                if kind == "mcp" && FIXED.contains(&name) {
                    return Err(SwitchError::Refused(format!(
                        "{name} is the server's own: it can't be turned off"
                    )));
                }
                let key = if kind == "mcp" {
                    PrefKey::McpOff
                } else {
                    PrefKey::SkillsOff
                };
                let mut set = self.names(key).await?;
                let changed = if on {
                    set.remove(name)
                } else {
                    set.insert(name.to_owned())
                };
                if changed {
                    self.keep(key, &set).await?;
                    let what = if kind == "mcp" {
                        format!("the {name} MCP server")
                    } else {
                        format!("the {name} skill")
                    };
                    let word = if on { "on" } else { "off" };
                    if let Err(error) = self
                        .activity
                        .note(format!("Turned {what} {word} in Settings"))
                        .await
                    {
                        tracing::warn!(%error, "an activity line could not be written");
                    }
                    self.session.reload(&format!("{what} turned {word}")).await;
                    self.push_runners().await?;
                }
            }
            "reconnect" => {
                if self.names(PrefKey::McpOff).await?.contains(name) {
                    return Err(SwitchError::Refused(format!(
                        "{name} is off: turn it on first"
                    )));
                }
                self.session
                    .reconnect(name)
                    .await
                    .map_err(SwitchError::Refused)?;
                if let Err(error) = self
                    .activity
                    .note(format!("Reconnected the {name} MCP server from Settings"))
                    .await
                {
                    tracing::warn!(%error, "an activity line could not be written");
                }
            }
            other => return Err(SwitchError::Refused(format!("unknown action {other}"))),
        }
        Ok(self.page().await?)
    }

    /// Tells every runner what is off, for the sessions the assistant starts there (a runner
    /// that doesn't answer gets it at the next change).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn push_runners(&self) -> Result<(), DbError> {
        let body = json!({
            "mcp": self.names(PrefKey::McpOff).await?,
            "skills": self.names(PrefKey::SkillsOff).await?,
        });
        for machine in self.runners.machines() {
            if let Err(error) = self
                .runners
                .api()
                .call(&machine, Verb::Post, "/off", Some(body.clone()))
                .await
            {
                tracing::debug!(%machine, %error, "a runner didn't take the switches");
            }
        }
        Ok(())
    }
}

/// A row with its name and state, the rest unknown.
fn row(name: String, state: String) -> McpRow {
    McpRow {
        name,
        state,
        error: None,
        source: None,
        kind: None,
        on: true,
        fixed: false,
        note: None,
    }
}

/// A row's details.
trait With {
    fn with(self, error: Option<String>, source: Option<String>, kind: Option<String>) -> Self;
}

impl With for McpRow {
    fn with(mut self, error: Option<String>, source: Option<String>, kind: Option<String>) -> Self {
        self.error = error;
        self.source = source;
        self.kind = kind;
        self
    }
}
