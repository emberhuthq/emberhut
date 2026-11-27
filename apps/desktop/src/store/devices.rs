//! The signed-in devices, for Settings › Devices: how sign-in stands on the server, each
//! device with "this one" and when it was last seen, signing one out, and a pairing code for
//! another device.

use std::sync::Arc;

use emberhut_api::auth::{AuthInfo, AuthMode, Device, PairingCode};
use emberhut_api::ids::DeviceId;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::Timestamp;

use super::{Store, update_or_log};

/// How sign-in stands on the server: `Required`, `On, not required yet`, or `Not set up`.
#[must_use]
pub(crate) fn sign_in_word(auth: &AuthInfo) -> &'static str {
    match (auth.bound, &auth.mode) {
        (true, AuthMode::Enforce) => "Required",
        (true, _) => "On, not required yet",
        (false, _) => "Not set up",
    }
}

/// A device's line: its name, ` · this one` for this device.
#[must_use]
pub(crate) fn device_name(device: &Device, this_one: Option<DeviceId>) -> String {
    let name = device.name.as_deref().unwrap_or("A device");
    if this_one == Some(device.id) {
        format!("{name} · this one")
    } else {
        name.to_owned()
    }
}

/// When a device was last seen, as `Last seen 2 hours ago`.
#[must_use]
pub(crate) fn last_seen(device: &Device, now: Timestamp) -> Option<String> {
    let seen: Timestamp = device.seen.as_ref()?.as_str().parse().ok()?;
    Some(format!("Last seen {}", relative(seen, now)))
}

/// How long ago `at` was, in words: `just now`, `5 minutes ago`, `2 hours ago`, `yesterday`,
/// `3 days ago`, `4 weeks ago`.
#[must_use]
pub(crate) fn relative(at: Timestamp, now: Timestamp) -> String {
    let seconds = (now.as_second() - at.as_second()).max(0);
    let count = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => count(seconds / 60, "minute"),
        3600..86_400 => count(seconds / 3600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..1_209_600 => count(seconds / 86_400, "day"),
        _ => count(seconds / 604_800, "week"),
    }
}

/// A pairing code made here for another device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NewCode {
    /// On its way.
    Making,
    /// The code, and until when it works.
    Made(PairingCode),
}

impl Store {
    /// This device's row among the signed-in devices, once it signed in.
    #[must_use]
    pub(crate) fn this_device(&self) -> Option<DeviceId> {
        self.backend.device()
    }

    /// The device whose "Sign out" was pressed and waits to be confirmed, if one does.
    #[must_use]
    pub(crate) fn sign_out_asked(&self) -> Option<DeviceId> {
        self.sign_out_asked
    }

    /// "Sign out" pressed on `device`'s row: it asks to be confirmed first, as a device signed
    /// out has to pair again.
    pub(crate) fn ask_to_sign_out(&mut self, device: DeviceId, cx: &mut Context<'_, Self>) {
        self.sign_out_asked = Some(device);
        cx.notify();
    }

    /// The sign-out isn't confirmed after all.
    pub(crate) fn cancel_sign_out(&mut self, cx: &mut Context<'_, Self>) {
        if self.sign_out_asked.take().is_some() {
            cx.notify();
        }
    }

