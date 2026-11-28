//! Picking an account for a session, and knowing how much each has left.
//!
//! A session starts on the account with the most room that may take its work; when its turn
//! ends on a usage limit, the account is out until it resets and the session moves on. The
//! numbers come from the agent's own usage view (asked every 10 minutes, as that account,
//! through the agent's `get_usage` control request) and from every rate-limit event a running
//! session sees.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use regex::Regex;
use sea_orm::{
    ConnectOptions, ConnectionTrait as _, Database, DatabaseConnection, DbBackend, FromQueryResult,
    Statement,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::config::Config;
use crate::engine::{Engine, Launch};
use crate::store::now;

pub mod list;
pub mod login;

pub use list::{Account, AccountsError, Provider};
pub use login::{Env, Logins};

/// An account's use as last seen. Percent figures are 0-100.
#[derive(Debug, Clone, Default, PartialEq, FromQueryResult)]
pub struct Use {
    /// The account.
    pub id: String,
    /// The login's email.
    pub email: Option<String>,
    /// Its plan.
    pub plan: Option<String>,
    /// Its five-hour window's use.
    pub used5h: Option<f64>,
    /// Its week's use.
    pub used7d: Option<f64>,
    /// When its five-hour window resets.
    pub resets5h: Option<String>,
    /// When its week resets.
    pub resets7d: Option<String>,
    /// Out until then.
    pub limited_until: Option<String>,
    /// Why it is out.
    pub limited_why: Option<String>,
    /// When it was last looked at.
    pub checked: Option<String>,
    /// What is wrong with it.
    pub error: Option<String>,
}

/// A change to an account's use; `None` leaves a figure as it is.
#[derive(Debug, Default, Clone)]
pub struct UseChange {
    /// The login's email.
    pub email: Option<String>,
    /// Its plan.
    pub plan: Option<String>,
    /// Its five-hour window's use.
    pub used5h: Option<f64>,
    /// Its week's use.
    pub used7d: Option<f64>,
    /// When its five-hour window resets.
    pub resets5h: Option<String>,
    /// When its week resets.
    pub resets7d: Option<String>,
    /// When it was looked at.
    pub checked: Option<String>,
    /// What is wrong with it (`Some(None)` clears it).
    pub error: Option<Option<String>>,
}

/// What a session's work is, for picking its account.
#[derive(Debug, Clone, Default)]
pub struct Work {
    /// Its project.
    pub project: Option<String>,
    /// The folder it was asked to start in.
    pub cwd: Option<String>,
    /// It asked for an opt-in account (by id, or a model only one runs): those go first.
    pub opt_in: bool,
    /// Only these accounts, no other (a pinned session).
    pub only: Option<Vec<String>>,
    /// The model it runs.
    pub model: Option<String>,
    /// Accounts left out (the one it just ran out on).
    pub exclude: Vec<String>,
    /// Stay on this one while it has room, so nothing moves mid-run.
    pub prefer: Option<String>,
}

/// An account that can take work now, and its room (0-100).
#[derive(Debug, Clone)]
pub struct Ranked {
    /// The account.
    pub account: Account,
    /// Its room.
    pub room: f64,
}

/// An account as the server reads it (`GET /accounts`): never a token or key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// `any`, or `groups:<names>` for one kept to its groups' work.
    pub scope: String,
    /// Its rank.
    pub order: Option<i64>,
    /// Whether it may be picked.
    pub enabled: bool,
    /// Whether it has its login or key.
    pub logged_in: bool,
    /// The login's email.
    pub email: Option<String>,
    /// Its plan.
    pub plan: Option<String>,
    /// How it reaches the model.
    pub provider: &'static str,
    /// Whether it is opt-in.
    pub opt_in: bool,
    /// Its five-hour window's use, percent.
    pub used5h: Option<f64>,
    /// Its week's use, percent.
    pub used7d: Option<f64>,
    /// When its five-hour window resets.
    pub resets5h: Option<String>,
    /// When its week resets.
    pub resets7d: Option<String>,
    /// Out until then.
    pub limited_until: Option<String>,
    /// Why it is out.
    pub limited_why: Option<String>,
    /// When it was last looked at.
    pub checked: Option<String>,
    /// What is wrong with it.
    pub error: Option<String>,
}

