//! Keeping paired apps and browser profiles across a re-import: `devices`, `sessions` and
//! `chrome_devices` copied from the database being replaced into the new one (`--keep-devices`). The import itself never fills `chrome_devices` (a source's browser profiles
//! stay in the archive), so without the copy every profile paired with Emberhut in between would
//! have to pair again. The id of the machine key it was served with ([`MACHINE_ID`]) comes
//! too, so the new one is served as the same machine.

use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryOrder,
};

use crate::entity::{chrome_device, device, session, setting};
use crate::{Database, DatabaseUrl, DbError, MACHINE_ID, begin_write};

/// What [`keep_devices`] copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeptDevices {
    /// Devices copied.
    pub devices: u64,
    /// Sessions copied.
    pub sessions: u64,
    /// Browser profiles copied.
    pub browser_profiles: u64,
    /// Whether the id of the machine key `from` was served with was carried (`from` had one).
    pub machine: bool,
}

/// Copies every device, session and browser profile of the Emberhut database at `from` into the one
/// at `into`, with the id of the machine key `from` was served with, in one transaction, and
/// checks the counts.
///
/// Which browser a task works in (`chrome_tasks`) is not copied: the import makes the tasks
/// again, and a task id of the old database may name another task in the new one.
///
/// Both must be Emberhut's and up to date; `into` must have no device and no browser profile yet.
/// The server of `from` must be stopped: a session it renewed after the copy would be one `into`
/// does not have.
///
/// # Errors
///
/// [`DbError::Missing`], [`DbError::NotEmberhut`] and [`DbError::Outdated`] for either file,
/// [`DbError::NotUnused`] when `into` has devices or browser profiles, [`DbError::OtherMachine`]
/// when `into` was served as another machine, [`DbError::CopyMismatch`] when the counts
/// differ after the copy (nothing is then kept), and [`DbError::Query`].
pub async fn keep_devices(from: &DatabaseUrl, into: &DatabaseUrl) -> Result<KeptDevices, DbError> {
    let old = Database::open_existing(from).await?;
    let new = Database::open_existing(into).await?;
    let result = copy(&old, &new, into).await;
    old.close().await?;
    new.close().await?;
    result
}

async fn copy(old: &Database, new: &Database, into: &DatabaseUrl) -> Result<KeptDevices, DbError> {
    let devices = device::Entity::find()
        .order_by_asc(device::Column::Id)
        .all(&old.conn)
        .await?;
    let sessions = session::Entity::find()
        .order_by_asc(session::Column::DeviceId)
        .all(&old.conn)
        .await?;
    let profiles = chrome_device::Entity::find()
        .order_by_asc(chrome_device::Column::Id)
        .all(&old.conn)
        .await?;
    let machine = setting::Entity::find_by_id(MACHINE_ID.name())
        .one(&old.conn)
        .await?;
    let txn = begin_write(&new.conn).await?;
    if device::Entity::find().count(&txn).await? > 0
        || chrome_device::Entity::find().count(&txn).await? > 0
    {
        return Err(DbError::NotUnused(into.path().to_path_buf()));
    }
    let machine = carry_machine(&txn, machine.as_ref(), into).await?;
    for row in &devices {
        row.clone()
            .into_active_model()
            .reset_all()
            .insert(&txn)
            .await?;
    }
    for row in &sessions {
        row.clone()
            .into_active_model()
            .reset_all()
            .insert(&txn)
            .await?;
    }
    for row in &profiles {
        row.clone()
            .into_active_model()
            .reset_all()
            .insert(&txn)
            .await?;
    }
    let kept = KeptDevices {
        devices: device::Entity::find().count(&txn).await?,
        sessions: session::Entity::find().count(&txn).await?,
        browser_profiles: chrome_device::Entity::find().count(&txn).await?,
        machine,
    };
    for (table, expected, found) in [
        ("devices", devices.len() as u64, kept.devices),
        ("sessions", sessions.len() as u64, kept.sessions),
        (
            "chrome_devices",
            profiles.len() as u64,
            kept.browser_profiles,
        ),
    ] {
        if expected != found {
            return Err(DbError::CopyMismatch {
                table,
                expected,
                found,
            });
        }
    }
    txn.commit().await?;
    Ok(kept)
}

/// Carries the remembered machine id `carried` (the old database's, if it has one) into the
/// new database in `txn`, unless that was served as another machine, and checks it is there.
/// Returns whether one was carried.
async fn carry_machine(
    txn: &impl ConnectionTrait,
    carried: Option<&setting::Model>,
    into: &DatabaseUrl,
) -> Result<bool, DbError> {
    let Some(carried) = carried else {
        return Ok(false);
    };
    match setting::Entity::find_by_id(MACHINE_ID.name())
        .one(txn)
        .await?
    {
        Some(found) if found.value != carried.value => {
            return Err(DbError::OtherMachine {
                path: into.path().to_path_buf(),
                found: found.value,
                carried: carried.value.clone(),
            });
        }
        Some(_) => {}
        None => {
            carried
                .clone()
                .into_active_model()
                .reset_all()
                .insert(txn)
                .await?;
        }
    }
    let kept = setting::Entity::find_by_id(MACHINE_ID.name())
        .one(txn)
        .await?;
    if kept.as_ref().map(|row| &row.value) != Some(&carried.value) {
        return Err(DbError::CopyMismatch {
            table: "settings",
            expected: 1,
            found: 0,
        });
    }
    Ok(true)
}
