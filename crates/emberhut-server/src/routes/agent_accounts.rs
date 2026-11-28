//! `GET /api/agent/accounts`: the agent CLI accounts the sessions run on, in the order they are
//! tried, and whether each can take work now: the failover pool as the server sees it. No
//! token is read and nothing is asked of the model provider.

use axum::Json;
use axum::extract::State;
use emberhut_api::agent_accounts::{AgentAccountState, AgentAccounts};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::convert::timestamp_to_api;
use crate::error::{ApiError, CauseChain};
use crate::extract::Viewer;

/// The accounts, each with when it takes work again if it is out, and which one the main room
/// runs on.
#[utoipa::path(
    get,
    path = "/api/agent/accounts",
    tag = "agent",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The accounts, in the order they are tried", body = AgentAccounts),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn accounts(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<AgentAccounts>, ApiError> {
    let pool = app.talk().assistant().accounts();
    let on = app
        .accounts()
        .assistant()
        .await
        .map_err(|error| {
            tracing::error!(error = %CauseChain(&error), "the account Main runs on wasn't read");
            ApiError::Internal
        })?
        .on;
    Ok(Json(AgentAccounts {
        moves: pool.len() > 1,
        accounts: pool
            .into_iter()
            .map(|(id, limit)| AgentAccountState {
                on: on.as_deref() == Some(id.as_str()),
                can_work: limit.is_none(),
                resting_until: limit.as_ref().map(|limit| timestamp_to_api(limit.until)),
                why: limit.map(|limit| limit.why),
                id,
            })
            .collect(),
    }))
}
