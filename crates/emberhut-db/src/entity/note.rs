//! `notes`: the owner's quick notes and the assistant's diary entries, one row per note.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A note.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notes")]
pub(crate) struct Model {
    /// The row's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The note's own id (the app's, or the assistant's for its day).
    #[sea_orm(unique)]
    pub(crate) uid: String,
    /// Who wrote it, as stored: the wire words,
    /// [`emberhut_core::memory::wire::OWNER`] or [`emberhut_core::memory::wire::ASSISTANT`].
    pub(crate) who: String,
    /// The day it belongs to.
    pub(crate) day: String,
    /// When it was written.
    pub(crate) at: i64,
    /// What it says.
    pub(crate) text: String,
    /// When the server first had it.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
