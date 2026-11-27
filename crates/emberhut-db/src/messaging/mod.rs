//! The owner's messages from outside the app, and the assistant's own WhatsApp number: one
//! store per source ([`SlackStore`], [`MailStore`], [`VoiceNoteStore`], [`WhatsAppStore`]).

mod mail;
mod slack;
mod voice_notes;
mod whatsapp;

pub use mail::MailStore;
pub use slack::SlackStore;
pub use voice_notes::{Outcome, VoiceNoteStore};
pub use whatsapp::{ChatChange, NewWaSent, WhatsAppStore};
