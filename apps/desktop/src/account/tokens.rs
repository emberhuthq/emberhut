//! The account's tokens as this device keeps them: the access token, the refresh token, when
//! the access token runs out, and the account's user id (the `sub` its ID token named), which a
//! link statement names. Kept as JSON in the Keychain item ([`super::vault`]), never in a file.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD;
use emberhut_identity::AccountId;
use serde::{Deserialize, Serialize};

use super::oidc::TokenAnswer;

/// How long before it runs out an access token is renewed.
pub(crate) const RENEW_BEFORE: Duration = Duration::from_secs(60);
/// How long an access token is taken to last when the provider doesn't say.
const ASSUMED_LIFETIME: Duration = Duration::from_mins(10);

/// The tokens, with the account they are for.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTokens {
    /// The format's version; a newer one isn't read.
    pub(crate) version: u32,
    /// The account's user id.
    pub(crate) account: AccountId,
    /// The access token, sent to the directory.
    pub(crate) access: String,
    /// The refresh token, for the next access token, when the provider gave one.
    pub(crate) refresh: Option<String>,
    /// When the access token runs out, in seconds since the Unix epoch.
    pub(crate) expires_at: u64,
}

impl AccountTokens {
    /// The format this app writes.
    pub(crate) const FORMAT_VERSION: u32 = 1;

    /// The tokens a sign-in gave, for `account`, received at `now`.
    #[must_use]
    pub(crate) fn from_answer(answer: TokenAnswer, account: AccountId, now: SystemTime) -> Self {
        Self {
            version: Self::FORMAT_VERSION,
            account,
            expires_at: expiry(answer.expires_in, now),
            access: answer.access_token,
            refresh: answer.refresh_token,
        }
    }

    /// These tokens after a renewal answered `answer` at `now`: a new access token, and the
    /// refresh token it turned over to (the old one when the provider keeps it).
    #[must_use]
    pub(crate) fn renewed(&self, answer: TokenAnswer, now: SystemTime) -> Self {
        Self {
            version: Self::FORMAT_VERSION,
            account: self.account.clone(),
            expires_at: expiry(answer.expires_in, now),
            access: answer.access_token,
            refresh: answer.refresh_token.or_else(|| self.refresh.clone()),
        }
    }

    /// Whether the access token still has more than [`RENEW_BEFORE`] left at `now`.
    #[must_use]
    pub(crate) fn is_fresh(&self, now: SystemTime) -> bool {
        seconds(now).saturating_add(RENEW_BEFORE.as_secs()) < self.expires_at
    }
}

// Its `Debug` shows the account and the expiry only.
impl fmt::Debug for AccountTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountTokens")
            .field("account", &self.account)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// When a token given at `now` for `expires_in` seconds runs out.
fn expiry(expires_in: Option<u64>, now: SystemTime) -> u64 {
    seconds(now).saturating_add(expires_in.unwrap_or(ASSUMED_LIFETIME.as_secs()))
}

/// `at` in whole seconds since the Unix epoch (0 before it).
fn seconds(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The audience of an ID token: one client id, or several.
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    /// One.
    One(String),
    /// Several.
    Many(Vec<String>),
}

impl Audience {
    fn names(&self, client_id: &str) -> bool {
        match self {
            Self::One(audience) => audience == client_id,
            Self::Many(audiences) => audiences.iter().any(|audience| audience == client_id),
        }
    }
}

