//! Device sessions: the rules for how a device that has proven itself stays signed in.
//!
//! A device gets two [`Token`]s: a short-lived access token it sends with every request, and
//! a refresh token it trades for a new pair when the access token runs out. Only the tokens'
//! hashes are stored.
//!
//! - A refresh turns both tokens over. The previous access token keeps working until its own
//!   expiry, so requests already on their way with it are not refused.
//! - The previous refresh token is kept, to tell a retry from a reuse. Presented within
//!   [`REFRESH_REUSE_GRACE`] of the turnover, it is most likely a client that never got the
//!   answer to its refresh (a dropped connection) and tries again: it gets a fresh pair, and the
//!   pair issued moments before stops working, since its holder never had it. The cost: a
//!   copy of the previous token used within that window gets the session, and the device's own
//!   newer tokens stop. Presented later, it is reused: the device is signed out.
//! - A device has one row per install: the same install signing in again takes over its row,
//!   and its old tokens stop working.
//! - A device that goes unused for [`IDLE_SIGN_OUT`] is signed out.

use jiff::{SignedDuration, Timestamp};

use crate::device;
use crate::ids::DeviceId;
use crate::random::{RandomError, RandomSource};
use crate::secret::SecretHash;
use crate::token::Token;

/// How long an access token works.
pub const ACCESS_TOKEN_LIFETIME: SignedDuration = SignedDuration::from_hours(1);

/// A device that has not used its session for this long is signed out.
pub const IDLE_SIGN_OUT: SignedDuration = SignedDuration::from_hours(14 * 24);

/// How long after a refresh the previous refresh token may still come back, as a client's
/// retry after a lost answer, and be given a fresh pair instead of signing the device out.
pub const REFRESH_REUSE_GRACE: SignedDuration = SignedDuration::from_mins(2);

/// How often a device's last-seen time is written down: a busy device would otherwise write
/// on every request.
pub const SEEN_RESOLUTION: SignedDuration = SignedDuration::from_mins(1);

/// Devices last seen before this instant are idle at `now`.
#[must_use]
pub fn idle_before(now: Timestamp) -> Timestamp {
    now - IDLE_SIGN_OUT
}

/// A fresh pair of tokens, as they are handed to a device. They cannot be shown again.
#[derive(Clone, Debug)]
pub struct IssuedTokens {
    /// The token the device sends with each request.
    pub access: Token,
    /// The token it trades for new ones when the access token runs out.
    pub refresh: Token,
    /// When the access token stops working.
    pub access_expires_at: Timestamp,
}

impl IssuedTokens {
    /// New tokens, issued at `now`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when `random` fails.
    pub fn issue(random: &dyn RandomSource, now: Timestamp) -> Result<Self, RandomError> {
        Ok(Self {
            access: Token::generate(random)?,
            refresh: Token::generate(random)?,
            access_expires_at: now + ACCESS_TOKEN_LIFETIME,
        })
    }
}

/// A device's open session: the device and the tokens it was just handed.
#[derive(Clone, Debug)]
pub struct Session {
    /// The device the session is for.
    pub device: DeviceId,
    /// Its tokens.
    pub tokens: IssuedTokens,
}

/// A stored access token's session, as found by the token's hash (current or previous).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessRecord {
    /// The device it belongs to.
    pub device: DeviceId,
    /// When this access token stops working.
    pub expires_at: Timestamp,
    /// When the device was last seen.
    pub device_seen_at: Timestamp,
}

impl AccessRecord {
    /// Whether the access token still works at `now`.
    #[must_use]
    pub fn works_at(&self, now: Timestamp) -> bool {
        now < self.expires_at
    }

    /// Whether the device's last-seen time is old enough at `now` to be written again.
    #[must_use]
    pub fn seen_is_stale(&self, now: Timestamp) -> bool {
        self.device_seen_at < now - SEEN_RESOLUTION
    }
}

/// Which of a session's refresh tokens a presented one is.
#[derive(Clone, Copy, Debug)]
pub enum RefreshMatch {
    /// The current one.
    Current,
    /// The one it replaced, at `rotated_at`.
    Previous {
        /// When it was replaced.
        rotated_at: Timestamp,
        /// The hash of the current refresh token as it was read, so that a re-issue replaces
        /// that pair only, and of two re-issues racing exactly one wins.
        current: SecretHash,
    },
}

