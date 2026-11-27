//! Pairing a device and renewing its session: `POST /api/auth/pair/new`, `POST /api/auth/pair`
//! and `POST /api/auth/refresh`; and the sign-in status, `GET /api/auth/status`.

use axum::Json;
use axum::extract::State;
use emberhut_api::auth::{
    AccessToken, AuthReply, AuthStatus, PairRequest, PairingCode, RefreshRequest, RefreshToken,
    SessionGrant,
};
use emberhut_api::error::ErrorBody;
use emberhut_api::ids::InstallId;
use emberhut_core::DeviceDetails;
use emberhut_core::session::Session;

use crate::app::AppState;
use crate::convert::{device_to_api, timestamp_to_api};
use crate::error::ApiError;
use crate::extract::{Invalid, JsonBody, MaybeSignedIn, SignedIn, Valid};
use crate::identity::Machine;
use crate::services::auth::AuthError;
use crate::services::pairing::CodeMaker;

/// A one-time code for a new device, made on a signed-in one. It works once, for 10 minutes.
/// With it comes the pairing link (`link`), for the app to show as a QR code.
#[utoipa::path(
    post,
    path = "/api/auth/pair/new",
    tag = "auth",
    security(("bearer" = [])),
    responses(
        (
            status = 200,
            description = "A code, shown as ABCD-EFGH, and its pairing link",
            body = PairingCode
        ),
        (
            status = 401,
            description = "Not a signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn pair_new(
    State(app): State<AppState>,
    SignedIn(device): SignedIn,
) -> Result<Json<PairingCode>, ApiError> {
    let issued = app.pairing().new_code(CodeMaker::Device(device)).await?;
    let code = issued.code.to_string();
    Ok(Json(PairingCode {
        ok: true,
        link: pairing_link(&app, &code),
        code,
        expires: timestamp_to_api(issued.expires_at),
    }))
}

/// The pairing link for `code`, when the server has an identity. A link too long for its QR
/// code (very long configured routes) is left out, and the code still works typed.
fn pairing_link(app: &AppState, code: &str) -> Option<String> {
    let machine = app.machine()?;
    match machine.pairing_link(code, app.pairing_routes()) {
        Ok(link) => Some(link.to_url()),
        Err(error) => {
            tracing::warn!(%error, "no pairing link: the code is shown alone");
            None
        }
    }
}

/// Trades a one-time code for this device's own session. The answer names the machine (its id
/// and public key, for the device to pin). A device's own `publicKey` is taken and not kept:
/// it is stored only once the proof that the device holds it (`keySignature`) is checked.
#[utoipa::path(
    post,
    path = "/api/auth/pair",
    tag = "auth",
    request_body = PairRequest,
    responses(
        (status = 200, description = "Paired: the device's session", body = AuthReply),
        (status = 400, description = "No code, or a field the route refuses", body = AuthReply),
        (status = 401, description = "A wrong, used or expired code", body = AuthReply),
        (status = 413, description = "The body is over the server's limit", body = AuthReply),
        (
            status = 429,
            description = "Too many wrong codes lately, for every caller",
            body = AuthReply,
            headers(("retry-after" = u64, description = "Seconds until a code is taken again"))
        ),
    )
)]
pub(super) async fn pair(
    State(app): State<AppState>,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<AuthReply>, ApiError> {
    let JsonBody(mut body) = body.map_err(ApiError::InvalidSignIn)?;
    // A device's key isn't kept until its proof is checked, so it is ignored whatever it holds:
    // a client that already sends one pairs as it would without it.
    if let Some(fields) = body.as_object_mut() {
        fields.remove("publicKey");
    }
    // The code first, as the contract reads it: missing, not text, or blank is "type the
    // code" (or the brake, while it is on), whatever else the body holds.
    let typed = matches!(
        body.get("code"),
        Some(serde_json::Value::String(code)) if !code.trim().is_empty()
    );
    if !typed {
        return Err(app.pairing().refuse_missing().into());
    }
    let Valid(request) = Valid::<PairRequest>::check(body).map_err(|invalid| {
        if invalid.breaks_rule_on("code") {
            // Typed, but longer than any code: a wrong code, counted like one.
            app.pairing().refuse_wrong().into()
        } else {
            ApiError::InvalidSignIn(invalid)
        }
    })?;
    let device = DeviceDetails::new(
        Some(&request.name),
        Some(&request.model),
        request.install.as_ref().map(InstallId::as_str),
    );
    let session = app.pairing().redeem(&request.code, &device).await?;
    Ok(Json(AuthReply {
        machine: app.machine().map(Machine::identity),
        ..accepted(&session)
    }))
}

/// A new access token, and a new refresh token in place of the one sent.
#[utoipa::path(
    post,
    path = "/api/auth/refresh",
    tag = "auth",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Renewed: new tokens", body = AuthReply),
        (status = 400, description = "A field the route refuses", body = AuthReply),
        (status = 413, description = "The body is over the server's limit", body = AuthReply),
        (
            status = 401,
            description = "The refresh token is not current: sign in again",
            body = AuthReply
        ),
    )
)]
pub(super) async fn refresh(
    State(app): State<AppState>,
    body: Result<Valid<RefreshRequest>, Invalid>,
) -> Result<Json<AuthReply>, ApiError> {
    // A body that isn't a refresh request names no token the server issued: sign in again,
    // as for any token it doesn't know. A broken rule on another field is a 400, and a body
    // over the limit a 413.
    let Valid(request) = body.map_err(|invalid| match invalid {
        Invalid::Shape(_) => ApiError::Auth(AuthError::RefreshRejected),
        invalid if invalid.breaks_rule_on("refresh") => ApiError::Auth(AuthError::RefreshRejected),
        invalid => ApiError::InvalidSignIn(invalid),
    })?;
    let install = request.install.as_ref().map(InstallId::as_str);
    let session = app
        .auth()
        .refresh(request.refresh.expose_secret(), install)
        .await?;
    Ok(Json(accepted(&session)))
}

/// The sign-in mode, whether the server is in use, and whether this request comes from a
/// signed-in device. It answers with or without a session, in either mode, so an app can ask
/// before it has one; only a signed-in device is sent the device list.
#[utoipa::path(
    get,
    path = "/api/auth/status",
    tag = "auth",
    security((), ("bearer" = [])),
    responses(
        (
            status = 200,
            description = "The status; `devices` is empty unless `signedIn`",
            body = AuthStatus
        ),
    )
)]
pub(super) async fn status(
    State(app): State<AppState>,
    MaybeSignedIn(device): MaybeSignedIn,
) -> Result<Json<AuthStatus>, ApiError> {
    let mut info = app.state().auth_info().await?;
    let signed_in = device.is_some();
    if !signed_in {
        // Names and kinds of devices are for the owner's devices only.
        info.devices.clear();
    }
    Ok(Json(AuthStatus { info, signed_in }))
}

/// The 200 answer for a session.
fn accepted(session: &Session) -> AuthReply {
    AuthReply::accepted(SessionGrant {
        access: AccessToken::new(session.tokens.access.as_str()),
        refresh: RefreshToken::new(session.tokens.refresh.as_str()),
        device: Some(device_to_api(session.device)),
        expires: Some(timestamp_to_api(session.tokens.access_expires_at)),
    })
}
