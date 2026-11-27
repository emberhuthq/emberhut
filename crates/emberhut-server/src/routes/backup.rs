//! `GET /api/backup`: the server's backup, a `.tar.gz` streamed as it is made.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::SignedIn;

/// The server's data: a consistent snapshot of the database (`emberhut.db`) and the files the
/// server keeps next to it, as one `.tar.gz`. The secrets directory is not in it (its keys are
/// kept and restored on their own), and one backup is made at a time.
///
/// Only a signed-in device gets it, in either auth mode: it holds every message and the
/// hashes of every secret, and report mode's pass for requests without a session is not
/// meant for that. The database is snapshotted before the answer starts, so a failure there
/// is a 500; after that, a failure can only cut the archive short.
#[utoipa::path(
    get,
    path = "/api/backup",
    tag = "backup",
    security(("bearer" = [])),
    responses(
        (
            status = 200,
            description = "The backup, streamed: `emberhut.db` (the database's snapshot), then \
                the data folder's files",
            content_type = "application/gzip",
            body = Vec<u8>,
            headers(("content-disposition" = String, description = "attachment, with the \
                file's name: emberhut-backup-<UTC day>.tar.gz"))
        ),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
        (
            status = 503,
            description = "A backup is being made already; try again when it is done",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn backup(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Response, ApiError> {
    let SignedIn(device) = device.map_err(|error| match error {
        ApiError::DeviceRequired => ApiError::SignInRequired,
        other => other,
    })?;
    let backups = app.backups();
    let disposition = format!("attachment; filename=\"{}\"", backups.file_name());
    let stream = backups.start().await?;
    tracing::info!(device = device.get(), "backup started");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition).map_err(|_| ApiError::Internal)?,
        )
        .body(Body::from_stream(stream))
        .map_err(|_| ApiError::Internal)
}
