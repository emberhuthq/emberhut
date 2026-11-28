//! Push alerts to the phones: iPhones through Apple's push service ([`apns`]), Android phones
//! through Firebase Cloud Messaging ([`fcm`]).
//!
//! A signed-in phone registers where to push (`POST /api/devices`), and each of the
//! assistant's replies then reaches every registered phone as an alert: titled with its room, grouped by it, and
//! opening it when tapped. A reply in a room that is on screen right now ([`Presence`]) is not
//! pushed: it is being read already. A token the push service says will never work again is
//! forgotten, and so are a device's tokens when it is signed out.
//!
//! The services themselves are behind [`ApnsSender`] and [`FcmSender`]: the real ones speak
//! HTTP to Apple and Google with the keys in the server's secrets directory, and tests use
//! fakes. Without a sender's keys that kind of phone gets no pushes, and the state's `push`
//! says whether any sender is ready.

use std::fmt;
use std::sync::Arc;

use emberhut_api::push::PushRegistration;
use emberhut_core::push::{Alert, ApnsDevice, ApnsEnv, ApnsToken, FcmToken, PushTokenError};
use emberhut_core::{Clock, DeviceId, RoomId};
use emberhut_db::{DbError, FamilyStore, PushStore, RoomStore, SettingStore};
use futures_util::future::BoxFuture;

use crate::services::family::{FAMILY_ROOMS, NOT_AVAILABLE};
use crate::services::presence::Presence;

pub mod apns;
pub mod fcm;
mod http;
pub mod keys;

/// What a push service said about one alert to one phone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// It took the alert.
    Delivered,
    /// The token will never work again (the app is gone, or the token isn't this app's): it
    /// is forgotten.
    Gone,
    /// It did not take the alert this time, for this reason (logged; the token is kept).
    Failed(String),
}

/// Sends alerts to iPhones.
pub trait ApnsSender: fmt::Debug + Send + Sync {
    /// Whether it can send at all: its key and settings are in place.
    fn ready(&self) -> BoxFuture<'_, bool>;

    /// Sends `alert` to `device`.
    fn send<'a>(&'a self, device: &'a ApnsDevice, alert: &'a Alert) -> BoxFuture<'a, Delivery>;
}

/// Sends alerts to Android phones.
pub trait FcmSender: fmt::Debug + Send + Sync {
    /// Whether it can send at all: its service account is in place.
    fn ready(&self) -> BoxFuture<'_, bool>;

    /// Sends `alert` to the phone with this registration token.
    fn send<'a>(&'a self, token: &'a str, alert: &'a Alert) -> BoxFuture<'a, Delivery>;
}

/// The push services the server has: either, both or none.
#[derive(Clone, Debug, Default)]
pub struct PushSenders {
    /// iPhones' pushes, when configured.
    pub apns: Option<Arc<dyn ApnsSender>>,
    /// Android phones' pushes, when configured.
    pub fcm: Option<Arc<dyn FcmSender>>,
}

/// Why a phone's push address was not taken.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PushError {
    /// The token is not one (400, with this message).
    #[error(transparent)]
    Token(#[from] PushTokenError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// A push service, as a delivery's log lines name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    /// Apple's, for iPhones.
    Apns,
    /// Google's, for Android phones.
    Fcm,
}

impl Service {
    /// Its name in the log.
    const fn name(self) -> &'static str {
        match self {
            Self::Apns => "APNs",
            Self::Fcm => "FCM",
        }
    }
}

/// How one round of pushes went: how many phones took the alert, and how many tokens were
/// forgotten.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushReport {
    /// Phones that took it.
    pub delivered: usize,
    /// Tokens forgotten because they will never work again.
    pub forgotten: usize,
    /// Phones that did not take it this time.
    pub failed: usize,
}

/// Registers phones and pushes alerts to them.
#[derive(Clone, Debug)]
pub struct PushService {
    store: PushStore,
    rooms: RoomStore,
    clock: Arc<dyn Clock>,
    presence: Presence,
    senders: PushSenders,
    /// The family members' rooms and phones: a reply in one of their rooms goes to their
    /// phones, never the owner's.
    family: Option<FamilyStore>,
    /// Where the owner's name for their ember is kept: their replies' alerts carry it.
    names: Option<SettingStore>,
}

impl PushService {
    /// A service over the phones' addresses in `store`, titling alerts with the rooms in
    /// `rooms`, holding back the rooms `presence` says are on screen, and sending through
    /// `senders`.
    #[must_use]
    pub fn new(
        store: PushStore,
        rooms: RoomStore,
        clock: Arc<dyn Clock>,
        presence: Presence,
        senders: PushSenders,
    ) -> Self {
        Self {
            store,
            rooms,
            clock,
            presence,
            senders,
            family: None,
            names: None,
        }
    }

    /// The same service, telling a family member's phones (in `family`) about the replies in
    /// their own rooms instead of the owner's.
    #[must_use]
    pub fn with_family(mut self, family: FamilyStore) -> Self {
        self.family = Some(family);
        self
    }

    /// The same service, titling the owner's reply alerts with the name they gave their ember
    /// (kept in `settings`) in place of "Emberhut".
    #[must_use]
    pub fn with_names(mut self, settings: SettingStore) -> Self {
        self.names = Some(settings);
        self
    }

