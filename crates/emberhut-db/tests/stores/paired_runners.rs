//! Paired runners and the codes they pair with, against a real SQLite database.

use crate::common::{at, database, reopen};
use emberhut_core::pairing::{CODE_LIFETIME, PairingCode};
use emberhut_core::{OsRandom, SecretHash};
use emberhut_db::{RunnerPairing, RunnerRedemption};

/// The names `build` may be paired under, as the server tries them.
fn names() -> Vec<String> {
    ["build", "build-2", "build-3"].map(str::to_owned).to_vec()
}

fn build(url: &str, token: &str) -> RunnerPairing {
    RunnerPairing {
        name: "build".to_owned(),
        url: url.to_owned(),
        words: Some("Build box".to_owned()),
        token: token.to_owned(),
        platform: Some("linux".to_owned()),
        version: Some("1.4.0".to_owned()),
    }
}

#[tokio::test]
async fn a_code_pairs_one_runner_once_and_the_runner_is_kept() {
    let (dir, db) = database().await;
    let store = db.paired_runners();
    let code = PairingCode::generate(&OsRandom).unwrap();
    let id = store
        .insert_code(&code.hash(), at(0), at(0) + CODE_LIFETIME)
        .await
        .unwrap();
    let record = store.find_code(&code.hash()).await.unwrap().unwrap();
    assert_eq!(
        (record.id, record.used, record.expires_at),
        (id, false, at(10))
    );
    assert_eq!(
        db.pairing().find(&code.hash()).await.unwrap(),
        None,
        "a runner code is not a device code"
    );

    let first = build("https://build.example.com", "token-1");
    let RunnerRedemption::Paired(paired) = store.pair(id, &names(), &first, at(1)).await.unwrap()
    else {
        panic!("an unused code pairs");
    };
    assert_eq!(paired.pairing, first);
    assert_eq!((paired.created, paired.paired), (at(1), at(1)));
    assert!(
        !format!("{paired:?}").contains("token-1"),
        "Debug hides the token"
    );
    assert!(store.find_code(&code.hash()).await.unwrap().unwrap().used);
    assert_eq!(
        store.pair(id, &names(), &first, at(2)).await.unwrap(),
        RunnerRedemption::AlreadyUsed
    );

    // Kept across a restart.
    db.close().await.unwrap();
    let db = reopen(&dir).await;
    let listed = db.paired_runners().list().await.unwrap();
    assert_eq!(listed, [paired]);
}

#[tokio::test]
async fn a_taken_name_pairs_the_next_free_one_and_the_first_runner_stays() {
    let (_dir, db) = database().await;
    let store = db.paired_runners();
    let first = store
        .insert_code(&SecretHash::of("AAAAAAAA"), at(0), at(10))
        .await
        .unwrap();
    store
        .pair(
            first,
            &names(),
            &build("https://old.example.com", "old"),
            at(1),
        )
        .await
        .unwrap();
    let second = store
        .insert_code(&SecretHash::of("BBBBBBBB"), at(2), at(12))
        .await
        .unwrap();
    let mut again = build("https://new.example.com", "new");
    again.words = None;
    let RunnerRedemption::Paired(paired) =
        store.pair(second, &names(), &again, at(3)).await.unwrap()
    else {
        panic!("a second code pairs under the next free name");
    };
    assert_eq!(paired.pairing.name, "build-2");
    assert_eq!(paired.pairing.url, "https://new.example.com");
    assert_eq!(paired.created, at(3));
    let listed = store.list().await.unwrap();
    let urls: Vec<(&str, &str, &str)> = listed
        .iter()
        .map(|runner| {
            (
                runner.pairing.name.as_str(),
                runner.pairing.url.as_str(),
                runner.pairing.token.as_str(),
            )
        })
        .collect();
    assert_eq!(
        urls,
        [
            ("build", "https://old.example.com", "old"),
            ("build-2", "https://new.example.com", "new")
        ],
        "the first runner keeps its address and token"
    );
}

#[tokio::test]
async fn with_every_name_taken_the_code_stays_unused() {
    let (_dir, db) = database().await;
    let store = db.paired_runners();
    let only = vec!["build".to_owned()];
    let first = store
        .insert_code(&SecretHash::of("EEEEEEEE"), at(0), at(10))
        .await
        .unwrap();
    store
        .pair(
            first,
            &only,
            &build("https://build.example.com", "t"),
            at(1),
        )
        .await
        .unwrap();
    let hash = SecretHash::of("FFFFFFFF");
    let second = store.insert_code(&hash, at(0), at(10)).await.unwrap();
    assert_eq!(
        store
            .pair(
                second,
                &only,
                &build("https://other.example.com", "u"),
                at(2)
            )
            .await
            .unwrap(),
        RunnerRedemption::NoFreeName
    );
    assert!(!store.find_code(&hash).await.unwrap().unwrap().used);
    assert_eq!(store.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_expired_code_pairs_nothing_and_is_cleared() {
    let (_dir, db) = database().await;
    let store = db.paired_runners();
    let hash = SecretHash::of("CCCCCCCC");
    let id = store.insert_code(&hash, at(0), at(10)).await.unwrap();
    assert_eq!(
        store
            .pair(
                id,
                &names(),
                &build("https://build.example.com", "t"),
                at(10)
            )
            .await
            .unwrap(),
        RunnerRedemption::Expired
    );
    assert_eq!(store.list().await.unwrap(), Vec::new());
    assert_eq!(store.delete_expired_codes(at(9)).await.unwrap(), 0);
    assert_eq!(store.delete_expired_codes(at(10)).await.unwrap(), 1);
    assert_eq!(store.find_code(&hash).await.unwrap(), None);
}

#[tokio::test]
async fn a_forgotten_runner_is_gone() {
    let (_dir, db) = database().await;
    let store = db.paired_runners();
    let id = store
        .insert_code(&SecretHash::of("DDDDDDDD"), at(0), at(10))
        .await
        .unwrap();
    store
        .pair(
            id,
            &names(),
            &build("https://build.example.com", "t"),
            at(1),
        )
        .await
        .unwrap();
    assert!(store.forget("build").await.unwrap());
    assert!(!store.forget("build").await.unwrap());
    assert_eq!(store.list().await.unwrap(), Vec::new());
}
