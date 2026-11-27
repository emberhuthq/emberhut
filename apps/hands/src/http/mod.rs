//! The HTTP layer: the caller's address and the bearer token on every request, each helper path as `POST` with a JSON
//! body answered by a [`Handler`] on a worker thread, and the extension's folder.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::extension::{ExtensionError, ExtensionFolder, MAX_ZIP};
use crate::fail::{Fail, Reply, field};
use crate::hands::{Handler, PATHS};
use crate::network::Callers;
use crate::token::Token;

/// What every request reaches.
#[derive(Clone)]
pub struct AppState {
    hands: Arc<dyn Handler>,
    token: Arc<Token>,
    callers: Callers,
    extension: Arc<ExtensionFolder>,
    /// One install at a time.
    installing: Arc<Mutex<()>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl AppState {
    /// The state over `hands`, answering `callers` with `token`, keeping `extension`.
    #[must_use]
    pub fn new(
        hands: Arc<dyn Handler>,
        token: Token,
        callers: Callers,
        extension: ExtensionFolder,
    ) -> Self {
        Self {
            hands,
            token: Arc::new(token),
            callers,
            extension: Arc::new(extension),
            installing: Arc::new(Mutex::new(())),
        }
    }
}

/// The helper's routes. Served with the caller's address
/// (`into_make_service_with_connect_info::<SocketAddr>`): a request without it is refused.
#[expect(
    clippy::disallowed_methods,
    reason = "the helper's routes are a private API on the Mac, not the server's documented one, \
              so no OpenAPI document lists them"
)]
pub fn router(state: AppState) -> Router {
    let mut router = Router::new();
    for path in PATHS {
        router = router.route(path, post(hands));
    }
    router
        .route("/extension", get(extension))
        .route("/extension/install", post(install))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(state.clone(), bearer))
        .with_state(state)
}

/// An answer: `status` with a JSON object.
fn answer(status: u16, body: Reply) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(Value::Object(body))).into_response()
}

/// A refusal's answer.
fn refused(fail: &Fail) -> Response {
    answer(fail.status, fail.body())
}

/// Only a request from an allowed address, with the token, gets in.
async fn bearer(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip());
    if !peer.is_some_and(|ip| state.callers.allow(ip)) {
        return refused(&Fail::never("not from an address this helper answers"));
    }
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if state.token.accepts(given) {
        next.run(request).await
    } else {
        refused(&Fail::with(401, "not authorized"))
    }
}

/// Any other path.
async fn not_found(uri: Uri) -> Response {
    refused(&Fail::missing(format!("no {}", uri.path())))
}

/// A helper path, answered on a worker thread (accessibility calls block).
async fn hands(State(state): State<AppState>, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_owned();
    // A body that isn't a JSON object reads as `{}`, as the reference helper reads it.
    let body: Reply = match serde_json::from_slice(&body) {
        Ok(Value::Object(body)) => body,
        _ => Reply::new(),
    };
    let handler = Arc::clone(&state.hands);
    let logged = path.clone();
    let what = [field(&body, "app"), field(&body, "action")]
        .into_iter()
        .find(|w| !w.is_empty())
        .unwrap_or_default();
    let done = tokio::task::spawn_blocking(move || handler.handle(&path, &body)).await;
    let result = done.unwrap_or_else(|error| {
        tracing::error!(%error, "a request's worker failed");
        Err(Fail::broke("Emberhut Hands failed on this request."))
    });
    let status = result.as_ref().map_or_else(|fail| fail.status, |_| 200);
    if logged != "/health" {
        tracing::info!(path = %logged, %what, status, "request");
    }
    match result {
        Ok(reply) => answer(200, reply),
        Err(fail) => refused(&fail),
    }
}

/// `GET /extension`: the version in the extension's folder, and the folder.
async fn extension(State(state): State<AppState>) -> Response {
    let folder = Arc::clone(&state.extension);
    let installed = tokio::task::spawn_blocking(move || folder.installed())
        .await
        .ok()
        .flatten();
    let body = match installed {
        Some(installed) => json!({
            "version": installed.version,
            "path": installed.path.display().to_string(),
        }),
        None => json!({ "version": null }),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// `POST /extension/install`: the zip as the body (`application/zip`, at most 20 MB).
async fn install(State(state): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let zipped = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next().map(str::trim) == Some("application/zip"));
    if !zipped {
        return refused(&Fail::bad(
            "the body must be the zip, as Content-Type: application/zip",
        ));
    }
    let Ok(zip) = axum::body::to_bytes(body, MAX_ZIP).await else {
        return refused(&Fail::bad("the zip is over 20 MB"));
    };
    let folder = Arc::clone(&state.extension);
    let installing = Arc::clone(&state.installing);
    let done = tokio::task::spawn_blocking(move || {
        let _one = installing.lock().unwrap_or_else(PoisonError::into_inner);
        folder.install(&zip)
    })
    .await;
    match done {
        Ok(Ok(installed)) => (
            StatusCode::OK,
            Json(json!({
                "version": installed.version,
                "path": installed.path.display().to_string(),
            })),
        )
            .into_response(),
        Ok(Err(ExtensionError::Refused(why))) => refused(&Fail::bad(why)),
        Ok(Err(error @ ExtensionError::Io(_))) => {
            tracing::error!(error = %crate::error_chain(&error), "the extension was not installed");
            refused(&Fail::bad("The extension's folder could not be written."))
        }
        Err(error) => {
            tracing::error!(%error, "the install's worker failed");
            refused(&Fail::broke("Emberhut Hands failed on this request."))
        }
    }
}

#[cfg(test)]
mod tests;
