//! `approval_files`: each request's files, as the server read them.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A file of a request.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "approval_files")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its request.
    pub(crate) request_id: i64,
    /// Its place in the request's list.
    pub(crate) position: i32,
    /// Its name, as the approver is shown it.
    pub(crate) name: String,
    /// Its size in bytes; none when an imported row lacks it.
    pub(crate) size: Option<i64>,
    /// The SHA-256 of its bytes.
    pub(crate) sha256: Vec<u8>,
}

impl ActiveModelBehavior for ActiveModel {}
