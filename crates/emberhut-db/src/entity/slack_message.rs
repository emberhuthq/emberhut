//! `slack`: the owner's Slack messages, one row per message.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A Slack message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "slack")]
pub(crate) struct Model {
    /// `<workspace>:<channel>:<ts>`, or the Mac helper's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// The workspace's name.
    pub(crate) workspace: String,
    /// The conversation's name.
    pub(crate) who: String,
    /// What it says.
    pub(crate) body: String,
    /// When it was sent.
    pub(crate) at: i64,
    /// The owner wrote it.
    pub(crate) from_me: bool,
    /// Who wrote it.
    pub(crate) sender: Option<String>,
    /// Slack's channel id, for one read from Slack.
    pub(crate) channel: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
