//! The hut's Claude accounts: the plans the person already pays for (Claude Pro or Max), each
//! signed in on its own CLI folder (`<data>/accounts/<id>`, its `CLAUDE_CONFIG_DIR`). The
//! person signs in through Anthropic's own flow: `claude auth login --claudeai` opens their
//! browser on this computer and the CLI keeps what it gets in its folder. The app never sees a
//! password or a token; it only asks the CLI whether a folder is signed in, as whom and on
//! which plan (`claude auth status --json`). The hut's server runs its sessions on the
//! signed-in folders (`--agent-accounts`): all of them, in order, when the person lets the
//! ember move to one with room as one runs out; else only the first.
//!
//! A debug or automation build may stand in for the sign-in ([`STANDIN_ENV`], the address of
//! a stand-in model): the folder is then signed in at once, its CLI pointed at the stand-in,
//! so an unseen run never opens a browser.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::process::Run;
use super::{HutError, LocalHut, READS_TEST_VARS, lock};
use crate::private_files;

/// The variable that, in a debug or automation build, names a stand-in model's address: a
/// sign-in then points the account's CLI at it instead of opening a browser.
pub(crate) const STANDIN_ENV: &str = "EMBERHUT_LOGIN_STANDIN";

/// The file a stand-in sign-in leaves in the account's folder, with who it stands for.
const STANDIN_FILE: &str = "standin.json";

/// One of the hut's accounts, as its CLI says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HutAccount {
    /// Its id (`claude-1`, …), the server's name for it.
    pub(crate) id: String,
    /// Whether the CLI is signed in on it.
    pub(crate) signed_in: bool,
    /// The email it is signed in as, when the CLI says.
    pub(crate) email: Option<String>,
    /// Its plan (`pro`, `max`), when the CLI says.
    pub(crate) plan: Option<String>,
}

impl HutAccount {
    /// The plan as the person knows it: "Claude Max", "Claude Pro", or "Claude".
    #[must_use]
    pub(crate) fn plan_name(&self) -> String {
        match self.plan.as_deref() {
            Some(plan) if !plan.is_empty() => {
                let mut chars = plan.chars();
                let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
                format!("Claude {}{}", first.unwrap_or_default(), chars.as_str())
            }
            _ => "Claude".to_owned(),
        }
    }
}

/// What `claude auth status --json` says (only what the app shows).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Status {
    logged_in: bool,
    email: Option<String>,
    subscription_type: Option<String>,
}

impl Status {
    /// What `text` (the command's output) says; signed out when it isn't that.
    fn read(text: &str) -> Self {
        serde_json::from_str(text.trim()).unwrap_or_default()
    }
}

/// The `--agent-accounts` value for `accounts` signed in (`id=dir`, comma-separated), in
/// order: every one when the ember `moves`, else the first; `None` when none is signed in.
#[must_use]
pub(crate) fn agent_accounts(
    accounts: &[(HutAccount, PathBuf)],
    moves: bool,
) -> Option<String> {
    let mut signed_in = accounts.iter().filter(|(account, _)| account.signed_in);
    let chosen: Vec<String> = if moves {
        signed_in
            .map(|(account, dir)| format!("{}={}", account.id, dir.display()))
            .collect()
    } else {
        signed_in
            .next()
            .map(|(account, dir)| format!("{}={}", account.id, dir.display()))
            .into_iter()
            .collect()
    };
    (!chosen.is_empty()).then(|| chosen.join(","))
}

impl LocalHut {
    /// The accounts folder.
    fn accounts_dir(&self) -> PathBuf {
        self.inner.paths.data().join("accounts")
    }

    /// Account `id`'s CLI folder.
    fn account_dir(&self, id: &str) -> PathBuf {
        self.accounts_dir().join(id)
    }

    /// The stand-in model's address, in a build that reads it and when it is set.
    fn standin(&self) -> Option<String> {
        if !READS_TEST_VARS {
            return None;
        }
        self.inner.paths.var(STANDIN_ENV)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    }

    /// The hut's accounts, in order, each as its CLI says now.
    #[must_use]
    pub(crate) fn accounts_now(&self) -> Vec<HutAccount> {
        self.read_state()
            .accounts
            .iter()
            .map(|id| self.account(id))
            .collect()
    }

