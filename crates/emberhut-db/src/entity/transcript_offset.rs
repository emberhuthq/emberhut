//! `transcript_offsets`: how far recall has read each agent transcript.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A transcript's place.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "transcript_offsets")]
pub(crate) struct Model {
    /// The transcript's path.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) path: String,
    /// The bytes read (always up to the end of a line).
    pub(crate) offset: i64,
    /// The lines read.
    pub(crate) line: i64,
}

impl ActiveModelBehavior for ActiveModel {}
