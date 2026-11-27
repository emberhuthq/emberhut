//! `chrome_devices`: a Chrome profile paired through the browser extension.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods that a table with no relations \
              leaves without an await"
)]

use sea_orm::entity::prelude::*;

/// A paired profile.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chrome_devices")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its short name for the tools (`chrome:<name>`).
    pub(crate) name: String,
    /// Its token's hash.
    pub(crate) token: String,
    /// When it was paired, in ms.
    pub(crate) created_at: i64,
    /// When it was last connected, in ms.
    pub(crate) seen_at: Option<i64>,
    /// When it was unpaired, in ms.
    pub(crate) revoked_at: Option<i64>,
    /// The name the owner gave it.
    pub(crate) label: Option<String>,
    /// The Google account the profile is signed in to.
    pub(crate) email: Option<String>,
    /// The machine it runs on.
    pub(crate) machine: Option<String>,
    /// Its operating system, as the extension says.
    pub(crate) os: Option<String>,
    /// The extension's version.
    pub(crate) version: Option<String>,
    /// Chrome's own version.
    pub(crate) agent: Option<String>,
    /// The extension's id.
    pub(crate) ext: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