/// A stored refresh token's session, as found by the token's hash.
#[derive(Clone, Debug)]
pub struct RefreshRecord {
    /// The device it belongs to.
    pub device: DeviceId,
    /// Whether the token is the session's current one or the previous one.
    pub matched: RefreshMatch,
    /// When the device was last seen.
    pub device_seen_at: Timestamp,
    /// The device's install id, if it has one.
    pub device_install: Option<String>,
}

/// What to do with a presented refresh token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshVerdict {
    /// Turn the tokens over.
    Renew,
    /// Refuse, and sign the device out: it has been idle too long.
    Idle,
    /// Issue a fresh pair in place of the current one: the previous token came back soon after
    /// the turnover, most likely from a client that lost the answer and is retrying.
    Reissue,
    /// Refuse, and sign the device out: the previous token was reused after the grace.
    Reused,
}

impl RefreshRecord {
    /// What to do with this token at `now`.
    #[must_use]
    pub fn verdict(&self, now: Timestamp) -> RefreshVerdict {
        match self.matched {
            RefreshMatch::Previous { rotated_at, .. }
                if now - rotated_at <= REFRESH_REUSE_GRACE =>
            {
                RefreshVerdict::Reissue
            }
            RefreshMatch::Previous { .. } => RefreshVerdict::Reused,
            RefreshMatch::Current if self.device_seen_at < idle_before(now) => RefreshVerdict::Idle,
            RefreshMatch::Current => RefreshVerdict::Renew,
        }
    }

    /// The install id to give the device, from the one its app `offered`: only a device that
    /// has none takes one. (Whether another device holds it already is for the store.)
    #[must_use]
    pub fn install_to_claim(&self, offered: Option<&str>) -> Option<String> {
        if self.device_install.is_some() {
            return None;
        }
        device::install_id(offered)
    }
}

/// A signed-in device, as the device list shows it. No tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    /// Its id.
    pub id: DeviceId,
    /// Its name.
    pub name: String,
    /// Its hardware model; may be empty.
    pub model: String,
    /// When it first signed in.
    pub created_at: Timestamp,
    /// When it last used its session.
    pub seen_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::OsRandom;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + seconds).unwrap()
    }

    fn record(matched: RefreshMatch, seen: i64) -> RefreshRecord {
        RefreshRecord {
            device: DeviceId::new(1),
            matched,
            device_seen_at: at(seen),
            device_install: None,
        }
    }

    #[test]
    fn issued_tokens_differ_and_last_an_hour() {
        let tokens = IssuedTokens::issue(&OsRandom, at(0)).unwrap();
        assert_ne!(tokens.access.as_str(), tokens.refresh.as_str());
        assert_eq!(tokens.access_expires_at, at(3_600));
    }

    #[test]
    fn access_token_works_until_it_expires() {
        let record = AccessRecord {
            device: DeviceId::new(1),
            expires_at: at(3_600),
            device_seen_at: at(0),
        };
        assert!(record.works_at(at(3_599)));
        assert!(!record.works_at(at(3_600)));
    }

    #[test]
    fn seen_time_is_written_once_a_minute() {
        let record = AccessRecord {
            device: DeviceId::new(1),
            expires_at: at(3_600),
            device_seen_at: at(0),
        };
        assert!(!record.seen_is_stale(at(60)));
        assert!(record.seen_is_stale(at(61)));
    }

    #[test]
    fn the_current_token_renews_until_the_device_is_idle_for_two_weeks() {
        let two_weeks = IDLE_SIGN_OUT.as_secs();
        let current = record(RefreshMatch::Current, 0);
        assert_eq!(current.verdict(at(two_weeks)), RefreshVerdict::Renew);
        assert_eq!(current.verdict(at(two_weeks + 1)), RefreshVerdict::Idle);
    }

    #[test]
    fn the_previous_token_is_reissued_for_two_minutes_then_a_reuse() {
        let previous = record(
            RefreshMatch::Previous {
                rotated_at: at(0),
                current: SecretHash::of("current"),
            },
            0,
        );
        assert_eq!(previous.verdict(at(120)), RefreshVerdict::Reissue);
        assert_eq!(previous.verdict(at(121)), RefreshVerdict::Reused);
    }

    #[test]
    fn only_a_device_without_an_install_id_takes_one() {
        let mut record = record(RefreshMatch::Current, 0);
        assert_eq!(
            record.install_to_claim(Some(" install-1 ")).as_deref(),
            Some("install-1")
        );
        assert_eq!(record.install_to_claim(Some("  ")), None);
        assert_eq!(record.install_to_claim(None), None);
        record.device_install = Some("install-0".to_owned());
        assert_eq!(record.install_to_claim(Some("install-1")), None);
    }
}
