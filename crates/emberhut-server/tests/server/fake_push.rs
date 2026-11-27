//! A push service that records what it is asked to send, and answers as the test says.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use emberhut_core::push::{Alert, ApnsDevice};
use emberhut_server::services::push::{ApnsSender, Delivery, FcmSender, PushSenders};
use futures_util::future::BoxFuture;

/// One alert sent to one phone: the token, and the alert.
pub(crate) type Sent = (String, Alert);

/// A fake APNs and FCM: ready unless told otherwise, delivering unless a token is told to be
/// gone or to fail.
#[derive(Debug)]
pub(crate) struct FakePush {
    ready: Mutex<bool>,
    answers: Mutex<HashMap<String, Delivery>>,
    sent: Mutex<Vec<Sent>>,
}

impl Default for FakePush {
    fn default() -> Self {
        Self {
            ready: Mutex::new(true),
            answers: Mutex::default(),
            sent: Mutex::default(),
        }
    }
}

impl FakePush {
    /// A fake for iPhones and one for Android phones, and the senders an app takes.
    pub(crate) fn pair() -> (Arc<Self>, Arc<Self>, PushSenders) {
        let apns = Arc::new(Self::default());
        let fcm = Arc::new(Self::default());
        let senders = PushSenders {
            apns: Some(Arc::clone(&apns) as _),
            fcm: Some(Arc::clone(&fcm) as _),
        };
        (apns, fcm, senders)
    }

    /// Whether it says it is ready.
    pub(crate) fn set_ready(&self, ready: bool) {
        *self.ready.lock().unwrap() = ready;
    }

    /// Its answer for `token` from now on.
    pub(crate) fn answer(&self, token: &str, delivery: Delivery) {
        self.answers
            .lock()
            .unwrap()
            .insert(token.to_owned(), delivery);
    }

    /// What it was asked to send, in order.
    pub(crate) fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn take(&self, token: &str, alert: &Alert) -> Delivery {
        self.sent
            .lock()
            .unwrap()
            .push((token.to_owned(), alert.clone()));
        self.answers
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .unwrap_or(Delivery::Delivered)
    }
}

impl ApnsSender for FakePush {
    fn ready(&self) -> BoxFuture<'_, bool> {
        let ready = *self.ready.lock().unwrap();
        Box::pin(async move { ready })
    }

    fn send<'a>(&'a self, device: &'a ApnsDevice, alert: &'a Alert) -> BoxFuture<'a, Delivery> {
        let delivery = self.take(&device.token, alert);
        Box::pin(async move { delivery })
    }
}

impl FcmSender for FakePush {
    fn ready(&self) -> BoxFuture<'_, bool> {
        let ready = *self.ready.lock().unwrap();
        Box::pin(async move { ready })
    }

    fn send<'a>(&'a self, token: &'a str, alert: &'a Alert) -> BoxFuture<'a, Delivery> {
        let delivery = self.take(token, alert);
        Box::pin(async move { delivery })
    }
}
