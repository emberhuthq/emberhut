//! The sessions: each one runs the agent in a folder, keeps its agent session id, takes its
//! next turn when the server sends one, can be interrupted or stopped, and reports every change
//! as it happens.
//!
//! - A turn is one message and its answer. Turns queue one behind the other (never folded into
//!   the running one), so each answer belongs to exactly one turn.
//! - A session starts on the account with the most room. When a turn ends on a usage limit, the
//!   account is marked out and the session moves to the next one and carries on with the same
//!   turn. All out: the turn fails, saying when one is free, and the session carries on then by
//!   itself.
//! - A runner restart (or a crashed agent process) resumes every turn that was running, by
//!   session id, with a short "carry on" message; queued turns follow.
//! - The owner's own sessions (`meta.direct`) are driven from an app: no work instructions,
//!   their own words, and a question (`AskUserQuestion`) goes to the app as a card while the
//!   session waits for the answer.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use std::fmt::Write as _;

use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, mpsc};

use super::folders::{self, WorktreeAsk};
use super::live::Watchers;
use super::prompts;
use crate::accounts::{self, Account, Accounts, Provider, Work};
use crate::config::Config;
use crate::engine::{Agent, AgentOutput, Engine, Launch, SessionStart};
use crate::files::Files;
use crate::report::Reporter;
use crate::store::{self, SessionChange, Store, event, now, session, turn};
use crate::text::{clip, diff_of, doing, mb, tool_summary};
use crate::wire::{self, Refused, event as ev, live};

/// A session with nothing to do closes its agent process after this long; its next turn resumes it.
const IDLE: Duration = Duration::from_mins(10);
/// How often the sessions waiting for a free account look for one.
const RESUME_EVERY: Duration = Duration::from_mins(1);
/// How often the watchdog looks for a running turn whose agent has gone quiet.
const WATCH_EVERY: Duration = Duration::from_mins(1);
/// A running turn whose agent has said nothing for this long is asked to stop, then carried on.
const QUIET: Duration = Duration::from_mins(30);

type Boxed<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// The running turn's output tokens so far.
#[derive(Debug, Clone, Copy, Default)]
struct Tok {
    out: u64,
    thought: u64,
    think: u64,
    chars: u64,
    at: Option<Instant>,
}

/// A sub-agent the session runs, for the live head.
#[derive(Debug, Clone)]
struct AgentInfo {
    id: String,
    description: String,
    tokens: u64,
    tools: u64,
    last: Option<String>,
    running: bool,
    started: String,
    ended: Option<Instant>,
}

/// A session's live state while the runner runs it.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate fact about the running turn, set and cleared at different times"
)]
struct Live {
    id: String,
    agent: Option<Agent>,
    /// Which agent process is the current one: a process replaced on purpose stops being heard.
    generation: u64,
    turn: Option<String>,
    limit: Option<(Option<String>, String)>,
    interrupting: bool,
    stopping: bool,
    moving: bool,
    reopen: bool,
    asked: bool,
    crashes: u32,
    last_text: String,
    told_at: Option<Instant>,
    used_at: Instant,
    background: i64,
    doing: String,
    partial: String,
    usage: Option<(u64, Option<f64>)>,
    thinking: String,
    think_at: Option<(Instant, String)>,
    tok: Tok,
    todos: Option<Vec<Value>>,
    creating: HashMap<String, Value>,
    agents: HashMap<String, AgentInfo>,
}

impl Live {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            agent: None,
            generation: 0,
            turn: None,
            limit: None,
            interrupting: false,
            stopping: false,
            moving: false,
            reopen: false,
            asked: false,
            crashes: 0,
            last_text: String::new(),
            told_at: None,
            used_at: Instant::now(),
            background: 0,
            doing: String::new(),
            partial: String::new(),
            usage: None,
            thinking: String::new(),
            think_at: None,
            tok: Tok::default(),
            todos: None,
            creating: HashMap::new(),
            agents: HashMap::new(),
        }
    }

    fn tokens(&self) -> u64 {
        self.tok.out + self.tok.thought + self.tok.think + self.tok.chars.div_ceil(4)
    }

    /// Clears what belonged to the turn that just ended.
    fn turn_ended(&mut self) {
        self.turn = None;
        self.last_text.clear();
        self.limit = None;
        self.interrupting = false;
        self.crashes = 0;
        self.doing.clear();
        self.partial.clear();
        self.asked = false;
        self.thinking.clear();
        self.think_at = None;
        self.tok = Tok::default();
    }
}

/// What the machine sample says of the sessions.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Turns running now.
    pub running: u64,
    /// Sessions with an agent process open.
    pub open: u64,
    /// Turns waiting.
    pub queued: u64,
    /// The sessions that are doing something, up to 40.
    pub list: Vec<Value>,
}

/// What `POST /sessions` takes.
pub type OpenAsk = Map<String, Value>;

/// The MCP servers and skills the owner turned off.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Off {
    /// MCP servers.
    pub mcp: Vec<String>,
    /// Skills.
    pub skills: Vec<String>,
}

fn meta_of(s: &session::Model) -> Map<String, Value> {
    s.meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn is_direct(s: &session::Model) -> bool {
    meta_of(s).get("direct") == Some(&Value::Bool(true))
}

/// A JSON value as text (a string as it is, a number or a bool printed).
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn iso_of(at: std::time::SystemTime) -> String {
    jiff::Timestamp::try_from(at).map_or_else(
        |_| now(),
        |t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    )
}

/// A model id as the agent takes it (an alias, a full id, `[1m]`, a provider's id).
///
/// # Errors
/// 400 for anything else.
pub fn model_of(m: Option<&Value>) -> Result<Option<String>, Refused> {
    let s = text(m);
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let ok = s.len() <= 160
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:/-[]".contains(c));
    if ok {
        Ok(Some(s.to_owned()))
    } else {
        Err(Refused::bad(format!(
            "no model {}",
            clip(&format!("{s:?}"), 90)
        )))
    }
}

fn effort_of(v: Option<&Value>) -> Result<Option<String>, Refused> {
    let e = text(v);
    if e.is_empty() {
        return Ok(None);
    }
    if wire::EFFORTS.contains(&e.as_str()) {
        Ok(Some(e))
    } else {
        Err(Refused::bad(format!("no effort {e}")))
    }
}

fn db(e: &store::StoreError) -> Refused {
    tracing::error!("{e}");
    Refused::new(500, "the runner's database failed")
}

/// A random id part: `n` bytes as lowercase hex.
fn hex(n: usize) -> String {
    let mut b = vec![0_u8; n];
    if getrandom::getrandom(&mut b).is_err() {
        // No system randomness: the time keeps ids apart well enough for one runner.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, x) in b.iter_mut().enumerate() {
            *x = u8::try_from((t >> (i * 8)) & 0xff).unwrap_or_default();
        }
    }
    b.iter().fold(String::with_capacity(n * 2), |mut s, x| {
        use std::fmt::Write as _;
        write!(s, "{x:02x}").unwrap_or_default();
        s
    })
}

/// A random UUID (version 4), as the agent takes a session id.
#[must_use]
pub fn uuid() -> String {
    let h = hex(16);
    let mut c: Vec<char> = h.chars().collect();
    c[12] = '4';
    c[16] = ['8', '9', 'a', 'b'][usize::from(
        c[16]
            .to_digit(16)
            .map_or(0, |d| u8::try_from(d % 4).unwrap_or(0)),
    )];
    let s: String = c.into_iter().collect();
    format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )
}

/// The runner's sessions.
#[derive(Debug)]
pub struct Sessions {
    store: Store,
    accounts: Arc<Accounts>,
    reporter: Arc<Reporter>,
    files: Arc<Files>,
    config: Arc<Config>,
    engine: Arc<dyn Engine>,
    lives: StdMutex<HashMap<String, Arc<Mutex<Live>>>>,
    /// Who watches each session live.
    pub watchers: Watchers,
    off: StdMutex<Off>,
    generations: AtomicU64,
    me: Weak<Self>,
}

