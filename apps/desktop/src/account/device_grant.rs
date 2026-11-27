//! The OAuth device authorization grant (RFC 8628), the sign-in every build can use: the app
//! shows a code, the person approves it on the provider's page in their browser, and the app
//! polls the token endpoint at the interval given, slower after `slow_down`, until it gets
//! tokens, is refused, or the code's `expires_in` has passed.

use std::fmt;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::http::Request;
use super::oidc::{SCOPES, TokenAnswer, TokenReply};

/// The interval when the provider gives none (RFC 8628, section 3.2).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// How much slower each `slow_down` makes the polls (RFC 8628, section 3.5).
const SLOWER: Duration = Duration::from_secs(5);
/// The longest interval taken from the provider, or reached by slowing down.
const MAX_INTERVAL: Duration = Duration::from_mins(1);
/// The longest a code is waited for, whatever `expires_in` says.
const MAX_LIFETIME: Duration = Duration::from_hours(1);

/// Starts the device grant at the provider's device authorization endpoint.
#[must_use]
pub(crate) fn start_request(endpoint: &str, client_id: &str) -> Request {
    Request::form(
        endpoint,
        vec![
            ("client_id", client_id.to_owned()),
            ("scope", SCOPES.to_owned()),
        ],
    )
}

/// The device authorization endpoint's answer.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct DeviceAuthorization {
    /// The code the app polls with. Never shown.
    pub(crate) device_code: String,
    /// The code the person types on the provider's page.
    pub(crate) user_code: String,
    /// The page.
    pub(crate) verification_uri: String,
    /// The page with the code filled in, when the provider gives it.
    #[serde(default)]
    pub(crate) verification_uri_complete: Option<String>,
    /// How long the codes work, in seconds.
    pub(crate) expires_in: u64,
    /// The least time between polls, in seconds.
    #[serde(default)]
    pub(crate) interval: Option<u64>,
}

impl DeviceAuthorization {
    /// The page to open: the one with the code filled in when there is one.
    #[must_use]
    pub(crate) fn page(&self) -> &str {
        self.verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri)
    }
}

// Its `Debug` leaves the device code out.
impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceAuthorization")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

/// How one poll went.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Poll {
    /// Tokens: the person approved.
    Granted(TokenAnswer),
    /// `authorization_pending`: not yet.
    Pending,
    /// `slow_down`: not yet, and poll less often.
    SlowDown,
    /// `access_denied`: the person said no.
    Denied,
    /// `expired_token`: the code ran out.
    Expired,
    /// Another refusal, or an answer that isn't one, which waiting won't change.
    Failed(String),
    /// No answer (the network): try again at the next interval.
    Unanswered,
}

impl Poll {
    /// A poll that the token endpoint answered with `reply`.
    #[must_use]
    pub(crate) fn from_reply(reply: TokenReply) -> Self {
        match reply {
            TokenReply::Granted(tokens) => Self::Granted(tokens),
            TokenReply::Refused(error) => match error.as_str() {
                "authorization_pending" => Self::Pending,
                "slow_down" => Self::SlowDown,
                "access_denied" => Self::Denied,
                "expired_token" => Self::Expired,
                _ => Self::Failed(error),
            },
            TokenReply::Unavailable { status: 429, .. } => Self::SlowDown,
            TokenReply::Unavailable { .. } => Self::Unanswered,
            TokenReply::Failed(status) => Self::Failed(format!("status {status}")),
        }
    }
}

/// How the grant ended without tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GrantEnd {
    /// The person said no on the provider's page.
    Denied,
    /// The code ran out before it was approved.
    Expired,
    /// The provider refused it otherwise.
    Failed(String),
}

impl fmt::Display for GrantEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied => f.write_str("Sign-in was declined on the sign-in page."),
            Self::Expired => f.write_str("The sign-in code ran out. Start again for a new one."),
            Self::Failed(why) => write!(f, "The account's provider refused the sign-in ({why})."),
        }
    }
}

/// What to do after a poll.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// Poll again after this long.
    Wait(Duration),
    /// Signed in.
    Done(TokenAnswer),
    /// Stop: no tokens will come.
    Stop(GrantEnd),
}

/// The grant under way: its interval and its deadline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceGrant {
    interval: Duration,
    deadline: Instant,
}

impl DeviceGrant {
    /// The grant `authorization` started, received at `started`.
    #[must_use]
    pub(crate) fn new(authorization: &DeviceAuthorization, started: Instant) -> Self {
        let lifetime = Duration::from_secs(authorization.expires_in).min(MAX_LIFETIME);
        Self {
            interval: authorization
                .interval
                .filter(|&seconds| seconds > 0)
                .map_or(DEFAULT_INTERVAL, Duration::from_secs)
                .min(MAX_INTERVAL),
            deadline: started.checked_add(lifetime).unwrap_or(started),
        }
    }