    /// The accounts with their folders, for the server's flags.
    pub(super) fn account_flags(&self) -> Option<String> {
        let state = self.read_state();
        let accounts: Vec<(HutAccount, PathBuf)> = state
            .accounts
            .iter()
            .map(|id| (self.account(id), self.account_dir(id)))
            .collect();
        agent_accounts(&accounts, state.moves)
    }

    /// Account `id` as its CLI says.
    fn account(&self, id: &str) -> HutAccount {
        let dir = self.account_dir(id);
        let status = match std::fs::read_to_string(dir.join(STANDIN_FILE)) {
            Ok(text) => Status::read(&text),
            Err(_) => self.ask_status(&dir),
        };
        HutAccount {
            id: id.to_owned(),
            signed_in: status.logged_in,
            email: status.email,
            plan: status.subscription_type,
        }
    }

    /// `claude auth status --json` on the folder `dir`.
    fn ask_status(&self, dir: &Path) -> Status {
        let Some(agent) = self.inner.paths.agent() else {
            return Status::default();
        };
        let run = Run::new(agent)
            .args(["auth", "status", "--json"])
            .env("CLAUDE_CONFIG_DIR", dir.as_os_str().to_owned());
        match self.inner.spawner.run(&run) {
            // It answers in JSON whether signed in or not.
            Ok(ran) => Status::read(&ran.stdout),
            Err(error) => {
                log::warn!("couldn't ask the CLI how account {} is signed in: {error}", dir.display());
                Status::default()
            }
        }
    }

    /// Adds an account and starts its sign-in: Anthropic's page opens in the browser, and
    /// the account is signed in once the person finishes there ([`LocalHut::accounts_now`]
    /// tells). Gives its id.
    ///
    /// # Errors
    ///
    /// Its folder can't be made, there is no `claude` on this computer, or it didn't start.
    pub(crate) fn add_account_now(&self) -> Result<String, HutError> {
        let _busy = lock(&self.inner.busy);
        self.prepare()?;
        let mut state = self.read_state();
        let id = (1..)
            .map(|n| format!("claude-{n}"))
            .find(|id| !state.accounts.contains(id) && !self.account_dir(id).exists())
            .unwrap_or_default();
        let dir = self.account_dir(&id);
        for folder in [self.accounts_dir(), dir.clone()] {
            private_files::folder(&folder).map_err(|source| HutError::Files {
                path: folder.clone(),
                source,
            })?;
        }
        state.accounts.push(id.clone());
        self.write_state(&state)?;
        self.sign_in(&dir)?;
        Ok(id)
    }

    /// Signs account `id` in again (its sign-in ran out, or the person closed the page).
    ///
    /// # Errors
    ///
    /// As [`LocalHut::add_account_now`].
    pub(crate) fn sign_in_again_now(&self, id: &str) -> Result<(), HutError> {
        self.sign_in(&self.account_dir(id))
    }

    /// Starts the CLI's sign-in on the folder `dir`, or a stand-in's.
    fn sign_in(&self, dir: &Path) -> Result<(), HutError> {
        if let Some(standin) = self.standin() {
            return write_standin(dir, &standin);
        }
        let agent = self.inner.paths.agent().ok_or(HutError::NoAgent)?;
        let run = Run::new(agent)
            .args(["auth", "login", "--claudeai"])
            .env("CLAUDE_CONFIG_DIR", dir.as_os_str().to_owned())
            .log(self.inner.paths.log());
        let login = self.inner.spawner.spawn(&run).map_err(HutError::Launch)?;
        lock(&self.inner.logins).push(login);
        Ok(())
    }

