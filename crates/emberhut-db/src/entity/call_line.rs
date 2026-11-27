//! `call_lines`: a call's live transcript, one row per line heard.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A line.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "call_lines")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The call.
    pub(crate) call_id: String,
    /// The track it was heard on (`mic`, `sys`).
    pub(crate) track: String,
    /// Seconds from the call's start.
    pub(crate) t: f64,
    /// What was said.
    pub(crate) text: String,
    /// The language it was heard in.
    pub(crate) lang: Option<String>,
    /// The transcriber was sure of the language.
    pub(crate) sure: bool,
    /// When it was heard.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