/// The account an ID token names: its `sub`, once its `iss` is `issuer`, its `aud` names
/// `client_id` and its `exp` hasn't passed at `now`. The token came straight from the
/// provider's token endpoint over TLS, and the id only fills in the link statement, whose
/// account the directory checks against its own verified token, so the signature isn't checked
/// here.
#[must_use]
pub(crate) fn subject_of(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    now: SystemTime,
) -> Option<AccountId> {
    #[derive(Deserialize)]
    struct Claims {
        iss: String,
        aud: Audience,
        exp: u64,
        sub: String,
    }
    let payload = id_token.split('.').nth(1)?;
    let bytes = BASE64URL_NOPAD
        .decode(payload.trim_end_matches('=').as_bytes())
        .ok()?;
    let claims: Claims = serde_json::from_slice(&bytes).ok()?;
    let fits = claims.iss.trim_end_matches('/') == issuer.trim_end_matches('/')
        && claims.aud.names(client_id)
        && claims.exp > seconds(now);
    fits.then(|| AccountId::new(claims.sub).ok()).flatten()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An unsigned JWT whose payload is `claims`.
    pub(crate) fn jwt(claims: &serde_json::Value) -> String {
        let part = |value: &serde_json::Value| BASE64URL_NOPAD.encode(value.to_string().as_bytes());
        format!(
            "{}.{}.sig",
            part(&serde_json::json!({"alg": "RS256"})),
            part(claims)
        )
    }

    fn answer(refresh: Option<&str>, expires_in: Option<u64>) -> TokenAnswer {
        TokenAnswer {
            access_token: "access".to_owned(),
            expires_in,
            refresh_token: refresh.map(str::to_owned),
            id_token: None,
        }
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// An ID token's claims for `sub`, from the tests' issuer to their client, not expired.
    pub(crate) fn id_claims(sub: &str) -> serde_json::Value {
        serde_json::json!({"iss": "https://accounts.example", "aud": "client-example",
            "exp": 4_000_000_000_u64, "sub": sub})
    }

    #[test]
    fn the_subject_is_read_from_an_id_token_for_this_client() {
        let read = |claims: serde_json::Value| {
            subject_of(
                &jwt(&claims),
                "https://accounts.example/",
                "client-example",
                at(1_000),
            )
        };
        assert_eq!(
            read(id_claims("user_example")).unwrap().as_str(),
            "user_example"
        );
        let mut listed = id_claims("user_example");
        listed["aud"] = serde_json::json!(["other", "client-example"]);
        assert!(
            read(listed).is_some(),
            "an audience list naming this client"
        );
        for (claim, value) in [
            ("iss", serde_json::json!("https://other.example")),
            ("aud", serde_json::json!("other-client")),
            ("exp", serde_json::json!(999)),
        ] {
            let mut wrong = id_claims("user_example");
            wrong[claim] = value;
            assert_eq!(read(wrong), None, "a wrong {claim}");
        }
        let mut no_sub = id_claims("x");
        no_sub.as_object_mut().unwrap().remove("sub");
        assert_eq!(read(no_sub), None);
        assert_eq!(
            subject_of("not a token", "https://accounts.example", "c", at(0)),
            None
        );
    }

    #[test]
    fn a_token_is_renewed_a_minute_before_it_runs_out() {
        let account = AccountId::new("user_example").unwrap();
        let tokens = AccountTokens::from_answer(answer(Some("r1"), Some(600)), account, at(1_000));
        assert_eq!(tokens.expires_at, 1_600);
        assert!(tokens.is_fresh(at(1_539)));
        assert!(!tokens.is_fresh(at(1_540)));
        let assumed = AccountTokens::from_answer(
            answer(None, None),
            AccountId::new("user_example").unwrap(),
            at(0),
        );
        assert_eq!(assumed.expires_at, 600, "ten minutes when not said");
    }

    #[test]
    fn a_renewal_keeps_the_refresh_token_unless_it_turned_over() {
        let account = AccountId::new("user_example").unwrap();
        let tokens = AccountTokens::from_answer(answer(Some("r1"), Some(600)), account, at(0));
        assert_eq!(
            tokens.renewed(answer(None, Some(600)), at(500)).refresh,
            Some("r1".to_owned())
        );
        let turned = tokens.renewed(answer(Some("r2"), Some(600)), at(500));
        assert_eq!(turned.refresh, Some("r2".to_owned()));
        assert_eq!(turned.expires_at, 1_100);
    }

    #[test]
    fn the_tokens_never_show_in_debug() {
        let account = AccountId::new("user_example").unwrap();
        let tokens = AccountTokens::from_answer(answer(Some("secret-r"), None), account, at(0));
        let shown = format!("{tokens:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }
}
