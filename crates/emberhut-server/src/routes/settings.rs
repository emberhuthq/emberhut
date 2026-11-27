//! `POST /api/settings`: the app's small changes, each an action of `crate::settings`, answered
//! with the state.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::settings::{SettingsReply, SettingsRequest};

use crate::app::AppState;
use crate::error::{ApiError, CauseChain};
use crate::extract::{JsonBody, Viewer};
use crate::settings::{self, ActionContext, ActionError, Body};

/// Does what the body's fields ask (several in one body run in a fixed order; a field no
/// action takes is left alone) and answers with the state of the body's room.
#[utoipa::path(
    post,
    path = "/api/settings",
    tag = "settings",
    security(("bearer" = [])),
    request_body = SettingsRequest,
    responses(
        (
            status = 200,
            description = "Done; the state of the body's room (Main by default)",
            body = SettingsReply
        ),
        (
            status = 400,
            description = "An action refused or failed (`{ ok: false, error }`; the ones \
                before it are done), or a body that isn't JSON",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (
            status = 401,
            description = "There is no session, and sign-in is enforced or an action needs a \
                signed-in device (`authRevoke`, in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn settings(
    State(app): State<AppState>,
    Viewer(caller): Viewer,
    JsonBody(body): JsonBody,
) -> Result<Json<SettingsReply>, SettingsFailure> {
    let body = Body::new(body);
    let room = app
        .state()
        .resolve(body.room())
        .await
        .map_err(ApiError::from)?;
    let ctx = ActionContext { app, room, caller };
    settings::apply(&ctx, &body).await?;
    let state = ctx
        .app
        .state()
        .state_for(ctx.room, ctx.caller)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SettingsReply { ok: true, state }))
}

/// What a client is told when an action failed underneath: a fixed sentence, as for every
/// server fault (the cause goes to the log, never to the client).
const FAILED: &str = "The change could not be made. Try again.";

/// A settings request not done: an action that refused or failed underneath (400
/// `{ ok: false, error }` for both) or the server's fault outside
/// the actions.
#[derive(Debug)]
pub(super) enum SettingsFailure {
    Refused(String),
    Api(ApiError),
}

impl From<ApiError> for SettingsFailure {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl From<ActionError> for SettingsFailure {
    fn from(error: ActionError) -> Self {
        match error {
            ActionError::Refused(reason) => Self::Refused(reason),
            ActionError::SignInRequired => Self::Api(ApiError::SignInRequired),
            ActionError::Failed(_) => {
                tracing::error!(error = %CauseChain(&error), "a settings action failed");
                Self::Refused(FAILED.to_owned())
            }
        }
    }
}

impl IntoResponse for SettingsFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Refused(error) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    ok: Some(false),
                    ..ErrorBody::new(error)
                }),
            )
                .into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn an_action_failing_underneath_is_400_ok_false_without_the_cause() {
        let failure = SettingsFailure::from(ActionError::failed(std::io::Error::other(
            "the disk is full",
        )));
        let response = failure.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({ "ok": false, "error": "The change could not be made. Try again." })
        );
    }
}
