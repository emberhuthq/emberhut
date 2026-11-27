//! The assistant's accounts: every account the runners use and its use, the Usage card, the
//! account the assistant itself runs on and the person's pin.
//!
//! Each runner keeps its own list of accounts and looks at their use itself; the server reads
//! every runner's list each minute ([`RunnerService::tick`]) and keeps it (the `usage` table).
//! "Check now" ([`AccountService::check_usage`]) asks every runner to look again, at most once in
//! 2 minutes. The person's pin is a pref (the `prefs` table): an account id the assistant runs
//! on until they set automatic again, unless it can't take work, when the assistant runs on the
//! one with the most room and says so (`pinOut`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_api as api;
use emberhut_core::Clock;
use emberhut_core::account::{AccountUse, SCOPE_ANY, check_pin, ranked, window};
use emberhut_db::{DbError, PrefKey, PrefStore, UsageStore};
use jiff::{SignedDuration, Timestamp};

use crate::convert::timestamp_to_api;
use crate::services::activity::ActivityLog;
use crate::services::runner_api::Verb;
use crate::services::runners::RunnerService;

/// The shortest time between two looks the person asks for.
pub const CHECK_EVERY: SignedDuration = SignedDuration::from_mins(2);

/// Why the pin wasn't changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AccountError {
    /// The account can't be pinned, in words.
    #[error(transparent)]
    Refused(#[from] emberhut_core::account::PinRefusal),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The accounts and the person's choice.
#[derive(Clone, Debug)]
pub struct AccountService {
    usage: UsageStore,
    prefs: PrefStore,
    runners: RunnerService,
    activity: ActivityLog,
    clock: Arc<dyn Clock>,
    asked: Arc<Mutex<Option<Timestamp>>>,
}

impl AccountService {
    /// The service over the stores, reading the runners through `runners`.
    #[must_use]
    pub fn new(
        usage: UsageStore,
        prefs: PrefStore,
        runners: RunnerService,
        activity: ActivityLog,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            usage,
            prefs,
            runners,
            activity,
            clock,
            asked: Arc::default(),
        }
    }

    async fn known(&self) -> Result<Vec<AccountUse>, DbError> {
        let runners = self.runners.machines();
        Ok(self
            .usage
            .all()
            .await?
            .into_iter()
            .filter(|account| runners.contains(&account.machine))
            .collect())
    }

    /// Every account the runners use, each with its machine: in the runners' configured order,
    /// each runner's in its own order.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn all(&self) -> Result<Vec<api::runner::accounts::ClaudeAccount>, DbError> {
        let now = self.clock.now();
        let accounts = self.known().await?;
        let mut rows = Vec::with_capacity(accounts.len());
        for machine in self.runners.machines() {
            let place = self.runners.place(&machine);
            for account in accounts.iter().filter(|account| account.machine == machine) {
                let limited = account.limited(now);
                rows.push(api::runner::accounts::ClaudeAccount {
                    id: account.id.clone(),
                    name: account.name.clone(),
                    scope: Some(account.scope.clone()),
                    order: account.rank,
                    enabled: Some(account.enabled),
                    logged_in: Some(account.logged_in),
                    email: account.email.clone(),
                    plan: account.plan.clone(),
                    used_5h: account.used5h,
                    used_7d: account.used7d,
                    resets_5h: account.resets5h.map(timestamp_to_api),
                    resets_7d: account.resets7d.map(timestamp_to_api),
                    limited_until: account
                        .limited_until
                        .filter(|_| limited)
                        .map(timestamp_to_api),
                    limited_why: account.limited_why.clone().filter(|_| limited),
                    checked: account.checked.map(timestamp_to_api),
                    error: account.error.clone(),
                    machine: Some(api::ids::MachineId::new(machine.clone())),
                    place: Some(place.clone()),
                });
            }
        }
        Ok(rows)
    }

    /// The Usage card: one row per account any work may run on (the freshest look of an
    /// account on several runners), names and percentages only; `best` has the most room.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn usage_card(&self) -> Result<Vec<api::usage::UsageRow>, DbError> {
        let now = self.clock.now();
        let accounts: Vec<AccountUse> = self
            .known()
            .await?
            .into_iter()
            .filter(|account| account.scope == SCOPE_ANY)
            .collect();
        let mut freshest: BTreeMap<&str, &AccountUse> = BTreeMap::new();
        let mut order = Vec::new();
        for account in &accounts {
            match freshest.get(account.id.as_str()) {
                Some(had) if had.checked >= account.checked => {}
                Some(_) => {
                    freshest.insert(&account.id, account);
                }
                None => {
                    order.push(account.id.as_str());
                    freshest.insert(&account.id, account);
                }
            }
        }
        let mut rows: Vec<(f64, api::usage::UsageRow)> = order
            .into_iter()
            .filter_map(|id| freshest.get(id).copied())
            .map(|account| {
                let (five, five_resets) = window(account.used5h, account.resets5h, now);
                let (week, week_resets) = window(account.used7d, account.resets7d, now);
                let mut places: Vec<String> = Vec::new();
                for other in accounts.iter().filter(|other| other.id == account.id) {
                    let place = self.runners.place(&other.machine);
                    if !places.contains(&place) {
                        places.push(place);
                    }
                }
                let room = 100.0 - five.unwrap_or(0.0).max(week.unwrap_or(0.0));
                (
                    room,
                    api::usage::UsageRow {
                        id: account.id.clone(),
                        name: account.short_name(),
                        scope: account.scope.clone(),
                        logged_in: account.logged_in,
                        used_5h: five,
                        resets_5h: five_resets.map(timestamp_to_api),
                        used_7d: week,
                        resets_7d: week_resets.map(timestamp_to_api),
                        limited_until: account
                            .limited_until
                            .filter(|_| account.limited(now))
                            .map(timestamp_to_api),
                        checked: account.checked.map(timestamp_to_api),
                        places,
                        best: false,
                    },
                )
            })
            .collect();
        let best = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, row))| {
                row.logged_in
                    && row.limited_until.is_none()
                    && (row.used_5h.is_some() || row.used_7d.is_some())
            })
            .max_by(|(_, (a, _)), (_, (b, _))| a.total_cmp(b))
            .map(|(at, _)| at);
        if let Some(at) = best
            && let Some((_, row)) = rows.get_mut(at)
        {
            row.best = true;
        }
        Ok(rows.into_iter().map(|(_, row)| row).collect())
    }

    /// The account the assistant runs on, and the person's choice.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn assistant(&self) -> Result<api::usage::AssistantAccount, DbError> {
        let accounts = self.known().await?;
        let pin = self
            .prefs
            .get(PrefKey::Pin)
            .await?
            .filter(|pin| accounts.iter().any(|account| &account.id == pin));
        let now = self.clock.now();
        let pin_out = pin.as_ref().is_some_and(|pin| {
            !ranked(&accounts, None, now)
                .iter()
                .any(|account| &account.id == pin)
        });
        Ok(api::usage::AssistantAccount {
            on: self.prefs.get(PrefKey::On).await?,
            mode: if pin.is_some() { "pinned" } else { "auto" }.to_owned(),
            pin,
            pin_out,
            // The assistant's runtime keeps no rooms open on accounts yet.
            rooms: Vec::new(),
            waits: None,
        })
    }

    /// Notes the account the assistant's main room runs on now (its runtime says).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_on(&self, id: Option<&str>) -> Result<(), DbError> {
        self.prefs.set(PrefKey::On, id).await
    }

    /// The person's choice: pin the assistant to `id`, or `None` (`auto`) to let it run on the
    /// account with the most room. The assistant's account after it, with the rooms that move
    /// later.
    ///
    /// # Errors
    ///
    /// [`AccountError::Refused`] for an account no runner lists or the assistant doesn't run
    /// on; [`AccountError::Database`].
    pub async fn use_account(
        &self,
        id: Option<&str>,
    ) -> Result<api::usage::AssistantAccount, AccountError> {
        let id = id.filter(|id| !id.is_empty() && *id != "auto");
        let accounts = self.known().await?;
        let line = if let Some(id) = id {
            check_pin(&accounts, id)?;
            let name = accounts
                .iter()
                .find(|account| account.id == id)
                .map_or_else(|| id.to_owned(), |account| account.name.clone());
            format!("Pinned the assistant to {name}")
        } else {
            "Set the assistant's account to Auto".to_owned()
        };
        self.prefs.set(PrefKey::Pin, id).await?;
        if let Err(error) = self.activity.note(line).await {
            tracing::warn!(%error, "an activity line could not be written");
        }
        let mut account = self.assistant().await?;
        account.waits = Some(Vec::new());
        Ok(account)
    }

    /// Asks every runner to look at its accounts' use now, and reads their lists again once
    /// they have, in the background: `false` (nothing asked) when the last ask was less than 2
    /// minutes ago.
    #[must_use]
    pub fn check_usage(&self) -> bool {
        let now = self.clock.now();
        {
            let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
            if asked.is_some_and(|at| now.duration_since(at) < CHECK_EVERY) {
                return false;
            }
            *asked = Some(now);
        }
        for machine in self.runners.machines() {
            let runners = self.runners.clone();
            tokio::spawn(async move {
                if let Err(error) = runners
                    .api()
                    .call(&machine, Verb::Post, "/accounts/check", None)
                    .await
                {
                    tracing::debug!(%machine, %error, "a runner's accounts could not be checked");
                    return;
                }
                if let Err(error) = runners.refresh_accounts(&machine).await {
                    tracing::debug!(%machine, %error, "a runner's accounts could not be read");
                }
            });
        }
        true
    }

    /// The state's `account`: the account the assistant runs on and its use (shares, 0 to 1),
    /// every field `null` until the runtime has said which one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn state_account(&self) -> Result<api::state::Account, DbError> {
        let on = self.prefs.get(PrefKey::On).await?;
        let accounts = self.known().await?;
        let found = on
            .as_deref()
            .and_then(|on| accounts.iter().find(|account| account.id == on));
        let share = |used: Option<f64>| used.map(|used| used / 100.0);
        Ok(match found {
            None => api::state::Account {
                id: on,
                logged_in: Some(false),
                ..api::state::Account::default()
            },
            Some(account) => api::state::Account {
                id: on.clone(),
                email: account.email.clone(),
                plan: account.plan.clone(),
                logged_in: Some(account.logged_in),
                used5h: share(account.used5h),
                resets5h: account.resets5h.map(timestamp_to_api),
                used7d: share(account.used7d),
                resets7d: account.resets7d.map(timestamp_to_api),
                limited_until: account
                    .limited_until
                    .filter(|_| account.limited(self.clock.now()))
                    .map(timestamp_to_api),
                at: account.checked.map(timestamp_to_api),
            },
        })
    }
}
