//! Runners paired at run time: a one-time code made on a signed-in device (`POST
//! /api/runners/codes`), a new runner's trade of it for its token (`POST /api/runner/pair`, no
//! session: the code is the proof), the list of every runner the server knows (`GET
//! /api/runners`), and forgetting a paired runner (`DELETE /api/runners/{name}`).

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::runner::pairing::{
    NewRunnerCode, PairRunner, PairedRunner, RunnerCode, RunnerRow, RunnerSource, RunnersPage,
};
use emberhut_core::pairing::PairingRefusal;
use serde_json::Value;

use crate::app::AppState;
use crate::convert::timestamp_to_api;
use crate::error::{ApiError, refusal_message, whole_seconds};
use crate::extract::{Invalid, JsonBody, SignedIn, Valid, Viewer};
use crate::peer::DirectTcp;
use crate::services::pairing::CodeMaker;
use crate::services::runner_install::{self, RELEASE_VERSION};
use crate::services::runner_pairing::{KnownRunner, PairingAsk, RunnerPairingError};

/// What the server says when it fails on its own side.
const FAULT: &str = "Something went wrong on the server.";

/// A refusal: the server's answer for a request it doesn't take (no session, a body that
/// breaks a rule), or a pairing's, as `{ error }`.
#[derive(Debug)]
pub(super) enum Refusal {
    /// The server's own answer for the error.
    Api(ApiError),
    /// A pairing's, a list's or a forget's.
    Pairing(RunnerPairingError),
}

impl From<ApiError> for Refusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl From<RunnerPairingError> for Refusal {
    fn from(error: RunnerPairingError) -> Self {
        Self::Pairing(error)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Api(error) => return error.into_response(),
            Self::Pairing(error) => error,
        };
        let (status, words) = match &error {
            RunnerPairingError::Refused(refusal) => (
                match refusal {
                    PairingRefusal::TooManyWrongCodes { .. } => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::UNAUTHORIZED,
                },
                refusal_message(*refusal).into_owned(),
            ),
            RunnerPairingError::Configured(name) => (
                StatusCode::CONFLICT,
                format!(
                    "The server's configuration names the runner {name}; it can't be paired or forgotten here."
                ),
            ),
            RunnerPairingError::NoFreeName(name) => (
                StatusCode::CONFLICT,
                format!(
                    "Every name near {name} is another runner's. Pick another name, or forget a runner you no longer use."
                ),
            ),
            RunnerPairingError::NotPaired(name) => (
                StatusCode::NOT_FOUND,
                format!("No paired runner is called {name}."),
            ),
            RunnerPairingError::Random(_) | RunnerPairingError::Database(_) => {
                tracing::error!(error = %crate::error::chain(&error), "a runner pairing failed");
                (StatusCode::INTERNAL_SERVER_ERROR, FAULT.to_owned())
            }
        };
        let mut response = (status, Json(ErrorBody::new(words))).into_response();
        if let RunnerPairingError::Refused(PairingRefusal::TooManyWrongCodes { retry_after }) =
            error
        {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from(whole_seconds(retry_after)),
            );
        }
        response
    }
}

