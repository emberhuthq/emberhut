//! The routes. Every one is a handler with `#[utoipa::path]`, collected by utoipa-axum's
//! router, so the `OpenAPI` document is made from the routes the server serves and cannot list
//! one it doesn't (plain `Router::route` is banned in `clippy.toml`).

use std::sync::LazyLock;

use axum::Router;
use utoipa::openapi::OpenApi as Document;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::SignedIn;

mod accounts;
mod agents;
mod agents_inbound;
mod approvals;
mod approve;
mod auth;
mod backup;
mod browser;
mod calendar;
mod calls;
mod connect;
mod dark;
mod document;
mod events;
mod family;
mod gmail;
mod health;
mod health_data;
mod history;
mod jobs;
mod link;
mod live;
mod mac;
mod mcp;
mod memory;
mod parties;
mod push;
mod runner;
mod secret_links;
mod settings;
mod settings_pages;
mod setup;
mod state;
mod talk;
mod tasks;
mod theme;
mod threads;
mod uploads;
mod voice;
mod whatsapp;
mod workers;

/// The name of the bearer security scheme the protected routes name.
const BEARER: &str = "bearer";

/// The document's fixed parts; the paths come from the routes.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "emberhut",
        description = "The HTTP/SSE API the Emberhut apps talk to. See docs/api.md.",
        license(name = "MIT OR Apache-2.0", identifier = "MIT OR Apache-2.0")
    ),
    modifiers(&BearerScheme),
    tags(
        (name = "auth", description = "Pairing a device and renewing its session"),
        (name = "setup", description = "What the server has set up, for the apps' onboarding"),
        (name = "connect", description = "The machine's identity, and linking it to an account"),
        (name = "state", description = "What the app shows for a room"),
        (name = "live", description = "What the assistant is doing, as it does it"),
        (name = "talk", description = "Saying something, and marking a room read"),
        (name = "settings", description = "The app's small changes, answered with the state"),
        (name = "theme", description = "The colour theme, the same on every device"),
        (name = "mcp", description = "The tools a room's agent session calls (MCP)"),
        (name = "files", description = "Files attached in the chat"),
        (name = "push", description = "Where the server pushes a phone's alerts"),
        (name = "secrets", description = "One-time pages for typing secrets in"),
        (name = "mac", description = "The owner's Mac, through its helper"),
        (name = "backup", description = "The server's backup"),
        (name = "threads", description = "The coding-thread app's threads"),
        (name = "tasks", description = "Tasks, what they produced, and what is scheduled"),
        (name = "memory", description = "What the assistant remembers: facts, rules and quick notes"),
        (name = "runners", description = "The runners on the person's machines, their sessions and machines"),
        (name = "calls", description = "Recorded calls, their transcripts and live transcript"),
        (name = "voice", description = "Voice calls with the assistant, and dictation"),
        (name = "calendar", description = "The owner's calendar"),
        (name = "health", description = "The owner's health and screen time, sent in by their devices"),
        (name = "messaging", description = "The owner's messages from outside the app, and the assistant's own WhatsApp number"),
        (name = "browser", description = "The browser the assistant drives, and the owner's Chrome profiles"),
        (name = "parties", description = "AI personas talking about a topic, turn by turn"),
        (name = "dark", description = "The off-the-record room, which the assistant never sees"),
        (name = "family", description = "A family member's own rooms, through the household server"),
        (name = "agents", description = "Configured agents, their channels, tools, accounts and policies, approvers and trusted requesters"),
        (name = "approvals", description = "Approval requests: what an agent's outward step waits on, decided once by its approver"),
        (name = "meta", description = "This document, and the server's health")
    )
)]
struct ApiDoc;

/// Adds the `bearer` scheme: a device's access token, `Authorization: Bearer <token>`.
struct BearerScheme;

impl Modify for BearerScheme {
    fn modify(&self, document: &mut Document) {
        document
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                BEARER,
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
    }
}

