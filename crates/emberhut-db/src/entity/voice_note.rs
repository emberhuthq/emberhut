//! `voice_notes`: voice notes, each transcribed once.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A voice note.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "voice_notes")]
pub(crate) struct Model {
    /// `slack:<file>`, `wa:<message>` or `im:<attachment>`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// `slack`, `wa` or `im`.
    pub(crate) source: String,
    /// What fetches its sound.
    #[sea_orm(column_name = "ref")]
    pub(crate) reference: String,
    /// Its file name.
    pub(crate) name: Option<String>,
    /// How long it is.
    pub(crate) seconds: Option<f64>,
    /// Where it is: `new`, `done`, `long`, `failed`, `gave_up`.
    pub(crate) status: String,
    /// What was said.
    pub(crate) text: Option<String>,
    /// Why the last try failed.
    pub(crate) error: Option<String>,
    /// How many rounds failed.
    pub(crate) tries: i32,
    /// When it is tried again.
    pub(crate) next_at: Option<i64>,
    /// The chat it was sent in.
    pub(crate) chat: Option<String>,
    /// That chat's name.
    pub(crate) chat_name: Option<String>,
    /// Who sent it.
    pub(crate) sender: Option<String>,
    /// When its message was sent.
    pub(crate) msg_at: Option<i64>,
    /// The kept message that shows it.
    pub(crate) msg: Option<String>,
    /// What that message shows for it now.
    pub(crate) shown: Option<String>,
    /// When it was first seen.
    pub(crate) at: i64,
    /// When it was done.
    pub(crate) done_at: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
