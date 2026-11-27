//! Approvals: one mechanism for every OK an agent's outward step needs, around a fixed core
//! that nothing configurable can loosen.
//!
//! - [`ActionKind`]: what an approval is for; [`fixed_core`]: which kinds are always the
//!   owner's, how a request's words raise its kind, and who approves.
//! - [`Approver`]: the owner, an external approver ([`ApproverRecord`]), or nobody.
//! - [`Payload`] and its [`PayloadHash`]: exactly what an approval covers.
//! - [`RequestState`]: a request's life, and when it expires ([`state`]).
//! - [`NewApproval`], [`ImportedApproval`], [`ApprovalRequest`]: a request made, imported, and
//!   stored.
//! - [`shown`]: what an approver is shown, with invisible characters written out.
//! - How a decision proves who made it: a signed [`link`], a [`webhook`] signature, a [`slack`]
//!   request signature.
//! - [`spend`]: a month's spending against an agent's cap.
//!
//! Rules only: where requests are kept and how they are routed is the server's.

pub mod fixed_core;
pub mod link;
pub mod shown;
pub mod slack;
pub mod spend;
pub mod state;
pub mod steps;
pub mod webhook;

mod approver;
mod kind;
mod mac;
mod payload;
mod request;

pub use approver::{
    APPROVER_NAME_MAX, Approver, ApproverError, ApproverId, ApproverRecord, InvalidApproverId,
    SlackIdentity, WebhookTarget,
};
pub use fixed_core::{Policy, PolicyError};
pub use kind::ActionKind;
pub use mac::{MAX_SIGNATURE_AGE, SignatureError};
pub use payload::{FileDigest, PAYLOAD_TAG, Payload, PayloadHash, Sha256Digest};
pub use request::{
    ApprovalFile, ApprovalRequest, DecisionRecord, FILE_NAME_MAX, FILES_MAX, IMPORT_TAG,
    ImportedApproval, NOTE_MAX, NewApproval, RequestError, SUMMARY_MAX, SlackPost, TARGET_MAX,
    TEXT_MAX, UseRecord,
};
pub use spend::{MonthKey, within_cap};
pub use state::{DecidedVia, Decision, RequestState};
