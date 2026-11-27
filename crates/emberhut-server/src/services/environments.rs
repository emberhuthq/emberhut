//! Settings → Environments: every place the assistant works and connection it reads through,
//! live: each runner (reachable or not, its sessions and accounts), each machine of the
//! coding-thread app, and the assistant's accounts. "Check now" looks at one again at once.
//! Nothing here stops or deletes anything.
//!
//! Other areas' places and connections (chat services, the browser, the person's devices)
//! join the page as their areas land, each as items of their own group.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_api as api;
use emberhut_api::environments::{
    EnvAction as Action, EnvFact as EnvironmentFact, EnvGroup as Group, EnvLastError as LastError,
    EnvList as List, EnvListItem as ListItem, EnvPage as EnvironmentsPage, EnvSummary as Summary,
    Environment,
};
use emberhut_core::Clock;
use emberhut_core::thread::ThreadStatus;
use emberhut_db::{DbError, ThreadStore};
use jiff::Timestamp;

use crate::convert::timestamp_to_api;
use crate::services::accounts::AccountService;
use crate::services::runners::RunnerService;
use crate::services::threads::ThreadService;

/// `POST /api/environments/check`'s body, read leniently (the
/// contract's typed form is [`emberhut_api::environments::EnvActionRequest`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct CheckRequest {
    /// The item.
    #[serde(default)]
    pub id: Option<String>,
    /// The action: `check`, the only one here.
    #[serde(default)]
    pub kind: Option<String>,
}

/// The group of the coding-thread app's machines (the app's id for it).
pub const CODING_APP_GROUP: &str = "t3";

/// The groups, in order: id, title, footer.
const GROUPS: [(&str, &str, Option<&str>); 3] = [
    (
        "runners",
        "Runners",
        Some("Where the assistant's sessions run."),
    ),
    (
        CODING_APP_GROUP,
        "Coding-thread app",
        Some("The other coding tool whose threads the assistant follows."),
    ),
    (
        "connections",
        "Connections",
        Some("What the assistant reads and sends through."),
    ),
];

/// Why a check wasn't done.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnvironmentError {
    /// Not an action this page has, in words.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The page.
#[derive(Clone, Debug)]
pub struct EnvironmentService {
    runners: RunnerService,
    threads: ThreadService,
    store: ThreadStore,
    accounts: AccountService,
    clock: Arc<dyn Clock>,
    errors: Arc<Mutex<BTreeMap<String, LastError>>>,
}

/// `at` as "now", "4m ago", "3h ago", "2d ago", or "never".
fn ago(at: Option<Timestamp>, now: Timestamp) -> String {
    let Some(at) = at else {
        return "never".to_owned();
    };
    let minutes = now.duration_since(at).as_secs_f64() / 60.0;
    if minutes < 1.0 {
        "now".to_owned()
    } else if minutes < 60.0 {
        format!("{:.0}m ago", minutes.floor())
    } else if minutes < 1440.0 {
        format!("{:.0}h ago", (minutes / 60.0).floor())
    } else {
        format!("{:.0}d ago", (minutes / 1440.0).floor())
    }
}

/// "Check now".
fn check() -> Action {
    Action {
        id: emberhut_api::environments::EnvActionKind::Check,
        label: "Check now".to_owned(),
    }
}

/// A fact.
fn fact(label: &str, value: String, state: Option<&str>) -> EnvironmentFact {
    EnvironmentFact {
        label: label.to_owned(),
        value,
        state: state.map(emberhut_api::environments::EnvState::from),
    }
}