static LIMIT_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)hit your (?:\w+ )?limit|usage limit reached|out of (?:extra )?usage|rate limit(?:ed)?\b.*(?:reset|try again)|limit · resets").ok()
});
static PROVIDER_DOWN_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)throttl|too many (?:tokens|requests)|\b(?:429|500|502|503|529)\b|service ?unavailable|overloaded|access ?denied|not authori[sz]ed|unrecognized ?client|security token|bearer token|invalid api key|marketplace|subscription|model access|on-demand throughput|quota|credits?\b|expired",
    )
    .ok()
});

/// Whether an error result says the turn ended on a usage limit.
#[must_use]
pub fn says_limit(text: &str) -> bool {
    LIMIT_RE.as_ref().is_some_and(|re| re.is_match(text))
}

/// Whether an error result is a provider's own refusal (throttled, over quota, its key refused,
/// the service down): a session moves on as at a limit.
#[must_use]
pub fn says_provider_down(text: &str) -> bool {
    PROVIDER_DOWN_RE
        .as_ref()
        .is_some_and(|re| re.is_match(text))
}

const WRONG: &str = "logged in as another account";
const NOT_LOGGED_IN: &str = "not logged in";

fn parse_time(s: &str) -> Option<Timestamp> {
    s.parse().ok()
}

