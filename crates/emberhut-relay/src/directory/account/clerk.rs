//! Clerk's tokens (ADR 0007, section 5, "Checking Clerk tokens", as amended by ADR 0008).
//!
//! - **Keys** come from the JWKS URL, cached by `kid` ([`KeyTiming`]).
//! - **The algorithm** is RS256, set here and never taken from the token: `none`, HS256 and
//!   every other algorithm are refused.
//! - **Every token**: `iss` is the configured issuer; `exp` (required) and `nbf` are checked
//!   with 30 s of leeway against the directory's [`Clock`]; `sub` is present, and an account's
//!   user id.
//! - **OAuth access tokens** (the desktop, the CLI, the web app) are told apart by their form:
//!   a header `typ` of `at+jwt` (RFC 9068), or else the configured claim that only access
//!   tokens carry (`client_id` by default). They must be issued to exactly one of the configured
//!   client ids, named by `client_id`, or by `azp` when there is no `client_id`.
//! - **Session tokens** (the phone, from the JWT template) are every other token. They must name
//!   the configured audience in `aud`, and an `azp`, when present, must be one of the allowed
//!   origins. An ID token, whose `aud` is a client id, fails here.
//! - An opaque token isn't a JWT and is refused.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use emberhut_identity::AccountId;
use jsonwebtoken::{Algorithm, Validation};
use serde_json::{Map, Value};
use url::{Host, Url};

use super::jwks::{KeySet, KeyTiming, Lookup};
use super::{AccountRefusal, AccountVerifier};
use crate::directory::Clock;

/// The clock skew allowed on `exp` and `nbf`, in seconds.
const LEEWAY_SECONDS: i64 = 30;

/// Clerk's settings for the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClerkSettings {
    /// The issuer, exactly as tokens carry it in `iss`.
    pub issuer: String,
    /// Where the signing keys are published.
    pub jwks_url: JwksUrl,
    /// The `aud` session tokens carry: the JWT template's audience (`emberhut-directory`).
    pub audience: String,
    /// The OAuth applications whose access tokens are taken: the native one's client id and the
    /// web app's.
    pub client_ids: Vec<String>,
    /// The origins a session token's `azp` may name.
    pub allowed_origins: Vec<String>,
    /// The claim that marks an access token whose header has no `typ: at+jwt`.
    pub access_token_claim: String,
    /// How often the keys are fetched.
    pub key_timing: KeyTiming,
}

/// A JWKS URL: `https`, or `http` only to a loopback address (a stand-in in tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwksUrl(Url);

impl JwksUrl {
    /// The URL.
    #[must_use]
    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

impl fmt::Display for JwksUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// Why a URL isn't a JWKS URL the directory takes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JwksUrlError {
    /// Not a URL.
    #[error("not a URL")]
    Invalid(#[from] url::ParseError),
    /// Neither `https` nor `http` to a loopback address.
    #[error("the keys must come over https (http only from a loopback address)")]
    NotHttps,
}

impl FromStr for JwksUrl {
    type Err = JwksUrlError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(text)?;
        let loopback = match url.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain(name)) => name == "localhost",
            None => false,
        };
        match url.scheme() {
            "https" if url.host().is_some() => Ok(Self(url)),
            "http" if loopback => Ok(Self(url)),
            _ => Err(JwksUrlError::NotHttps),
        }
    }
}

/// Claims that ID tokens or session tokens carry as well, so none of them can mark an access
/// token: the registered claims (RFC 7519), those of OIDC, and Clerk's session claims.
const SHARED_CLAIMS: &[&str] = &[
    "iss",
    "sub",
    "aud",
    "exp",
    "nbf",
    "iat",
    "jti",
    "azp",
    "sid",
    "nonce",
    "auth_time",
    "at_hash",
    "c_hash",
    "acr",
    "amr",
    "email",
    "name",
    "v",
    "fva",
    "sts",
    "pla",
    "fea",
    "o",
    "org_id",
    "org_role",
    "org_slug",
    "org_permissions",
];

/// Clerk's tokens can't be checked with these settings.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClerkSetupError {
    /// The client that fetches the keys couldn't be made.
    #[error("the client that fetches Clerk's keys couldn't be made")]
    Client(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The session tokens' audience is also an OAuth client id, so an ID token issued to that
    /// client would pass as a session token.
    #[error("the session tokens' audience can't be one of the OAuth client ids")]
    AudienceIsAClient,
    /// The claim meant to mark access tokens is one that other tokens carry too.
    #[error("{0} can't mark an access token: ID or session tokens carry it too")]
    SharedClaim(String),
}

