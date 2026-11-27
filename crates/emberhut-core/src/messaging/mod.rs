//! The owner's messages from outside the app: Slack, email (Gmail, read only), voice notes as
//! text, and the assistant's own WhatsApp number. These are the rules, with no I/O: how chats
//! are named, how a message reads, what may be sent and when. The server fetches, stores and
//! sends; the database keeps the rows these types describe.
//!
//! Sending anything as the owner is guarded the same way everywhere: the text is normalised
//! ([`text::same`]), checked for what the channel would show as formatting, and goes out only
//! after the owner's OK on that exact text.

pub mod mail;
pub mod news;
pub mod slack;
pub mod text;
pub mod voice;
pub mod whatsapp;

/// How the owner's own messages are labelled where a sender's name goes.
pub const OWNER: &str = "Owner";

/// How the assistant's own messages are labelled where a sender's name goes.
pub const ASSISTANT: &str = "Assistant";
