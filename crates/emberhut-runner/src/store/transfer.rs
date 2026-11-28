//! `transfers`: every file in or out, and every refusal.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A file transfer or a refused one.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "transfers")]
pub struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub id: i64,
    /// When.
    pub at: Option<String>,
    /// `in` (into a session's inbox) or `out` (to the server).
    pub dir: Option<String>,
    /// The session.
    pub session: Option<String>,
    /// The file.
    pub path: Option<String>,
    /// Its size in bytes.
    pub size: Option<i64>,
    /// Who asked.
    pub by: Option<String>,
    /// 1 when it went through.
    pub ok: Option<i64>,
    /// Why it was refused.
    pub why: Option<String>,
    /// 1 once the session was told it arrived.
    pub told: i64,
}

impl ActiveModelBehavior for ActiveModel {}