/// Checks Clerk's tokens, as [`ClerkSettings`] says.
#[derive(Debug)]
pub struct ClerkVerifier {
    settings: ClerkSettings,
    keys: Arc<KeySet>,
    clock: Arc<dyn Clock>,
    validation: Validation,
}

/// Why a token was refused, for the logs.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum Rejected {
    #[error("not a JWT")]
    NotAJwt,
    #[error("signed with another algorithm than RS256")]
    Algorithm,
    #[error("no kid")]
    NoKeyId,
    #[error("signed by a key the provider doesn't publish")]
    UnknownKey,
    #[error("the signature doesn't verify")]
    Signature,
    #[error("another issuer")]
    Issuer,
    #[error("expired")]
    Expired,
    #[error("not valid yet")]
    NotYetValid,
    #[error("no account's user id in sub")]
    Subject,
    #[error("a session token without the directory's audience")]
    Audience,
    #[error("a session token for an origin that isn't allowed")]
    Origin,
    #[error("an access token for another client")]
    Client,
    #[error("the provider's keys can't be had")]
    Unavailable(Duration),
}

impl ClerkVerifier {
    /// A verifier for `settings`, telling the time by `clock`. No key is fetched until a token
    /// needs one.
    ///
    /// # Errors
    ///
    /// [`ClerkSetupError`] when the settings would let one kind of token pass as another, or
    /// the HTTP client can't be made.
    pub fn new(settings: ClerkSettings, clock: Arc<dyn Clock>) -> Result<Self, ClerkSetupError> {
        if settings.client_ids.contains(&settings.audience) {
            return Err(ClerkSetupError::AudienceIsAClient);
        }
        let claim = settings.access_token_claim.as_str();
        if SHARED_CLAIMS.contains(&claim) {
            return Err(ClerkSetupError::SharedClaim(claim.to_owned()));
        }
        let keys = Arc::new(
            KeySet::new(settings.jwks_url.0.clone(), settings.key_timing)
                .map_err(|error| ClerkSetupError::Client(Box::new(error)))?,
        );
        // The signature and algorithm only: the claims are checked below, against `clock`.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.required_spec_claims = HashSet::new();
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.validate_aud = false;
        validation.leeway = 0;
        Ok(Self {
            settings,
            keys,
            clock,
            validation,
        })
    }

    async fn check(&self, token: &str) -> Result<AccountId, Rejected> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| Rejected::NotAJwt)?;
        if header.alg != Algorithm::RS256 {
            return Err(Rejected::Algorithm);
        }
        let kid = header.kid.as_deref().ok_or(Rejected::NoKeyId)?;
        let key = match self.keys.key(kid).await {
            Lookup::Found(key) => key,
            Lookup::Unknown => return Err(Rejected::UnknownKey),
            Lookup::Unavailable { retry_after } => return Err(Rejected::Unavailable(retry_after)),
        };
        let claims = jsonwebtoken::decode::<Map<String, Value>>(token, &key, &self.validation)
            .map_err(|_| Rejected::Signature)?
            .claims;
        let access_form = header.typ.as_deref().is_some_and(is_access_token_type);
        self.check_claims(&claims, access_form)
    }

    fn check_claims(
        &self,
        claims: &Map<String, Value>,
        access_form: bool,
    ) -> Result<AccountId, Rejected> {
        let text = |name: &str| claims.get(name).and_then(Value::as_str);
        if text("iss") != Some(self.settings.issuer.as_str()) {
            return Err(Rejected::Issuer);
        }
        let now = i64::try_from(self.clock.now().get() / 1_000).unwrap_or(i64::MAX);
        let exp = claims
            .get("exp")
            .and_then(Value::as_i64)
            .ok_or(Rejected::Expired)?;
        if now >= exp.saturating_add(LEEWAY_SECONDS) {
            return Err(Rejected::Expired);
        }
        match claims.get("nbf") {
            None => {}
            Some(nbf) => {
                let nbf = nbf.as_i64().ok_or(Rejected::NotYetValid)?;
                if now.saturating_add(LEEWAY_SECONDS) < nbf {
                    return Err(Rejected::NotYetValid);
                }
            }
        }
        let account = text("sub")
            .and_then(|sub| AccountId::new(sub).ok())
            .ok_or(Rejected::Subject)?;
        let access_token = access_form
            || claims
                .get(&self.settings.access_token_claim)
                .is_some_and(|marker| !marker.is_null());
        if access_token {
            let client = match claims.get("client_id") {
                Some(client_id) => client_id.as_str(),
                None => text("azp"),
            };
            if !client.is_some_and(|client| self.settings.client_ids.iter().any(|id| id == client))
            {
                return Err(Rejected::Client);
            }
        } else {
            if !names_audience(claims.get("aud"), &self.settings.audience) {
                return Err(Rejected::Audience);
            }
            match claims.get("azp") {
                None => {}
                Some(azp) => {
                    let azp = azp.as_str().ok_or(Rejected::Origin)?;
                    if !self
                        .settings
                        .allowed_origins
                        .iter()
                        .any(|origin| origin == azp)
                    {
                        return Err(Rejected::Origin);
                    }
                }
            }
        }
        Ok(account)
    }
}

