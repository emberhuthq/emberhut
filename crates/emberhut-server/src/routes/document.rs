//! `GET /api/openapi.json`: this server's `OpenAPI` document.

use axum::http::header;
use axum::response::IntoResponse;

use crate::error::ApiError;

/// The document, as JSON. It is public: it describes the API, not anyone's data.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    tag = "meta",
    responses((
        status = 200,
        description = "This OpenAPI document",
        content_type = "application/json"
    ))
)]
pub(super) async fn openapi_json() -> Result<impl IntoResponse, ApiError> {
    let document = super::openapi_json().map_err(|error| {
        tracing::error!(%error, "cannot serve the OpenAPI document");
        ApiError::Internal
    })?;
    Ok(([(header::CONTENT_TYPE, "application/json")], document))
}
