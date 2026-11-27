//! `approvers`: external approvers and how they are reached.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An external approver.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "approvers")]
pub(crate) struct Model {
    /// Its id, a slug.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Its name, as the apps show it.
    pub(crate) name: String,
    /// Its Slack team's id.
    pub(crate) slack_workspace: Option<String>,
    /// Its Slack user's id.
    pub(crate) slack_user: Option<String>,
    /// Where requests are posted to its own app.
    pub(crate) webhook_url: Option<String>,
    /// The secret file both sides sign webhook calls with.
    pub(crate) webhook_secret: Option<String>,
    /// A signed link may be sent to it.
    pub(crate) link: bool,
    /// When it was made.
    pub(crate) created_at: i64,
    /// When it last changed.
    pub(crate) updated_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
