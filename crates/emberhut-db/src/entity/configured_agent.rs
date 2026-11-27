//! `configured_agents`: configured assistants; the default agent (`main`) is made by the migration.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A configured assistant.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "configured_agents")]
pub(crate) struct Model {
    /// Its id, a slug.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Its name, as the apps show it.
    pub(crate) name: String,
    /// Its system prompt; none for the default agent.
    pub(crate) persona: Option<String>,
    /// Who it works for, a display name.
    pub(crate) principal: String,
    /// Its own room.
    #[sea_orm(unique)]
    pub(crate) room_id: i64,
    /// The project it is scoped to.
    pub(crate) project_id: Option<i64>,
    /// Its monthly spending cap, in cents; none: no spending.
    pub(crate) spend_cap_cents: Option<i64>,
    /// The cap's currency code.
    pub(crate) spend_currency: Option<String>,
    /// A disabled agent takes no turns and its approvals are not routed.
    pub(crate) enabled: bool,
    /// Its tasks' workers may run shell commands.
    pub(crate) worker_shell: bool,
    /// Its room was made with it, so it goes when the agent is deleted.
    pub(crate) made_room: bool,
    /// When it was made.
    pub(crate) created_at: i64,
    /// When it last changed.
    pub(crate) updated_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