    /// Takes account `id` off: the CLI signs out of it and its folder goes.
    ///
    /// # Errors
    ///
    /// The hut's state can't be written.
    pub(crate) fn remove_account_now(&self, id: &str) -> Result<(), HutError> {
        let _busy = lock(&self.inner.busy);
        let mut state = self.read_state();
        state.accounts.retain(|known| known != id);
        self.write_state(&state)?;
        let dir = self.account_dir(id);
        if let Some(agent) = self.inner.paths.agent()
            && !dir.join(STANDIN_FILE).exists()
        {
            let run = Run::new(agent)
                .args(["auth", "logout"])
                .env("CLAUDE_CONFIG_DIR", dir.as_os_str().to_owned());
            if let Err(error) = self.inner.spawner.run(&run) {
                log::warn!("couldn't sign the CLI out of account {id}: {error}");
            }
        }
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            log::warn!("couldn't remove account {id}'s folder: {error}");
        }
        Ok(())
    }

    /// Puts account `id` first, so the next work goes to it.
    ///
    /// # Errors
    ///
    /// The hut's state can't be written.
    pub(crate) fn put_first_now(&self, id: &str) -> Result<(), HutError> {
        let _busy = lock(&self.inner.busy);
        let mut state = self.read_state();
        if let Some(at) = state.accounts.iter().position(|known| known == id) {
            let account = state.accounts.remove(at);
            state.accounts.insert(0, account);
            self.write_state(&state)?;
        }
        Ok(())
    }

    /// Whether the ember moves to an account with room when one runs out.
    #[must_use]
    pub(crate) fn moves_now(&self) -> bool {
        self.read_state().moves
    }

    /// Lets the ember move between the accounts as one runs out, or not.
    ///
    /// # Errors
    ///
    /// The hut's state can't be written.
    pub(crate) fn set_moves_now(&self, moves: bool) -> Result<(), HutError> {
        let _busy = lock(&self.inner.busy);
        let mut state = self.read_state();
        state.moves = moves;
        self.write_state(&state)
    }

    /// Starts the hut again on the accounts as they are now, and gives its address.
    ///
    /// # Errors
    ///
    /// As [`LocalHut::start`].
    pub(crate) fn use_accounts_now(&self) -> Result<crate::config::ServerUrl, HutError> {
        let _busy = lock(&self.inner.busy);
        self.light(true).map(crate::config::ServerUrl::loopback)
    }
}

/// A stand-in sign-in on the folder `dir`: the CLI pointed at the stand-in model at `url`, and
/// who it stands for.
fn write_standin(dir: &Path, url: &str) -> Result<(), HutError> {
    let settings = serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": url,
            "ANTHROPIC_AUTH_TOKEN": "preview-key",
            "ANTHROPIC_MODEL": "preview-stand-in",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": "preview-stand-in",
        }
    });
    let n = dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit('-').next())
        .unwrap_or("1")
        .to_owned();
    let who = serde_json::json!({
        "loggedIn": true,
        "email": if n == "1" { "you@example.com".to_owned() } else { format!("work{n}@example.com") },
        "subscriptionType": if n == "1" { "max" } else { "pro" },
    });
    for (name, value) in [("settings.json", settings), (STANDIN_FILE, who)] {
        let path = dir.join(name);
        let bytes = serde_json::to_vec_pretty(&value).unwrap_or_default();
        std::io::Write::write_all(
            &mut private_files::file(&path).map_err(|source| HutError::Files {
                path: path.clone(),
                source,
            })?,
            &bytes,
        )
        .map_err(|source| HutError::Files { path, source })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, signed_in: bool) -> (HutAccount, PathBuf) {
        (
            HutAccount {
                id: id.to_owned(),
                signed_in,
                ..HutAccount::default()
            },
            PathBuf::from(format!("/h/accounts/{id}")),
        )
    }

    #[test]
    fn the_server_runs_on_the_signed_in_accounts_and_moves_only_when_asked() {
        let accounts = [account("claude-1", true), account("claude-2", false), account("claude-3", true)];
        assert_eq!(
            agent_accounts(&accounts, true).as_deref(),
            Some("claude-1=/h/accounts/claude-1,claude-3=/h/accounts/claude-3")
        );
        assert_eq!(
            agent_accounts(&accounts, false).as_deref(),
            Some("claude-1=/h/accounts/claude-1")
        );
        assert_eq!(agent_accounts(&[account("claude-2", false)], true), None);
    }

    #[test]
    fn the_cli_status_reads_who_and_which_plan() {
        let status = Status::read(r#"{"loggedIn":true,"email":"a@b.c","subscriptionType":"max","orgId":"x"}"#);
        assert!(status.logged_in);
        assert_eq!(status.email.as_deref(), Some("a@b.c"));
        assert!(!Status::read("not json").logged_in);
        let max = HutAccount { plan: Some("max".into()), ..HutAccount::default() };
        assert_eq!(max.plan_name(), "Claude Max");
        assert_eq!(HutAccount::default().plan_name(), "Claude");
    }
}
