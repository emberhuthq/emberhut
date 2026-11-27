//! The accounts the assistant's sessions run on, and moving a session to another when its
//! account runs out of usage.
//!
//! A session stays on the account it is on while that one can take work; once it is out (the
//! service refused a turn, or said so in the turn's answer), it is marked out until the time
//! the service gave (an hour when it gave none) and the session moves to the first account in
//! the configured order that can take work. With every account out, nothing moves and the
//! answer reaches the chat as it came.
//!
//! An answer that says the CLI isn't logged in at all ([`says_not_logged_in`]) is read here
//! too. It is not an account running out, so nothing moves: the agent has nothing to think
//! with until its model provider is set up.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Timestamp};

/// How long an account is out when the service didn't say until when.
pub const OUT_WITHOUT_A_TIME: SignedDuration = SignedDuration::from_hours(1);

/// How long a pretend limit (the `testLimit` setting) keeps an account out.
pub const TEST_LIMIT: SignedDuration = SignedDuration::from_mins(10);

/// Why an account can't take work, and until when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limit {
    /// Until when it is out.
    pub until: Timestamp,
    /// Why, in a few words ("hit its five-hour limit").
    pub why: String,
}

/// A turn's account ran out: why, and until when if the service said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitHit {
    /// Why, in a few words.
    pub why: String,
    /// When the account takes work again, if the service said.
    pub until: Option<Timestamp>,
}

impl LimitHit {
    /// The pretend limit of the `testLimit` setting: out for [`TEST_LIMIT`] from `now`.
    #[must_use]
    pub fn test(now: Timestamp) -> Self {
        Self {
            why: "hit a test limit".to_owned(),
            until: now.checked_add(TEST_LIMIT).ok(),
        }
    }

    /// The limit a rejected rate-limit notice gives: `kind` is the service's window name
    /// (`five_hour`, `seven_day`), written with a dash (`five-hour`).
    #[must_use]
    pub fn rejected(kind: Option<&str>, until: Option<Timestamp>) -> Self {
        let kind = kind.unwrap_or("usage").replacen('_', "-", 1);
        Self {
            why: format!("hit its {kind} limit"),
            until,
        }
    }

    /// The limit an answer names without a time ("You've hit your limit").
    #[must_use]
    pub fn usage() -> Self {
        Self {
            why: "hit its usage limit".to_owned(),
            until: None,
        }
    }
}

/// The accounts, in the order they are tried, and which are out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountPool {
    order: Vec<String>,
    out: BTreeMap<String, Limit>,
}