/// A one-time code a new runner pairs with, made on a signed-in device, with the server's
/// address the runner is to use and the lines that install and pair a runner with both. The
/// code works once, for 10 minutes.
#[utoipa::path(
    post,
    path = "/api/runners/codes",
    tag = "runners",
    security(("bearer" = [])),
    request_body(content = NewRunnerCode, description = "May be empty"),
    responses(
        (status = 200, description = "A code, shown as ABCD-EFGH, and the lines to run", body = RunnerCode),
        (status = 400, description = "A server address the server doesn't take", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn new_code(
    State(app): State<AppState>,
    direct: Option<Extension<DirectTcp>>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<NewRunnerCode>, Invalid>,
) -> Result<Json<RunnerCode>, Refusal> {
    let device = SignedIn::acting(device)?;
    let Valid(asked) = body.map_err(ApiError::Invalid)?;
    let came_in_on = direct
        .and_then(|Extension(direct)| direct.local)
        .map(|local| runner_install::came_in_on(app.direct_scheme(), local));
    let pairing = app.runner_pairing();
    let server = pairing.address(asked.server.as_deref(), came_in_on.as_deref());
    let issued = pairing.new_code(CodeMaker::Device(device)).await?;
    let code = issued.code.to_string();
    Ok(Json(RunnerCode {
        install: runner_install::install_lines(&server.url, &code, RELEASE_VERSION),
        code,
        expires_at: timestamp_to_api(issued.expires_at),
        server: server.url,
        reachable: server.reachable,
    }))
}

/// A new runner trades a one-time code for its name and token. A name another runner has is
/// not taken over: the runner is paired under the first free one near it (`build-2`), which
/// the answer gives.
#[utoipa::path(
    post,
    path = "/api/runner/pair",
    tag = "runners",
    request_body = PairRunner,
    responses(
        (status = 200, description = "Paired: the runner's name and token, the token shown once", body = PairedRunner),
        (status = 400, description = "A name or address the server doesn't take, or a body that isn't a pairing", body = ErrorBody),
        (status = 401, description = "No code, or a wrong, used or expired one", body = ErrorBody),
        (status = 409, description = "Every name near the one asked for is another runner's", body = ErrorBody),
        (
            status = 429,
            description = "Too many wrong codes lately, for every caller",
            body = ErrorBody,
            headers(("retry-after" = u64, description = "Seconds until a code is taken again"))
        ),
    )
)]
pub(super) async fn pair(
    State(app): State<AppState>,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<PairedRunner>, Refusal> {
    let JsonBody(body) = body.map_err(ApiError::Invalid)?;
    let pairing = app.runner_pairing();
    // The code first: missing, not text, or blank is "type the code" (or the brake, while it
    // is on), whatever else the body holds.
    let typed = matches!(body.get("code"), Some(Value::String(code)) if !code.trim().is_empty());
    if !typed {
        return Err(pairing.refuse_missing().into());
    }
    let Valid(ask) = Valid::<PairRunner>::check(body).map_err(|invalid| {
        if invalid.breaks_rule_on("code") {
            // Typed, but longer than any code: a wrong code, counted like one.
            Refusal::from(pairing.refuse_wrong())
        } else {
            Refusal::from(ApiError::Invalid(invalid))
        }
    })?;
    let paired = pairing
        .pair(PairingAsk {
            code: ask.code,
            name: ask.name,
            url: ask.url.trim_end_matches('/').to_owned(),
            words: given(ask.place),
            platform: given(Some(ask.platform)),
            version: given(Some(ask.version)),
        })
        .await?;
    Ok(Json(PairedRunner {
        name: paired.pairing.name,
        token: paired.pairing.token,
    }))
}

/// `text` trimmed, when it isn't blank.
fn given(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Every runner the server knows: the ones its configuration names, in their order, then the
/// paired ones, by name. No token is listed.
#[utoipa::path(
    get,
    path = "/api/runners",
    tag = "runners",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The runners", body = RunnersPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn list(State(app): State<AppState>, _viewer: Viewer) -> Json<RunnersPage> {
    let runners = app
        .runner_pairing()
        .list()
        .into_iter()
        .map(|known| match known {
            KnownRunner::Configured { name, place, url } => RunnerRow {
                name,
                place,
                url,
                source: RunnerSource::Flag,
                platform: None,
                version: None,
                created: None,
            },
            KnownRunner::Paired(paired) => RunnerRow {
                place: paired
                    .pairing
                    .words
                    .unwrap_or_else(|| paired.pairing.name.clone()),
                name: paired.pairing.name,
                url: paired.pairing.url,
                source: RunnerSource::Paired,
                platform: paired.pairing.platform,
                version: paired.pairing.version,
                created: Some(timestamp_to_api(paired.created)),
            },
        })
        .collect();
    Json(RunnersPage { runners })
}

/// Forgets a paired runner: its token stops working at once, and the server no longer calls
/// it. A runner the server's configuration names can't be forgotten here.
#[utoipa::path(
    delete,
    path = "/api/runners/{name}",
    tag = "runners",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "The paired runner's name")),
    responses(
        (status = 200, description = "Forgotten", body = OkReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No paired runner of that name", body = ErrorBody),
        (status = 409, description = "A runner the server's configuration names", body = ErrorBody),
    )
)]
pub(super) async fn forget(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<OkReply>, Refusal> {
    SignedIn::acting(device)?;
    let name = path.map_or_else(|_| String::new(), |Path(name)| name);
    app.runner_pairing().forget(&name).await?;
    Ok(Json(OkReply::OK))
}
