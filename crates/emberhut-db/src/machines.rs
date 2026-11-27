//! Runner machines' samples: each one's latest ([`MachineStore::latest`]) and a day of
//! 5-minute points ([`MachineStore::points_since`]). Samples and points are kept as the JSON
//! the server makes of them; the rules that read them are `emberhut-core`'s.

use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::entity::{machine_latest, machine_point};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// A machine's latest sample, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSample {
    /// The machine.
    pub machine: String,
    /// When the sample came.
    pub at: Timestamp,
    /// The sample, as JSON.
    pub data: String,
}

/// Keeps the runner machines' samples and points.
#[derive(Clone, Debug)]
pub struct MachineStore {
    conn: DatabaseConnection,
}

impl MachineStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Every machine's latest sample.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn latest(&self) -> Result<Vec<StoredSample>, DbError> {
        machine_latest::Entity::find()
            .order_by_asc(machine_latest::Column::Machine)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(StoredSample {
                    machine: row.machine,
                    at: instant(row.at)?,
                    data: row.data,
                })
            })
            .collect()
    }

    /// Keeps `sample` as its machine's latest, and folds `point` into its bucket's point with
    /// `fold` (given the point stored there, if any, it returns the one to keep). Points before
    /// `prune_before` (milliseconds since the epoch) are dropped when it is given.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn take(
        &self,
        sample: &StoredSample,
        bucket: i64,
        fold: impl FnOnce(Option<&str>) -> String + Send,
        prune_before: Option<i64>,
    ) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        machine_latest::Entity::insert(machine_latest::ActiveModel {
            machine: Set(sample.machine.clone()),
            at: Set(millis(sample.at)),
            data: Set(sample.data.clone()),
        })
        .on_conflict(
            OnConflict::column(machine_latest::Column::Machine)
                .update_columns([machine_latest::Column::At, machine_latest::Column::Data])
                .to_owned(),
        )
        .exec(&txn)
        .await?;
        let was = machine_point::Entity::find_by_id((sample.machine.clone(), bucket))
            .one(&txn)
            .await?;
        let data = fold(was.as_ref().map(|row| row.data.as_str()));
        machine_point::Entity::insert(machine_point::ActiveModel {
            machine: Set(sample.machine.clone()),
            bucket: Set(bucket),
            data: Set(data),
        })
        .on_conflict(
            OnConflict::columns([
                machine_point::Column::Machine,
                machine_point::Column::Bucket,
            ])
            .update_column(machine_point::Column::Data)
            .to_owned(),
        )
        .exec(&txn)
        .await?;
        if let Some(before) = prune_before {
            machine_point::Entity::delete_many()
                .filter(machine_point::Column::Bucket.lt(before))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// `machine`'s points from the bucket `from` (milliseconds since the epoch) on, oldest
    /// first, as JSON.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn points_since(&self, machine: &str, from: i64) -> Result<Vec<String>, DbError> {
        Ok(machine_point::Entity::find()
            .filter(machine_point::Column::Machine.eq(machine))
            .filter(machine_point::Column::Bucket.gte(from))
            .order_by_asc(machine_point::Column::Bucket)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| row.data)
            .collect())
    }
}
