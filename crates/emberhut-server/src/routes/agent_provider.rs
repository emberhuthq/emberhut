//! `GET`/`PUT`/`DELETE /api/agent/provider` and `POST /api/agent/provider/test`: the model
//! provider the agent runs on, from a signed-in device only (in either auth mode): saving one
//! spends the owner's money, and even what is read back names part of a key.
//!
//! No answer carries a key.

use axum::Json;
use axum::extract::State;
use emberhut_api::agent_provider::{AgentProvider, ProviderRequest, ProviderTest};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::{ApiError, CauseChain};
use crate::extract::{Invalid, SignedIn, Valid};

/// The provider saved, its key's last four characters, its base URL and models, when it last
/// accepted them, whether the agent can run on it, and whether it does now.
#[utoipa::path(
    get,
    path = "/api/agent/provider",
    tag = "agent",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "What is saved, without the key", body = AgentProvider),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn status(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<AgentProvider>, ApiError> {
    SignedIn::acting(device)?;
    app.agent_provider()
        .status()
        .await
        .map(Json)
        .map_err(|error| {
            tracing::error!(error = %CauseChain(&error), "the agent's provider could not be read");
            ApiError::Internal
        })
}

/// Checks a provider (one request that needs the key, within 15 s; none for `claude`) and,
/// once it passes, saves it in place of whatever was saved, another provider's key removed.
/// The agent's sessions run on it from their next turn while no accounts are configured.
#[utoipa::path(
    put,
    path = "/api/agent/provider",
    tag = "agent",
    security(("bearer" = [])),
    request_body = ProviderRequest,
    responses(
        (status = 200, description = "Checked and saved", body = AgentProvider),
        (
            status = 400,
            description = "Not what the provider takes (a key, base URL or model missing or \
                malformed); nothing was saved",
            body = ErrorBody
        ),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (
            status = 422,
            description = "The provider refused the key, or is too old to run the agent; nothing \
                was saved",
            body = ErrorBody
        ),
        (
            status = 502,
            description = "The provider didn't answer the check; nothing was saved",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn save(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<ProviderRequest>, Invalid>,
) -> Result<Json<AgentProvider>, ApiError> {
    SignedIn::acting(device)?;
    let Valid(body) = body?;
    Ok(Json(app.agent_provider().save(&body).await?))
}

/// Removes the saved provider and its key: the agent's next sessions run on the agent CLI's own
/// login.
#[utoipa::path(
    delete,
    path = "/api/agent/provider",
    tag = "agent",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Nothing is saved now", body = AgentProvider),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn remove(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<AgentProvider>, ApiError> {
    SignedIn::acting(device)?;
    Ok(Json(app.agent_provider().remove().await?))
}

/// Checks a provider as `PUT` does and lists its models, saving nothing: for choosing a model,
/// and for finding a local Ollama.
#[utoipa::path(
    post,
    path = "/api/agent/provider/test",
    tag = "agent",
    security(("bearer" = [])),
    request_body = ProviderRequest,
    responses(
        (status = 200, description = "It let the server in; its models", body = ProviderTest),
        (status = 400, description = "Not what the provider takes", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (
            status = 422,
            description = "The provider refused the key, or is too old to run the agent",
            body = ErrorBody
        ),
        (status = 502, description = "The provider didn't answer", body = ErrorBody),
    )
)]
pub(super) async fn test(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<ProviderRequest>, Invalid>,
) -> Result<Json<ProviderTest>, ApiError> {
    SignedIn::acting(device)?;
    let Valid(body) = body?;
    Ok(Json(app.agent_provider().test(&body).await?))
}
