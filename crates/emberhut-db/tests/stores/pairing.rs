//! Pairing codes against a real SQLite database.

use crate::common::{at, database, tokens};
use emberhut_core::pairing::{CODE_LIFETIME, PairingCode};
use emberhut_core::{DeviceDetails, OsRandom, SecretHash};
use emberhut_db::Redemption;

fn new_device() -> DeviceDetails {
    DeviceDetails::new(Some("Phone"), Some("Model 2"), Some("phone-1"))
}

#[tokio::test]
async fn a_stored_code_is_found_by_its_hash() {
    let (_dir, db) = database().await;
    let pairing = db.pairing();
    let code = PairingCode::generate(&OsRandom).unwrap();
    let id = pairing
        .insert(&code.hash(), at(0), at(0) + CODE_LIFETIME)
        .await
        .unwrap();

    let record = pairing.find(&code.hash()).await.unwrap().unwrap();
    assert_eq!(record.id, id);
    assert!(!record.used);
    assert_eq!(record.expires_at, at(10));
    assert_eq!(
        pairing.find(&SecretHash::of("ABCDEFGH")).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn a_code_pairs_one_device_once() {
    let (_dir, db) = database().await;
    let pairing = db.pairing();
    let code = PairingCode::generate(&OsRandom).unwrap();
    let id = pairing.insert(&code.hash(), at(0), at(10)).await.unwrap();

    let issued = tokens(1);
    let Redemption::Paired(device) = pairing
        .redeem(id, &new_device(), &issued, at(1))
        .await
        .unwrap()
    else {
        panic!("an unused code pairs");
    };
    let session = db
        .sessions()
        .find_by_access(&issued.access.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.device, device);
    assert!(pairing.find(&code.hash()).await.unwrap().unwrap().used);

    let second = tokens(2);
    assert_eq!(
        pairing
            .redeem(id, &new_device(), &second, at(2))
            .await
            .unwrap(),
        Redemption::AlreadyUsed
    );
    assert!(
        db.sessions()
            .find_by_access(&second.access.hash())
            .await
            .unwrap()
            .is_none(),
        "a refused code opens no session"
    );
}

#[tokio::test]
async fn an_expired_code_pairs_nothing() {
    let (_dir, db) = database().await;
    let pairing = db.pairing();
    let code = PairingCode::generate(&OsRandom).unwrap();
    let id = pairing.insert(&code.hash(), at(0), at(10)).await.unwrap();
    let issued = tokens(10);
    assert_eq!(
        pairing
            .redeem(id, &new_device(), &issued, at(10))
            .await
            .unwrap(),
        Redemption::Expired
    );
}

#[tokio::test]
async fn expired_codes_are_cleared_out() {
    let (_dir, db) = database().await;
    let pairing = db.pairing();
    let old = PairingCode::generate(&OsRandom).unwrap();
    let fresh = PairingCode::generate(&OsRandom).unwrap();
    pairing.insert(&old.hash(), at(0), at(10)).await.unwrap();
    pairing.insert(&fresh.hash(), at(5), at(15)).await.unwrap();

    assert_eq!(pairing.delete_expired(at(10)).await.unwrap(), 1);
    assert_eq!(pairing.find(&old.hash()).await.unwrap(), None);
    assert!(pairing.find(&fresh.hash()).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_code_redeemed_at_once_by_many_pairs_exactly_one_device() {
    let (_dir, db) = database().await;
    let code = PairingCode::generate(&OsRandom).unwrap();
    let id = db
        .pairing()
        .insert(&code.hash(), at(0), at(10))
        .await
        .unwrap();
    let mut tries = tokio::task::JoinSet::new();
    for n in 0..8 {
        let pairing = db.pairing();
        tries.spawn(async move {
            let device = DeviceDetails::new(Some("Phone"), None, Some(&format!("phone-{n}")));
            pairing.redeem(id, &device, &tokens(1), at(1)).await
        });
    }
    let mut paired = 0;
    while let Some(result) = tries.join_next().await {
        match result.unwrap().unwrap() {
            Redemption::Paired(_) => paired += 1,
            other => assert_eq!(other, Redemption::AlreadyUsed),
        }
    }
    assert_eq!(paired, 1);
}
