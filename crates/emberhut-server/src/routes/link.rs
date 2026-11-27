//! `POST /api/connect/link-statement`: the machine signs a link statement for the account a
//! signed-in device on the same computer names (ADR 0007, section 5, "How a machine is
//! linked"). The desktop posts it to the account's directory with its own account token; the
//! server never sees one.
//!
//! The statement links the machine to whatever account it names, so only a device on this
//! computer may ask for one: the request has to come straight from a loopback TCP peer, over a
//! connection the server's own TCP listener accepted ([`DirectTcp`]). A request a relayed or
//! sealed listener serves, one from any other address, and one a proxy passed on with any of
//! the forwarding headers below, is refused before its session or body is read. Forwarding
//! headers are never trusted to name a peer.
//!
//! What this can't tell apart is a forward on this computer that adds no header (a layer-4
//! port forward, or a proxy set to add none): its requests come from loopback. Those count as
//! local, which is accepted: whoever reaches the machine through one already needs a device
//! session, which reads and acts on everything the machine holds; the account grants no
//! access to the machine; and a machine linked to one account can't be moved to another
//! without unlinking it there first (the directory's 409).

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use emberhut_api::connect::{LinkStatementRequest, SignedLink};
use emberhut_api::error::ErrorBody;
use emberhut_identity::UnixMillis;

use crate::app::AppState;
use crate::error::{ApiError, Invalid};
use crate::extract::{SignedIn, Valid};
use crate::peer::DirectTcp;

/// The headers a proxy, a tunnel or a forwarder adds: a loopback peer that sends one is
/// passing on a request from elsewhere.
const FORWARDING: [HeaderName; 10] = [
    HeaderName::from_static("forwarded"),
    HeaderName::from_static("x-forwarded-for"),
    HeaderName::from_static("x-forwarded-host"),
    HeaderName::from_static("x-forwarded-proto"),
    HeaderName::from_static("x-forwarded-port"),
    HeaderName::from_static("x-real-ip"),
    HeaderName::from_static("x-client-ip"),
    HeaderName::from_static("true-client-ip"),
    HeaderName::from_static("cf-connecting-ip"),
    HeaderName::from_static("via"),
];

/// What the route says to any other peer.
const LOCAL_ONLY: &str = "Only an app on this computer can link it to an account.";

/// A request that came straight from a loopback TCP peer.
#[derive(Clone, Copy, Debug)]
pub(super) struct LoopbackPeer;

impl LoopbackPeer {
    /// Whether the request in `parts` came straight from a loopback TCP peer.
    fn check(parts: &Parts) -> Result<Self, LinkRefusal> {
        // Only the server's own TCP listener marks a request as direct, and only then is the
        // connection info's peer the socket's. Any other transport is refused, whatever
        // connection info it carries.
        let direct = parts.extensions.get::<DirectTcp>().is_some();
        let loopback = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|ConnectInfo(peer)| peer.ip().to_canonical().is_loopback());
        let forwarded = FORWARDING
            .iter()
            .any(|name| parts.headers.contains_key(name));
        if direct && loopback && !forwarded {
            Ok(Self)
        } else {
            Err(LinkRefusal::NotLocal)
        }
    }
}

impl FromRequestParts<AppState> for LoopbackPeer {
    type Rejection = LinkRefusal;

    fn from_request_parts(
        parts: &mut Parts,
        _: &AppState,
    ) -> impl Future<Output = Result<Self, LinkRefusal>> + Send {
        std::future::ready(Self::check(parts))
    }
}

/// Why the route signs nothing.
#[derive(Debug)]
pub(super) enum LinkRefusal {
    /// Not straight from a loopback TCP peer: 403.
    NotLocal,
    /// The API's own refusals (no session, a bad body, no identity).
    Api(ApiError),
}

impl From<ApiError> for LinkRefusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for LinkRefusal {
    fn into_response(self) -> Response {
        match self {
            Self::NotLocal => {
                let body = ErrorBody {
                    ok: None,
                    error: LOCAL_ONLY.to_owned(),
                    sign_in: false,
                };
                (StatusCode::FORBIDDEN, Json(body)).into_response()
            }
            Self::Api(error) => error.into_response(),
        }
    }
}

/// The machine's signed link to `account` at `relayDomain`, under its name, made now: the body
/// the desktop posts, as it is, to the directory's `POST /v1/machines`. Served only to a
/// signed-in device on this computer.
#[utoipa::path(
    post,
    path = "/api/connect/link-statement",
    tag = "connect",
    security(("bearer" = [])),
    request_body = LinkStatementRequest,
    responses(
        (status = 200, description = "The signed link", body = SignedLink),
        (
            status = 400,
            description = "Not an account's user id or a relay domain",
            body = ErrorBody
        ),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (
            status = 403,
            description = "Not straight from a loopback TCP peer (another address, or passed on by a proxy, forward or relay)",
            body = ErrorBody
        ),
        (status = 404, description = "A server without an identity of its own", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn link_statement(
    State(app): State<AppState>,
    // First, so its refusal answers before the session or the body is read.
    _peer: LoopbackPeer,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<LinkStatementRequest>, Invalid>,
) -> Result<Json<SignedLink>, LinkRefusal> {
    SignedIn::acting(device)?;
    let Valid(request) = body.map_err(ApiError::Invalid)?;
    let machine = app.machine().ok_or(ApiError::NotFound)?;
    let now = u64::try_from(app.clock().now().as_millisecond())
        .ok()
        .and_then(|millis| UnixMillis::new(millis).ok())
        .ok_or(ApiError::Internal)?;
    tracing::info!(
        relay_domain = %request.relay_domain.as_str(),
        "signed a link statement for an app on this computer"
    );
    Ok(Json(machine.sign_link(
        request.account,
        request.relay_domain,
        now,
    )))
}
