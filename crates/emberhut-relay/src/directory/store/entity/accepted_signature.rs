//! `accepted_signatures`: the signatures of link and unlink statements already accepted, each
//! kept until its statement can no longer be fresh, so none is accepted twice.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An accepted statement's signature.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "accepted_signatures")]
pub(crate) struct Model {
    /// The signature, 64 bytes.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) signature: Vec<u8>,
    /// When its statement stops being fresh: its time and 5 minutes.
    pub(crate) expires_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
