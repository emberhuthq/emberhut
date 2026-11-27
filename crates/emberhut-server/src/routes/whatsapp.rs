//! The assistant's own WhatsApp number: `GET` and `POST /api/whatsapp` (its state, the kill
//! switch and linking, for a signed-in device only, report mode included: the state shows the
//! chats and what was sent, and the switch is the owner's), `POST /api/whatsapp/in` (what the
//! WhatsApp daemon tells the server; only with the daemon's token), and `GET /wa/{token}` (the
//! one-time QR page; the link is the key).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::whatsapp::{
    WhatsAppAnswer, WhatsAppEvent, WhatsAppInReply, WhatsAppRequest, WhatsAppState,
};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn};
use crate::services::messaging::whatsapp::WaInError;

/// The number's state: the switch, the connection, the chats and the last messages sent.
#[utoipa::path(
    get,
    path = "/api/whatsapp",
    tag = "messaging",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The state", body = WhatsAppState),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn state(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<WhatsAppState>, ApiError> {
    signed_in(device)?;
    app.messaging()
        .whatsapp()
        .state()
        .await
        .map(Json)
        .map_err(|error| {
            tracing::error!(%error, "the WhatsApp state couldn't be read");
            ApiError::Internal
        })
}

/// The kill switch (`on`, `off`), linking (`link`, `qr`, `link-task`), or the state.
#[utoipa::path(
    post,
    path = "/api/whatsapp",
    tag = "messaging",
    security(("bearer" = [])),
    request_body = WhatsAppRequest,
    responses(
        (status = 200, description = "Done, as `kind` asked", body = WhatsAppAnswer),
        (status = 400, description = "Not a request this route takes", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn change(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    signed_in(device)?;
    let Ok(request) = serde_json::from_value::<WhatsAppRequest>(body) else {
        return Ok(refuse("Not a request this route takes."));
    };
    let answer = app
        .messaging()
        .whatsapp()
        .api(&request)
        .await
        .map_err(|error| {
            tracing::error!(%error, "the WhatsApp request failed");
            ApiError::Internal
        })?;
    Ok(Json(answer).into_response())
}

/// What the daemon's number sees: its state, groups, messages.
#[utoipa::path(
    post,
    path = "/api/whatsapp/in",
    tag = "messaging",
    security(("bearer" = [])),
    request_body = WhatsAppEvent,
    responses(
        (status = 200, description = "Taken", body = WhatsAppInReply),
        (status = 400, description = "Not an event the server knows", body = ErrorBody),
        (status = 401, description = "Not the daemon's token", body = ErrorBody),
    )
)]
pub(super) async fn take(
    State(app): State<AppState>,
    headers: HeaderMap,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    let whatsapp = app.messaging().whatsapp();
    if !whatsapp.is_daemon(token).await {
        let body = ErrorBody::new("the WhatsApp daemon uses its token");
        return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
    }
    let Ok(event) = serde_json::from_value::<WhatsAppEvent>(body) else {
        return Ok(refuse("Not an event the server knows."));
    };
    match whatsapp.take(&event).await {
        Ok(reply) => Ok(Json(reply).into_response()),
        Err(WaInError::UnknownKind(kind)) => Ok(refuse(&format!("unknown kind {kind}"))),
        Err(error) => {
            tracing::error!(%error, "the WhatsApp daemon's news wasn't taken");
            Err(ApiError::Internal)
        }
    }
}

/// The one-time page with the live QR that links the number.
#[utoipa::path(
    get,
    path = "/wa/{token}",
    tag = "messaging",
    params(("token" = String, Path, description = "The page's token: the link is the key")),
    responses(
        (
            status = 200,
            description = "The QR (it refreshes by itself), or that it is linked or expired",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 404,
            description = "The page expired, or never existed",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn qr(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    let (status, body) = app.messaging().whatsapp().qr_page(&token).await;
    super::gmail::html(status, &body)
}

/// The signed-in device, or the sign-in error the apps know (401 `signIn`).
fn signed_in(device: Result<SignedIn, ApiError>) -> Result<SignedIn, ApiError> {
    device.map_err(|error| match error {
        ApiError::DeviceRequired => ApiError::SignInRequired,
        other => other,
    })
}

/// A 400 with `error`.
fn refuse(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorBody::new(error))).into_response()
}