    /// The owner's name for their ember, when they gave one.
    async fn ember_name(&self) -> Option<String> {
        let settings = self.names.as_ref()?;
        match crate::services::ember_name::custom_name(settings).await {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(%error, "no ember name for the push");
                None
            }
        }
    }

    /// Keeps a phone's push address, as `device`'s: an Android phone's FCM token when its
    /// `platform` is `android`, an iPhone's APNs token (for the sandbox or production service)
    /// otherwise. It goes when `device` is signed out.
    ///
    /// # Errors
    ///
    /// [`PushError::Token`] when the token is not one, [`PushError::Database`].
    pub async fn register(
        &self,
        registration: &PushRegistration,
        device: DeviceId,
    ) -> Result<(), PushError> {
        let now = self.clock.now();
        if registration.is_android() {
            let token = FcmToken::parse(&registration.token)?;
            self.store.register_fcm(&token, device, now).await?;
            tracing::info!("an Android phone registered for pushes");
        } else {
            let token = ApnsToken::parse(&registration.token)?;
            let env = ApnsEnv::from_wire(registration.env.as_deref());
            self.store.register_apns(&token, env, device, now).await?;
            tracing::info!(env = env.as_str(), "an iPhone registered for pushes");
        }
        Ok(())
    }

    /// Whether any push service is ready: the state's `push`.
    pub async fn ready(&self) -> bool {
        if let Some(apns) = &self.senders.apns
            && apns.ready().await
        {
            return true;
        }
        match &self.senders.fcm {
            Some(fcm) => fcm.ready().await,
            None => false,
        }
    }

    /// Tells the phones about the assistant's `reply` in `room`, unless the room is on screen
    /// (then it is being read already) or the reply has no text to show. Never fails: a push
    /// that doesn't go out is logged, and the reply is in the chat either way.
    pub async fn reply(&self, room: RoomId, reply: &str) -> PushReport {
        if self.presence.is_looked_at(room) {
            tracing::debug!(%room, "no push: the room is on screen");
            return PushReport::default();
        }
        let name = match self.rooms.get(room).await {
            Ok(found) => found.map(|room| room.name),
            Err(error) => {
                tracing::warn!(%room, %error, "no room name for the push");
                None
            }
        };
        let alert = Alert::reply(room, name.as_deref(), reply);
        if alert.body().is_empty() {
            return PushReport::default();
        }
        if let Some(family) = &self.family {
            match family.person_of(room).await {
                Ok(Some(_)) if !FAMILY_ROOMS => {
                    tracing::warn!(%room, "no push: {NOT_AVAILABLE}");
                    return PushReport::default();
                }
                // A family member's own rooms keep "Emberhut": the name is the owner's.
                Ok(Some(person)) => return self.push_family(family, &person, &alert).await,
                Ok(None) => {}
                Err(error) => {
                    // Not knowing whose room it is, nobody is told rather than the wrong person.
                    tracing::error!(%room, %error, "could not read whose room a reply is in");
                    return PushReport::default();
                }
            }
        }
        let alert = match self.ember_name().await {
            Some(from) => Alert::reply_from(Some(&from), room, name.as_deref(), reply),
            None => alert,
        };
        self.push(&alert).await
    }

    /// Sends `alert` to `person`'s own iPhones (a family member's phones are iPhones).
    async fn push_family(&self, family: &FamilyStore, person: &str, alert: &Alert) -> PushReport {
        let mut report = PushReport::default();
        let Some(apns) = self.senders.apns.as_ref() else {
            return report;
        };
        if !apns.ready().await {
            return report;
        }
        let devices = match family.devices_of(person).await {
            Ok(devices) => devices,
            Err(error) => {
                tracing::error!(%error, "could not read a family member's phones to push to");
                return report;
            }
        };
        for (token, env) in devices {
            let device = ApnsDevice {
                token,
                env: ApnsEnv::from_wire(Some(&env)),
            };
            match apns.send(&device, alert).await {
                Delivery::Delivered => report.delivered += 1,
                Delivery::Gone | Delivery::Failed(_) => {
                    report.failed += 1;
                    tracing::warn!("a push to a family member's phone did not go out");
                }
            }
        }
        report
    }

    /// Sends `alert` to every registered phone of each ready service, and forgets the tokens
    /// a service says are gone. Never fails: what doesn't go out is logged.
    pub async fn push(&self, alert: &Alert) -> PushReport {
        let mut report = PushReport::default();
        if let Some(apns) = &self.senders.apns
            && apns.ready().await
        {
            match self.store.apns_devices().await {
                Ok(devices) => {
                    for device in &devices {
                        let delivery = apns.send(device, alert).await;
                        self.settle(&mut report, Service::Apns, delivery, &device.token)
                            .await;
                    }
                }
                Err(error) => tracing::error!(%error, "could not read the iPhones to push to"),
            }
        }
        if let Some(fcm) = &self.senders.fcm
            && fcm.ready().await
        {
            match self.store.fcm_devices().await {
                Ok(tokens) => {
                    for token in &tokens {
                        let delivery = fcm.send(token, alert).await;
                        self.settle(&mut report, Service::Fcm, delivery, token)
                            .await;
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "could not read the Android phones to push to");
                }
            }
        }
        report
    }

    /// Counts one delivery, forgetting the token when it is gone.
    async fn settle(
        &self,
        report: &mut PushReport,
        service: Service,
        delivery: Delivery,
        token: &str,
    ) {
        let name = service.name();
        match delivery {
            Delivery::Delivered => report.delivered += 1,
            Delivery::Gone => {
                let forgotten = match service {
                    Service::Apns => self.store.forget_apns(token).await,
                    Service::Fcm => self.store.forget_fcm(token).await,
                };
                match forgotten {
                    Ok(()) => {
                        report.forgotten += 1;
                        tracing::info!(service = name, "forgot a push token that no longer works");
                    }
                    Err(error) => {
                        tracing::error!(service = name, %error, "could not forget a token");
                    }
                }
            }
            Delivery::Failed(reason) => {
                report.failed += 1;
                tracing::warn!(service = name, reason, "a push did not go out");
            }
        }
    }
}
