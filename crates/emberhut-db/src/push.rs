//! Where to push: the iPhones' APNs device tokens and the Android phones' FCM registration
//! tokens, each with the signed-in device that registered it. A phone registering a token again
//! updates its row; a token the push service says is gone is forgotten, and a device's tokens go
//! when it is signed out ([`SessionStore::sign_out`](crate::SessionStore::sign_out)).

use emberhut_core::DeviceId;
use emberhut_core::push::{ApnsDevice, ApnsEnv, ApnsToken, FcmToken};
use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::DbError;
use crate::entity::{fcm_device, push_device};
use crate::time::millis;

/// Reads and writes the phones' push addresses.
#[derive(Clone, Debug)]
pub struct PushStore {
    conn: DatabaseConnection,
}

impl PushStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps an iPhone's token, for `env`'s service, registered by `device` at `now`. The same
    /// token again takes the new service, device and time.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when `device` is not a device's id.
    pub async fn register_apns(
        &self,
        token: &ApnsToken,
        env: ApnsEnv,
        device: DeviceId,
        now: Timestamp,
    ) -> Result<(), DbError> {
        let row = push_device::ActiveModel {
            token: Set(token.as_str().to_owned()),
            env: Set(env.as_str().to_owned()),
            device_id: Set(device.get()),
            registered_at: Set(millis(now)),
        };
        push_device::Entity::insert(row)
            .on_conflict(
                OnConflict::column(push_device::Column::Token)
                    .update_columns([
                        push_device::Column::Env,
                        push_device::Column::DeviceId,
                        push_device::Column::RegisteredAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Keeps an Android phone's token, registered by `device` at `now`. The same token again
    /// takes the new device and time.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when `device` is not a device's id.
    pub async fn register_fcm(
        &self,
        token: &FcmToken,
        device: DeviceId,
        now: Timestamp,
    ) -> Result<(), DbError> {
        let row = fcm_device::ActiveModel {
            token: Set(token.as_str().to_owned()),
            device_id: Set(device.get()),
            registered_at: Set(millis(now)),
        };
        fcm_device::Entity::insert(row)
            .on_conflict(
                OnConflict::column(fcm_device::Column::Token)
                    .update_columns([
                        fcm_device::Column::DeviceId,
                        fcm_device::Column::RegisteredAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Every registered iPhone, the longest registered first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn apns_devices(&self) -> Result<Vec<ApnsDevice>, DbError> {
        let rows = push_device::Entity::find()
            .order_by_asc(push_device::Column::RegisteredAt)
            .order_by_asc(push_device::Column::Token)
            .all(&self.conn)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ApnsDevice {
                env: ApnsEnv::from_wire(Some(&row.env)),
                token: row.token,
            })
            .collect())
    }

    /// Every registered Android phone's token, the longest registered first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn fcm_devices(&self) -> Result<Vec<String>, DbError> {
        let rows = fcm_device::Entity::find()
            .order_by_asc(fcm_device::Column::RegisteredAt)
            .order_by_asc(fcm_device::Column::Token)
            .all(&self.conn)
            .await?;
        Ok(rows.into_iter().map(|row| row.token).collect())
    }

    /// Forgets an iPhone's token (Apple says it will never work again).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn forget_apns(&self, token: &str) -> Result<(), DbError> {
        push_device::Entity::delete_many()
            .filter(push_device::Column::Token.eq(token))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Forgets an Android phone's token (Google says it will never work again).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn forget_fcm(&self, token: &str) -> Result<(), DbError> {
        fcm_device::Entity::delete_many()
            .filter(fcm_device::Column::Token.eq(token))
            .exec(&self.conn)
            .await?;
        Ok(())
    }
}