fn iso(secs: Option<f64>) -> Option<String> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "seconds since 1970 fit an i64 for any real time"
    )]
    let t = Timestamp::from_second(secs.filter(|s| *s > 0.0)? as i64).ok()?;
    Some(t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn later(d: Duration) -> String {
    let t = Timestamp::now() + jiff::SignedDuration::try_from(d).unwrap_or_default();
    t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Whether `at` is less than `within` from now (or past).
fn soon(at: Option<&str>, within: Duration) -> bool {
    at.and_then(parse_time).is_some_and(|t| {
        t.duration_since(Timestamp::now())
            < jiff::SignedDuration::try_from(within).unwrap_or_default()
    })
}

/// Whether a use says the account is out now.
#[must_use]
pub fn limited(u: Option<&Use>) -> bool {
    u.and_then(|u| u.limited_until.as_deref())
        .and_then(parse_time)
        .is_some_and(|t| t > Timestamp::now())
}

/// Room left, 0-100, counting a window that resets within the hour (five hours) or 12 hours
/// (the week) as fresh.
#[must_use]
pub fn room(u: Option<&Use>) -> f64 {
    let five = if soon(
        u.and_then(|u| u.resets5h.as_deref()),
        Duration::from_secs(3600),
    ) {
        0.0
    } else {
        u.and_then(|u| u.used5h).unwrap_or(0.0)
    };
    let week = if soon(
        u.and_then(|u| u.resets7d.as_deref()),
        Duration::from_hours(12),
    ) {
        0.0
    } else {
        u.and_then(|u| u.used7d).unwrap_or(0.0)
    };
    100.0 - five.max(week)
}

/// The runner's accounts: the list, their use and the pick.
#[derive(Debug)]
pub struct Accounts {
    file: PathBuf,
    groups: std::collections::BTreeMap<String, crate::config::Group>,
    default_account: Option<String>,
    skills_dir: Option<PathBuf>,
    home: PathBuf,
    db: DatabaseConnection,
    uses: Mutex<HashMap<String, Use>>,
    /// Logins and each session's environment.
    pub logins: Logins,
    engine: Arc<dyn Engine>,
}

impl Accounts {
    /// The accounts of this configuration; their use is kept in `accounts.db` in the runner's home.
    ///
    /// # Errors
    /// The use database can't be opened.
    pub async fn open(
        config: &Config,
        logins: Logins,
        engine: Arc<dyn Engine>,
    ) -> Result<Self, sea_orm::DbErr> {
        Self::connect(
            config,
            &format!(
                "sqlite://{}?mode=rwc",
                config.home.join("accounts.db").display()
            ),
            logins,
            engine,
        )
        .await
    }

    /// Accounts whose use is kept in memory (for tests).
    ///
    /// # Errors
    /// The database can't be made.
    pub async fn in_memory(
        config: &Config,
        logins: Logins,
        engine: Arc<dyn Engine>,
    ) -> Result<Self, sea_orm::DbErr> {
        Self::connect(config, "sqlite::memory:", logins, engine).await
    }

    async fn connect(
        config: &Config,
        url: &str,
        logins: Logins,
        engine: Arc<dyn Engine>,
    ) -> Result<Self, sea_orm::DbErr> {
        let mut o = ConnectOptions::new(url);
        o.max_connections(1).min_connections(1).sqlx_logging(false);
        let db = Database::connect(o).await?;
        for sql in [
            "PRAGMA journal_mode = WAL",
            "PRAGMA synchronous = NORMAL",
            "PRAGMA busy_timeout = 10000",
            "create table if not exists usage (id text primary key, email text, plan text, used5h real, used7d real, resets5h text, \
             resets7d text, limited_until text, limited_why text, checked text, error text)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        let rows = Use::find_by_statement(Statement::from_string(
            DbBackend::Sqlite,
            "select * from usage",
        ))
        .all(&db)
        .await?;
        Ok(Self {
            file: config.accounts_file.clone(),
            groups: config.groups.clone(),
            default_account: config.default_account.clone(),
            skills_dir: config.skills_dir.clone(),
            home: config.home.clone(),
            db,
            uses: Mutex::new(rows.into_iter().map(|u| (u.id.clone(), u)).collect()),
            logins,
            engine,
        })
    }

    /// The list, read each time (the owner may edit it while the runner runs). A list that
    /// can't be read is empty, with the reason logged.
    #[must_use]
    pub fn list(&self) -> Vec<Account> {
        list::read_accounts(&self.file).unwrap_or_else(|e| {
            tracing::warn!("accounts: {e}");
            Vec::new()
        })
    }

    /// The list, or why it can't be read.
    ///
    /// # Errors
    /// The file is wrong.
    pub fn try_list(&self) -> Result<Vec<Account>, AccountsError> {
        list::read_accounts(&self.file)
    }

    /// One account.
    #[must_use]
    pub fn get(&self, id: Option<&str>) -> Option<Account> {
        let id = id?;
        self.list().into_iter().find(|a| a.id == id)
    }

    /// An account's name, or its id, or "an account".
    #[must_use]
    pub fn name_of(&self, id: Option<&str>) -> String {
        self.get(id)
            .map(|a| a.name)
            .or_else(|| id.map(str::to_owned))
            .unwrap_or_else(|| "an account".to_owned())
    }

    /// The default account, when it names a listed, enabled one.
    #[must_use]
    pub fn default_account(&self) -> Option<String> {
        self.get(self.default_account.as_deref())
            .filter(Account::is_enabled)
            .map(|a| a.id)
    }

    /// The groups a session's work is in.
    #[must_use]
    pub fn groups_of(&self, project: Option<&str>, cwd: Option<&str>) -> Vec<String> {
        self.groups
            .iter()
            .filter(|(_, g)| g.holds(project, cwd))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Its use as last seen.
    #[must_use]
    pub fn use_of(&self, id: &str) -> Option<Use> {
        self.uses.lock().ok().and_then(|m| m.get(id).cloned())
    }

    async fn write(&self, u: &Use) {
        let sql = "insert into usage(id, email, plan, used5h, used7d, resets5h, resets7d, limited_until, limited_why, checked, error) \
                   values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) on conflict(id) do update set email = excluded.email, plan = excluded.plan, \
                   used5h = excluded.used5h, used7d = excluded.used7d, resets5h = excluded.resets5h, resets7d = excluded.resets7d, \
                   limited_until = excluded.limited_until, limited_why = excluded.limited_why, checked = excluded.checked, error = excluded.error";
        let stmt = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            sql,
            [
                u.id.clone().into(),
                u.email.clone().into(),
                u.plan.clone().into(),
                u.used5h.into(),
                u.used7d.into(),
                u.resets5h.clone().into(),
                u.resets7d.clone().into(),
                u.limited_until.clone().into(),
                u.limited_why.clone().into(),
                u.checked.clone().into(),
                u.error.clone().into(),
            ],
        );
        if let Err(e) = self.db.execute_raw(stmt).await {
            tracing::warn!(account = %u.id, "cannot save its use: {e}");
        }
    }

    async fn update(&self, id: &str, f: impl FnOnce(&mut Use)) -> Use {
        let u = {
            let Ok(mut m) = self.uses.lock() else {
                return Use::default();
            };
            let u = m.entry(id.to_owned()).or_insert_with(|| Use {
                id: id.to_owned(),
                ..Use::default()
            });
            f(u);
            u.clone()
        };
        self.write(&u).await;
        u
    }

    /// Changes an account's use.
    pub async fn save(&self, id: &str, c: UseChange) -> Use {
        self.update(id, |u| {
            macro_rules! set {
                ($($f:ident),*) => { $( if let Some(v) = c.$f { u.$f = Some(v); } )* };
            }
            set!(email, plan, used5h, used7d, resets5h, resets7d, checked);
            if let Some(e) = c.error {
                u.error = e;
            }
        })
        .await
    }

    /// What a running session saw (a `rate_limit_event`'s `rate_limit_info`): the windows' use,
    /// and a refusal (out until it resets).
    pub async fn observe(&self, id: &str, info: &Value) {
        let w = info.get("unifiedWindows");
        let pct = |k: &str| {
            w.and_then(|w| w.get(k))
                .and_then(|x| x.get("utilization"))
                .and_then(Value::as_f64)
                .map(|x| (x * 1000.0).round() / 10.0)
        };
        let reset = |k: &str| {
            iso(w
                .and_then(|w| w.get(k))
                .and_then(|x| x.get("resetsAt"))
                .and_then(Value::as_f64))
        };
        self.save(
            id,
            UseChange {
                used5h: pct("five_hour"),
                used7d: pct("seven_day"),
                resets5h: reset("five_hour"),
                resets7d: reset("seven_day"),
                checked: Some(now()),
                ..UseChange::default()
            },
        )
        .await;
        if refused(info) {
            let kind = info
                .get("rateLimitType")
                .and_then(Value::as_str)
                .unwrap_or("limit");
            self.mark_limited(
                id,
                iso(info.get("resetsAt").and_then(Value::as_f64)),
                &format!("refused: {kind}"),
            )
            .await;
        }
    }

    /// Out until `until` (no time known: an hour, then it's looked at again).
    pub async fn mark_limited(&self, id: &str, until: Option<String>, why: &str) {
        let until = until.unwrap_or_else(|| later(Duration::from_hours(1)));
        self.update(id, |u| {
            u.limited_until = Some(until);
            u.limited_why = Some(why.to_owned());
        })
        .await;
    }

    /// Forgets that it was out.
    pub async fn clear_limit(&self, id: &str) {
        self.update(id, |u| {
            u.limited_until = None;
            u.limited_why = None;
        })
        .await;
    }

    /// Records what is wrong with an account, now.
    async fn failed(&self, id: &str, why: String) -> Option<Use> {
        let change = UseChange {
            error: Some(Some(why)),
            checked: Some(now()),
            ..UseChange::default()
        };
        Some(self.save(id, change).await)
    }

    /// Asks the agent, as this account, who it is logged in as and how much it has used.
    async fn ask_agent(&self, a: &Account) -> Result<(Value, Option<Value>), String> {
        let env = self.logins.env_for(a, self.skills_dir.as_deref());
        let launch = Launch {
            cwd: self.home.clone(),
            set_env: env.set,
            remove_env: env.remove,
            strict_mcp: true,
            model: Some("haiku".to_owned()),
            ..Launch::default()
        };
        let within = Duration::from_secs(30);
        let (agent, _out) = self.engine.start(&launch).map_err(|e| e.to_string())?;
        let looked = async {
            let init = agent
                .request(json!({ "subtype": "initialize" }), within)
                .await?;
            let usage = agent
                .request(
                    json!({ "subtype": "get_usage", "skip_behaviors": true }),
                    within,
                )
                .await
                .ok();
            Ok::<_, crate::engine::EngineError>((
                init.get("account").cloned().unwrap_or(Value::Null),
                usage,
            ))
        }
        .await;
        agent.close().await;
        looked.map_err(|e| e.to_string().chars().take(160).collect())
    }

    /// Looks at one account's use now, as the agent's usage view shows it, without a turn or a
    /// model call.
    pub async fn probe(&self, a: &Account) -> Option<Use> {
        if a.provider() != Provider::Claude {
            let plan = if a.provider() == Provider::Bedrock {
                "aws-bedrock"
            } else {
                "api"
            };
            let error = (!self.logins.logged_in(a)).then(|| NOT_LOGGED_IN.to_owned());
            let change = UseChange {
                plan: Some(plan.to_owned()),
                error: Some(error),
                checked: Some(now()),
                ..UseChange::default()
            };
            return Some(self.save(&a.id, change).await);
        }
        if !self.logins.logged_in(a) {
            return self.failed(&a.id, NOT_LOGGED_IN.to_owned()).await;
        }
        let (info, usage) = match self.ask_agent(a).await {
            Ok(x) => x,
            Err(why) => return self.failed(&a.id, why).await,
        };
        let Some(email) = info
            .get("email")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
        else {
            return self.failed(&a.id, NOT_LOGGED_IN.to_owned()).await;
        };
        if let Some(want) = &a.email
            && !want.eq_ignore_ascii_case(&email)
        {
            let why = format!("{WRONG}: {email}, not {want}");
            self.save(
                &a.id,
                UseChange {
                    email: Some(email),
                    ..UseChange::default()
                },
            )
            .await;
            return self.failed(&a.id, why).await;
        }
        let r = usage.as_ref().and_then(|u| u.get("rate_limits"));
        let win = |k: &str, f: &str| r.and_then(|r| r.get(k)).and_then(|w| w.get(f)).cloned();
        let text = |v: Option<Value>| v.and_then(|v| v.as_str().map(str::to_owned));
        let change = UseChange {
            email: Some(email),
            plan: info
                .get("subscriptionType")
                .and_then(Value::as_str)
                .map(str::to_owned),
            used5h: win("five_hour", "utilization").and_then(|v| v.as_f64()),
            used7d: win("seven_day", "utilization").and_then(|v| v.as_f64()),
            resets5h: text(win("five_hour", "resets_at")),
            resets7d: text(win("seven_day", "resets_at")),
            checked: Some(now()),
            error: Some(None),
        };
        let u = self.save(&a.id, change).await;
        // A window it was refused in that has reset is open again.
        if u.limited_until.is_some()
            && u.used5h.unwrap_or(0.0) < 98.0
            && u.used7d.unwrap_or(0.0) < 98.0
        {
            self.clear_limit(&a.id).await;
        }
        self.use_of(&a.id)
    }

    /// Looks at every enabled account's use, one after the other.
    pub async fn probe_all(&self) {
        let list = self.list();
        self.logins.refresh(&list).await;
        for a in list.iter().filter(|a| a.is_enabled()) {
            self.probe(a).await;
        }
    }

    /// The accounts a pinned session or a provider-only model may run on (`None`: any).
    #[must_use]
    pub fn only_for(&self, pin: Option<&str>, model: Option<&str>) -> Option<Vec<String>> {
        if let Some(p) = pin {
            return Some(vec![p.to_owned()]);
        }
        model.filter(|m| list::is_bedrock_model(m)).map(|_| {
            self.list()
                .into_iter()
                .filter(|a| a.provider() == Provider::Bedrock)
                .map(|a| a.id)
                .collect()
        })
    }

    /// The accounts that can take this work now, best first: allowed for its groups, logged in,
    /// not out, able to run its model. Opt-in accounts go first for work that asks for them and
    /// last otherwise; the default account goes before the rest; nearly full ones (under 10%
    /// left) after the others; then the one it is on, the lowest order and the most room.
    #[must_use]
    pub fn ranked(&self, w: &Work) -> Vec<Ranked> {
        let def = self.default_account();
        let in_groups = self.groups_of(w.project.as_deref(), w.cwd.as_deref());
        let mut r: Vec<Ranked> = self
            .list()
            .into_iter()
            .filter(|a| {
                a.takes(&in_groups)
                    && !w.exclude.contains(&a.id)
                    && w.only.as_ref().is_none_or(|o| o.contains(&a.id))
                    && a.can_run(w.model.as_deref())
                    && self.logins.logged_in(a)
            })
            .filter_map(|a| {
                let u = self.use_of(&a.id);
                let bad = u
                    .as_ref()
                    .and_then(|u| u.error.as_deref())
                    .is_some_and(|e| e == NOT_LOGGED_IN || e.starts_with(WRONG));
                (!limited(u.as_ref()) && !bad).then(|| Ranked {
                    room: room(u.as_ref()),
                    account: a,
                })
            })
            .collect();
        let side = |a: &Account| {
            u8::from(a.is_opt_in() && Some(&a.id) != def.as_ref()) ^ u8::from(w.opt_in)
        };
        let first = |a: &Account| u8::from(Some(&a.id) != def.as_ref());
        r.sort_by(|x, y| {
            side(&x.account)
                .cmp(&side(&y.account))
                .then(first(&x.account).cmp(&first(&y.account)))
                .then((x.room < 10.0).cmp(&(y.room < 10.0)))
                .then(
                    (Some(&y.account.id) == w.prefer.as_ref())
                        .cmp(&(Some(&x.account.id) == w.prefer.as_ref())),
                )
                .then(
                    x.account
                        .order
                        .unwrap_or(i64::MAX)
                        .cmp(&y.account.order.unwrap_or(i64::MAX)),
                )
                .then(y.room.total_cmp(&x.room))
        });
        r
    }

    /// The best account for this work now.
    #[must_use]
    pub fn pick(&self, w: &Work) -> Option<Account> {
        self.ranked(w).into_iter().next().map(|r| r.account)
    }

    /// When the first account that could take this work is free again.
    #[must_use]
    pub fn next_free(&self, w: &Work) -> Option<String> {
        let in_groups = self.groups_of(w.project.as_deref(), w.cwd.as_deref());
        self.list()
            .into_iter()
            .filter(|a| {
                a.takes(&in_groups)
                    && w.only.as_ref().is_none_or(|o| o.contains(&a.id))
                    && self.logins.logged_in(a)
            })
            .filter_map(|a| self.use_of(&a.id).and_then(|u| u.limited_until))
            .min()
    }

    /// Why this machine never takes the work: it is in groups no account here takes.
    #[must_use]
    pub fn refusal(&self, project: Option<&str>, cwd: Option<&str>) -> Option<String> {
        let in_groups = self.groups_of(project, cwd);
        if in_groups.is_empty() || self.list().iter().any(|a| a.takes(&in_groups)) {
            return None;
        }
        Some(format!(
            "no account on this runner takes {} work: start it on a runner that has one",
            in_groups.join(" or ")
        ))
    }

    /// Why an account isn't picked now, in words.
    #[must_use]
    pub fn low_why(&self, id: &str) -> String {
        let a = self.get(Some(id));
        let u = self.use_of(id);
        let n = a.as_ref().map_or(id, |a| a.name.as_str());
        if limited(u.as_ref()) {
            return format!(
                "{n} is out until {}",
                u.and_then(|u| u.limited_until).unwrap_or_default()
            );
        }
        let bad = u
            .as_ref()
            .and_then(|u| u.error.as_deref())
            .is_some_and(|e| e == NOT_LOGGED_IN || e.starts_with(WRONG));
        if a.as_ref().is_none_or(|a| !self.logins.logged_in(a)) || bad {
            return format!("{n} isn't logged in");
        }
        let (five, week) = (
            u.as_ref().and_then(|u| u.used5h).unwrap_or(0.0),
            u.as_ref().and_then(|u| u.used7d).unwrap_or(0.0),
        );
        if week >= five {
            format!("{n} is at {week:.0}% for the week")
        } else {
            format!("{n} is at {five:.0}% of its 5 hours")
        }
    }

    /// Every account as the server reads it.
    #[must_use]
    pub fn summary(&self) -> Vec<AccountView> {
        self.list()
            .into_iter()
            .map(|a| {
                let u = self.use_of(&a.id);
                let out = limited(u.as_ref());
                let groups = a.groups.clone().unwrap_or_default();
                AccountView {
                    scope: if a.groups_only == Some(true) && !groups.is_empty() {
                        format!("groups:{}", groups.join(","))
                    } else {
                        "any".to_owned()
                    },
                    order: a.order,
                    enabled: a.is_enabled(),
                    logged_in: self.logins.logged_in(&a),
                    email: u.as_ref().and_then(|u| u.email.clone()),
                    plan: u.as_ref().and_then(|u| u.plan.clone()),
                    provider: a.provider().word(),
                    opt_in: a.is_opt_in(),
                    used5h: u.as_ref().and_then(|u| u.used5h),
                    used7d: u.as_ref().and_then(|u| u.used7d),
                    resets5h: u.as_ref().and_then(|u| u.resets5h.clone()),
                    resets7d: u.as_ref().and_then(|u| u.resets7d.clone()),
                    limited_until: if out {
                        u.as_ref().and_then(|u| u.limited_until.clone())
                    } else {
                        None
                    },
                    limited_why: if out {
                        u.as_ref().and_then(|u| u.limited_why.clone())
                    } else {
                        None
                    },
                    checked: u.as_ref().and_then(|u| u.checked.clone()),
                    error: u.as_ref().and_then(|u| u.error.clone()),
                    id: a.id,
                    name: a.name,
                }
            })
            .collect()
    }
}

/// Whether a rate-limit event says the account refused the turn (and no paid overage covers it).
#[must_use]
pub fn refused(info: &Value) -> bool {
    info.get("status").and_then(Value::as_str) == Some("rejected")
        && info.get("overageStatus").and_then(Value::as_str) != Some("allowed")
}

/// When a refusal's window resets, from a rate-limit event.
#[must_use]
pub fn resets_of(info: &Value) -> Option<String> {
    iso(info.get("resetsAt").and_then(Value::as_f64))
}

/// A time `d` from now, as the runner writes times.
#[must_use]
pub fn from_now(d: Duration) -> String {
    later(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NoEngine;
    impl Engine for NoEngine {
        fn start(
            &self,
            _: &Launch,
        ) -> Result<
            (
                crate::engine::Agent,
                tokio::sync::mpsc::UnboundedReceiver<crate::engine::AgentOutput>,
            ),
            crate::engine::EngineError,
        > {
            Err(crate::engine::EngineError::Ended)
        }
    }

    async fn accounts(list: &str) -> (tempfile::TempDir, Accounts) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_str().unwrap().to_owned();
        std::fs::write(dir.path().join("accounts.json"), list).unwrap();
        std::fs::write(
            dir.path().join(crate::config::CONFIG_FILE),
            r#"{"groups": {"client": {"folders": "client"}}, "defaultAccount": "b"}"#,
        )
        .unwrap();
        let env = move |k: &str| matches!(k, "HOME" | "EMBERHUT_RUNNER_HOME").then(|| home.clone());
        let c = Config::load(&env, "x").unwrap();
        for id in ["a", "b", "c", "k"] {
            std::fs::create_dir_all(dir.path().join(format!(".agent-{id}"))).unwrap();
            std::fs::write(
                dir.path().join(format!(".agent-{id}/.credentials.json")),
                "{}",
            )
            .unwrap();
        }
        let logins = Logins::new(dir.path().to_path_buf(), dir.path().join("secrets"), false);
        let a = Accounts::in_memory(&c, logins, Arc::new(NoEngine))
            .await
            .unwrap();
        (dir, a)
    }

    const LIST: &str = r#"[
        {"id": "a", "name": "A", "configDir": "~/.agent-a", "order": 1},
        {"id": "b", "name": "B", "configDir": "~/.agent-b", "order": 2},
        {"id": "c", "name": "C", "configDir": "~/.agent-c", "optIn": true},
        {"id": "k", "name": "K", "configDir": "~/.agent-k", "groups": ["client"], "groupsOnly": true}
    ]"#;

    #[tokio::test]
    async fn the_default_goes_first_then_order_and_room() {
        let (_d, a) = accounts(LIST).await;
        let ids = |w: &Work| {
            a.ranked(w)
                .into_iter()
                .map(|r| r.account.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&Work::default()), ["b", "a", "c"]);
        a.save(
            "b",
            UseChange {
                used5h: Some(95.0),
                ..UseChange::default()
            },
        )
        .await;
        assert_eq!(
            ids(&Work::default()),
            ["b", "a", "c"],
            "the default stays first even nearly full"
        );
        a.save(
            "a",
            UseChange {
                used5h: Some(5.0),
                ..UseChange::default()
            },
        )
        .await;
        assert_eq!(
            ids(&Work {
                exclude: vec!["b".to_owned()],
                ..Work::default()
            }),
            ["a", "c"]
        );
        assert_eq!(
            ids(&Work {
                opt_in: true,
                ..Work::default()
            })[0],
            "c"
        );
        assert_eq!(
            ids(&Work {
                cwd: Some("/w/client/app".to_owned()),
                ..Work::default()
            }),
            ["k"]
        );
    }

    #[tokio::test]
    async fn an_account_out_is_skipped_until_it_resets() {
        let (_d, a) = accounts(LIST).await;
        a.mark_limited("b", None, "hit its five_hour limit").await;
        let w = Work::default();
        assert_eq!(a.pick(&w).unwrap().id, "a");
        assert!(a.low_why("b").contains("is out until"));
        assert!(a.next_free(&w).is_some());
        a.clear_limit("b").await;
        assert_eq!(a.pick(&w).unwrap().id, "b");
        let view = a.summary();
        assert_eq!(
            view.iter().find(|v| v.id == "k").unwrap().scope,
            "groups:client"
        );
    }

    #[tokio::test]
    async fn a_refused_rate_limit_marks_the_account_out() {
        let (_d, a) = accounts(LIST).await;
        a.observe("a", &json!({"status": "rejected", "rateLimitType": "five_hour", "resetsAt": 4_102_444_800_u64,
            "unifiedWindows": {"five_hour": {"utilization": 1.0}}}))
            .await;
        let u = a.use_of("a").unwrap();
        assert_eq!(u.used5h, Some(100.0));
        assert!(limited(Some(&u)));
        assert!(says_limit("You've hit your limit · resets 3pm") && !says_limit("all good"));
    }
}
