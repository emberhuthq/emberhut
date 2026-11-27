//! The sessions on the runners, read and driven from a client: a session's transcript and its
//! live stream, the person's reply, Stop and settings, a new session of the person's own (with
//! the folders and branches its sheet offers), the agents working now and the Claude accounts
//! they run on, and the threads of an older tool still running, read only.
//!
//! The routes, all under `/api/runner/sessions/{id}` for one session ([`paths`](crate::paths)):
//!
//! | route | what |
//! |-------|------|
//! | `GET  transcript?after=&limit=&agent=` | [`transcript::Transcript`]: the newest steps, or those after one |
//! | `GET  live?after=&agent=` | server-sent [`transcript::SessionLive`] events from that step on |
//! | `POST send` | [`actions::SendToSession`]: the person's message, queued behind a running turn |
//! | `POST stop` | the step it's on ends and nothing queued runs |
//! | `POST interrupt` | only the running turn ends; what is queued runs next |
//! | `POST settings` | [`actions::SessionSettings`]: model and effort for the next turn |
//!
//! A task's worker is watched the same way as session `w-<task>` (no settings). A refusal is
//! 400, 404, 409 or 502 with an [`ErrorBody`](crate::error::ErrorBody).

pub mod accounts;
pub mod actions;
pub mod agents;
pub mod new_session;
pub mod steps;
pub mod t3;
pub mod transcript;
