//! The routes of `docs/directory.md`, version 1.
//!
//! - `GET /v1/machines`: the account's machines.
//! - `POST /v1/machines`: link a machine with its `SignedLink`.
//! - `DELETE /v1/machines/{id}`: unlink a machine from the account.
//! - `POST /v1/machines/{id}/unlink`: the machine leaves its account by itself, with no
//!   account token.
//!
//! Every refusal is a [`Refusal`]. Answers allow CORS for the web app's origin only.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use emberhut_api::connect::{LinkError, SignedLink};
use emberhut_api::directory::{self as api, MachineList, UnlinkError, UnlinkRequest};
use emberhut_identity::{AccountId, MachineId, UnixMillis};
use serde::de::DeserializeOwned;
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::account::AccountVerifier;
use super::refusal::Refusal;
use super::store::{NewLink, Statement};
use super::{FRESH_FOR, Shared, id_prefix, json};

/// The largest body taken. A link is well under a kilobyte.
const MAX_BODY: usize = 16 * 1024;

/// How long a browser may keep a preflight's answer.
const PREFLIGHT_MAX_AGE: Duration = Duration::from_secs(600);

type AppState<V> = State<Arc<Shared<V>>>;

/// The directory's router.
#[expect(
    clippy::disallowed_methods,
    reason = "the directory's API is specified by directory.md, not by the server's OpenAPI document"
)]
pub(super) fn router<V: AccountVerifier>(shared: Arc<Shared<V>>) -> Router {
    Router::new()
        .route(api::MACHINES, get(list::<V>).post(link::<V>))
        .route("/v1/machines/{id}", delete(unlink::<V>))
        .route("/v1/machines/{id}/unlink", post(unlink_itself::<V>))
        .fallback(|| async { Refusal::not_found("The directory has no such route.") })
        .method_not_allowed_fallback(|| async { Refusal::method_not_allowed() })
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(cors())
        .with_state(shared)
}

/// CORS for the web app's origin only, with the two headers clients send, no credentials,
/// and `Retry-After` readable.
fn cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list([HeaderValue::from_static(
            api::ALLOWED_ORIGIN,
        )]))
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([header::RETRY_AFTER])
        .max_age(PREFLIGHT_MAX_AGE)
}

async fn list<V: AccountVerifier>(
    State(shared): AppState<V>,
    headers: HeaderMap,
) -> Result<Response, Refusal> {
    let account = shared.account(&headers).await?;
    let machines = shared
        .store
        .machines(&account)
        .await
        .map_err(|error| Refusal::store(&error))?;
    let now = shared.clock.now();
    Ok(json(&MachineList {
        machines: machines
            .into_iter()
            .map(|machine| shared.listed(machine, now))
            .collect(),
    }))
}