/// Every route, with its document.
#[expect(clippy::too_many_lines, reason = "every route, a line each")]
fn api() -> OpenApiRouter<AppState> {
    let router = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(auth::pair_new))
        .routes(routes!(auth::pair))
        .routes(routes!(auth::refresh))
        .routes(routes!(connect::probe))
        .routes(routes!(link::link_statement))
        .routes(routes!(state::state))
        .routes(routes!(live::live))
        .routes(routes!(talk::talk))
        .routes(routes!(talk::seen))
        .routes(routes!(threads::activity))
        .routes(routes!(document::openapi_json))
        .routes(routes!(settings::settings))
        .routes(routes!(theme::theme, theme::set_theme))
        .routes({
            let (schemas, paths, method) = routes!(mcp::mcp_room);
            (schemas, paths, method.fallback(mcp::post_only))
        })
        .routes(routes!(history::history))
        .routes(routes!(uploads::upload))
        .routes(routes!(uploads::file))
        .routes(routes!(health::health))
        .routes(routes!(health::lag))
        .routes(routes!(auth::status))
        .routes(routes!(setup::setup))
        .routes(routes!(push::register))
        .routes(routes!(secret_links::page, secret_links::save))
        .routes(routes!(mac::mac))
        .routes(routes!(mac::lock))
        .routes(routes!(backup::backup))
        .routes(routes!(tasks::list, tasks::create))
        .routes(routes!(tasks::detail))
        .routes(routes!(tasks::message))
        .routes(routes!(tasks::artifact))
        .routes(routes!(tasks::thumb))
        .routes(routes!(tasks::scheduled))
        .routes(routes!(events::events))
        .routes(routes!(memory::facts))
        .routes(routes!(memory::notes, memory::save_note))
        .routes(routes!(runner::events))
        .routes(routes!(runner::machine))
        .routes(routes!(runner::mcp))
        .routes(routes!(runner::machines))
        .routes(routes!(runner::sessions, runner::start))
        .routes(routes!(runner::folders))
        .routes(routes!(runner::branches))
        .routes(routes!(runner::transcript))
        .routes(routes!(runner::live))
        .routes(routes!(runner::send))
        .routes(routes!(runner::stop))
        .routes(routes!(runner::interrupt))
        .routes(routes!(runner::settings))
        .routes(routes!(runner::worker_transcript))
        .routes(routes!(runner::worker_live))
        .routes(routes!(runner::worker_send))
        .routes(routes!(runner::worker_stop))
        .routes(routes!(runner::worker_interrupt))
        .routes(routes!(jobs::jobs))
        .routes(routes!(jobs::stop))
        .routes(routes!(settings_pages::switches, settings_pages::switch))
        .routes(routes!(settings_pages::environments))
        .routes(routes!(settings_pages::check))
        .routes(routes!(accounts::accounts))
        .routes(routes!(accounts::use_account))
        .routes(routes!(accounts::check_usage))
        .routes(routes!(calls::record))
        .routes(routes!(calls::track))
        .routes(routes!(calls::chunk))
        .routes(routes!(calls::calls))
        .routes(routes!(calls::transcript))
        .routes(routes!(calls::live))
        .routes(routes!(voice::session))
        .routes(routes!(voice::ask))
        .routes(routes!(voice::done))
        .routes(routes!(voice::transcribe))
        .routes(routes!(calendar::calendar))
        .routes(routes!(health_data::health))
        .routes(routes!(health_data::screentime))
        .routes(routes!(whatsapp::state, whatsapp::change))
        .routes(routes!(whatsapp::take))
        .routes(routes!(whatsapp::qr))
        .routes(routes!(gmail::connect))
        .routes(routes!(gmail::callback))
        .routes({
            let (schemas, paths, method) = routes!(workers::mcp_worker);
            (schemas, paths, method.fallback(mcp::post_only))
        })
        .routes(routes!(workers::ultra_gate))
        .routes(routes!(browser::session))
        .routes(routes!(browser::frame))
        .routes(routes!(browser::input))
        .routes(routes!(browser::upload))
        .routes(routes!(browser::chrome))
        .routes(routes!(browser::chrome_ws))
        .routes(routes!(browser::pair_page))
        .routes(routes!(browser::pair_status))
        .routes(routes!(browser::emberhut_extension_zip))
        .routes(routes!(parties::list, parties::create))
        .routes(routes!(parties::get))
        .routes(routes!(parties::live))
        .routes(routes!(parties::say))
        .routes(routes!(parties::pause))
        .routes(routes!(parties::resume))
        .routes(routes!(parties::stop))
        .routes(routes!(parties::caps))
        .routes(routes!(parties::delete))
        .routes(routes!(dark::page, dark::say, dark::clear))
        .routes(routes!(agents::list, agents::create))
        .routes(routes!(agents::page, agents::change, agents::delete))
        .routes(routes!(agents::channels))
        .routes(routes!(agents::tools))
        .routes(routes!(agents::accounts))
        .routes(routes!(agents::policies))
        .routes(routes!(agents_inbound::inbound))
        .routes(routes!(agents_inbound::notes))
        .routes(routes!(agents::approvers, agents::create_approver))
        .routes(routes!(agents::change_approver, agents::delete_approver))
        .routes(routes!(
            agents::trusted_requesters,
            agents::replace_trusted_requesters
        ))
        .routes(routes!(approvals::list))
        .routes(routes!(approvals::slack_action))
        .routes(routes!(approvals::get))
        .routes(routes!(approvals::decide))
        .routes(routes!(approvals::withdraw))
        .routes(routes!(approvals::use_approval))
        .routes(routes!(approvals::webhook_answer))
        .routes(routes!(approve::page_of, approve::decide));
    family::routes(router)
}