    /// Signs `device` (named `name`) out on the server, confirmed; the list follows with the
    /// next state. Signing out this device also ends its session here once the server agrees:
    /// the sign-in screen shows at once.
    pub(crate) fn sign_out_device(
        &mut self,
        device: DeviceId,
        name: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.sign_out_asked = None;
        let failure = format!("Couldn't sign out {name}");
        let this_one = self.this_device() == Some(device);
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        self.hide_action_problem(cx);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    backend.revoke_device(device)?;
                    if this_one && let Err(error) = backend.sign_out_here() {
                        log::warn!("signed out, but the session kept here stays: {error}");
                    }
                    Ok(())
                })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| match done {
                Ok(()) if this_one => store.signed_out(cx),
                Ok(()) => {}
                Err(error) => store.action_failed(&failure, &error, cx),
            });
        })
        .detach();
        cx.notify();
    }

    /// The pairing code made here for another device, if one was asked for.
    #[must_use]
    pub(crate) fn new_code(&self) -> Option<&NewCode> {
        self.new_code.as_ref()
    }

    /// Makes a one-time pairing code for another device to sign in with.
    pub(crate) fn make_pairing_code(&mut self, cx: &mut Context<'_, Self>) {
        if self.new_code == Some(NewCode::Making) {
            return;
        }
        self.new_code = Some(NewCode::Making);
        self.hide_action_problem(cx);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let made = cx
                .background_spawn(async move { backend.new_pairing_code() })
                .await;
            update_or_log(&this, cx, |store, cx| {
                match made {
                    Ok(code) => store.new_code = Some(NewCode::Made(code)),
                    Err(error) => {
                        store.new_code = None;
                        store.action_failed("Couldn't make a pairing code", &error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::RoomId;
    use gpui_kit::{Entity, TestAppContext};
    use serde_json::json;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::store::ThemeState;

    fn auth(mode: &str, bound: bool) -> AuthInfo {
        serde_json::from_value(json!({"mode": mode, "bound": bound, "devices": []})).unwrap()
    }

    fn device(id: i64, name: Option<&str>, seen: Option<&str>) -> Device {
        serde_json::from_value(
            json!({"id": id, "name": name, "model": null, "created": null, "seen": seen}),
        )
        .unwrap()
    }

    #[test]
    fn sign_in_reads_as_the_server_has_it() {
        assert_eq!(sign_in_word(&auth("enforce", true)), "Required");
        assert_eq!(sign_in_word(&auth("report", true)), "On, not required yet");
        assert_eq!(sign_in_word(&auth("report", false)), "Not set up");
    }

    #[test]
    fn this_device_is_marked_and_each_says_when_it_was_seen() {
        let now: Timestamp = "2026-01-10T12:00:00Z".parse().unwrap();
        let mac = device(3, Some("Example Mac"), Some("2026-01-10T10:00:00Z"));
        assert_eq!(
            device_name(&mac, Some(DeviceId::new(3))),
            "Example Mac · this one"
        );
        assert_eq!(device_name(&mac, Some(DeviceId::new(4))), "Example Mac");
        assert_eq!(device_name(&device(5, None, None), None), "A device");
        assert_eq!(
            last_seen(&mac, now).as_deref(),
            Some("Last seen 2 hours ago")
        );
        assert_eq!(last_seen(&device(5, None, None), now), None);
    }

    #[test]
    fn how_long_ago_reads_in_words() {
        let now: Timestamp = "2026-01-10T12:00:00Z".parse().unwrap();
        let ago = |at: &str| relative(at.parse().unwrap(), now);
        assert_eq!(ago("2026-01-10T11:59:30Z"), "just now");
        assert_eq!(ago("2026-01-10T11:59:00Z"), "1 minute ago");
        assert_eq!(ago("2026-01-10T11:00:00Z"), "1 hour ago");
        assert_eq!(ago("2026-01-09T11:00:00Z"), "yesterday");
        assert_eq!(ago("2026-01-07T12:00:00Z"), "3 days ago");
        assert_eq!(ago("2025-12-13T12:00:00Z"), "4 weeks ago");
    }

    fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
        let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
    }

    #[gpui_kit::test]
    fn signing_a_device_out_goes_to_the_server_and_a_refusal_shows(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).signed_in_as(DeviceId::new(3)));
        backend.push_action(Err(Error::Refused {
            status: Some(400),
            message: Some("No such device".into()),
        }));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            assert_eq!(store.this_device(), Some(DeviceId::new(3)));
            store.sign_out_device(DeviceId::new(7), "Example phone", cx);
        });
        cx.run_until_parked();
        assert!(backend.calls().contains(&Call::Revoke(DeviceId::new(7))));
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.action_problem(),
                Some("Couldn't sign out Example phone: No such device")
            );
        });
    }

    #[gpui_kit::test]
    fn signing_a_device_out_waits_to_be_confirmed(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.ask_to_sign_out(DeviceId::new(7), cx);
            assert_eq!(store.sign_out_asked(), Some(DeviceId::new(7)));
            store.cancel_sign_out(cx);
            assert_eq!(store.sign_out_asked(), None);
        });
        cx.run_until_parked();
        assert!(backend.calls().is_empty(), "nothing sent");
    }

    #[gpui_kit::test]
    fn signing_this_device_out_ends_its_session_here_at_once(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).signed_in_as(DeviceId::new(3)));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
            store.ask_to_sign_out(DeviceId::new(3), cx);
            store.sign_out_device(DeviceId::new(3), "Example Mac", cx);
            assert_eq!(store.sign_out_asked(), None);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [Call::Revoke(DeviceId::new(3)), Call::SignOutHere],
            "signed out there, then here"
        );
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.gate(),
                crate::store::gate::Gate::SignIn { can_wait: false }
            );
        });
    }

    #[gpui_kit::test]
    fn signing_another_device_out_leaves_this_one_signed_in(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).signed_in_as(DeviceId::new(3)));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
            store.sign_out_device(DeviceId::new(7), "Example phone", cx);
        });
        cx.run_until_parked();
        assert_eq!(backend.calls(), [Call::Revoke(DeviceId::new(7))]);
        store.read_with(cx, |store, _| {
            assert_eq!(store.gate(), crate::store::gate::Gate::Open);
        });
    }

    #[gpui_kit::test]
    fn a_pairing_code_is_made_for_another_device(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.make_pairing_code(cx);
            assert_eq!(store.new_code(), Some(&NewCode::Making));
            store.make_pairing_code(cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend
                .calls()
                .iter()
                .filter(|call| **call == Call::NewPairingCode)
                .count(),
            1,
            "one at a time"
        );
        store.read_with(cx, |store, _| {
            assert!(
                matches!(store.new_code(), Some(NewCode::Made(code)) if code.code == "ABCD-EFGH")
            );
        });
    }
}