impl AccountPool {
    /// The accounts `ids`, tried in this order (a repeated id counts once).
    #[must_use]
    pub fn new(ids: impl IntoIterator<Item = String>) -> Self {
        let mut order: Vec<String> = Vec::new();
        for id in ids {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        Self {
            order,
            out: BTreeMap::new(),
        }
    }

    /// The accounts, in order.
    #[must_use]
    pub fn ids(&self) -> &[String] {
        &self.order
    }

    /// Whether there are no accounts (sessions run on the CLI's own login, and never move).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Why `id` is out at `now`, if it is.
    #[must_use]
    pub fn limit(&self, id: &str, now: Timestamp) -> Option<&Limit> {
        self.out.get(id).filter(|limit| limit.until > now)
    }

    /// Whether `id` is one of the accounts and can take work at `now`.
    #[must_use]
    pub fn can_work(&self, id: &str, now: Timestamp) -> bool {
        self.order.iter().any(|known| known == id) && self.limit(id, now).is_none()
    }

    /// Where a session on `current` runs at `now`: there while it can take work, else the
    /// first account in order that can and isn't in `exclude`. None when none can.
    #[must_use]
    pub fn choose(&self, current: Option<&str>, exclude: &[&str], now: Timestamp) -> Option<&str> {
        let usable = |id: &str| !exclude.contains(&id) && self.can_work(id, now);
        if let Some(current) = current
            && usable(current)
            && let Some(kept) = self.order.iter().find(|id| id.as_str() == current)
        {
            return Some(kept);
        }
        self.order.iter().map(String::as_str).find(|id| usable(id))
    }

    /// Marks `id` out after `hit`, until its time (or [`OUT_WITHOUT_A_TIME`] from `now`).
    pub fn mark_out(&mut self, id: &str, hit: &LimitHit, now: Timestamp) {
        let until = hit
            .until
            .filter(|until| *until > now)
            .or_else(|| now.checked_add(OUT_WITHOUT_A_TIME).ok())
            .unwrap_or(now);
        self.out.insert(
            id.to_owned(),
            Limit {
                until,
                why: hit.why.clone(),
            },
        );
    }
}

/// Whether a turn's answer says its account ran out (`You've hit your limit · resets 5pm`,
/// `usage limit reached`), the wordings the CLI uses. Only an error result is read this way:
/// the assistant talks about limits itself.
#[must_use]
pub fn says_out_of_usage(text: &str) -> bool {
    let text = text.to_lowercase();
    hit_your_limit(&text)
        || text.contains("usage limit reached")
        || text.contains("out of usage")
        || text.contains("out of extra usage")
        || rate_limited_until(&text)
        || text.contains("limit · resets")
}

/// The longest answer that is read as the CLI's own line about its login rather than the
/// assistant's words.
const LOGIN_LINE: usize = 200;

/// Whether a turn's answer is the CLI saying it has nothing to sign in with (`Not logged in ·
/// Please run /login`, `Invalid API key · Please run /login`), the wordings Claude Code uses.
/// An error result (`error`) counts when it asks for `/login` or starts with `not logged in`;
/// any other answer only when it is that one short line, both at once, as the assistant may
/// talk about logging in itself.
#[must_use]
pub fn says_not_logged_in(text: &str, error: bool) -> bool {
    let text = text.trim().to_lowercase();
    let not_logged_in = text.starts_with("not logged in");
    if error {
        asks_for_login(&text) || not_logged_in
    } else {
        text.chars().count() <= LOGIN_LINE && not_logged_in && asks_for_login(&text)
    }
}

/// `run /login` as the CLI writes it: `/login` not run into a longer word.
fn asks_for_login(text: &str) -> bool {
    text.match_indices("run /login").any(|(at, found)| {
        text[at + found.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
    })
}

/// `hit your limit`, or `hit your <word> limit`.
fn hit_your_limit(text: &str) -> bool {
    text.match_indices("hit your ").any(|(at, found)| {
        let rest = &text[at + found.len()..];
        if rest.starts_with("limit") {
            return true;
        }
        let word: usize = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .map(char::len_utf8)
            .sum();
        word > 0 && rest[word..].starts_with(" limit")
    })
}

/// `rate limit` (or `rate limited`) as a word, with `reset` or `try again` after it.
fn rate_limited_until(text: &str) -> bool {
    text.match_indices("rate limit").any(|(at, found)| {
        let mut rest = &text[at + found.len()..];
        if let Some(after) = rest.strip_prefix("ed") {
            rest = after;
        }
        let ends_word = rest
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        ends_word && (rest.contains("reset") || rest.contains("try again"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn pool() -> AccountPool {
        AccountPool::new(["personal", "work", "spare", "work"].map(str::to_owned))
    }

    #[test]
    fn a_session_stays_where_it_is_while_that_account_can_work() {
        let now = at("2026-11-26T10:00:00Z");
        let pool = pool();
        assert_eq!(pool.ids(), ["personal", "work", "spare"]);
        assert_eq!(pool.choose(Some("work"), &[], now), Some("work"));
        assert_eq!(pool.choose(None, &[], now), Some("personal"));
        assert_eq!(pool.choose(Some("gone"), &[], now), Some("personal"));
    }

    #[test]
    fn an_account_that_is_out_is_left_until_its_time() {
        let now = at("2026-11-26T10:00:00Z");
        let mut pool = pool();
        let hit = LimitHit::rejected(Some("five_hour"), Some(at("2026-11-26T12:00:00Z")));
        assert_eq!(hit.why, "hit its five-hour limit");
        pool.mark_out("personal", &hit, now);
        assert_eq!(pool.choose(Some("personal"), &[], now), Some("work"));
        assert_eq!(pool.choose(None, &["work"], now), Some("spare"));
        assert_eq!(
            pool.limit("personal", now).map(|l| l.why.as_str()),
            Some("hit its five-hour limit")
        );
        let later = at("2026-11-26T12:00:01Z");
        assert_eq!(pool.choose(None, &[], later), Some("personal"));
        assert!(pool.limit("personal", later).is_none());
    }

    #[test]
    fn no_time_given_is_an_hour_and_none_left_is_none() {
        let now = at("2026-11-26T10:00:00Z");
        let mut pool = AccountPool::new(["only".to_owned()]);
        pool.mark_out("only", &LimitHit::usage(), now);
        assert_eq!(
            pool.limit("only", now).map(|l| l.until),
            Some(at("2026-11-26T11:00:00Z"))
        );
        assert_eq!(pool.choose(Some("only"), &[], now), None);
        assert!(AccountPool::default().is_empty());
        let test = LimitHit::test(now);
        assert_eq!(test.until, Some(at("2026-11-26T10:10:00Z")));
    }

    #[test]
    fn the_cli_s_wordings_for_running_out_are_recognised() {
        for text in [
            "You've hit your limit · resets 5pm",
            "You've hit your weekly limit",
            "Claude usage limit reached. Your limit will reset at 5pm",
            "You're out of extra usage",
            "API Error: Rate limited, please try again later",
            "rate limit exceeded; resets in 2h",
        ] {
            assert!(says_out_of_usage(text), "{text}");
        }
        for text in [
            "I'll keep an eye on the limit",
            "The rate limiter is configured",
            "hit your target",
            "",
        ] {
            assert!(!says_out_of_usage(text), "{text}");
        }
    }

    #[test]
    fn the_cli_s_wordings_for_no_login_are_recognised() {
        for text in [
            "Not logged in · Please run /login",
            "  NOT LOGGED IN · please run /login.\n",
            "Invalid API key · Please run /login",
            "OAuth token has expired. Please run /login",
            "Not logged in",
        ] {
            assert!(says_not_logged_in(text, true), "{text}");
        }
        assert!(says_not_logged_in(
            "Not logged in · Please run /login",
            false
        ));
        for (text, error) in [
            ("Invalid API key · Please run /login", false),
            ("To sign in again, run /login in a terminal.", false),
            ("Not logged in to Gmail yet.", false),
            ("Please run /loginhelper", true),
            ("", true),
        ] {
            assert!(!says_not_logged_in(text, error), "{text}");
        }
        let long = format!("Not logged in · Please run /login {}", "word ".repeat(50));
        assert!(!says_not_logged_in(&long, false));
        assert!(says_not_logged_in(&long, true));
    }
}