/// An account as a list's item, and whether it can take work now.
fn account_item(
    account: &api::runner::accounts::ClaudeAccount,
    now: Timestamp,
) -> (ListItem, bool) {
    let out = account
        .limited_until
        .as_ref()
        .and_then(|until| until.as_str().parse::<Timestamp>().ok())
        .is_some_and(|until| until > now);
    let enabled = account.enabled.unwrap_or(true);
    let logged_in = account.logged_in.unwrap_or(false);
    let account_scope = account
        .scope
        .as_deref()
        .unwrap_or(emberhut_core::account::SCOPE_ANY);
    let room: f64 = 100.0
        - account
            .used_5h
            .unwrap_or(0.0)
            .max(account.used_7d.unwrap_or(0.0));
    let usable = enabled && logged_in && !out && room > 5.0;
    let value = if !logged_in {
        "Not logged in".to_owned()
    } else if !enabled {
        "Off".to_owned()
    } else if out {
        format!(
            "Out until {}",
            account
                .limited_until
                .as_ref()
                .map_or("?", api::time::Timestamp::as_str)
        )
    } else if usable {
        format!(
            "Room {}%",
            emberhut_core::machine::num(room.round().max(0.0))
        )
    } else {
        "Nearly full".to_owned()
    };
    let scope = if account_scope == emberhut_core::account::SCOPE_ANY {
        "any work".to_owned()
    } else {
        format!("{account_scope} work only")
    };
    let pct = |used: Option<f64>| {
        used.map_or_else(
            || "?".to_owned(),
            |used| format!("{}%", emberhut_core::machine::num(used.round())),
        )
    };
    let use_text = if account.used_5h.is_some() || account.used_7d.is_some() {
        format!(
            " · 5h {}, week {}",
            pct(account.used_5h),
            pct(account.used_7d)
        )
    } else {
        String::new()
    };
    let email = account
        .email
        .as_deref()
        .map_or_default(|email| format!("{email} · "));
    let state = if !logged_in || out {
        "down"
    } else if !enabled {
        "off"
    } else if usable {
        "ok"
    } else {
        "warn"
    };
    (
        ListItem {
            name: account.name.clone(),
            value: Some(value),
            note: Some(format!("{email}{scope}{use_text}")),
            state: Some(state.into()),
        },
        usable,
    )
}

