//! `POST /api/uploads?name=` (store a file for the chat) and `GET /uploads/{path}` (a stored
//! file, in byte ranges when asked).

use std::io::SeekFrom;

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use emberhut_api::error::ErrorBody;
use emberhut_api::uploads::{UploadQuery, UploadReply};
use emberhut_core::upload::{ByteRange, REPORT_POLICY};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
use tokio_util::io::ReaderStream;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::Viewer;
use crate::services::uploads::Served;

/// How long a client may keep a whole file it fetched (a stored file never changes).
const CACHE: &str = "private, max-age=3600";

/// Stores the file that is the request's body, as it arrives, under the name `?name=` gives
/// (none: `file`). The answer's file is sent back whole with the message it goes with.
#[utoipa::path(
    post,
    path = "/api/uploads",
    tag = "files",
    security(("bearer" = [])),
    params(UploadQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The file itself, raw"
    ),
    responses(
        (status = 200, description = "Stored", body = UploadReply),
        (
            status = 400,
            description = "Not stored: over 500 MB, cut off, or the server couldn't write it",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn upload(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<UploadQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<UploadReply>, ApiError> {
    let name = query.map_or_else(
        |_| UploadQuery::UNNAMED.to_owned(),
        |Query(query)| query.name,
    );
    let length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.trim().parse::<u64>().ok());
    let uploads = app.uploads();
    let stored = uploads
        .receive(&name, length, body.into_data_stream())
        .await?;
    Ok(Json(UploadReply {
        ok: true,
        file: uploads.reply_file(&stored),
    }))
}

/// A file in the uploads folder, by the `url` of its chat file. With `Range: bytes=a-b`
/// (`a-` or `-n` too) only those bytes, as video players ask; a report (HTML) is served
/// sandboxed. Media players that can't send a header may put the token in `?token=`.
#[utoipa::path(
    get,
    path = "/uploads/{path}",
    tag = "files",
    security(("bearer" = [])),
    params(
        ("path" = String, Path, description = "The file's stored name"),
        ("range" = Option<String>, Header, description = "One byte range: bytes=a-b, a- or -n"),
    ),
    responses(
        (status = 200, description = "The whole file", content_type = "application/octet-stream"),
        (status = 206, description = "The bytes asked for", content_type = "application/octet-stream"),
        (status = 404, description = "No such file"),
        (status = 416, description = "The range starts past the end of the file"),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn file(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let opened = match path {
        Ok(Path(name)) => app.uploads().open(&name).await.map_err(|error| {
            tracing::error!(%error, "a stored file could not be opened");
            ApiError::Internal
        })?,
        // A name that doesn't decode (a malformed escape) names no file.
        Err(_) => None,
    };
    let Some(served) = opened else {
        return Ok(empty(StatusCode::NOT_FOUND));
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|range| range.to_str().ok());
    send(served, range).await
}

/// The file, whole or the part `range` asks for.
pub(super) async fn send(served: Served, range: Option<&str>) -> Result<Response, ApiError> {
    let Served {
        mut file,
        size,
        content_type,
    } = served;
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes");
    if content_type.starts_with("text/html") {
        response = response
            .header(header::CONTENT_SECURITY_POLICY, REPORT_POLICY)
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(header::REFERRER_POLICY, "no-referrer");
    }
    let built = match ByteRange::of(range, size) {
        ByteRange::Whole => response
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, size)
            .header(header::CACHE_CONTROL, CACHE)
            .body(Body::from_stream(ReaderStream::new(file))),
        ByteRange::Part { start, end } => {
            file.seek(SeekFrom::Start(start)).await.map_err(|error| {
                tracing::error!(%error, "a stored file could not be read");
                ApiError::Internal
            })?;
            let length = end - start + 1;
            response
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"))
                .header(header::CONTENT_LENGTH, length)
                .body(Body::from_stream(ReaderStream::new(file.take(length))))
        }
        ByteRange::Unsatisfiable => Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{size}"))
            .body(Body::empty()),
    };
    built.map_err(|_| ApiError::Internal)
}

/// An answer with only a status, and no body (as for a missing file).
pub(super) fn empty(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    response
}