/// The signed-in device behind a request that acts for the owner (runs something, spends
/// money, sends, changes data, or hands out a credential), in either auth mode: report mode's
/// pass without a session is for reading only. Without one, 401 `signIn`.
fn acting(device: Result<SignedIn, ApiError>) -> Result<SignedIn, ApiError> {
    device.map_err(|error| match error {
        ApiError::DeviceRequired => ApiError::SignInRequired,
        other => other,
    })
}

/// Whether a `POST` to `path` may take longer than the request timeout: a recording's audio
/// arriving, a recording being transcribed, a voice call's request waiting for its answer, or
/// the Mac's recorder sending its tracks before it answers.
pub(crate) fn takes_long(path: &str) -> bool {
    use emberhut_api::paths;
    [
        paths::CALLS_IN,
        paths::CALLS_RECORD,
        paths::TRANSCRIBE,
        paths::VOICE_ASK,
    ]
    .contains(&path)
}

/// Every route, without middleware or state; any other path answers 404.
pub(crate) fn router() -> Router<AppState> {
    let (router, _) = api().split_for_parts();
    router.fallback(not_found)
}

/// The `OpenAPI` document of every route the server serves.
#[must_use]
pub fn openapi() -> Document {
    api().split_for_parts().1
}

/// The document as it is served and committed: pretty-printed JSON with a final newline.
static DOCUMENT_JSON: LazyLock<Result<String, String>> = LazyLock::new(|| {
    openapi()
        .to_pretty_json()
        .map(|mut json| {
            json.push('\n');
            json
        })
        .map_err(|error| error.to_string())
});

/// The `OpenAPI` document could not be written as JSON.
#[derive(Clone, Debug, thiserror::Error)]
#[error("the OpenAPI document cannot be written as JSON: {0}")]
pub struct DocumentError(String);

/// The document as it is served at `/api/openapi.json` and committed as
/// `crates/emberhut-server/openapi.json`. Made once; `serve` refuses to start without it.
///
/// # Errors
///
/// [`DocumentError`] when the document cannot be written as JSON.
pub fn openapi_json() -> Result<&'static str, DocumentError> {
    DOCUMENT_JSON
        .as_deref()
        .map_err(|error| DocumentError(error.clone()))
}

/// Any path no route has.
async fn not_found() -> ApiError {
    ApiError::NotFound
}