    /// How long to wait before each poll, now.
    #[must_use]
    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }

    /// What follows `poll`, answered at `now`.
    pub(crate) fn after(&mut self, poll: Poll, now: Instant) -> Next {
        match poll {
            Poll::Granted(tokens) => return Next::Done(tokens),
            Poll::Denied => return Next::Stop(GrantEnd::Denied),
            Poll::Expired => return Next::Stop(GrantEnd::Expired),
            Poll::Failed(why) => return Next::Stop(GrantEnd::Failed(why)),
            Poll::SlowDown => self.interval = (self.interval + SLOWER).min(MAX_INTERVAL),
            Poll::Pending | Poll::Unanswered => {}
        }
        if now
            .checked_add(self.interval)
            .is_none_or(|next| next >= self.deadline)
        {
            Next::Stop(GrantEnd::Expired)
        } else {
            Next::Wait(self.interval)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorization(expires_in: u64, interval: Option<u64>) -> DeviceAuthorization {
        DeviceAuthorization {
            device_code: "device-secret".to_owned(),
            user_code: "WDJB-MJHT".to_owned(),
            verification_uri: "https://accounts.example/device".to_owned(),
            verification_uri_complete: None,
            expires_in,
            interval,
        }
    }

    fn tokens() -> TokenAnswer {
        TokenAnswer {
            access_token: "a".to_owned(),
            expires_in: Some(600),
            refresh_token: Some("r".to_owned()),
            id_token: None,
        }
    }

    #[test]
    fn the_start_asks_for_the_scopes_as_a_public_client() {
        let request = start_request("https://accounts.example/oauth/device", "client");
        assert_eq!(request.field("client_id"), Some("client"));
        assert_eq!(request.field("scope"), Some("openid offline_access"));
    }

    #[test]
    fn polls_wait_the_interval_and_slow_down_when_told() {
        let start = Instant::now();
        let mut grant = DeviceGrant::new(&authorization(600, Some(5)), start);
        assert_eq!(grant.interval(), Duration::from_secs(5));
        assert_eq!(
            grant.after(Poll::Pending, start),
            Next::Wait(Duration::from_secs(5))
        );
        assert_eq!(
            grant.after(Poll::SlowDown, start),
            Next::Wait(Duration::from_secs(10))
        );
        assert_eq!(
            grant.after(Poll::Unanswered, start),
            Next::Wait(Duration::from_secs(10)),
            "a slower interval stays"
        );
        assert_eq!(
            grant.after(Poll::Granted(tokens()), start),
            Next::Done(tokens())
        );
    }

    #[test]
    fn no_interval_is_five_seconds() {
        let grant = DeviceGrant::new(&authorization(600, None), Instant::now());
        assert_eq!(grant.interval(), DEFAULT_INTERVAL);
        let zero = DeviceGrant::new(&authorization(600, Some(0)), Instant::now());
        assert_eq!(zero.interval(), DEFAULT_INTERVAL);
    }

    #[test]
    fn the_providers_numbers_are_kept_within_bounds() {
        let start = Instant::now();
        let mut grant = DeviceGrant::new(&authorization(u64::MAX, Some(u64::MAX)), start);
        assert_eq!(grant.interval(), MAX_INTERVAL);
        assert_eq!(
            grant.after(Poll::SlowDown, start),
            Next::Wait(MAX_INTERVAL),
            "slowing down stops at a minute"
        );
        assert_eq!(
            grant.after(Poll::Pending, start + MAX_LIFETIME),
            Next::Stop(GrantEnd::Expired),
            "an hour at most"
        );
    }

    #[test]
    fn the_grant_gives_up_at_expires_in() {
        let start = Instant::now();
        let mut grant = DeviceGrant::new(&authorization(12, Some(5)), start);
        assert_eq!(
            grant.after(Poll::Pending, start + Duration::from_secs(5)),
            Next::Wait(Duration::from_secs(5))
        );
        assert_eq!(
            grant.after(Poll::Pending, start + Duration::from_secs(10)),
            Next::Stop(GrantEnd::Expired),
            "the next poll would come after the code ran out"
        );
    }

    #[test]
    fn a_refusal_ends_the_grant() {
        let start = Instant::now();
        let mut grant = DeviceGrant::new(&authorization(600, Some(5)), start);
        assert_eq!(
            grant.after(Poll::Denied, start),
            Next::Stop(GrantEnd::Denied)
        );
        assert_eq!(
            grant.after(Poll::Expired, start),
            Next::Stop(GrantEnd::Expired)
        );
        assert_eq!(
            grant.after(Poll::Failed("invalid_client".into()), start),
            Next::Stop(GrantEnd::Failed("invalid_client".into()))
        );
    }

    #[test]
    fn the_token_endpoints_words_are_the_polls() {
        let refused = |error: &str| Poll::from_reply(TokenReply::Refused(error.to_owned()));
        assert_eq!(refused("authorization_pending"), Poll::Pending);
        assert_eq!(refused("slow_down"), Poll::SlowDown);
        assert_eq!(refused("access_denied"), Poll::Denied);
        assert_eq!(refused("expired_token"), Poll::Expired);
        assert_eq!(
            refused("invalid_grant"),
            Poll::Failed("invalid_grant".into())
        );
        let unavailable = |status| TokenReply::Unavailable {
            status,
            retry_after: None,
        };
        assert_eq!(Poll::from_reply(unavailable(502)), Poll::Unanswered);
        assert_eq!(
            Poll::from_reply(unavailable(429)),
            Poll::SlowDown,
            "too many polls is slow_down, not the end"
        );
        assert_eq!(
            Poll::from_reply(TokenReply::Failed(404)),
            Poll::Failed("status 404".into())
        );
    }

    #[test]
    fn the_device_code_never_shows_in_debug() {
        let shown = format!("{:?}", authorization(600, None));
        assert!(!shown.contains("device-secret"), "{shown}");
        assert!(shown.contains("WDJB-MJHT"), "{shown}");
    }
}
