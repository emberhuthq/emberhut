//! `corrections`: what the assistant was corrected on, and its lesson's trial.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A correction.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "corrections")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// When it happened.
    pub(crate) at: i64,
    /// What the owner said.
    pub(crate) said: String,
    /// What went wrong.
    pub(crate) wrong: String,
    /// The lesson: a trial rule while `trial` is none.
    pub(crate) lesson: Option<String>,
    /// Who logged it: the assistant ([`emberhut_core::memory::wire::ASSISTANT`]) or `auto`, the
    /// server.
    pub(crate) source: String,
    /// How its trial ended: `kept` or `dropped`; none while it runs (or lapses).
    pub(crate) trial: Option<String>,
    /// Who ended it: the owner ([`emberhut_core::memory::wire::OWNER`]) or `review`.
    pub(crate) trial_by: Option<String>,
    /// When it was ended.
    pub(crate) trial_at: Option<i64>,
    /// The standing rule it became, when kept.
    pub(crate) rule: Option<i64>,
    /// The lesson as one short line to the owner, in the second person, for the app.
    pub(crate) lesson_to_owner: Option<String>,
    pub(crate) triaged: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