impl EnvironmentService {
    /// The page over the runners, the coding-thread app and the accounts.
    #[must_use]
    pub fn new(
        runners: RunnerService,
        threads: ThreadService,
        store: ThreadStore,
        accounts: AccountService,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            runners,
            threads,
            store,
            accounts,
            clock,
            errors: Arc::default(),
        }
    }

    /// The page as it is now.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one item per place, in the page's order"
    )]
    pub async fn page(&self) -> Result<EnvironmentsPage, DbError> {
        let now = self.clock.now();
        let accounts = self.accounts.all().await?;
        let mut items = Vec::new();
        let looks = self.runners.looks();
        for machine in self.runners.machines() {
            let look = looks.get(&machine).cloned().unwrap_or_default();
            items.push(self.runner_item(&machine, &look, &accounts, now).await?);
        }
        for (machine, error) in self.threads.machine_errors() {
            let threads: Vec<_> = self
                .store
                .on_machine(&machine)
                .await?
                .into_iter()
                .filter(emberhut_core::thread::CodingThread::is_top_level)
                .collect();
            let running = threads
                .iter()
                .filter(|t| t.status == ThreadStatus::Running)
                .count();
            items.push(Environment {
                id: machine.clone().into(),
                group: CODING_APP_GROUP.into(),
                name: format!("Coding-thread app on {machine}"),
                icon: "server.rack".to_owned(),
                tint: "gray".to_owned(),
                legacy: None,
                state: if error.is_some() { "down" } else { "ok" }.into(),
                status: if error.is_some() {
                    "Unreachable"
                } else {
                    "Reachable"
                }
                .to_owned(),
                detail: format!("{running} running · {} threads", threads.len()),
                at: None,
                hint: None,
                facts: vec![
                    fact(
                        "Reachable",
                        error
                            .as_ref()
                            .map_or_else(|| "Yes".to_owned(), |error| format!("No: {error}")),
                        Some(if error.is_some() { "down" } else { "ok" }),
                    ),
                    fact(
                        "Threads",
                        format!("{running} running · {} in all", threads.len()),
                        None,
                    ),
                ],
                error,
                lists: Vec::new(),
                actions: vec![check()],
                last_error: None,
            });
        }
        items.push(self.accounts_item(&accounts, now).await?);
        let mut kept = self.errors.lock().unwrap_or_else(PoisonError::into_inner);
        for item in &mut items {
            if let Some(error) = &item.error
                && kept
                    .get(item.id.as_str())
                    .is_none_or(|last| &last.error != error)
            {
                kept.insert(
                    item.id.as_str().to_owned(),
                    LastError {
                        error: error.clone(),
                        at: timestamp_to_api(now),
                    },
                );
            }
            item.last_error = kept.get(item.id.as_str()).cloned();
        }
        drop(kept);
        let count = |state: &str| {
            u32::try_from(
                items
                    .iter()
                    .filter(|item| item.state.as_str() == state && item.legacy != Some(true))
                    .count(),
            )
            .unwrap_or(u32::MAX)
        };
        let summary = Summary {
            ok: count("ok"),
            warn: count("warn"),
            down: count("down"),
        };
        let groups = GROUPS
            .iter()
            .map(|(id, title, footer)| Group {
                id: (*id).into(),
                title: (*title).to_owned(),
                footer: footer.map(str::to_owned),
                items: items
                    .iter()
                    .filter(|item| item.group.as_str() == *id)
                    .cloned()
                    .collect(),
            })
            .filter(|group| !group.items.is_empty())
            .collect();
        Ok(EnvironmentsPage {
            at: timestamp_to_api(now),
            summary,
            groups,
            pairing: None,
            note: None,
            link: None,
        })
    }

    async fn runner_item(
        &self,
        machine: &str,
        look: &crate::services::runners::RunnerLook,
        accounts: &[api::runner::accounts::ClaudeAccount],
        now: Timestamp,
    ) -> Result<Environment, DbError> {
        let place = self.runners.place(machine);
        let items: Vec<(ListItem, bool)> = accounts
            .iter()
            .filter(|account| {
                account.machine.as_ref().map_or(
                    api::runner::accounts::LINUX_RUNNER,
                    api::ids::MachineId::as_str,
                ) == machine
            })
            .map(|account| account_item(account, now))
            .collect();
        let usable = items.iter().filter(|(_, usable)| *usable).count();
        let sessions = self.store.on_machine(machine).await?;
        let running: Vec<_> = sessions
            .iter()
            .filter(|thread| thread.status == ThreadStatus::Running)
            .collect();
        let error = look.error.clone();
        let state = if error.is_some() {
            "down"
        } else if usable == 0 {
            "warn"
        } else {
            "ok"
        };
        let status = if error.is_some() {
            "Offline".to_owned()
        } else if running.is_empty() {
            "Online".to_owned()
        } else {
            format!("{} running", running.len())
        };
        let plural = if items.len() == 1 { "" } else { "s" };
        Ok(Environment {
            id: machine.into(),
            group: "runners".into(),
            name: format!("Runner on the {place}"),
            icon: "server.rack".to_owned(),
            tint: "teal".to_owned(),
            legacy: None,
            state: state.into(),
            status,
            detail: format!(
                "{machine} · {usable} of {} account{plural} with room",
                items.len()
            ),
            at: look.at.map(timestamp_to_api),
            hint: error.as_ref().map(|_| {
                format!(
                    "The runner on the {place} isn't answering: the machine may be off or away \
                     from the network. Its sessions carry on when it's back."
                )
            }),
            facts: vec![
                fact(
                    "Reachable",
                    error.as_ref().map_or_else(
                        || format!("Yes · {}", ago(look.at, now)),
                        |error| format!("No: {error}"),
                    ),
                    Some(if error.is_some() { "down" } else { "ok" }),
                ),
                fact(
                    "Sessions",
                    format!("{} running · {} in all", running.len(), sessions.len()),
                    None,
                ),
            ],
            error,
            lists: vec![
                List {
                    title: "Accounts".to_owned(),
                    items: items.into_iter().map(|(item, _)| item).collect(),
                    empty: Some("No accounts listed yet".to_owned()),
                },
                List {
                    title: "Running now".to_owned(),
                    items: running
                        .iter()
                        .map(|thread| ListItem {
                            name: thread.title.clone(),
                            value: None,
                            note: Some(
                                thread
                                    .doing
                                    .clone()
                                    .unwrap_or_else(|| thread.project.clone()),
                            ),
                            state: Some("ok".into()),
                        })
                        .collect(),
                    empty: Some("Nothing running".to_owned()),
                },
            ],
            actions: vec![check()],
            last_error: None,
        })
    }

    async fn accounts_item(
        &self,
        accounts: &[api::runner::accounts::ClaudeAccount],
        now: Timestamp,
    ) -> Result<Environment, DbError> {
        let items: Vec<(ListItem, bool)> = accounts
            .iter()
            .map(|account| {
                let (mut item, usable) = account_item(account, now);
                item.note = Some(format!(
                    "{} · {}",
                    account.place.as_deref().unwrap_or_default(),
                    item.note.unwrap_or_default()
                ));
                (item, usable)
            })
            .collect();
        let usable = items.iter().filter(|(_, usable)| *usable).count();
        let on = self.accounts.assistant().await?.on;
        let on_name = on
            .as_deref()
            .and_then(|on| accounts.iter().find(|account| account.id == on))
            .map(|account| account.name.clone())
            .or(on)
            .unwrap_or_else(|| "?".to_owned());
        let logged_out = items
            .iter()
            .any(|(item, _)| item.value.as_deref() == Some("Not logged in"));
        Ok(Environment {
            id: "accounts".into(),
            group: "connections".into(),
            name: "Assistant accounts".to_owned(),
            icon: "person.2.circle.fill".to_owned(),
            tint: "orange".to_owned(),
            legacy: None,
            state: if usable == 0 {
                "down"
            } else if items.iter().any(|(item, _)| {
                item.state
                    .as_ref()
                    .map(emberhut_api::environments::EnvState::as_str)
                    == Some("down")
            }) {
                "warn"
            } else {
                "ok"
            }
            .into(),
            status: format!("{usable} of {} with room", items.len()),
            detail: format!("The assistant runs on {on_name}"),
            at: None,
            error: None,
            hint: logged_out.then(|| {
                "An account that isn't logged in: log it in yourself on its machine.".to_owned()
            }),
            facts: vec![fact(
                "Picked from",
                "accounts for any work, by room left".to_owned(),
                None,
            )],
            lists: vec![List {
                title: "Accounts".to_owned(),
                items: items.into_iter().map(|(item, _)| item).collect(),
                empty: Some("No accounts listed yet".to_owned()),
            }],
            actions: vec![check()],
            last_error: None,
        })
    }

    /// Looks at the item `id` again now (`kind` `check`, the only action here): a runner's
    /// health and accounts, the coding-thread app, or every runner's accounts. The page after.
    ///
    /// # Errors
    ///
    /// [`EnvironmentError::Refused`] for another action; [`EnvironmentError::Database`].
    pub async fn check(&self, id: &str, kind: &str) -> Result<EnvironmentsPage, EnvironmentError> {
        if kind != "check" {
            return Err(EnvironmentError::Refused(format!("unknown action {kind}")));
        }
        if self.runners.is_runner(id) {
            self.runners.tick_one(id).await;
        } else if id == "accounts" {
            for machine in self.runners.machines() {
                if let Err(error) = self.runners.refresh_accounts(&machine).await {
                    tracing::debug!(%machine, %error, "a runner's accounts could not be read");
                }
            }
        } else if self.threads.machine_errors().contains_key(id) {
            if let Err(error) = self.threads.poll().await {
                tracing::warn!(%error, "the coding-thread app could not be polled");
            }
        } else {
            for machine in self.runners.machines() {
                self.runners.tick_one(&machine).await;
            }
        }
        Ok(self.page().await?)
    }
}
