//! `calls`: recorded and shared calls, one row per call.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A call.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "calls")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// Who and what it is.
    pub(crate) title: String,
    /// The task its transcript is for.
    pub(crate) task: Option<i64>,
    /// The room its transcript goes to.
    pub(crate) room_id: i64,
    /// Where it is.
    pub(crate) status: String,
    /// Its audio files, as JSON: `[{ file, who }]`.
    pub(crate) tracks: String,
    /// The shared recording it was copied from.
    pub(crate) source: Option<String>,
    /// The transcript's file.
    pub(crate) transcript: Option<String>,
    /// Why the transcription failed.
    pub(crate) error: Option<String>,
    /// How long, in seconds.
    pub(crate) seconds: Option<f64>,
    /// When it started.
    pub(crate) at: i64,
    /// When it was transcribed or failed.
    pub(crate) done_at: Option<i64>,
    /// Its audio was deleted.
    pub(crate) audio_gone: bool,
}

impl ActiveModelBehavior for ActiveModel {}