impl AccountVerifier for ClerkVerifier {
    async fn verify(&self, token: &str) -> Result<AccountId, AccountRefusal> {
        self.check(token).await.map_err(|rejected| {
            tracing::debug!(reason = %rejected, "an account token was refused");
            match rejected {
                Rejected::Unavailable(retry_after) => AccountRefusal::Unavailable { retry_after },
                _ => AccountRefusal::SignIn,
            }
        })
    }
}

/// Whether a header's `typ` is RFC 9068's, in either spelling it allows.
fn is_access_token_type(typ: &str) -> bool {
    typ.eq_ignore_ascii_case("at+jwt") || typ.eq_ignore_ascii_case("application/at+jwt")
}

/// Whether `aud` (a string, or an array of them) names `audience`.
fn names_audience(aud: Option<&Value>, audience: &str) -> bool {
    match aud {
        Some(Value::String(one)) => one == audience,
        Some(Value::Array(many)) => many.iter().any(|one| one.as_str() == Some(audience)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use emberhut_identity::UnixMillis;
    use serde_json::json;

    use super::*;

    #[derive(Debug)]
    struct At(u64);

    impl Clock for At {
        fn now(&self) -> UnixMillis {
            UnixMillis::new(self.0 * 1_000).unwrap()
        }
    }

    const NOW: u64 = 1_795_561_073;

    fn verifier() -> ClerkVerifier {
        ClerkVerifier::new(settings(), Arc::new(At(NOW))).unwrap()
    }

    fn settings() -> ClerkSettings {
        ClerkSettings {
            issuer: "https://clerk.example".to_owned(),
            jwks_url: "https://clerk.example/.well-known/jwks.json"
                .parse()
                .unwrap(),
            audience: "emberhut-directory".to_owned(),
            client_ids: vec!["native".to_owned(), "web".to_owned()],
            allowed_origins: vec!["https://app.example".to_owned()],
            access_token_claim: "client_id".to_owned(),
            key_timing: KeyTiming::default(),
        }
    }

    #[test]
    fn settings_that_let_one_kind_of_token_pass_as_another_are_refused() {
        let mut audience_is_a_client = settings();
        audience_is_a_client.audience = "web".to_owned();
        assert!(matches!(
            ClerkVerifier::new(audience_is_a_client, Arc::new(At(NOW))),
            Err(ClerkSetupError::AudienceIsAClient)
        ));
        for claim in [
            "azp", "aud", "sub", "iss", "sid", "exp", "iat", "nbf", "jti", "nonce",
        ] {
            let mut shared = settings();
            shared.access_token_claim = claim.to_owned();
            assert!(
                matches!(
                    ClerkVerifier::new(shared, Arc::new(At(NOW))),
                    Err(ClerkSetupError::SharedClaim(_))
                ),
                "{claim}"
            );
        }
        let mut scope = settings();
        scope.access_token_claim = "scope".to_owned();
        assert!(ClerkVerifier::new(scope, Arc::new(At(NOW))).is_ok());
    }

    fn session(extra: &Value) -> Map<String, Value> {
        let mut claims = json!({
            "iss": "https://clerk.example",
            "sub": "user_2example0account",
            "exp": NOW + 60,
            "aud": "emberhut-directory",
        });
        merge(&mut claims, extra);
        claims.as_object().unwrap().clone()
    }

    fn merge(claims: &mut Value, extra: &Value) {
        for (name, value) in extra.as_object().unwrap() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(name);
            } else {
                claims[name] = value.clone();
            }
        }
    }

    fn check(claims: &Map<String, Value>, access_form: bool) -> Result<String, Rejected> {
        verifier()
            .check_claims(claims, access_form)
            .map(|account| account.as_str().to_owned())
    }

    #[test]
    fn a_session_token_names_its_account() {
        assert_eq!(
            check(&session(&json!({})), false),
            Ok("user_2example0account".to_owned())
        );
        let listed = session(&json!({ "aud": ["other", "emberhut-directory"] }));
        assert!(check(&listed, false).is_ok());
        let from_the_app = session(&json!({ "azp": "https://app.example" }));
        assert!(check(&from_the_app, false).is_ok());
    }

    #[test]
    fn every_token_needs_the_issuer_a_subject_and_a_lifetime() {
        for (extra, why) in [
            (json!({ "iss": "https://other.example" }), Rejected::Issuer),
            (json!({ "iss": null }), Rejected::Issuer),
            (json!({ "sub": null }), Rejected::Subject),
            (json!({ "sub": "" }), Rejected::Subject),
            (json!({ "sub": "no spaces allowed" }), Rejected::Subject),
            (json!({ "exp": null }), Rejected::Expired),
            (json!({ "exp": "later" }), Rejected::Expired),
            (json!({ "nbf": "now" }), Rejected::NotYetValid),
        ] {
            assert_eq!(check(&session(&extra), false), Err(why), "{extra}");
        }
    }

    #[test]
    fn times_have_thirty_seconds_of_leeway() {
        let ok = |extra: Value| check(&session(&extra), false);
        assert!(ok(json!({ "exp": NOW - 29 })).is_ok());
        assert_eq!(ok(json!({ "exp": NOW - 30 })), Err(Rejected::Expired));
        assert!(ok(json!({ "nbf": NOW + 30 })).is_ok());
        assert_eq!(ok(json!({ "nbf": NOW + 31 })), Err(Rejected::NotYetValid));
    }

    #[test]
    fn a_session_token_needs_the_audience_and_an_allowed_origin() {
        for (extra, why) in [
            (json!({ "aud": null }), Rejected::Audience),
            (json!({ "aud": "another" }), Rejected::Audience),
            (json!({ "aud": ["another"] }), Rejected::Audience),
            (json!({ "azp": "https://evil.example" }), Rejected::Origin),
            (json!({ "azp": 7 }), Rejected::Origin),
        ] {
            assert_eq!(check(&session(&extra), false), Err(why), "{extra}");
        }
    }

    #[test]
    fn an_id_token_is_refused() {
        // Same keys and issuer, a client id as its audience, and no access-token marker.
        let id_token = session(&json!({ "aud": "native", "azp": "native" }));
        assert_eq!(check(&id_token, false), Err(Rejected::Audience));
    }

    #[test]
    fn an_access_token_is_issued_to_one_of_the_clients() {
        let access = |extra: Value| session(&merged(json!({ "aud": null }), &extra));
        // Marked by its header.
        assert!(check(&access(json!({ "client_id": "native" })), true).is_ok());
        assert!(check(&access(json!({ "azp": "web" })), true).is_ok());
        // Marked by its claim alone.
        assert!(check(&access(json!({ "client_id": "web" })), false).is_ok());
        for (extra, typ) in [
            (json!({ "client_id": "another" }), true),
            (json!({ "client_id": "another", "azp": "native" }), true),
            (json!({ "azp": "another" }), true),
            (json!({}), true),
            (json!({ "client_id": 1 }), false),
            (json!({ "client_id": "another" }), false),
        ] {
            assert_eq!(
                check(&access(extra.clone()), typ),
                Err(Rejected::Client),
                "{extra}"
            );
        }
    }

    fn merged(mut base: Value, extra: &Value) -> Value {
        merge(&mut base, extra);
        base
    }

    #[test]
    fn the_access_token_type_has_two_spellings() {
        assert!(is_access_token_type("at+jwt"));
        assert!(is_access_token_type("application/at+JWT"));
        assert!(!is_access_token_type("JWT"));
    }

    #[test]
    fn keys_come_over_https_or_from_loopback() {
        for good in [
            "https://clerk.example/.well-known/jwks.json",
            "http://127.0.0.1:8080/jwks",
            "http://[::1]:8080/jwks",
            "http://localhost:8080/jwks",
        ] {
            assert!(good.parse::<JwksUrl>().is_ok(), "{good}");
        }
        for bad in [
            "http://clerk.example/jwks",
            "http://192.0.2.1/jwks",
            "file:///jwks",
            "jwks",
        ] {
            assert!(bad.parse::<JwksUrl>().is_err(), "{bad}");
        }
    }
}
