//! The stores against a real SQLite database: each test gets a fresh, migrated file.
//!
//! One test binary with a module per store, so the helpers in `common` are shared without
//! being compiled (and warned about) once per file.

mod agents;
mod approvals;
mod builds;
mod calls;
mod common;
mod dark;
mod files;
mod import;
mod memory;
mod messaging;
mod pairing;
mod parties;
mod push;
mod rooms;
mod secret_links;
mod sessions;
mod settings;
mod tasks;
mod threads;
mod workers;
