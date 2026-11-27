//! The emberhut server's domain rules, with no I/O: sessions and tokens, pairing codes, rooms and
//! messages, the chat's files, the runtime's lag, response tags, tasks with their projects,
//! areas and plan, and agents with their approvals. Time and randomness come in
//! through [`Clock`] and [`RandomSource`], so every rule can be tested at any instant with any
//! bytes.
//!
//! The database (`emberhut-db`) stores these types and the server (`emberhut-server`) applies these
//! rules to requests.

pub mod account;
pub mod agents;
pub mod approvals;
pub mod briefs;
pub mod browser;
pub mod builds;
pub mod calendar;
pub mod calls;
pub mod chrome;
mod clock;
pub mod dark;
mod device;
mod etag;
pub mod event;
pub mod failover;
pub mod guard;
pub mod health_data;
mod ids;
pub mod lag;
pub mod log;
pub mod machine;
pub mod memory;
pub mod message;
pub mod messaging;
pub mod pairing;
pub mod party;
pub mod plan;
pub mod private_network;
pub mod push;
mod random;
pub mod refs;
pub mod room;
pub mod runner;
pub mod same_task;
pub mod schedule;
mod secret;
pub mod secret_link;
pub mod session;
pub mod task;
pub mod theme;
pub mod thread;
mod token;
pub mod ultra;
pub mod upload;
pub mod wake;
mod wire;

pub use clock::{Clock, ManualClock, SystemClock};
pub use device::DeviceDetails;
pub use etag::ETag;
pub use ids::{
    ApprovalId, DeviceId, EventId, FileId, LogLineId, MessageId, PairingCodeId, PartyId,
    PartyMessageId, RoomId, SecretLinkId,
};
pub use random::{OsRandom, RandomError, RandomSource};
pub use secret::SecretHash;
pub use token::Token;
pub use wire::UnknownWord;
