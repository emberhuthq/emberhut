//! `devices`: every device that has signed in, one row per app install.

use sea_orm::entity::prelude::*;

/// A signed-in device.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "devices")]
pub(crate) struct Model {
    /// The device's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// What the device calls itself ("Main laptop").
    pub(crate) name: String,
    /// Its hardware model ("Pixel 8"), as the app reports it; may be empty.
    pub(crate) hardware: String,
    /// The app's own id for this install, kept across sign-ins: one row per install.
    #[sea_orm(unique)]
    pub(crate) install: Option<String>,
    /// When the device first signed in.
    pub(crate) created_at: i64,
    /// When the device last used its session (to the minute).
    pub(crate) seen_at: i64,
    /// When the device was signed out; `None` while it may sign in.
    pub(crate) revoked_at: Option<i64>,
    /// Its current session, while it has one.
    #[sea_orm(has_one)]
    pub(crate) session: HasOne<super::session::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
