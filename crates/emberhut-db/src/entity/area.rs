//! `areas`: the parts of a project that run for a while on their own.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An area.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "areas")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its project.
    pub(crate) pid: i64,
    /// Its name.
    pub(crate) name: String,
    /// `active`, `paused` or `done`.
    pub(crate) status: String,
    /// What done looks like.
    pub(crate) goal: Option<String>,
    /// A short line on what it is.
    pub(crate) about: Option<String>,
    /// When it was made.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