impl Sessions {
    /// The sessions of this runner.
    #[must_use]
    pub fn new(
        store: Store,
        accounts: Arc<Accounts>,
        reporter: Arc<Reporter>,
        files: Arc<Files>,
        config: Arc<Config>,
        engine: Arc<dyn Engine>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            store,
            accounts,
            reporter,
            files,
            config,
            engine,
            lives: StdMutex::default(),
            watchers: Watchers::default(),
            off: StdMutex::default(),
            generations: AtomicU64::new(1),
            me: me.clone(),
        })
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    fn live(&self, id: &str) -> Arc<Mutex<Live>> {
        let Ok(mut m) = self.lives.lock() else {
            return Arc::new(Mutex::new(Live::new(id)));
        };
        Arc::clone(
            m.entry(id.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(Live::new(id)))),
        )
    }

    fn all_lives(&self) -> Vec<Arc<Mutex<Live>>> {
        self.lives
            .lock()
            .map_or_else(|_| Vec::new(), |m| m.values().cloned().collect())
    }

    async fn session(&self, id: &str) -> Option<session::Model> {
        self.store.session(id).await.ok().flatten()
    }

    async fn turn_row(&self, id: Option<&str>) -> Option<turn::Model> {
        self.store.turn(id?).await.ok().flatten()
    }

    async fn change(&self, id: &str, c: SessionChange) {
        if let Err(e) = self.store.change_session(id, c).await {
            tracing::error!(session = id, "{e}");
        }
    }

    async fn save_turn(&self, t: turn::Model) {
        if let Err(e) = self.store.save_turn(t).await {
            tracing::error!("{e}");
        }
    }

    // ── Shapes ──────────────────────────────────────────────────────────────────────────────

    /// A session as the protocol carries it.
    #[must_use]
    pub fn shape(&self, s: &session::Model) -> Map<String, Value> {
        let meta = s
            .meta
            .as_deref()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .unwrap_or(Value::Null);
        obj(json!({
            "id": s.id, "title": s.title, "cwd": s.cwd, "project": s.project, "status": s.status, "account": s.account,
            "claude": s.claude, "worktree": s.worktree, "branch": s.branch, "meta": meta, "created": s.created,
            "updated": s.updated, "model": s.model.clone().or_else(|| self.config.model.clone()), "effort": s.effort,
        }))
    }

    /// A turn as the protocol carries it.
    #[must_use]
    pub fn turn_shape(t: Option<&turn::Model>) -> Value {
        let Some(t) = t else { return Value::Null };
        let accounts = t
            .accounts
            .as_deref()
            .and_then(|a| serde_json::from_str::<Value>(a).ok())
            .unwrap_or_else(|| json!([]));
        json!({ "id": t.id, "n": t.n, "prompt": t.prompt, "by": t.by, "requested": t.requested, "started": t.started,
                "ended": t.ended, "outcome": t.outcome, "last": t.last, "accounts": accounts })
    }

    /// A stored step as the transcript carries it.
    #[must_use]
    pub fn step_of(e: &event::Model) -> Value {
        let mut m = obj(json!({ "seq": e.seq, "turn": e.turn, "at": e.at, "kind": e.kind }));
        if let Some(Value::Object(data)) =
            e.data.as_deref().and_then(|d| serde_json::from_str(d).ok())
        {
            for (k, v) in data {
                m.entry(k).or_insert(v);
            }
        }
        Value::Object(m)
    }

    fn head_with(
        &self,
        s: &session::Model,
        l: Option<&Live>,
        started: Option<String>,
        queued: u64,
    ) -> Value {
        let mut h = self.shape(s);
        let running = l.is_some_and(|l| l.turn.is_some());
        let agents: Vec<Value> = l.map_or_else(Vec::new, |l| {
            l.agents
                .values()
                .filter(|a| a.running || a.ended.is_some_and(|e| e.elapsed() < Duration::from_mins(10)))
                .map(|a| json!({ "id": a.id, "description": a.description, "tokens": a.tokens, "tools": a.tools,
                                 "last": a.last, "running": a.running, "started": a.started }))
                .collect()
        });
        h.insert("running".to_owned(), Value::Bool(running));
        h.insert(
            "doing".to_owned(),
            l.filter(|l| l.turn.is_some() && !l.doing.is_empty())
                .map_or(Value::Null, |l| Value::String(l.doing.clone())),
        );
        h.insert(
            "since".to_owned(),
            started.map_or(Value::Null, Value::String),
        );
        h.insert(
            "turn".to_owned(),
            l.and_then(|l| l.turn.clone())
                .map_or(Value::Null, Value::String),
        );
        h.insert(
            "tokens".to_owned(),
            l.filter(|l| l.turn.is_some())
                .map_or(Value::Null, |l| json!(l.tokens())),
        );
        h.insert("agents".to_owned(), Value::Array(agents));
        h.insert("queued".to_owned(), json!(queued));
        Value::Object(h)
    }

    async fn head_locked(&self, l: &Live) -> Option<Value> {
        let s = self.session(&l.id).await?;
        let started = self
            .turn_row(l.turn.as_deref())
            .await
            .and_then(|t| t.started);
        let queued = self.store.queued(&l.id).await.unwrap_or(0);
        Some(self.head_with(&s, Some(l), started, queued))
    }

    /// The session's head: as stored, plus what runs now (its step, since when, tokens,
    /// sub-agents, what waits).
    pub async fn head_of(&self, id: &str) -> Option<Value> {
        let live = self.live(id);
        let l = live.lock().await;
        self.head_locked(&l).await
    }

    /// What the running turn is typing and thinking now, for a watcher who joins late.
    pub async fn typing(&self, id: &str) -> Option<(String, String, String, Option<String>)> {
        let live = self.live(id);
        let l = live.lock().await;
        let turn = l.turn.clone()?;
        Some((
            turn,
            l.partial.clone(),
            l.thinking.clone(),
            l.think_at.as_ref().map(|(_, s)| s.clone()),
        ))
    }

    async fn push_head(&self, l: &Live) {
        if self.watchers.has(&l.id)
            && let Some(h) = self.head_locked(l).await
        {
            self.watchers
                .send(&l.id, &json!({ "type": live::HEAD, "session": h }));
        }
    }

    // ── Steps and events ────────────────────────────────────────────────────────────────────

    /// Writes a transcript step and sends it to whoever watches live.
    async fn note(&self, l: &mut Live, turn: Option<&str>, kind: &str, data: Value) {
        let mut data = obj(data);
        data.retain(|_, v| !v.is_null());
        let parent = data.contains_key("parent");
        match self
            .store
            .add_event(&l.id, turn, kind, &Value::Object(data))
            .await
        {
            Ok(e) => {
                if self.watchers.has(&l.id) {
                    self.watchers.send(
                        &l.id,
                        &json!({ "type": live::STEP, "step": Self::step_of(&e) }),
                    );
                }
            }
            Err(e) => tracing::error!(session = %l.id, "cannot write a step: {e}"),
        }
        if kind == "text" && !parent {
            l.partial.clear();
        }
    }

    /// Reports a change of a session to the server, with the session as it is now.
    async fn tell(&self, l: &Live, kind: &str, extra: Value) {
        let Some(s) = self.session(&l.id).await else {
            return;
        };
        let mut body = obj(extra);
        body.insert("session".to_owned(), Value::Object(self.shape(&s)));
        self.reporter.tell(&l.id, kind, body).await;
        self.push_head(l).await;
    }

    /// Writes a `system` step in a session's transcript (a file arrived, for one).
    pub async fn note_system(&self, id: &str, words: &str) {
        let live = self.live(id);
        let mut l = live.lock().await;
        self.note(&mut l, None, "system", json!({ "text": words }))
            .await;
    }

    // ── Accounts ────────────────────────────────────────────────────────────────────────────

    fn work(&self, s: &session::Model) -> Work {
        let meta = meta_of(s);
        let model = s.model.clone().or_else(|| self.config.model.clone());
        let pin = meta.get("pin").and_then(Value::as_str);
        let only = self.accounts.only_for(pin, model.as_deref());
        Work {
            project: s.project.clone(),
            cwd: Some(s.origin.clone().unwrap_or_else(|| s.cwd.clone())),
            opt_in: meta.get("optIn") == Some(&Value::Bool(true)) || only.is_some(),
            only,
            model,
            ..Work::default()
        }
    }

    fn account_for(&self, s: &session::Model) -> Option<Account> {
        self.accounts.pick(&Work {
            prefer: s.account.clone(),
            ..self.work(s)
        })
    }

    fn name_of(&self, id: Option<&str>) -> String {
        self.accounts.name_of(id)
    }

    /// The MCP servers and skills turned off: the sessions the assistant starts next get them.
    pub fn set_off(&self, body: &Value) -> Off {
        let names = |k: &str| -> Vec<String> {
            body.get(k)
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|n| !n.is_empty() && n.len() <= 120)
                        .map(str::to_owned)
                        .collect()
                })
        };
        let off = Off {
            mcp: names("mcp"),
            skills: names("skills"),
        };
        if let Ok(mut o) = self.off.lock() {
            o.clone_from(&off);
        }
        off
    }

    fn off_settings(&self) -> Map<String, Value> {
        let off = self
            .off
            .lock()
            .map_or_else(|_| Off::default(), |o| o.clone());
        let mut m = Map::new();
        if !off.mcp.is_empty() {
            m.insert(
                "deniedMcpServers".to_owned(),
                off.mcp.iter().map(|n| json!({ "serverName": n })).collect(),
            );
        }
        if !off.skills.is_empty() {
            m.insert(
                "skillOverrides".to_owned(),
                Value::Object(
                    off.skills
                        .iter()
                        .map(|n| (n.clone(), json!("off")))
                        .collect(),
                ),
            );
        }
        m
    }

    // ── The agent process ───────────────────────────────────────────────────────────────────

    /// Opens the session's agent process: resumed by its id once it has a transcript, else
    /// started with that id.
    #[expect(
        clippy::too_many_lines,
        reason = "one launch, every one of its options in one place"
    )]
    async fn open_agent(&self, l: &mut Live) -> Result<(Account, bool), String> {
        let s = self
            .session(&l.id)
            .await
            .ok_or_else(|| "the session is gone".to_owned())?;
        let a = self
            .account_for(&s)
            .ok_or_else(|| "no account has room now".to_owned())?;
        if Some(&a.id) != s.account.as_ref() {
            self.change(
                &l.id,
                SessionChange {
                    account: Some(Some(a.id.clone())),
                    ..SessionChange::default()
                },
            )
            .await;
        }
        let claude = s.claude.clone().unwrap_or_else(uuid);
        let resumed = s.claude.is_some()
            && self
                .accounts
                .logins
                .link_session(&claude, &a, &self.accounts.list());
        l.moving = false;
        l.reopen = false;
        let direct = is_direct(&s);
        let origin = PathBuf::from(s.origin.clone().unwrap_or_else(|| s.cwd.clone()));
        let cwd = PathBuf::from(&s.cwd);
        let have = folders::instruction_files(&cwd);
        let missing: Vec<String> = folders::instruction_files(&origin)
            .into_iter()
            .filter(|f| !have.contains(f))
            .collect();
        let project = (!missing.is_empty()).then(|| prompts::project_note(&origin, &missing));
        let model = s.model.clone().or_else(|| self.config.model.clone());
        let mcp = if self.config.share_mcp {
            self.reporter.shared_mcp()
        } else {
            Map::new()
        };
        let mut settings = obj(json!({
            "autoMemoryDirectory": self.accounts.logins.memory_dir(&origin).await,
            "includeCoAuthoredBy": false,
            "attribution": { "commit": "", "pr": "", "sessionUrl": false },
        }));
        if !direct {
            settings.extend(self.off_settings());
        }
        let env = self
            .accounts
            .logins
            .env_for(&a, self.config.skills_dir.as_deref());
        let mut set_env = env.set;
        set_env.push(("CLAUDE_CODE_ENABLE_TODO_TOOLS".to_owned(), "1".to_owned()));
        let launch = Launch {
            cwd: cwd.clone(),
            set_env,
            remove_env: env.remove,
            model: model.map(|m| a.model_for(&m)),
            effort: s.effort.clone(),
            session: if resumed {
                SessionStart::Resume(claude.clone())
            } else {
                SessionStart::New(claude.clone())
            },
            append_system_prompt: Some(prompts::system_append(
                &self.config.instructions,
                direct,
                &self.files.inbox_of(&l.id),
                project.as_deref(),
            )),
            settings: Some(Value::Object(settings)),
            mcp_servers: (!mcp.is_empty()).then_some(Value::Object(mcp)),
            add_dirs: if same_dir(&cwd, &origin) {
                Vec::new()
            } else {
                vec![origin]
            },
            // A session the assistant runs puts its questions in its answer, where the server reads them.
            disallowed_tools: if direct {
                Vec::new()
            } else {
                vec!["AskUserQuestion".to_owned()]
            },
            ask_runner: direct,
            partial_messages: true,
            strict_mcp: false,
        };
        let (agent, rx) = self.engine.start(&launch).map_err(|e| e.to_string())?;
        if s.claude.is_none() {
            self.change(
                &l.id,
                SessionChange {
                    claude: Some(Some(claude)),
                    ..SessionChange::default()
                },
            )
            .await;
        }
        l.generation = self.generations.fetch_add(1, Ordering::Relaxed);
        l.agent = Some(agent);
        if let Some(me) = self.me.upgrade() {
            tokio::spawn(listen(me, l.id.clone(), l.generation, rx, a.id.clone()));
        }
        Ok((a, resumed))
    }

    /// Closes the session's agent process (on purpose: it is not heard any more).
    async fn close(&self, l: &mut Live, why: &str) {
        if let Some(agent) = l.agent.take() {
            l.moving = true;
            l.generation = 0;
            agent.close().await;
            tracing::info!(session = %l.id, "closed ({why})");
        }
    }

    /// Says something to the session's agent.
    async fn say(&self, l: &Live, text: &str) -> Result<(), String> {
        let agent = l
            .agent
            .clone()
            .ok_or_else(|| "the agent isn't running".to_owned())?;
        agent.say(text).await.map_err(|e| e.to_string())
    }

    async fn on_exit(&self, generation: u64, l: &mut Live, why: &str) {
        if l.generation != generation {
            return;
        }
        l.agent = None;
        if l.moving || l.turn.is_none() {
            return;
        }
        tracing::warn!(session = %l.id, "ended mid-turn: {why}");
        l.crashes += 1;
        if l.crashes <= 2 {
            let turn = l.turn.clone();
            self.note(
                l,
                turn.as_deref(),
                "system",
                json!({ "text": format!("The agent stopped ({}); carrying on", clip(why, 160)) }),
            )
            .await;
            return self.restart_turn(l, prompts::RESTARTED).await;
        }
        self.finish_turn(
            l,
            "failed",
            &format!("The agent stopped and didn't come back: {}", clip(why, 300)),
        )
        .await;
    }

    async fn on_message(&self, l: &mut Live, m: &Value, account: &str) {
        l.used_at = Instant::now();
        let kind = m.get("type").and_then(Value::as_str).unwrap_or_default();
        let subtype = m.get("subtype").and_then(Value::as_str).unwrap_or_default();
        let parent = m
            .get("parent_tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match (kind, subtype) {
            ("system", "init") => {
                let claude = m
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.change(
                    &l.id,
                    SessionChange {
                        materialized: Some(1),
                        claude: claude.map(Some),
                        ..SessionChange::default()
                    },
                )
                .await;
                l.crashes = 0;
            }
            ("system", "compact_boundary") => {
                let turn = l.turn.clone();
                self.note(
                    l,
                    turn.as_deref(),
                    "system",
                    json!({ "text": "Conversation compacted" }),
                )
                .await;
            }
            ("system", "background_tasks_changed") => {
                let n = m.get("tasks").and_then(Value::as_array).map_or(0, |t| {
                    t.iter()
                        .filter(|t| t.get("ambient") != Some(&Value::Bool(true)))
                        .count()
                });
                l.background = i64::try_from(n).unwrap_or(0);
                self.change(
                    &l.id,
                    SessionChange {
                        background: Some(l.background),
                        ..SessionChange::default()
                    },
                )
                .await;
            }
            ("system", "task_started" | "task_progress" | "task_notification" | "task_updated") => {
                return self.agent_seen(l, m).await;
            }
            ("system", "thinking_tokens") => {
                l.tok.think = m
                    .get("estimated_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                self.tokens(l, false);
            }
            ("rate_limit_event", _) => {
                let info = m.get("rate_limit_info").cloned().unwrap_or(Value::Null);
                self.accounts.observe(account, &info).await;
                if accounts::refused(&info) {
                    let kind = info
                        .get("rateLimitType")
                        .and_then(Value::as_str)
                        .unwrap_or("usage");
                    l.limit = Some((accounts::resets_of(&info), format!("hit its {kind} limit")));
                }
            }
            ("stream_event", _) if parent.is_none() => self.streamed(l, m),
            ("assistant", _) => self.assistant(l, m, account).await,
            ("user", _) => self.tool_results(l, m, parent).await,
            ("result", _) => self.end_of_turn(l, m).await,
            _ => {}
        }
    }

    async fn tool_results(&self, l: &mut Live, m: &Value, parent: Option<String>) {
        let Some(content) = m.pointer("/message/content").and_then(Value::as_array) else {
            return;
        };
        for c in content
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("tool_result"))
        {
            let out = match c.get("content") {
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter(|x| x.get("type").and_then(Value::as_str) == Some("text"))
                    .map(|x| text(x.get("text")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => text(other),
            };
            let error = c.get("is_error") == Some(&Value::Bool(true));
            let call = c
                .get("tool_use_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            // Long output keeps its start and its end (a command's last lines say how it went),
            // with its real line count.
            let chars: Vec<char> = out.chars().collect();
            let shown = if chars.len() > 4000 {
                format!(
                    "{}\n…\n{}",
                    chars[..2800].iter().collect::<String>(),
                    chars[chars.len() - 1200..].iter().collect::<String>()
                )
            } else {
                out.clone()
            };
            let lines = if out.is_empty() {
                0
            } else {
                out.split('\n').count()
            };
            let turn = l.turn.clone();
            self.note(l, turn.as_deref(), "result", json!({ "id": call, "error": error, "text": shown, "lines": lines, "parent": parent })).await;
            if parent.is_none()
                && !error
                && let Some(call) = call
            {
                self.todos_seen(l, "result", &call, None, Some(&out)).await;
            }
        }
    }

    /// Words and thinking as they are typed: to live watchers only, never stored.
    #[expect(
        clippy::many_single_char_names,
        reason = "the live state, the message and its parts, read side by side"
    )]
    fn streamed(&self, l: &mut Live, m: &Value) {
        let e = m.get("event").cloned().unwrap_or(Value::Null);
        let et = e.get("type").and_then(Value::as_str).unwrap_or_default();
        let block = e.pointer("/content_block/type").and_then(Value::as_str);
        let d = e.get("delta");
        let dt = d.and_then(|d| d.get("type")).and_then(Value::as_str);
        let send = |p: Value| self.watchers.send(&l.id, &p);
        match (et, block, dt) {
            ("content_block_start", Some("text"), _) => {
                l.partial.clear();
                send(json!({ "type": live::DELTA, "turn": l.turn, "start": true, "text": "" }));
            }
            ("content_block_start", Some("thinking"), _) => {
                l.tok.thought += l.tok.think;
                l.tok.think = 0;
                l.thinking.clear();
                let since = now();
                l.think_at = Some((Instant::now(), since.clone()));
                send(
                    json!({ "type": live::THINK, "turn": l.turn, "start": true, "text": "", "since": since }),
                );
            }
            ("content_block_delta", _, Some("text_delta")) => {
                let t = text(d.and_then(|d| d.get("text")));
                if !t.is_empty() {
                    l.partial.push_str(&t);
                    l.tok.chars += t.chars().count() as u64;
                    send(json!({ "type": live::DELTA, "turn": l.turn, "text": t }));
                }
            }
            ("content_block_delta", _, Some("thinking_delta")) => {
                let t = text(d.and_then(|d| d.get("thinking")));
                if !t.is_empty() {
                    l.thinking.push_str(&t);
                    send(json!({ "type": live::THINK, "turn": l.turn, "text": t }));
                }
            }
            ("content_block_delta", _, Some("input_json_delta")) => {
                l.tok.chars += text(d.and_then(|d| d.get("partial_json"))).chars().count() as u64;
            }
            ("message_delta", _, _) if e.pointer("/usage/output_tokens").is_some() => {
                // The call's real count replaces the estimate.
                let out = e
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                l.tok = Tok {
                    out: l.tok.out + out,
                    at: l.tok.at,
                    ..Tok::default()
                };
                return self.tokens(l, true);
            }
            _ => {}
        }
        self.tokens(l, false);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one message, every kind of block it can hold"
    )]
    async fn assistant(&self, l: &mut Live, m: &Value, account: &str) {
        if l.turn.is_none() {
            // The agent took a turn on its own (a background job it waited on ended).
            self.self_turn(l).await;
        }
        let per_token = self
            .accounts
            .get(Some(account))
            .is_some_and(|a| a.provider() != Provider::Claude);
        let later = |mins: u64| accounts::from_now(Duration::from_mins(mins));
        if l.limit.is_none() {
            l.limit = match m.get("error").and_then(Value::as_str) {
                Some("rate_limit") if per_token => {
                    Some((Some(later(15)), "was throttled by its provider".to_owned()))
                }
                Some("rate_limit") => Some((None, "hit its usage limit".to_owned())),
                Some(e @ ("billing_error" | "server_error")) if per_token => {
                    Some((Some(later(15)), format!("got {e} from its provider")))
                }
                Some(
                    e @ ("authentication_failed" | "oauth_org_not_allowed" | "account_on_hold"),
                ) => Some((Some(later(360)), format!("login problem ({e})"))),
                _ => None,
            };
        }
        let parent = m
            .get("parent_tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let content = m
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for c in content {
            let turn = l.turn.clone();
            match c.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let t = text(c.get("thinking"));
                    if parent.is_none() || !t.trim().is_empty() {
                        let ms = if parent.is_none() {
                            l.think_at.as_ref().map(|(at, _)| at.elapsed().as_millis())
                        } else {
                            None
                        };
                        self.note(
                            l,
                            turn.as_deref(),
                            "thinking",
                            json!({ "text": clip(&t, 8000), "ms": ms, "parent": parent }),
                        )
                        .await;
                        if parent.is_none() {
                            l.thinking.clear();
                            l.think_at = None;
                        }
                    }
                }
                Some("text") => {
                    let t = text(c.get("text"));
                    if !t.trim().is_empty() {
                        if parent.is_none() {
                            l.last_text.clone_from(&t);
                        }
                        self.note(
                            l,
                            turn.as_deref(),
                            "text",
                            json!({ "text": clip(&t, 12_000), "parent": parent }),
                        )
                        .await;
                    }
                }
                Some("tool_use") => {
                    let name = text(c.get("name"));
                    if name.is_empty() {
                        continue;
                    }
                    let input = c.get("input").cloned().unwrap_or_else(|| json!({}));
                    let call = c.get("id").and_then(Value::as_str).map(str::to_owned);
                    let now_ = doing(&name, &input);
                    let shown = serde_json::to_string_pretty(&input).unwrap_or_default();
                    self.note(
                        l,
                        turn.as_deref(),
                        "tool",
                        json!({ "id": call, "name": name, "summary": tool_summary(&name, &input), "doing": now_, "parent": parent,
                                "input": clip(&shown, 4000), "diff": diff_of(&name, &input) }),
                    )
                    .await;
                    if parent.is_none()
                        && let Some(call) = &call
                    {
                        self.todos_seen(l, &name, call, Some(&input), None).await;
                    }
                    l.doing = if parent.is_some() {
                        format!("Helper agent · {now_}")
                    } else {
                        now_.clone()
                    };
                    self.push_head(l).await;
                    if parent.is_none()
                        && l.told_at
                            .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
                    {
                        l.told_at = Some(Instant::now());
                        let t = self.turn_row(l.turn.as_deref()).await;
                        self.tell(
                            l,
                            ev::ACTIVITY,
                            json!({ "turn": Self::turn_shape(t.as_ref()), "doing": now_ }),
                        )
                        .await;
                    }
                }
                _ => {}
            }
        }
    }

    /// The agent started a turn by itself: it becomes a turn here too, so the server sees the
    /// session working.
    async fn self_turn(&self, l: &mut Live) {
        let Some(s) = self.session(&l.id).await else {
            return;
        };
        let n = s.seq + 1;
        let id = format!("{}:{n}", l.id);
        let t = turn::Model {
            id: id.clone(),
            session: l.id.clone(),
            n,
            prompt: Some(String::new()),
            by: Some("self".to_owned()),
            requested: Some(now()),
            started: Some(now()),
            ended: None,
            outcome: None,
            last: None,
            accounts: Some(json!([s.account]).to_string()),
        };
        if self.store.add_turn(t.clone()).await.is_err() {
            return;
        }
        self.change(
            &l.id,
            SessionChange {
                seq: Some(n),
                status: Some("running".to_owned()),
                ..SessionChange::default()
            },
        )
        .await;
        l.turn = Some(id);
        self.tell(
            l,
            ev::TURN_STARTED,
            json!({ "turn": Self::turn_shape(Some(&t)) }),
        )
        .await;
    }

    #[expect(
        clippy::many_single_char_names,
        reason = "the live state, the message and its parts, read side by side"
    )]
    async fn end_of_turn(&self, l: &mut Live, m: &Value) {
        if l.turn.is_none() {
            return;
        }
        // A result while this turn's message still waits in the agent's queue isn't this turn's
        // (a resumed session first wraps up a background job a restart cut off).
        if m.get("queued_turn_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
        {
            return;
        }
        let answer = Some(text(m.get("result")))
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| l.last_text.clone());
        let u = m.get("usage").cloned().unwrap_or(Value::Null);
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        l.usage = Some((
            n("input_tokens")
                + n("cache_creation_input_tokens")
                + n("cache_read_input_tokens")
                + n("output_tokens"),
            m.get("total_cost_usd").and_then(Value::as_f64),
        ));
        let is_error = m.get("is_error") == Some(&Value::Bool(true));
        // An error result saying so, never an answer that talks about limits.
        let limit_text = is_error && accounts::says_limit(&answer);
        let on = match self.session(&l.id).await {
            Some(s) => self.accounts.get(s.account.as_deref()),
            None => None,
        };
        if l.limit.is_none()
            && is_error
            && on.is_some_and(|a| a.provider() != Provider::Claude)
            && accounts::says_provider_down(&answer)
        {
            let flat: String = answer.split_whitespace().collect::<Vec<_>>().join(" ");
            l.limit = Some((
                Some(accounts::from_now(Duration::from_mins(15))),
                format!("was refused by its provider ({})", clip(&flat, 120)),
            ));
        }
        if !l.interrupting && !l.stopping && (l.limit.is_some() || limit_text) {
            let (until, why) = l
                .limit
                .clone()
                .unwrap_or((None, "hit its usage limit".to_owned()));
            return self.move_account(l, &why, until).await;
        }
        let outcome = if l.interrupting || l.stopping {
            "interrupted"
        } else if m.get("subtype").and_then(Value::as_str) == Some("success") && !is_error {
            "completed"
        } else {
            "failed"
        };
        let last = if !answer.is_empty() || outcome != "failed" {
            answer
        } else {
            let errors: Vec<String> = m
                .get("errors")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |e| e.iter().map(|x| text(Some(x))).collect());
            if errors.is_empty() {
                format!(
                    "Turn ended: {}",
                    m.get("subtype").and_then(Value::as_str).unwrap_or("error")
                )
            } else {
                errors.join("; ")
            }
        };
        self.finish_turn(l, outcome, &last).await;
    }

    fn finish_turn<'a>(&'a self, l: &'a mut Live, outcome: &'a str, last: &'a str) -> Boxed<'a> {
        Box::pin(async move {
            let Some(id) = l.turn.clone() else { return };
            let Some(mut t) = self.turn_row(Some(&id)).await else {
                return;
            };
            t.ended = Some(now());
            t.outcome = Some(outcome.to_owned());
            t.last = Some(last.to_owned());
            self.save_turn(t.clone()).await;
            // The owner's session that asked a question waits for the answer.
            let status = match outcome {
                "completed" if l.asked => "waiting",
                "cancelled" => "interrupted",
                o => o,
            };
            self.change(
                &l.id,
                SessionChange {
                    status: Some(status.to_owned()),
                    ..SessionChange::default()
                },
            )
            .await;
            self.note(l, Some(&id), "end", json!({ "outcome": outcome, "n": t.n }))
                .await;
            l.turn_ended();
            let mut extra = obj(json!({ "turn": Self::turn_shape(Some(&t)) }));
            if let Some((tokens, cost)) = l.usage.take() {
                extra.insert(
                    "usage".to_owned(),
                    json!({ "tokens": tokens, "cost": cost }),
                );
            }
            self.tell(l, ev::TURN_FINISHED, Value::Object(extra)).await;
            if l.stopping {
                l.stopping = false;
                self.close(l, "stopped").await;
                return;
            }
            if l.reopen {
                // Its next turn opens with the model and effort just set.
                self.close(l, "new settings").await;
            }
            self.pump(l).await;
        })
    }

    /// An owner's session asks a question: it goes to the app as a card, the agent is told to
    /// end its turn, and the session waits.
    async fn ask_owner(&self, l: &mut Live, request_id: &str, input: &Value, call: Option<String>) {
        let qs = input
            .get("questions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let questions: Vec<Value> = qs
            .iter()
            .map(|q| {
                let options: Vec<Value> = q
                    .get("options")
                    .and_then(Value::as_array)
                    .map_or_else(Vec::new, |os| {
                        os.iter()
                            .map(|o| {
                                let d = text(o.get("description"));
                                json!({ "label": clip(&text(o.get("label")), 80), "description": (!d.is_empty()).then(|| clip(&d, 300)) })
                            })
                            .collect()
                    })
                    ;
                json!({ "question": clip(&text(q.get("question")), 600), "header": q.get("header"),
                        "multiSelect": q.get("multiSelect") == Some(&Value::Bool(true)), "options": options })
            })
            .collect();
        let turn = l.turn.clone();
        self.note(
            l,
            turn.as_deref(),
            "question",
            json!({ "id": call, "questions": questions }),
        )
        .await;
        l.asked = true;
        let t = self.turn_row(l.turn.as_deref()).await;
        let words: Vec<String> = qs.iter().map(|q| text(q.get("question"))).collect();
        self.tell(
            l,
            ev::QUESTION,
            json!({ "turn": Self::turn_shape(t.as_ref()), "text": words.join(" / ") }),
        )
        .await;
        if let Some(agent) = &l.agent
            && let Err(e) = agent
                .answer(request_id, Err(prompts::ASKED.to_owned()))
                .await
        {
            tracing::warn!(session = %l.id, "cannot answer the question's permission: {e}");
        }
    }

    /// The account ran out mid-turn: it's marked out, and the session moves to the account with
    /// the most room and carries on with the same turn. None left: the turn fails, and the
    /// session carries on when the first account is free again.
    async fn move_account(&self, l: &mut Live, why: &str, until: Option<String>) {
        let Some(s) = self.session(&l.id).await else {
            return;
        };
        let from = s.account.clone();
        if let Some(f) = &from {
            self.accounts.mark_limited(f, until, why).await;
        }
        let work = self.work(&s);
        let next = self.accounts.pick(&Work {
            exclude: from.iter().cloned().collect(),
            ..work.clone()
        });
        let from_name = self.name_of(from.as_deref());
        let t = self.turn_row(l.turn.as_deref()).await;
        let Some(next) = next else {
            let free = self.accounts.next_free(&work);
            let at = free
                .clone()
                .unwrap_or_else(|| accounts::from_now(Duration::from_hours(1)));
            self.change(
                &l.id,
                SessionChange {
                    resume_at: Some(Some(at)),
                    ..SessionChange::default()
                },
            )
            .await;
            let when = free
                .as_ref()
                .map_or_else(|| "in an hour".to_owned(), |f| format!("at {f}"));
            let turn = l.turn.clone();
            self.note(l, turn.as_deref(), "system", json!({ "text": format!("{from_name} {why}; every other account is out too. Carrying on {when}.") })).await;
            self.tell(l, ev::ACCOUNT, json!({ "turn": Self::turn_shape(t.as_ref()), "from": from, "to": null, "why": why, "resumeAt": free, "fromName": from_name })).await;
            let last = format!(
                "Stopped: {from_name} {why} and every other account is out too. It carries on by itself {when}."
            );
            return self.finish_turn(l, "failed", &last).await;
        };
        self.close(l, &format!("moving to {}", next.id)).await;
        self.change(
            &l.id,
            SessionChange {
                account: Some(Some(next.id.clone())),
                ..SessionChange::default()
            },
        )
        .await;
        if let Some(mut t) = t.clone() {
            let mut list: Vec<Value> = t
                .accounts
                .as_deref()
                .and_then(|a| serde_json::from_str(a).ok())
                .unwrap_or_default();
            list.push(json!(next.id));
            t.accounts = Some(Value::Array(list).to_string());
            self.save_turn(t).await;
        }
        let turn = l.turn.clone();
        self.note(l, turn.as_deref(), "system", json!({ "text": format!("Moved from {from_name} to {}: {from_name} {why}", next.name) })).await;
        let t = self.turn_row(l.turn.as_deref()).await;
        self.tell(l, ev::ACCOUNT, json!({ "turn": Self::turn_shape(t.as_ref()), "from": from, "to": next.id, "why": why, "fromName": from_name, "toName": next.name })).await;
        tracing::info!(session = %l.id, "{} {why}; moved to {}", from.as_deref().unwrap_or("?"), next.id);
        l.limit = None;
        self.restart_turn(l, prompts::MOVED).await;
    }

    /// The same turn, carried on in a new agent process (after a move, a restart or a crash).
    async fn restart_turn(&self, l: &mut Live, why: &str) {
        match self.open_agent(l).await {
            Ok((_, resumed)) => {
                // No transcript yet (it stopped before its first message was written): the
                // turn's own message again.
                let words = if resumed {
                    why.to_owned()
                } else {
                    self.turn_row(l.turn.as_deref())
                        .await
                        .and_then(|t| t.prompt)
                        .filter(|p| !p.is_empty())
                        .unwrap_or_else(|| why.to_owned())
                };
                if let Err(e) = self.say(l, &words).await {
                    self.finish_turn(l, "failed", &e).await;
                }
            }
            Err(e) => self.finish_turn(l, "failed", &e).await,
        }
    }

    /// Between turns an open session goes where it should be now: onto the default account when
    /// one is set, off one nearly full. Not while its background jobs or sub-agents run (they
    /// live in that process).
    async fn rehome(&self, l: &mut Live) {
        if l.agent.is_none() || l.background > 0 || l.agents.values().any(|a| a.running) {
            return;
        }
        let Some(s) = self.session(&l.id).await else {
            return;
        };
        let Some(to) = self.account_for(&s) else {
            return;
        };
        if Some(&to.id) == s.account.as_ref() {
            return;
        }
        let why = if Some(&to.id) == self.accounts.default_account().as_ref() {
            format!("{} is the default account", to.name)
        } else {
            s.account
                .as_deref()
                .map_or_else(|| "no account".to_owned(), |a| self.accounts.low_why(a))
        };
        self.close(l, &format!("moving to {}", to.id)).await;
        let from_name = self.name_of(s.account.as_deref());
        self.note(
            l,
            None,
            "system",
            json!({ "text": format!("Moved from {from_name} to {}: {why}", to.name) }),
        )
        .await;
        self.tell(l, ev::ACCOUNT, json!({ "from": s.account, "to": to.id, "why": why, "fromName": from_name, "toName": to.name })).await;
    }

    /// The next queued turn starts when nothing runs.
    fn pump<'a>(&'a self, l: &'a mut Live) -> Boxed<'a> {
        Box::pin(async move {
            if l.turn.is_some() {
                return;
            }
            let Ok(Some(mut t)) = self.store.next_turn(&l.id).await else {
                if let Some(me) = self.me.upgrade() {
                    let id = l.id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(IDLE + Duration::from_secs(1)).await;
                        me.idle_close(&id).await;
                    });
                }
                return;
            };
            self.rehome(l).await;
            l.turn = Some(t.id.clone());
            t.started = Some(now());
            self.save_turn(t.clone()).await;
            if l.agent.is_none()
                && let Err(e) = self.open_agent(l).await
            {
                return self.finish_turn(l, "failed", &e).await;
            }
            let account = self.session(&l.id).await.and_then(|s| s.account);
            t.accounts = Some(json!([account]).to_string());
            self.save_turn(t.clone()).await;
            self.change(
                &l.id,
                SessionChange {
                    status: Some("running".to_owned()),
                    ..SessionChange::default()
                },
            )
            .await;
            if let Err(e) = self.say(l, t.prompt.as_deref().unwrap_or_default()).await {
                return self.finish_turn(l, "failed", &e).await;
            }
            self.tell(
                l,
                ev::TURN_STARTED,
                json!({ "turn": Self::turn_shape(Some(&t)) }),
            )
            .await;
        })
    }

    async fn idle_close(&self, id: &str) {
        let live = self.live(id);
        let mut l = live.lock().await;
        if l.agent.is_some() && l.turn.is_none() && l.background == 0 && l.used_at.elapsed() >= IDLE
        {
            self.close(&mut l, "idle").await;
        }
    }

    // ── Sub-agents, tokens and to-dos (for the live head) ──────────────────────────────────

    async fn agent_seen(&self, l: &mut Live, m: &Value) {
        if let Some(tt) = m.get("task_type").and_then(Value::as_str)
            && !(tt.contains("agent") || tt.contains("workflow"))
        {
            // A background shell command.
            return;
        }
        let subtype = m.get("subtype").and_then(Value::as_str).unwrap_or_default();
        let key = text(m.get("task_id"));
        if !l.agents.contains_key(&key)
            && subtype == "task_started"
            && let Some(call) = m.get("tool_use_id").and_then(Value::as_str)
        {
            l.agents.insert(
                key.clone(),
                AgentInfo {
                    id: call.to_owned(),
                    description: String::new(),
                    tokens: 0,
                    tools: 0,
                    last: None,
                    running: true,
                    started: now(),
                    ended: None,
                },
            );
        }
        let Some(i) = l.agents.get_mut(&key) else {
            return;
        };
        let description = text(m.get("description"));
        if !description.is_empty() && (subtype == "task_started" || i.description.is_empty()) {
            i.description = clip(&description, 120);
        }
        if let Some(u) = m.get("usage") {
            i.tokens = u
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(i.tokens);
            i.tools = u
                .get("tool_uses")
                .and_then(Value::as_u64)
                .unwrap_or(i.tools);
        }
        if subtype == "task_progress" {
            let last = [
                text(m.get("summary")),
                description,
                text(m.get("last_tool_name")),
            ]
            .into_iter()
            .find(|s| !s.is_empty());
            if let Some(last) = last {
                i.last = Some(clip(&last, 120));
            }
        }
        let status = m.pointer("/patch/status").and_then(Value::as_str);
        if i.running
            && (subtype == "task_notification"
                || (subtype == "task_updated" && status.is_some_and(|s| s != "running")))
        {
            i.running = false;
            i.ended = Some(Instant::now());
        }
        self.push_head(l).await;
    }

    /// The turn's output tokens so far, to live watchers at most every 400 ms (and at once when
    /// a call's real count lands).
    fn tokens(&self, l: &mut Live, at_once: bool) {
        if l.turn.is_none()
            || !self.watchers.has(&l.id)
            || (!at_once
                && l.tok
                    .at
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(400)))
        {
            return;
        }
        l.tok.at = Some(Instant::now());
        self.watchers.send(
            &l.id,
            &json!({ "type": live::TOKENS, "turn": l.turn, "tokens": l.tokens() }),
        );
    }

    /// Its to-do list, as a `todos` step each time it changes: `TodoWrite` gives the whole list;
    /// `TaskCreate` adds one (its id comes back in the result: "Task #3 created…"), `TaskUpdate`
    /// changes one. After a restart it carries on from the last one.
    async fn todos_seen(
        &self,
        l: &mut Live,
        name: &str,
        call: &str,
        input: Option<&Value>,
        result: Option<&str>,
    ) {
        if l.todos.is_none() {
            let newest = self
                .store
                .last_event_of(&l.id, "todos")
                .await
                .ok()
                .flatten();
            let list = newest
                .and_then(|e| e.data)
                .and_then(|d| serde_json::from_str::<Value>(&d).ok())
                .and_then(|v| v.get("todos").and_then(Value::as_array).cloned())
                .unwrap_or_default();
            l.todos = Some(list);
        }
        let mut list = l.todos.clone().unwrap_or_default();
        let field = |k: &str| input.and_then(|i| i.get(k));
        match name {
            "TodoWrite" if field("todos").is_some_and(Value::is_array) => {
                list = field("todos")
                    .and_then(Value::as_array)
                    .map_or_else(Vec::new, |ts| {
                        ts.iter()
                            .map(|t| {
                                let mut o = obj(json!({ "content": clip(&text(t.get("content")), 300), "status": text(t.get("status")) }));
                                let af = text(t.get("activeForm"));
                                if !af.is_empty() {
                                    o.insert("activeForm".to_owned(), json!(clip(&af, 120)));
                                }
                                Value::Object(o)
                            })
                            .collect()
                    })
                    ;
            }
            "TaskCreate" => {
                let mut o = obj(
                    json!({ "content": clip(&text(field("subject")), 300), "status": "pending" }),
                );
                let af = text(field("activeForm"));
                if !af.is_empty() {
                    o.insert("activeForm".to_owned(), json!(clip(&af, 120)));
                }
                l.creating.insert(call.to_owned(), Value::Object(o));
                return;
            }
            "TaskUpdate" if field("taskId").is_some_and(|v| !v.is_null()) => {
                let id = text(field("taskId"));
                let status = text(field("status"));
                if status == "deleted" {
                    list.retain(|t| text(t.get("id")) != id);
                } else {
                    for t in list.iter_mut().filter(|t| text(t.get("id")) == id) {
                        if let Value::Object(o) = t {
                            if !status.is_empty() {
                                o.insert("status".to_owned(), json!(status));
                            }
                            let subject = text(field("subject"));
                            if !subject.is_empty() {
                                o.insert("content".to_owned(), json!(clip(&subject, 300)));
                            }
                            let af = text(field("activeForm"));
                            if !af.is_empty() {
                                o.insert("activeForm".to_owned(), json!(clip(&af, 120)));
                            }
                        }
                    }
                }
            }
            "result" if l.creating.contains_key(call) => {
                let Some(mut t) = l.creating.remove(call) else {
                    return;
                };
                let tid = result.and_then(|r| r.split('#').nth(1)).map(|r| {
                    r.chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>()
                });
                if let (Some(tid), Value::Object(o)) = (tid.filter(|t| !t.is_empty()), &mut t) {
                    o.insert("id".to_owned(), json!(tid));
                }
                list.push(t);
            }
            _ => return,
        }
        l.todos = Some(list.clone());
        let turn = l.turn.clone();
        self.note(l, turn.as_deref(), "todos", json!({ "todos": list }))
            .await;
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────────────

    /// Opens a session: in a folder (and its own worktree when asked), on an account that may
    /// take its work, with its first turn.
    ///
    /// # Errors
    /// The folder, model, effort or account is wrong, or no account can take it now.
    #[expect(
        clippy::too_many_lines,
        reason = "every check before a session is made, in the order they refuse"
    )]
    pub async fn open(&self, b: &OpenAsk) -> Result<Value, Refused> {
        let raw = text(b.get("cwd"));
        let user = self.config.user.clone();
        let cwd = if raw == "~" || raw.starts_with("~/") {
            crate::config::untilde(&raw, &user).display().to_string()
        } else {
            raw
        };
        if cwd.is_empty() || !Path::new(&cwd).is_dir() {
            return Err(Refused::bad(format!(
                "no folder {} on {}",
                if cwd.is_empty() { "(none given)" } else { &cwd },
                self.config.name
            )));
        }
        let meta = b
            .get("meta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let direct = meta.get("direct") == Some(&Value::Bool(true));
        let model = model_of(b.get("model"))?;
        let effort = effort_of(b.get("effort"))?;
        let project = Some(text(b.get("project"))).filter(|p| !p.is_empty());
        let first = text(b.get("prompt"));
        if let Some(why) = self.accounts.refusal(project.as_deref(), Some(&cwd)) {
            return Err(Refused::conflict(why));
        }
        let id = format!("r-{}", hex(4));
        let first_line: String = first
            .trim()
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(70)
            .collect();
        let folder_name = Path::new(&cwd)
            .file_name()
            .map_or_else(Default::default, |n| n.to_string_lossy().into_owned());
        let given = text(b.get("title"));
        let title: String = (if !given.is_empty() {
            given
        } else if !first_line.is_empty() {
            first_line
        } else if direct {
            folder_name
        } else {
            "Session".to_owned()
        })
        .chars()
        .take(120)
        .collect();
        let mut prompt = first;
        let mut dir = cwd.clone();
        let mut wt = None;
        if let Some(ask) = b.get("worktree").and_then(WorktreeAsk::from_json) {
            let w = folders::make_worktree(
                Path::new(&cwd),
                &ask,
                &title,
                &self.config.worktrees_dir,
                &self.config.branch_prefix,
            )
            .await?;
            if same_dir(&w.top, Path::new(&cwd)) {
                dir = w.dir.display().to_string();
            } else {
                write!(
                    prompt,
                    "\n\nYour git worktree for this work: {} (branch {}, from {}). Make your changes there, not in the main checkout.",
                    w.dir.display(),
                    w.branch,
                    w.top.display()
                )
                .unwrap_or_default();
            }
            wt = Some(w);
        }
        let asked = text(b.get("account"));
        let want = if asked.is_empty() {
            None
        } else {
            Some(self.accounts.get(Some(&asked)).ok_or_else(|| {
                Refused::bad(format!("no account {asked} on {}", self.config.name))
            })?)
        };
        // An opt-in account asked for by id pins the session there; a model only one provider
        // runs keeps it there. Either one that can't take it now is refused, never started
        // quietly elsewhere.
        let pin = want
            .as_ref()
            .filter(|w| w.is_opt_in())
            .map(|w| w.id.clone());
        let only = self.accounts.only_for(pin.as_deref(), model.as_deref());
        if let (Some(w), Some(o)) = (&want, &only)
            && !o.contains(&w.id)
        {
            return Err(Refused::conflict(format!(
                "{} doesn't run on {}",
                model.as_deref().unwrap_or("that model"),
                w.name
            )));
        }
        let work = Work {
            project: project.clone(),
            cwd: Some(cwd.clone()),
            opt_in: only.is_some(),
            only: only.clone(),
            model: model.clone().or_else(|| self.config.model.clone()),
            ..Work::default()
        };
        let ranked = self.accounts.ranked(&work);
        let a = match want {
            Some(w) if ranked.iter().any(|r| r.account.id == w.id) => Some(w),
            _ => ranked.into_iter().next().map(|r| r.account),
        };
        let Some(a) = a else {
            if let Some(o) = &only {
                let names: Vec<String> = o.iter().map(|x| self.name_of(Some(x))).collect();
                let names = if names.is_empty() {
                    "no listed account".to_owned()
                } else {
                    names.join(", ")
                };
                return Err(Refused::conflict(format!(
                    "{names} can't take work now: not started on another account"
                )));
            }
            let free = self
                .accounts
                .next_free(&work)
                .map_or_else(String::new, |f| format!(" (the first is free at {f})"));
            return Err(Refused::conflict(format!("no account has room now{free}")));
        };
        let mut keep = meta;
        if only.is_some() {
            keep.insert("optIn".to_owned(), Value::Bool(true));
        }
        if let Some(p) = &pin {
            keep.insert("pin".to_owned(), json!(p));
        }
        let s = session::Model {
            id: id.clone(),
            title: Some(title.clone()),
            cwd: dir.clone(),
            project,
            model: model.clone(),
            effort,
            account: Some(a.id.clone()),
            claude: Some(uuid()),
            materialized: 0,
            status: if prompt.trim().is_empty() {
                "idle"
            } else {
                "queued"
            }
            .to_owned(),
            worktree: wt.as_ref().map(|w| w.dir.display().to_string()),
            branch: wt.as_ref().map(|w| w.branch.clone()),
            meta: (!keep.is_empty()).then(|| Value::Object(keep).to_string()),
            seq: 0,
            resume_at: None,
            created: Some(now()),
            updated: Some(now()),
            background: 0,
            origin: Some(cwd),
        };
        self.store
            .add_session(s.clone())
            .await
            .map_err(|e| db(&e))?;
        tracing::info!(session = %id, "\"{title}\" in {dir} on {}{}", a.id, if direct { " (the owner's)" } else { "" });
        {
            let live = self.live(&id);
            let l = live.lock().await;
            self.tell(&l, ev::SESSION, json!({})).await;
        }
        let turn = if prompt.trim().is_empty() {
            Value::Null
        } else {
            let by = Some(text(b.get("by")))
                .filter(|b| !b.is_empty())
                .unwrap_or_else(|| wire::DEFAULT_BY.to_owned());
            json!(self.send(&id, &prompt, &by, b.get("files")).await?.0)
        };
        let s = self.session(&id).await.unwrap_or(s);
        let mut out = self.shape(&s);
        out.insert("turn".to_owned(), turn);
        Ok(Value::Object(out))
    }

    /// Queues a turn: it runs when the one before it ends. Files that arrived since the last
    /// turn are named in it. Returns the turn's id and whether it waits behind another.
    ///
    /// # Errors
    /// No such session, or nothing to send.
    pub async fn send(
        &self,
        id: &str,
        said: &str,
        by: &str,
        files: Option<&Value>,
    ) -> Result<(String, bool), Refused> {
        let live = self.live(id);
        let mut l = live.lock().await;
        let s = self
            .session(id)
            .await
            .ok_or_else(|| Refused::missing(format!("no session {id} on {}", self.config.name)))?;
        if said.trim().is_empty() {
            return Err(Refused::bad("nothing to send"));
        }
        let who = if by.is_empty() { wire::DEFAULT_BY } else { by };
        let direct = is_direct(&s);
        // The agent knows it isn't the owner typing.
        let mut prompt = if who == "assistant" && direct {
            format!("{}{said}", wire::FROM_ASSISTANT)
        } else {
            said.to_owned()
        };
        let arrived = self.files.arrived(id).await;
        if !arrived.is_empty() {
            let list: Vec<String> = arrived
                .iter()
                .map(|f| format!("{} ({}, from {})", f.path, mb(f.size), f.by))
                .collect();
            write!(
                prompt,
                "\n\n(Files sent to you since your last turn, in {}: {})",
                self.files.inbox_of(id).display(),
                list.join(", ")
            )
            .unwrap_or_default();
        }
        let n = s.seq + 1;
        let tid = format!("{id}:{n}");
        let t = turn::Model {
            id: tid.clone(),
            session: id.to_owned(),
            n,
            prompt: Some(prompt.clone()),
            by: Some(who.to_owned()),
            requested: Some(now()),
            started: None,
            ended: None,
            outcome: None,
            last: None,
            accounts: None,
        };
        self.store.add_turn(t.clone()).await.map_err(|e| db(&e))?;
        let queued = l.turn.is_some();
        self.change(
            id,
            SessionChange {
                seq: Some(n),
                resume_at: Some(None),
                status: (!queued).then(|| "queued".to_owned()),
                ..SessionChange::default()
            },
        )
        .await;
        let shown: Vec<Value> = files.and_then(Value::as_array).map_or_else(Vec::new, |fs| {
            fs.iter()
                .filter(|f| f.get("url").is_some_and(Value::is_string))
                .take(20)
                .map(|f| {
                    let name = Some(text(f.get("name")))
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| "file".to_owned());
                    let kind = Some(text(f.get("kind")))
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| "file".to_owned());
                    json!({ "name": name, "url": text(f.get("url")), "kind": kind })
                })
                .collect()
        });
        let mut step =
            obj(json!({ "text": clip(if direct { said } else { &prompt }, 12_000), "by": who }));
        if !shown.is_empty() {
            step.insert("files".to_owned(), Value::Array(shown));
        }
        self.note(&mut l, Some(&tid), "user", Value::Object(step))
            .await;
        self.tell(
            &l,
            ev::TURN_QUEUED,
            json!({ "turn": Self::turn_shape(Some(&t)) }),
        )
        .await;
        self.pump(&mut l).await;
        Ok((tid, queued))
    }

    /// Ends the running turn; with `stop` the queued turns go too and the process closes after
    /// it. Whether a turn was running.
    ///
    /// # Errors
    /// No such session.
    pub async fn interrupt(&self, id: &str, stop: bool) -> Result<bool, Refused> {
        let live = self.live(id);
        let agent = {
            let mut l = live.lock().await;
            if self.session(id).await.is_none() {
                return Err(Refused::missing(format!(
                    "no session {id} on {}",
                    self.config.name
                )));
            }
            if stop {
                for mut t in self.store.waiting_turns(id).await.map_err(|e| db(&e))? {
                    t.ended = Some(now());
                    t.outcome = Some("cancelled".to_owned());
                    self.save_turn(t.clone()).await;
                    self.tell(
                        &l,
                        ev::TURN_FINISHED,
                        json!({ "turn": Self::turn_shape(Some(&t)) }),
                    )
                    .await;
                }
                self.change(
                    id,
                    SessionChange {
                        resume_at: Some(None),
                        ..SessionChange::default()
                    },
                )
                .await;
            }
            if l.turn.is_none() {
                if stop {
                    self.close(&mut l, "stopped").await;
                }
                return Ok(false);
            }
            if stop {
                l.stopping = true;
            } else {
                l.interrupting = true;
            }
            let Some(agent) = l.agent.clone() else {
                self.finish_turn(&mut l, "interrupted", "Stopped").await;
                return Ok(true);
            };
            agent
        };
        // Outside the session's lock: the agent's answer comes through its messages, which take it.
        if let Err(e) = agent.interrupt().await {
            tracing::warn!(
                session = id,
                "the agent didn't take the interrupt ({e}); stopping its process"
            );
            let mut l = live.lock().await;
            if l.turn.is_some() {
                self.close(&mut l, "interrupt not taken").await;
                self.finish_turn(&mut l, "interrupted", "Stopped").await;
            }
        }
        Ok(true)
    }

    /// The owner changes a session's model, effort, title or account: its next turn opens with
    /// them (a running turn ends first). Returns the session's new head.
    ///
    /// # Errors
    /// No such session, or a setting is wrong.
    pub async fn settings(&self, id: &str, b: &Map<String, Value>) -> Result<Value, Refused> {
        let live = self.live(id);
        let mut l = live.lock().await;
        let s = self
            .session(id)
            .await
            .ok_or_else(|| Refused::missing(format!("no session {id} on {}", self.config.name)))?;
        let effort = match b.get("effort") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => Some(effort_of(Some(v))?),
        };
        let model = match b.get("model") {
            None => None,
            Some(v) => Some(model_of(Some(v))?),
        };
        let account = text(b.get("account"));
        let to = if account.is_empty() {
            None
        } else {
            Some(
                self.move_to(
                    &s,
                    &account,
                    model
                        .clone()
                        .flatten()
                        .or_else(|| s.model.clone())
                        .as_deref(),
                )?,
            )
        };
        let title = text(b.get("title"));
        self.change(
            id,
            SessionChange {
                model,
                effort,
                title: (!title.is_empty()).then(|| title.chars().take(120).collect()),
                ..SessionChange::default()
            },
        )
        .await;
        if let Some(to) = to {
            let mut meta = meta_of(&s);
            meta.remove("optIn");
            meta.remove("pin");
            if to.is_opt_in() {
                meta.insert("optIn".to_owned(), Value::Bool(true));
                meta.insert("pin".to_owned(), json!(to.id));
            }
            self.change(
                id,
                SessionChange {
                    account: Some(Some(to.id.clone())),
                    meta: Some(Some(Value::Object(meta).to_string())),
                    ..SessionChange::default()
                },
            )
            .await;
            let from_name = self.name_of(s.account.as_deref());
            self.note(
                &mut l,
                None,
                "system",
                json!({ "text": format!("Moved from {from_name} to {}: asked for", to.name) }),
            )
            .await;
            self.tell(&l, ev::ACCOUNT, json!({ "from": s.account, "to": to.id, "why": "asked for", "fromName": from_name, "toName": to.name })).await;
        }
        if l.turn.is_some() {
            l.reopen = true;
        } else {
            self.close(&mut l, "new settings").await;
        }
        self.tell(&l, ev::NOTE, json!({ "text": "Settings changed" }))
            .await;
        self.head_locked(&l)
            .await
            .ok_or_else(|| Refused::missing(format!("no session {id}")))
    }

    fn move_to(
        &self,
        s: &session::Model,
        id: &str,
        model: Option<&str>,
    ) -> Result<Account, Refused> {
        let a = self
            .accounts
            .get(Some(id))
            .ok_or_else(|| Refused::bad(format!("no account {id} on {}", self.config.name)))?;
        let only = self
            .accounts
            .only_for(a.is_opt_in().then_some(a.id.as_str()), model);
        if only.as_ref().is_some_and(|o| !o.contains(&a.id)) {
            return Err(Refused::conflict(format!(
                "{} doesn't run on {}",
                model.unwrap_or("that model"),
                a.name
            )));
        }
        let w = Work {
            opt_in: a.is_opt_in(),
            only: Some(vec![a.id.clone()]),
            ..self.work(s)
        };
        if self.accounts.ranked(&w).is_empty() {
            return Err(Refused::conflict(format!(
                "{} can't take this session now ({})",
                a.name,
                self.accounts.low_why(&a.id)
            )));
        }
        Ok(a)
    }

    // ── Start, timers and the watchdog ──────────────────────────────────────────────────────

    /// Carries on what a restart cut off: running turns resume, sessions that waited on
    /// background jobs hear they stopped, queued turns run. Then the timers start.
    pub async fn boot(self: &Arc<Self>) {
        for t in self.store.cut_off_turns().await.unwrap_or_default() {
            let live = self.live(&t.session);
            let mut l = live.lock().await;
            if l.turn.is_some() {
                continue;
            }
            l.turn = Some(t.id.clone());
            self.note(
                &mut l,
                Some(&t.id),
                "system",
                json!({ "text": "The runner restarted; carrying on" }),
            )
            .await;
            self.tell(&l, ev::NOTE, json!({ "turn": Self::turn_shape(Some(&t)), "text": "The runner restarted mid-turn; the session carries on from where it was." })).await;
            tracing::info!(session = %t.session, "resuming turn {} after a restart", t.n);
            self.restart_turn(&mut l, prompts::RESTARTED).await;
        }
        for s in self
            .store
            .sessions_with_background()
            .await
            .unwrap_or_default()
        {
            self.change(
                &s.id,
                SessionChange {
                    background: Some(0),
                    ..SessionChange::default()
                },
            )
            .await;
            let running = self.live(&s.id).lock().await.turn.is_some();
            if !running && let Err(e) = self.send(&s.id, prompts::JOBS_LOST, "runner", None).await {
                tracing::warn!(session = %s.id, "{e}");
            }
        }
        for id in self
            .store
            .sessions_with_waiting_turns()
            .await
            .unwrap_or_default()
        {
            let live = self.live(&id);
            let mut l = live.lock().await;
            self.pump(&mut l).await;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(RESUME_EVERY);
            loop {
                every.tick().await;
                me.resume_due().await;
            }
        });
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(WATCH_EVERY);
            loop {
                every.tick().await;
                me.watchdog().await;
            }
        });
    }

    /// Sessions waiting for a free account carry on when one is.
    async fn resume_due(&self) {
        for s in self.store.sessions_due(&now()).await.unwrap_or_default() {
            if self.accounts.pick(&self.work(&s)).is_none() {
                continue;
            }
            self.change(
                &s.id,
                SessionChange {
                    resume_at: Some(None),
                    ..SessionChange::default()
                },
            )
            .await;
            if let Err(e) = self.send(&s.id, prompts::FREE_AGAIN, "runner", None).await {
                tracing::warn!(session = %s.id, "{e}");
            }
        }
    }

    /// A running turn whose agent has gone quiet for half an hour (no message at all, not
    /// waiting on background jobs or sub-agents) is interrupted and carried on in a new process.
    async fn watchdog(&self) {
        for live in self.all_lives() {
            let Ok(mut l) = live.try_lock() else { continue };
            let quiet = l.turn.is_some()
                && l.background == 0
                && !l.agents.values().any(|a| a.running)
                && l.used_at.elapsed() > QUIET;
            if !quiet {
                continue;
            }
            tracing::warn!(session = %l.id, "no word from the agent in {} minutes; carrying the turn on in a new process", QUIET.as_secs() / 60);
            let turn = l.turn.clone();
            self.note(
                &mut l,
                turn.as_deref(),
                "system",
                json!({ "text": "The agent went quiet; carrying on in a new process" }),
            )
            .await;
            self.close(&mut l, "quiet").await;
            l.used_at = Instant::now();
            self.restart_turn(&mut l, prompts::RESTARTED).await;
        }
    }

    /// What the machine sample says of the sessions.
    pub async fn summary(&self) -> Summary {
        let mut running = 0;
        let mut open = 0;
        let mut busy = HashMap::new();
        for live in self.all_lives() {
            let l = live.lock().await;
            running += u64::from(l.turn.is_some());
            open += u64::from(l.agent.is_some());
            busy.insert(l.id.clone(), l.turn.is_some());
        }
        let rows = self.store.sessions(200).await.unwrap_or_default();
        let list = rows
            .iter()
            .filter(|r| matches!(r.status.as_str(), "running" | "queued" | "waiting") || busy.get(&r.id) == Some(&true))
            .take(40)
            .map(|r| json!({ "id": r.id, "title": clip(r.title.as_deref().unwrap_or_default(), 120), "status": r.status, "account": r.account, "running": busy.get(&r.id) == Some(&true) }))
            .collect();
        Summary {
            running,
            open,
            queued: self.store.queued_all().await.unwrap_or(0),
            list,
        }
    }

    /// How many turns run now.
    pub async fn running_count(&self) -> u64 {
        let mut n = 0;
        for live in self.all_lives() {
            n += u64::from(live.lock().await.turn.is_some());
        }
        n
    }

    /// Closes every agent process (running turns resume at the next start).
    pub async fn shutdown(&self) {
        for live in self.all_lives() {
            let mut l = live.lock().await;
            if let Some(agent) = l.agent.take() {
                l.generation = 0;
                agent.close().await;
            }
        }
        self.watchers.close_all();
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Reads one agent process's output for as long as it runs.
fn listen(
    me: Arc<Sessions>,
    id: String,
    generation: u64,
    mut rx: mpsc::UnboundedReceiver<AgentOutput>,
    account: String,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    // Boxed: a listener can open the next agent process, which starts the next listener.
    Box::pin(async move {
        let live = me.live(&id);
        while let Some(out) = rx.recv().await {
            let mut l = live.lock().await;
            if l.generation != generation {
                return;
            }
            match out {
                AgentOutput::Message(m) => me.on_message(&mut l, &m, &account).await,
                AgentOutput::Permission {
                    request_id,
                    tool,
                    input,
                    tool_use_id,
                } => {
                    if tool == "AskUserQuestion" {
                        me.ask_owner(&mut l, &request_id, &input, tool_use_id).await;
                    } else if let Some(agent) = &l.agent
                        && let Err(e) = agent.answer(&request_id, Ok(input)).await
                    {
                        tracing::warn!(session = %id, "cannot answer a permission question: {e}");
                    }
                }
                AgentOutput::Exited { why } => {
                    me.on_exit(generation, &mut l, &why).await;
                    return;
                }
            }
        }
        let mut l = live.lock().await;
        me.on_exit(generation, &mut l, "its output ended").await;
    })
}

/// When this process started, as the runner writes times (for its log).
#[must_use]
pub fn started_at() -> String {
    iso_of(std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_models_are_what_the_protocol_takes() {
        assert!(wire::is_session_id(&format!("r-{}", hex(4))));
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_eq!(
            model_of(Some(&json!("claude-opus-5-5[1m]")))
                .unwrap()
                .as_deref(),
            Some("claude-opus-5-5[1m]")
        );
        assert!(model_of(Some(&json!("rm -rf /"))).is_err());
        assert_eq!(model_of(Some(&json!(""))).unwrap(), None);
        assert!(effort_of(Some(&json!("loud"))).is_err());
    }
}
