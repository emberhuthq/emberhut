//! `usage`: each assistant account's use, as its runner last reported it.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One account on one machine.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "usage")]
pub(crate) struct Model {
    /// The machine whose runner reported it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// The account's id there.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Its name, as shown.
    pub(crate) name: String,
    /// What work it takes (`any`, or the runner's own word).
    pub(crate) scope: String,
    /// Whether it may be picked.
    pub(crate) enabled: bool,
    /// Whether it is logged in.
    pub(crate) logged_in: bool,
    /// Its place in the runner's order (lower first).
    pub(crate) rank: Option<i64>,
    /// The login's email.
    pub(crate) email: Option<String>,
    /// Its plan.
    pub(crate) plan: Option<String>,
    /// Its use of the 5-hour window, 0 to 100.
    pub(crate) used5h: Option<f64>,
    /// Its use of the week, 0 to 100.
    pub(crate) used7d: Option<f64>,
    /// When the 5-hour window resets.
    pub(crate) resets5h: Option<i64>,
    /// When the week resets.
    pub(crate) resets7d: Option<i64>,
    /// It is out until then.
    pub(crate) limited_until: Option<i64>,
    /// Why it is out.
    pub(crate) limited_why: Option<String>,
    /// When its runner last looked at its use.
    pub(crate) checked: Option<i64>,
    /// What went wrong at that look.
    pub(crate) error: Option<String>,
    /// When the server got this.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
