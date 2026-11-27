//! The phones' push addresses against a real SQLite database: the `push_devices` table (the
//! iPhones' APNs tokens) and the `fcm_devices` table (the Android phones' FCM tokens).

use crate::common::{at, database, tokens};
use emberhut_core::push::{ApnsDevice, ApnsEnv, ApnsToken, FcmToken};
use emberhut_core::{DeviceDetails, DeviceId};
use emberhut_db::Database;

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a bad fixture fails the test"
)]
fn apns(digit: char) -> ApnsToken {
    ApnsToken::parse(&digit.to_string().repeat(64)).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a bad fixture fails the test"
)]
fn fcm(name: &str) -> FcmToken {
    FcmToken::parse(&format!("{name}:{}", "x".repeat(40))).unwrap()
}

/// A new signed-in device, `name`, for push rows to belong to.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a bad fixture fails the test"
)]
async fn phone(db: &Database, name: &str) -> DeviceId {
    let details = DeviceDetails::new(Some(name), Some("Model 1"), None);
    db.sessions()
        .open(&details, &tokens(0), at(0))
        .await
        .unwrap()
}

#[tokio::test]
async fn an_iphone_registering_again_keeps_one_row_with_its_new_service() {
    let (_dir, db) = database().await;
    let push = db.push();
    let owner = phone(&db, "Phone").await;
    assert_eq!(push.apns_devices().await.unwrap(), Vec::<ApnsDevice>::new());
    push.register_apns(&apns('a'), ApnsEnv::Sandbox, owner, at(0))
        .await
        .unwrap();
    push.register_apns(&apns('b'), ApnsEnv::Production, owner, at(1))
        .await
        .unwrap();
    push.register_apns(&apns('a'), ApnsEnv::Production, owner, at(2))
        .await
        .unwrap();
    assert_eq!(
        push.apns_devices().await.unwrap(),
        [
            ApnsDevice {
                token: "b".repeat(64),
                env: ApnsEnv::Production
            },
            ApnsDevice {
                token: "a".repeat(64),
                env: ApnsEnv::Production
            },
        ]
    );
}

#[tokio::test]
async fn an_android_phone_registering_again_keeps_one_row() {
    let (_dir, db) = database().await;
    let push = db.push();
    let owner = phone(&db, "Phone").await;
    push.register_fcm(&fcm("one"), owner, at(0)).await.unwrap();
    push.register_fcm(&fcm("two"), owner, at(1)).await.unwrap();
    push.register_fcm(&fcm("one"), owner, at(2)).await.unwrap();
    assert_eq!(
        push.fcm_devices().await.unwrap(),
        [fcm("two").as_str(), fcm("one").as_str()]
    );
}

#[tokio::test]
async fn a_gone_token_is_forgotten_and_only_that_one() {
    let (_dir, db) = database().await;
    let push = db.push();
    let owner = phone(&db, "Phone").await;
    push.register_apns(&apns('a'), ApnsEnv::Sandbox, owner, at(0))
        .await
        .unwrap();
    push.register_apns(&apns('b'), ApnsEnv::Sandbox, owner, at(0))
        .await
        .unwrap();
    push.register_fcm(&fcm("one"), owner, at(0)).await.unwrap();
    push.register_fcm(&fcm("two"), owner, at(0)).await.unwrap();
    push.forget_apns(apns('a').as_str()).await.unwrap();
    push.forget_fcm(fcm("two").as_str()).await.unwrap();
    // Forgetting a token that isn't there is no error.
    push.forget_fcm("never-registered").await.unwrap();
    let iphones = push.apns_devices().await.unwrap();
    assert_eq!(iphones.len(), 1);
    assert_eq!(iphones[0].token, "b".repeat(64));
    assert_eq!(push.fcm_devices().await.unwrap(), [fcm("one").as_str()]);
}

#[tokio::test]
async fn signing_a_device_out_forgets_its_push_addresses_and_only_its() {
    let (_dir, db) = database().await;
    let push = db.push();
    let lost = phone(&db, "Lost").await;
    let kept = phone(&db, "Kept").await;
    push.register_apns(&apns('a'), ApnsEnv::Sandbox, lost, at(0))
        .await
        .unwrap();
    push.register_apns(&apns('b'), ApnsEnv::Sandbox, kept, at(0))
        .await
        .unwrap();
    push.register_fcm(&fcm("one"), lost, at(0)).await.unwrap();
    push.register_fcm(&fcm("two"), kept, at(0)).await.unwrap();

    db.sessions().sign_out(lost, at(1)).await.unwrap();
    let iphones = push.apns_devices().await.unwrap();
    assert_eq!(iphones.len(), 1);
    assert_eq!(iphones[0].token, "b".repeat(64));
    assert_eq!(push.fcm_devices().await.unwrap(), [fcm("two").as_str()]);
}

#[tokio::test]
async fn a_push_address_needs_a_device() {
    let (_dir, db) = database().await;
    assert!(
        db.push()
            .register_fcm(&fcm("one"), DeviceId::new(404), at(0))
            .await
            .is_err(),
        "the foreign key refuses an id no device has"
    );
}
