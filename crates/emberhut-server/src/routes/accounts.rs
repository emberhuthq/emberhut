//! The assistant's accounts: `GET /api/accounts` (every account the runners use, and the one
//! the assistant runs on), `POST /api/accounts/use` (the person's pin) and `POST
//! /api/usage/check` (look at every account's use now).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::accounts::AccountsPage;
use emberhut_api::error::ErrorBody;
use emberhut_api::usage::{UsageCheckReply, UseAccountReply, UseAccountRequest};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, JsonBody, SignedIn, Viewer};
use crate::services::accounts::AccountError;

/// Why an account request failed: 400 `{ ok: false, error }` for a refused pin, 500 for the
/// server's own failure, or 401 `signIn` for a change without a signed-in device.
#[derive(Debug)]
pub(super) enum AccountRefusal {
    /// The accounts refused it, or failed.
    Account(AccountError),
    /// The request may not make it ([`SignedIn::acting`]).
    Api(ApiError),
}

impl IntoResponse for AccountRefusal {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Account(error) => error,
            Self::Api(error) => return error.into_response(),
        };
        let (status, error) = match &error {
            AccountError::Refused(why) => (StatusCode::BAD_REQUEST, why.to_string()),
            other => {
                tracing::error!(error = %other, "an account request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on the server.".to_owned(),
                )
            }
        };
        let body = ErrorBody {
            ok: Some(false),
            error,
            sign_in: false,
        };
        (status, Json(body)).into_response()
    }
}

impl From<emberhut_db::DbError> for AccountRefusal {
    fn from(error: emberhut_db::DbError) -> Self {
        Self::Account(AccountError::Database(error))
    }
}

impl From<ApiError> for AccountRefusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

/// Every account the runners use, each with its machine and use, and the account the
/// assistant runs on.
#[utoipa::path(
    get,
    path = "/api/accounts",
    tag = "runners",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The accounts", body = AccountsPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn accounts(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<AccountsPage>, AccountRefusal> {
    Ok(Json(AccountsPage {
        accounts: app.accounts().all().await?,
        assistant: app.accounts().assistant().await?,
    }))
}

/// Pins the assistant to an account (`{ account }`), or `auto` (or nothing) to let it run on
/// the one with the most room: the assistant's account after it, and the Usage card.
#[utoipa::path(
    post,
    path = "/api/accounts/use",
    tag = "runners",
    security(("bearer" = [])),
    request_body = UseAccountRequest,
    responses(
        (status = 200, description = "Pinned, or set to automatic", body = UseAccountReply),
        (status = 400, description = "Not an account the assistant runs on (`ok: false`)", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn use_account(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<UseAccountReply>, AccountRefusal> {
    SignedIn::acting(device)?;
    // Read leniently: no account (or `auto`) is automatic.
    let asked = body.ok().and_then(|JsonBody(body)| {
        body.get("account")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    });
    let account = app
        .accounts()
        .use_account(asked.as_deref())
        .await
        .map_err(AccountRefusal::Account)?;
    Ok(Json(UseAccountReply {
        ok: true,
        account,
        usage: app.accounts().usage_card().await?,
    }))
}

/// Asks every runner to look at its accounts' use now, in the background (at most once in 2
/// minutes); the state's `usage` brings the numbers as they land.
#[utoipa::path(
    post,
    path = "/api/usage/check",
    tag = "runners",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "`asked`: a look was started now", body = UsageCheckReply),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn check_usage(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Json<UsageCheckReply> {
    Json(UsageCheckReply {
        ok: true,
        asked: app.accounts().check_usage(),
    })
}
