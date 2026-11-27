//! `GET /api/connect/probe?nonce=&kind=`: the machine's signed answer to a device checking a
//! direct route (ADR 0007, section 2, "What answers means"; api.md, "Connect").
//!
//! The probe needs no session and carries no token: a route has to prove the machine before a
//! device sends it one. The machine signs the device's nonce and route kind together with how
//! it was reached (directly, on the local socket address that accepted the connection) and the
//! host the request asked for, in its normal form. An answer passed on from another route (a
//! remembered address forwarded elsewhere) carries that route's binding, and fails the
//! device's check.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::connect::{ProbeAnswer, ProbeQuery};
use emberhut_api::error::ErrorBody;
use emberhut_identity::{Host, LocalAddr, Reached};

use crate::app::AppState;
use crate::error::ApiError;
use crate::peer::DirectTcp;

/// What a probe without a nonce or kind the contract reads is told.
const BAD_QUERY: &str = "A probe needs `nonce`, 32 random bytes in base64url without padding, \
    and `kind`, 1 to 32 characters from a-z, 0-9 and -.";

/// What a probe whose host can't be signed is told.
const BAD_HOST: &str = "The request's Host isn't a host name or address the probe can sign.";

/// The machine's signature over the probe's nonce and kind, and over how (`via`), where
/// (`local`) and as what (`host`) it was reached. No session is needed. Every answer carries
/// `Access-Control-Allow-Origin: *`, so a page on any origin can check a route.
#[utoipa::path(
    get,
    path = "/api/connect/probe",
    tag = "connect",
    params(ProbeQuery),
    responses(
        (
            status = 200,
            description = "The signed answer",
            body = ProbeAnswer,
            headers(("access-control-allow-origin" = String, description = "`*`"))
        ),
        (
            status = 400,
            description = "A nonce that isn't 32 bytes, a kind that isn't 1 to 32 of `a-z`, \
                `0-9` and `-`, or a request host that isn't one",
            body = ErrorBody
        ),
        (
            status = 404,
            description = "A server without an identity of its own (a legacy host)",
            body = ErrorBody
        ),
        (status = 503, description = "The server is shutting down", body = ErrorBody),
    )
)]
pub(super) async fn probe(
    State(app): State<AppState>,
    direct: Option<axum::Extension<DirectTcp>>,
    query: Result<Query<ProbeQuery>, QueryRejection>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let direct = direct.map(|axum::Extension(direct)| direct);
    let mut response = match answer(&app, direct, query, &uri, &headers) {
        Ok(answer) => Json(answer).into_response(),
        Err(refusal) => refusal.into_response(),
    };
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// The signed answer, or why there is none.
fn answer(
    app: &AppState,
    direct: Option<DirectTcp>,
    query: Result<Query<ProbeQuery>, QueryRejection>,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<ProbeAnswer, ProbeRefusal> {
    if app.stopping().is_cancelled() {
        return Err(ApiError::ShuttingDown.into());
    }
    let Ok(Query(query)) = query else {
        return Err(ProbeRefusal::Query);
    };
    let host = requested_host(uri, headers)
        .and_then(|host| Host::normalize(host, app.direct_scheme()).ok())
        .ok_or(ProbeRefusal::Host)?;
    let machine = app.machine().ok_or(ApiError::NotFound)?;
    // Only the server's own TCP listener says where a request reached it; a transport that
    // doesn't can't have its binding signed.
    let Some(local) = direct.and_then(|direct| direct.local) else {
        tracing::error!(
            "a probe came over a transport that doesn't say where it reached the server"
        );
        return Err(ApiError::Internal.into());
    };
    let reached = Reached::Direct(LocalAddr::new(local));
    Ok(machine.sign_probe(query.nonce, &query.kind, reached, host))
}

/// The host the request asked for: HTTP/2's `:authority` (or an absolute request target's),
/// else the `Host` header.
fn requested_host<'a>(uri: &'a Uri, headers: &'a HeaderMap) -> Option<&'a str> {
    uri.authority()
        .map(axum::http::uri::Authority::as_str)
        .or_else(|| headers.get(header::HOST)?.to_str().ok())
}

/// Why the probe signs nothing.
#[derive(Debug)]
enum ProbeRefusal {
    /// The nonce or the kind doesn't read: 400.
    Query,
    /// The request's host isn't one: 400.
    Host,
    /// The API's own refusals (shutting down, no identity, a fault).
    Api(ApiError),
}

impl From<ApiError> for ProbeRefusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for ProbeRefusal {
    fn into_response(self) -> Response {
        let bad = |error| (StatusCode::BAD_REQUEST, Json(ErrorBody::new(error))).into_response();
        match self {
            Self::Query => bad(BAD_QUERY),
            Self::Host => bad(BAD_HOST),
            Self::Api(error) => error.into_response(),
        }
    }
}
