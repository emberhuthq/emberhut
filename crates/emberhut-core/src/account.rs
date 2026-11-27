//! The assistant's accounts: the coding-agent accounts the runners use, each one's use of its
//! 5-hour window and its week, and which one the assistant itself runs on.
//!
//! Every runner keeps its own list of accounts and looks at their use itself; the server keeps
//! what each runner last said ([`AccountUse`]) and the person's pin (an account id, or none for
//! automatic). Nothing here names an account: the ids and names are the runners'.

use jiff::{SignedDuration, Timestamp};

/// The scope of an account any work may run on (the runners' word).
pub const SCOPE_ANY: &str = "any";

/// A runner's account and its use, as the runner last reported it.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountUse {
    /// The machine whose runner reported it.
    pub machine: String,
    /// Its id there.
    pub id: String,
    /// Its name, as shown.
    pub name: String,
    /// What work it takes: [`SCOPE_ANY`], or another word of the runner's.
    pub scope: String,
    /// Whether it may be picked.
    pub enabled: bool,
    /// Whether it is logged in.
    pub logged_in: bool,
    /// Its place in the runner's order (lower first).
    pub rank: Option<i64>,
    /// The login's email.
    pub email: Option<String>,
    /// Its plan.
    pub plan: Option<String>,
    /// Its use of the 5-hour window, 0 to 100.
    pub used5h: Option<f64>,
    /// Its use of the week, 0 to 100.
    pub used7d: Option<f64>,
    /// When the 5-hour window resets.
    pub resets5h: Option<Timestamp>,
    /// When the week resets.
    pub resets7d: Option<Timestamp>,
    /// It is out until then.
    pub limited_until: Option<Timestamp>,
    /// Why it is out.
    pub limited_why: Option<String>,
    /// When its runner last looked at its use.
    pub checked: Option<Timestamp>,
    /// What went wrong at that look.
    pub error: Option<String>,
}

/// What a runner's error says when its look found no login.
pub const NOT_LOGGED_IN: &str = "not logged in";

/// What a runner's error starts with when the account's folder is signed in as another login.
pub const WRONG_LOGIN: &str = "logged in as another account";

impl AccountUse {
    /// A new account with nothing known of its use.
    #[must_use]
    pub fn new(machine: impl Into<String>, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            machine: machine.into(),
            id: id.into(),
            name: name.into(),
            scope: SCOPE_ANY.to_owned(),
            enabled: true,
            logged_in: false,
            rank: None,
            email: None,
            plan: None,
            used5h: None,
            used7d: None,
            resets5h: None,
            resets7d: None,
            limited_until: None,
            limited_why: None,
            checked: None,
            error: None,
        }
    }

    /// Whether it is out (refused) at `now`.
    #[must_use]
    pub fn limited(&self, now: Timestamp) -> bool {
        self.limited_until.is_some_and(|until| until > now)
    }

    /// The room it has left at `now`, 0 to 100: a 5-hour window that resets within the hour, or
    /// a week that resets within 12 hours, counts as fresh.
    #[must_use]
    pub fn room(&self, now: Timestamp) -> f64 {
        let soon = |at: Option<Timestamp>, hours: i64| {
            at.is_some_and(|at| at.duration_since(now) < SignedDuration::from_hours(hours))
        };
        let five = if soon(self.resets5h, 1) {
            0.0
        } else {
            self.used5h.unwrap_or(0.0)
        };
        let week = if soon(self.resets7d, 12) {
            0.0
        } else {
            self.used7d.unwrap_or(0.0)
        };
        100.0 - five.max(week)
    }

    /// Whether the assistant may run on it now: on, logged in, not out, of any work's scope, and
    /// its login the one it is for.
    #[must_use]
    pub fn can_take_work(&self, now: Timestamp) -> bool {
        self.enabled
            && self.logged_in
            && self.scope == SCOPE_ANY
            && !self.limited(now)
            && self.error.as_deref() != Some(NOT_LOGGED_IN)
            && !self
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with(WRONG_LOGIN))
    }

    /// Its name without what is in brackets: `Work` for `Work (team)`.
    #[must_use]
    pub fn short_name(&self) -> String {
        short_name(&self.name)
    }
}

/// `name` without anything in brackets, trimmed.
#[must_use]
pub fn short_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0_u32;
    for c in name.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The accounts the assistant may run on now, best first: nearly full ones (under 10% left)
