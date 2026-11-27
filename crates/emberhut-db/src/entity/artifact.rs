//! `artifacts`: what a task produced: files on this machine, links, X replies.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// Something a task produced.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "artifacts")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its task.
    pub(crate) issue: i64,
    /// The file's path or the link.
    #[sea_orm(column_name = "ref")]
    pub(crate) reference: String,
    /// `image`, `video`, `pdf`, `md`, `html`, `doc`, `link`, `x_reply`.
    pub(crate) kind: String,
    /// A line about it.
    pub(crate) caption: Option<String>,
    /// When it was attached.
    pub(crate) at: i64,
    /// An X reply's post and replies (JSON).
    pub(crate) data: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
