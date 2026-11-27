//! A blocking client for the emberhut API: sessions, token renewal and the live stream.
//!
//! It speaks the contract in [`emberhut_api`]. Every call blocks; the desktop app runs them on background threads and shares
//! one [`Client`] between them.
//!
//! ```no_run
//! use emberhut_client::{Client, ClientConfig, FileSessionStore, StatePoll, StateReply};
//! use emberhut_api::ids::RoomId;
//!
//! let config = ClientConfig::new("https://emberhut.example.com")?;
//! let client = Client::new(&config, FileSessionStore::new("/path/to/session.json"))?;
//! if !client.is_signed_in() {
//!     client.pair("ABCD-EFGH")?; // a code made on a signed-in device
//! }
//! let mut etag = None;
//! match client.state(&StatePoll::new(RoomId::MAIN).held(etag.clone()).looking(true))? {
//!     StateReply::Fresh { state, etag: tag } => {
//!         etag = tag;
//!         println!("{} rooms", state.rooms.len());
//!     }
//!     StateReply::Unchanged => {}
//!     StateReply::SignInRequired => println!("pair this device first"),
//!     _ => {} // a newer kind of answer
//! }
//! # let _ = etag;
//! # Ok::<(), emberhut_client::Error>(())
//! ```
//!
//! # Sessions
//!
//! A [`Session`] (this install's id and, once paired, its tokens) lives in a [`SessionStore`]:
//! [`FileSessionStore`] (a 0600 file, written atomically) or [`MemorySessionStore`]. One
//! process per store. The access token is renewed one renewal at a time, so concurrent calls
//! never send a used refresh token; a token the server refuses although it looks good is
//! renewed once and the request sent again.
//!
//! # Transports
//!
//! Every request goes through a [`transport::Transport`]: [`Client::new`] makes a
//! [`transport::Direct`] to the configured base URL, and [`Client::with_transport`] takes any
//! other (one that wraps a `Direct` to change requests and answers on their way, or that reaches
//! the machine another way).
//!
//! # Building a client for a server that spells some wire words differently
//!
//! A server may spell some of the contract's wire words its own way: a value, an object key, a
//! header name, a task key's prefix or a link's scheme. Built with `EMBERHUT_WIRE_DIALECT`
//! naming a dialect file (a JSON table of the server's spellings and the contract's), this crate
//! rewrites every request on its way out and every answer on its way in, so the app above it
//! sees only the contract's words. Without the variable nothing is rewritten. The file's format,
//! the rules it is applied by (the same in the mobile app and the browser extension) and the
//! functions that apply it to one text or JSON document are in [`dialect`].
//!
//! # Logging
//!
//! This crate never logs a token, and `Debug` on every type here hides them. The HTTP library
//! underneath logs through the `log` crate, and at its `trace` level it writes the raw wire,
//! `Authorization` header and refresh body included. An app that collects `log` records must
//! cap those targets at `debug`: add [`LOG_DIRECTIVES`] to its filter.
//!
//! # Errors
//!
//! Every call returns an [`Error`]: the server out of reach, a refusal with the server's own
//! words, sign-in required, an answer that doesn't decode, or the session store failing.

mod actions;
mod agent_provider;
mod approvals;
mod browser;
mod client;
mod config;
mod days;
pub mod dialect;
mod error;
mod exchange;
mod file_store;
mod live;
mod party;
mod releases;
mod renewal;
mod runner;
mod session;
mod setup;
pub mod sse;
pub mod transport;
mod upload;
mod working;

pub use client::{Client, StatePoll, StateReply, is_server_path};
pub use config::ClientConfig;
pub use config::InvalidBaseUrl;
pub use error::{Error, NetworkError};
pub use file_store::FileSessionStore;
pub use live::{LiveEnd, RoomEvent};
pub use releases::Downloaded;
pub use session::{
    MemorySessionStore, Session, SessionStore, SessionTokens, StoreError, stored_version,
};
pub use upload::{Progress, Upload};

/// The largest file [`Client::download`] takes: 4 GiB, far past the 500 MiB the server stores
/// from an upload, so only a file that has no business on a chat stops there.
pub const MAX_DOWNLOAD: u64 = 4 << 30;

/// Filter directives (`env_logger`/`tracing-subscriber` `EnvFilter` syntax) that keep the HTTP
/// library's unredacted wire trace out of an app's logs. Append them to the app's filter:
/// `format!("{filter},{LOG_DIRECTIVES}")`; a later directive for a target wins.
pub const LOG_DIRECTIVES: &str = "ureq=debug,ureq_proto=debug,rustls=info";