/// last, `prefer` first while it has room, then the runner's order, then the most room.
#[must_use]
pub fn ranked<'a>(
    accounts: &'a [AccountUse],
    prefer: Option<&str>,
    now: Timestamp,
) -> Vec<&'a AccountUse> {
    let mut usable: Vec<&AccountUse> = accounts
        .iter()
        .filter(|account| account.can_take_work(now))
        .collect();
    usable.sort_by(|a, b| {
        let full = |x: &AccountUse| x.room(now) < 10.0;
        let preferred = |x: &AccountUse| Some(x.id.as_str()) == prefer;
        full(a)
            .cmp(&full(b))
            .then_with(|| preferred(b).cmp(&preferred(a)))
            .then_with(|| a.rank.unwrap_or(i64::MAX).cmp(&b.rank.unwrap_or(i64::MAX)))
            .then_with(|| b.room(now).total_cmp(&a.room(now)))
    });
    usable
}

/// Why the person's pin can't be taken.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PinRefusal {
    /// No runner lists an account with that id.
    #[error("No account \"{asked}\". Accounts: {known}, or auto.")]
    Unknown {
        /// The id asked for.
        asked: String,
        /// The accounts it could be, comma-separated.
        known: String,
    },
    /// The account isn't one the assistant runs on (off, or only for some work).
    #[error("{0} isn't one the assistant runs on: it's off or only for some work.")]
    NotForTheAssistant(String),
}

/// Checks that `id` may be pinned: an account some runner lists, on, of any work's scope.
///
/// # Errors
///
/// [`PinRefusal`].
pub fn check_pin(accounts: &[AccountUse], id: &str) -> Result<(), PinRefusal> {
    let Some(account) = accounts.iter().find(|account| account.id == id) else {
        let mut known: Vec<&str> = accounts
            .iter()
            .filter(|account| account.scope == SCOPE_ANY)
            .map(|account| account.id.as_str())
            .collect();
        known.dedup();
        return Err(PinRefusal::Unknown {
            asked: id.to_owned(),
            known: known.join(", "),
        });
    };
    if account.scope != SCOPE_ANY || !account.enabled {
        return Err(PinRefusal::NotForTheAssistant(account.name.clone()));
    }
    Ok(())
}

/// A window's use at `now`: a window whose reset has passed reads 0 and no reset.
#[must_use]
pub fn window(
    used: Option<f64>,
    resets: Option<Timestamp>,
    now: Timestamp,
) -> (Option<f64>, Option<Timestamp>) {
    match resets {
        Some(at) if at <= now => (Some(0.0), None),
        _ => (used, resets),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn account(id: &str, used: f64) -> AccountUse {
        let mut account = AccountUse::new("runner", id, format!("Account {id} (team)"));
        account.logged_in = true;
        account.used5h = Some(used);
        account
    }

    #[test]
    fn the_room_counts_a_window_about_to_reset_as_fresh() {
        let now = at("2026-11-25T10:00:00Z");
        let mut a = account("a", 80.0);
        a.used7d = Some(30.0);
        assert!((a.room(now) - 20.0).abs() < f64::EPSILON);
        a.resets5h = Some(at("2026-11-25T10:30:00Z"));
        assert!((a.room(now) - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ranking_puts_nearly_full_ones_last_and_the_preferred_first() {
        let now = at("2026-11-25T10:00:00Z");
        let mut out = account("out", 0.0);
        out.limited_until = Some(at("2026-11-25T11:00:00Z"));
        let mut off = account("off", 0.0);
        off.enabled = false;
        let accounts = [
            account("full", 95.0),
            account("roomy", 10.0),
            account("half", 50.0),
            out,
            off,
        ];
        let ids =
            |list: Vec<&AccountUse>| list.into_iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(ranked(&accounts, None, now)), ["roomy", "half", "full"]);
        assert_eq!(
            ids(ranked(&accounts, Some("half"), now)),
            ["half", "roomy", "full"]
        );
    }

    #[test]
    fn a_pin_must_name_an_account_the_assistant_runs_on() {
        let mut team = account("team", 0.0);
        team.scope = "team".to_owned();
        let accounts = [account("a", 0.0), team];
        assert_eq!(check_pin(&accounts, "a"), Ok(()));
        assert_eq!(
            check_pin(&accounts, "nope").unwrap_err().to_string(),
            "No account \"nope\". Accounts: a, or auto."
        );
        assert_eq!(
            check_pin(&accounts, "team").unwrap_err().to_string(),
            "Account team (team) isn't one the assistant runs on: it's off or only for some work."
        );
        assert_eq!(short_name("Account team (team)"), "Account team");
    }

    #[test]
    fn a_window_past_its_reset_reads_empty() {
        let now = at("2026-11-25T10:00:00Z");
        assert_eq!(
            window(Some(40.0), Some(at("2026-11-25T09:00:00Z")), now),
            (Some(0.0), None)
        );
        let later = Some(at("2026-11-25T11:00:00Z"));
        assert_eq!(window(Some(40.0), later, now), (Some(40.0), later));
    }
}
