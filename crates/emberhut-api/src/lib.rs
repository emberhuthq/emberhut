//! The emberhut HTTP/SSE contract: the request and response types that every client and the server
//! share. Serde only: no I/O and no HTTP library.
//!
//! The contract is described in `docs/api.md` in the repository.
//!
//! # Conventions on the wire
//!
//! - Field names are camelCase; ids are bare numbers or strings ([`ids`]).
//! - A field the server always writes is required here, `null` included (an `Option`). A field the
//!   server leaves out when it has nothing is skipped when this crate writes it too, and a group
//!   the server writes together (a waiting work row's `need`, `asked`, `checked`) is one flattened
//!   struct. For every field it models, a server built on these types writes the documented shape;
//!   the contract tests check it both ways, against a list of what isn't modelled yet.
//! - Reading is tolerant: unknown fields are ignored, and fields an older server lacks default.
//! - A vocabulary the server defines in full (a task's status, a project's health, an ask's
//!   kind) is an open enum: its known words, and `Other` for a word a newer server adds. A word
//!   that comes from elsewhere stays text: an artifact's kind (what the work produced), a runner
//!   session's status (the runner's), a task's owner (a person's or the assistant's id), what a
//!   machine's OS reports.
//! - Times are kept as the server's text ([`time`]).
//!
//! # Features
//!
//! - `openapi`: every request and response type derives `utoipa::ToSchema` (query types
//!   `utoipa::IntoParams` too), for the server's `OpenAPI` document.
//! - `validate`: the request types derive `garde::Validate`, with the rules each field's doc
//!   gives.
//!
//! ```
//! use emberhut_api::state::State;
//!
//! let body = r#"{
//!     "room": 1, "rooms": [], "on": true, "busy": false, "live": null, "unread": false,
//!     "pending": 0, "chat": [], "earlier": false, "aFieldFromANewerServer": 42
//! }"#;
//! let state: State = serde_json::from_str(body).unwrap();
//! assert!(state.room.is_main());
//! assert!(state.issues.is_empty());
//! ```

pub mod accounts;
pub mod agent_provider;
pub mod agents;
pub mod answers;
pub mod approvals;
pub mod areas;
pub mod auth;
pub mod background;
pub mod browser;
pub mod builds;
pub mod calendar;
pub mod calls;
pub mod chat;
pub mod choices;
pub mod chrome;
pub mod connect;
pub mod dark;
pub mod directory;
pub mod ember_name;
pub mod environments;
pub mod error;
pub mod facts;
pub mod family;
pub mod headers;
pub mod health;
pub mod health_data;
pub mod health_today;
pub mod ids;
pub mod interrupt;
pub mod jobs;
pub mod live;
pub mod mac;
pub mod machines;
pub mod mcp;
pub mod notes;
pub mod parties;
pub mod party;
pub mod patch;
pub mod paths;
pub mod plan;
pub mod projects;
pub mod push;
pub mod relay;
pub mod releases;
pub mod rooms;
pub mod runner;
pub mod runners;
pub mod scheduled;
pub mod seen;
pub mod settings;
pub mod setup;
pub mod state;
pub mod switches;
pub mod talk;
pub mod task_actions;
pub mod task_live;
pub mod task_message;
pub mod task_page;
pub mod task_plan;
pub mod task_rename;
pub mod task_search;
pub mod tasks;
pub mod theme;
pub mod threads;
pub mod time;
pub mod transcribe;
pub mod ultra;
pub mod uploads;
pub mod usage;
pub mod voice;
pub mod wake;
pub mod whatsapp;
pub mod wire;
pub mod work;
pub mod x_reply;

pub mod query;

mod open_enum;
#[cfg(feature = "validate")]
mod rules;
mod serde_util;
