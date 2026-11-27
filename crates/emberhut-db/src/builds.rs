//! The newest build of the app per platform ([`LatestBuilds`]).

use emberhut_core::builds::{BuildReport, LatestBuilds, Platform};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue, DatabaseConnection, EntityTrait};

use crate::entity::latest_build;
use crate::{DbError, begin_write};

/// Reads and raises the newest builds.
#[derive(Clone, Debug)]
pub struct BuildStore {
    conn: DatabaseConnection,
}

impl BuildStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The newest build reported per platform. A row this server doesn't know (another
    /// platform, a number out of range) is left out.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn latest(&self) -> Result<LatestBuilds, DbError> {
        let rows = latest_build::Entity::find().all(&self.conn).await?;
        Ok(from_rows(rows))
    }

    /// Takes `report`: its platform's number moves up to it, never down. Returns the newest
    /// builds after it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn report(&self, report: BuildReport) -> Result<LatestBuilds, DbError> {
        let txn = begin_write(&self.conn).await?;
        let raised = from_rows(latest_build::Entity::find().all(&txn).await?).with(report);
        let number = raised.get(report.platform).unwrap_or(report.number);
        latest_build::Entity::insert(latest_build::ActiveModel {
            platform: ActiveValue::Set(report.platform.as_str().to_owned()),
            number: ActiveValue::Set(i64::from(number)),
        })
        .on_conflict(
            OnConflict::column(latest_build::Column::Platform)
                .update_column(latest_build::Column::Number)
                .to_owned(),
        )
        .exec(&txn)
        .await?;
        txn.commit().await?;
        Ok(raised)
    }
}

fn from_rows(rows: Vec<latest_build::Model>) -> LatestBuilds {
    let mut builds = LatestBuilds::default();
    for row in rows {
        let (Some(platform), Ok(number)) = (
            Platform::from_name(&row.platform),
            u32::try_from(row.number),
        ) else {
            continue;
        };
        match platform {
            Platform::Ios => builds.ios = Some(number),
            Platform::Mac => builds.mac = Some(number),
        }
    }
    builds
}
