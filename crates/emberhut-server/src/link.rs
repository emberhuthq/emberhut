//! `emberhut-server link` and `emberhut-server unlink`: the machine joins an emberhut.com account, or leaves
//! it, from its own command line (ADR 0007, section 5; `docs/directory.md`).
//!
//! `link` signs in to the account with the device grant ([`crate::oauth`]), signs a link
//! statement naming the account's user id, the relay's domain, the machine's name and the
//! time, and posts it to the directory's `POST /v1/machines` with the access token, which it
//! then drops. `unlink` signs an unlink statement and posts it to
//! `POST /v1/machines/{id}/unlink`, with no token: a machine can always leave an account it
//! can no longer sign in to.
//!
//! The account's settings are compiled into official builds; a build from source has none,
//! and these commands say how to turn the account on.

use std::io::{self, Write};
use std::time::{Duration, SystemTime};

use emberhut_api::directory::{self, DirectoryError, LinkedMachine};
use emberhut_identity::{AccountId, AccountIdError, RelayDomain, UnixMillis};
use reqwest::{Client, StatusCode};
use url::Url;

use crate::config::{AccountConfig, LinkConfig, UnlinkConfig};
use crate::http::read_body;
use crate::identity::{self, KeyFileError, Machine};
use crate::oauth::{self, AccountToken, DeviceGrant, OauthError};

/// How long one request to the directory may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The most of the directory's answer that is read.
const ANSWER_LIMIT: usize = 64 * 1024;