async fn link<V: AccountVerifier>(
    State(shared): AppState<V>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, Refusal> {
    let account = shared.account(&headers).await?;
    let link: SignedLink = read_json(&headers, body)?;
    let id = link
        .verify(&account, &shared.config.relay_domain)
        .map_err(|error| match error {
            LinkError::OtherRelay => Refusal::bad_request("This statement is for another relay."),
            LinkError::NotYours => Refusal::not_yours(),
            LinkError::Signature(_) => Refusal::bad_signature(),
            _ => Refusal::bad_request("The statement can't be accepted."),
        })?;
    let now = shared.fresh(link.time)?;
    let new = NewLink {
        public_key: link.public_key,
        name: link.name,
        account,
        statement: Statement {
            machine: id,
            signature: link.signature,
            time: link.time,
        },
    };
    let stored = shared
        .store
        .link(new, shared.config.account_cap.get(), now)
        .await?;
    tracing::info!(machine = %id_prefix(&id), "linked a machine");
    Ok(json(&shared.listed(stored, now)))
}

async fn unlink<V: AccountVerifier>(
    State(shared): AppState<V>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, Refusal> {
    let account = shared.account(&headers).await?;
    let not_here = || Refusal::not_found("There is no such machine in this account.");
    let id: MachineId = id.parse().map_err(|_| not_here())?;
    let removed = shared
        .store
        .unlink_from(&account, &id)
        .await
        .map_err(|error| Refusal::store(&error))?;
    if !removed {
        return Err(not_here());
    }
    tracing::info!(machine = %id_prefix(&id), "unlinked a machine from its account");
    shared.announce_unlink(id);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn unlink_itself<V: AccountVerifier>(
    State(shared): AppState<V>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, Refusal> {
    let id: MachineId = id
        .parse()
        .map_err(|_| Refusal::bad_request("The path doesn't name a machine."))?;
    let request: UnlinkRequest = read_json(&headers, body)?;
    request
        .verify(&id, &shared.config.relay_domain)
        .map_err(|error| match error {
            UnlinkError::OtherRelay => Refusal::bad_request("This statement is for another relay."),
            UnlinkError::OtherMachine => {
                Refusal::bad_request("The key isn't the machine's in the path.")
            }
            UnlinkError::Signature(_) => Refusal::bad_signature(),
            _ => Refusal::bad_request("The statement can't be accepted."),
        })?;
    // Counted only once the machine's signature verifies, so someone who knows only its id
    // can't hold its own unlink back.
    shared
        .per_machine
        .take(&id.to_string())
        .map_err(Refusal::limit)?;
    let now = shared.fresh(request.time)?;
    let statement = Statement {
        machine: id,
        signature: request.signature,
        time: request.time,
    };
    shared.store.unlink_itself(&statement, now).await?;
    tracing::info!(machine = %id_prefix(&id), "a machine left its account");
    shared.announce_unlink(id);
    Ok(StatusCode::NO_CONTENT.into_response())
}

impl<V: AccountVerifier> Shared<V> {
    /// The account the request's token is for, within its request limit.
    async fn account(&self, headers: &HeaderMap) -> Result<AccountId, Refusal> {
        let token = bearer(headers).ok_or_else(Refusal::sign_in)?;
        let account = self.verifier.verify(token).await?;
        self.per_account
            .take(account.as_str())
            .map_err(Refusal::limit)?;
        Ok(account)
    }
}

impl<V> Shared<V> {
    /// Now, if a statement made at `time` is fresh.
    fn fresh(&self, time: UnixMillis) -> Result<UnixMillis, Refusal> {
        let now = self.clock.now();
        if time.distance(now) > FRESH_FOR {
            Err(Refusal::stale())
        } else {
            Ok(now)
        }
    }
}

/// The token of an `Authorization: Bearer <token>` header.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// A JSON body: `Content-Type: application/json`, of `T`'s shape. Fields `T` doesn't know are
/// ignored.
fn read_json<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<T, Refusal> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"));
    if !is_json {
        return Err(Refusal::bad_request(
            "Send the body as JSON (Content-Type: application/json).",
        ));
    }
    let body = body.map_err(|_| Refusal::bad_request("The body is too large."))?;
    serde_json::from_slice(&body)
        .map_err(|_| Refusal::bad_request("The body isn't a statement the directory can read."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(name: header::HeaderName, value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn a_bearer_token_is_read_from_its_header() {
        assert_eq!(
            bearer(&with(header::AUTHORIZATION, "Bearer abc")),
            Some("abc")
        );
        assert_eq!(
            bearer(&with(header::AUTHORIZATION, "bearer  abc ")),
            Some("abc")
        );
        assert_eq!(bearer(&with(header::AUTHORIZATION, "Basic abc")), None);
        assert_eq!(bearer(&with(header::AUTHORIZATION, "Bearer ")), None);
        assert_eq!(bearer(&with(header::AUTHORIZATION, "abc")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn a_body_must_say_it_is_json() {
        let body = || Ok(Bytes::from_static(b"{}"));
        let read = |headers: &HeaderMap| read_json::<serde_json::Value>(headers, body());
        assert!(read(&with(header::CONTENT_TYPE, "application/json")).is_ok());
        assert!(
            read(&with(
                header::CONTENT_TYPE,
                "Application/JSON; charset=utf-8"
            ))
            .is_ok()
        );
        assert!(read(&with(header::CONTENT_TYPE, "text/plain")).is_err());
        assert!(read(&HeaderMap::new()).is_err());
    }
}
