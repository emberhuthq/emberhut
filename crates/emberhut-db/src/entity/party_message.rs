//! `party_messages`: what was said in a party, in order.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]
#![expect(
    clippy::struct_field_names,
    reason = "`model` is the column of the turn's model, not a name for the row"
)]

use sea_orm::entity::prelude::*;

/// One line said in a party.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "party_messages")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its party.
    pub(crate) party: i64,
    /// The host, or a persona's id.
    pub(crate) who: String,
    /// The speaker's name.
    pub(crate) name: String,
    /// `turn`, `say` or `steer`.
    pub(crate) kind: String,
    /// What was said.
    pub(crate) text: String,
    /// The model a turn was said with.
    pub(crate) model: Option<String>,
    /// Tokens it took.
    pub(crate) tokens: i64,
    /// What it cost, USD.
    pub(crate) cost: f64,
    /// Stopped mid-turn.
    pub(crate) cut: bool,
    /// When it was said.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