/// Why linking or unlinking failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    /// This build has no account settings, and none were given.
    #[error(
        "the emberhut.com account is off in this build: {missing} not set. Official builds set \
         them when they are built; to turn it on, set EMBERHUT_ACCOUNT_ISSUER, \
         EMBERHUT_ACCOUNT_CLIENT_ID and EMBERHUT_RELAY_DOMAIN (when building, or when running), or \
         pass --account-issuer, --account-client-id and --relay-domain. A machine needs no \
         account: devices pair with it directly, and a self-hosted relay lets machines in by \
         its allow list"
    )]
    AccountOff {
        /// The settings that are missing.
        missing: String,
    },
    /// The machine's key couldn't be had.
    #[error(transparent)]
    Key(#[from] KeyFileError),
    /// The HTTP client couldn't be made.
    #[error(transparent)]
    Http(#[from] crate::HttpSetupError),
    /// Signing in to the account failed.
    #[error(transparent)]
    SignIn(#[from] OauthError),
    /// The account's user id can't be named in a statement.
    #[error("the account's user id can't be named in a link")]
    Account(#[from] AccountIdError),
    /// The system clock is before 1970 or past what a statement can carry.
    #[error("the system clock is out of range")]
    Clock,
    /// The directory couldn't be reached.
    #[error("cannot reach the account directory at {url}")]
    Unreachable {
        /// Where.
        url: Url,
        /// Why not.
        #[source]
        source: reqwest::Error,
    },
    /// The directory refused, in its own words.
    #[error("the account directory refused ({status}): {message}")]
    Refused {
        /// Its status.
        status: StatusCode,
        /// Its sentence, or what was wrong with its answer.
        message: String,
    },
    /// The directory answered with another machine than the one that signed the link.
    #[error("the account directory answered for another machine, {0}")]
    OtherMachine(emberhut_identity::MachineId),
    /// A request's body couldn't be written.
    #[error("cannot write the request")]
    Encode(#[source] serde_json::Error),
    /// What the command prints couldn't be written.
    #[error("cannot print")]
    Output(#[from] io::Error),
}

/// The account's settings, all present.
#[derive(Debug)]
struct Account {
    issuer: Url,
    client_id: String,
    relay_domain: RelayDomain,
    directory: Url,
}

/// The relay's domain and the directory's address: what `unlink` needs.
fn directory_of(config: &AccountConfig) -> Result<(RelayDomain, Url), LinkError> {
    let relay_domain = config
        .relay_domain
        .clone()
        .ok_or_else(|| LinkError::AccountOff {
            missing: "EMBERHUT_RELAY_DOMAIN is".to_owned(),
        })?;
    let directory = match &config.directory_url {
        Some(url) => url.clone(),
        None => Url::parse(&format!("https://{}", relay_domain.as_str())).map_err(|_| {
            LinkError::AccountOff {
                missing: "a valid EMBERHUT_DIRECTORY_URL is".to_owned(),
            }
        })?,
    };
    Ok((relay_domain, oauth::secure(directory)?))
}

/// Every account setting `link` needs, or which are missing.
fn account_of(config: &AccountConfig) -> Result<Account, LinkError> {
    if let (Some(issuer), Some(client_id), Some(_)) =
        (&config.issuer, &config.client_id, &config.relay_domain)
    {
        let (relay_domain, directory) = directory_of(config)?;
        return Ok(Account {
            issuer: issuer.clone(),
            client_id: client_id.clone(),
            relay_domain,
            directory,
        });
    }
    let missing: Vec<&str> = [
        (config.issuer.is_none(), "EMBERHUT_ACCOUNT_ISSUER"),
        (config.client_id.is_none(), "EMBERHUT_ACCOUNT_CLIENT_ID"),
        (config.relay_domain.is_none(), "EMBERHUT_RELAY_DOMAIN"),
    ]
    .into_iter()
    .filter_map(|(absent, name)| absent.then_some(name))
    .collect();
    let verb = if missing.len() == 1 { "is" } else { "are" };
    Err(LinkError::AccountOff {
        missing: format!("{} {verb}", missing.join(", ")),
    })
}

/// `path` under the directory's address (which may have a path of its own).
fn at(directory: &Url, path: &str) -> Result<Url, LinkError> {
    let joined = format!("{}{path}", directory.as_str().trim_end_matches('/'));
    Url::parse(&joined).map_err(|_| LinkError::AccountOff {
        missing: "a valid EMBERHUT_DIRECTORY_URL is".to_owned(),
    })
}

/// Now, as a statement carries it.
fn now() -> Result<UnixMillis, LinkError> {
    UnixMillis::try_from(SystemTime::now()).map_err(|_| LinkError::Clock)
}

/// Links this machine to the account the person signs in to, printing to `out` what they need
/// to do and what happened.
///
/// # Errors
///
/// [`LinkError`]: the account off in this build, the key, the sign-in or the directory.
pub async fn link(config: &LinkConfig, out: &mut (dyn Write + Send)) -> Result<(), LinkError> {
    let account = account_of(&config.account)?;
    let key = identity::existing_key(&config.machine_key.path(&config.database))?;
    let machine = Machine::new(
        key,
        identity::machine_name(config.machine_name.name.as_ref()),
    );
    let http = crate::http::shared_client()?;
    let grant = DeviceGrant::new(http.clone(), account.client_id.clone());
    let endpoints = grant.discover(&account.issuer).await?;
    let authorization = grant.start(&endpoints).await?;
    writeln!(
        out,
        "To link this machine to your emberhut.com account, open {} and enter the code {} \
         (it works for {} minutes).",
        authorization.verification_uri,
        authorization.user_code,
        authorization.expires_in.as_secs().div_ceil(60)
    )?;
    if let Some(complete) = &authorization.verification_uri_complete {
        writeln!(out, "Or open {complete}, which has the code in it.")?;
    }
    writeln!(out, "Waiting for the sign-in...")?;
    out.flush()?;
    let token = grant.wait_for_token(&endpoints, &authorization).await?;
    let user = AccountId::new(grant.user_id(&endpoints, &token).await?)?;
    let signed = machine.sign_link(user, account.relay_domain.clone(), now()?);
    let linked = post_link(&http, &account.directory, &signed, &token).await?;
    // The token was for this one request.
    drop(token);
    if *linked.machine.id() != machine.id() {
        return Err(LinkError::OtherMachine(*linked.machine.id()));
    }
    writeln!(
        out,
        "Linked \"{}\" ({}) to your account at {}.",
        linked.name.as_str(),
        machine.id(),
        account.relay_domain.as_str()
    )?;
    Ok(())
}

/// Takes this machine off its account, with its own signature and no sign-in.
///
/// # Errors
///
/// [`LinkError`]: no relay domain, the key, or the directory's refusal (404 when the machine
/// isn't linked).
pub async fn unlink(config: &UnlinkConfig, out: &mut (dyn Write + Send)) -> Result<(), LinkError> {
    let (relay_domain, directory) = directory_of(&config.account)?;
    let key = identity::existing_key(&config.machine_key.path(&config.database))?;
    let machine = Machine::new(key, identity::machine_name(None));
    let request = machine.sign_unlink(relay_domain.clone(), now()?);
    let url = at(&directory, &directory::unlink_path(&machine.id()))?;
    let http = crate::http::shared_client()?;
    send(&url, http.post(url.clone()).json_body(&request)?).await?;
    writeln!(
        out,
        "Took this machine ({}) off its account at {}.",
        machine.id(),
        relay_domain.as_str()
    )?;
    Ok(())
}

/// Posts the signed link with the account's token: the machine as the directory lists it.
async fn post_link(
    http: &Client,
    directory: &Url,
    signed: &emberhut_api::connect::SignedLink,
    token: &AccountToken,
) -> Result<LinkedMachine, LinkError> {
    let url = at(directory, directory::MACHINES)?;
    let request = http
        .post(url.clone())
        .bearer_auth(token.expose_secret())
        .json_body(signed)?;
    let body = send(&url, request).await?;
    serde_json::from_slice(&body).map_err(|error| LinkError::Refused {
        status: StatusCode::OK,
        message: format!("an answer that isn't a machine: {error}"),
    })
}

/// Sends a request to the directory: the body of a successful answer, or its refusal.
async fn send(url: &Url, request: reqwest::RequestBuilder) -> Result<Vec<u8>, LinkError> {
    let unreachable = |source| LinkError::Unreachable {
        url: url.clone(),
        source,
    };
    let response = request
        .header("accept", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(unreachable)?;
    let status = response.status();
    let body = read_body(response, ANSWER_LIMIT)
        .await
        .map_err(unreachable)?;
    if status.is_success() {
        return Ok(body.bytes);
    }
    let message = serde_json::from_slice::<DirectoryError>(&body.bytes)
        .map_or_else(|_| body.text(), |refusal| refusal.error);
    Err(LinkError::Refused { status, message })
}

/// A JSON body, without reqwest's `json` feature.
trait JsonBody: Sized {
    /// The request with `body` as its JSON body.
    fn json_body<T: serde::Serialize>(self, body: &T) -> Result<Self, LinkError>;
}

impl JsonBody for reqwest::RequestBuilder {
    fn json_body<T: serde::Serialize>(self, body: &T) -> Result<Self, LinkError> {
        let bytes = serde_json::to_vec(body).map_err(LinkError::Encode)?;
        Ok(self.header("content-type", "application/json").body(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(issuer: bool, client: bool, relay: bool) -> AccountConfig {
        AccountConfig {
            issuer: issuer.then(|| "https://accounts.example.com".parse().unwrap()),
            client_id: client.then(|| "client-1".to_owned()),
            relay_domain: relay.then(|| "relay.example".parse().unwrap()),
            directory_url: None,
        }
    }

    #[test]
    fn without_settings_the_account_is_off_and_says_how_to_turn_it_on() {
        let error = account_of(&settings(false, false, false)).unwrap_err();
        let shown = error.to_string();
        assert!(
            shown.starts_with("the emberhut.com account is off in this build"),
            "{shown}"
        );
        assert!(
            shown.contains(
                "EMBERHUT_ACCOUNT_ISSUER, EMBERHUT_ACCOUNT_CLIENT_ID, EMBERHUT_RELAY_DOMAIN are"
            ),
            "{shown}"
        );
        let one = account_of(&settings(true, false, true)).unwrap_err();
        assert!(
            one.to_string()
                .contains("EMBERHUT_ACCOUNT_CLIENT_ID is not set"),
            "{one}"
        );
        assert!(matches!(
            directory_of(&settings(true, true, false)),
            Err(LinkError::AccountOff { .. })
        ));
    }

    #[test]
    fn the_directory_is_at_the_relay_s_domain_unless_set() {
        let account = account_of(&settings(true, true, true)).unwrap();
        assert_eq!(account.directory.as_str(), "https://relay.example/");
        assert_eq!(
            at(&account.directory, directory::MACHINES)
                .unwrap()
                .as_str(),
            "https://relay.example/v1/machines"
        );
        let mut elsewhere = settings(true, true, true);
        elsewhere.directory_url = Some("https://example.com/accounts/".parse().unwrap());
        let (_, directory) = directory_of(&elsewhere).unwrap();
        assert_eq!(
            at(&directory, directory::MACHINES).unwrap().as_str(),
            "https://example.com/accounts/v1/machines"
        );
        elsewhere.directory_url = Some("http://example.com".parse().unwrap());
        assert!(matches!(
            directory_of(&elsewhere),
            Err(LinkError::SignIn(OauthError::Insecure(_)))
        ));
    }
}
