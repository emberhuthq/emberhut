//! `files`: the files stored for the chat, one row per file in the uploads folder.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes async methods that need no await for an entity without relations"
)]

use sea_orm::entity::prelude::*;

/// What kind of file it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub(crate) enum Kind {
    /// A picture.
    #[sea_orm(string_value = "image")]
    Image,
    /// A video.
    #[sea_orm(string_value = "video")]
    Video,
    /// Anything else.
    #[sea_orm(string_value = "file")]
    File,
}

/// A stored file.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "files")]
pub(crate) struct Model {
    /// The file's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its name in the uploads folder.
    #[sea_orm(unique)]
    pub(crate) stored_name: String,
    /// Its name as given.
    pub(crate) name: String,
    /// Its kind.
    pub(crate) kind: Kind,
    /// Its size in bytes.
    pub(crate) size: i64,
    /// When it was stored.
    pub(crate) created_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
