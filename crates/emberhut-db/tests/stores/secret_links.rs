//! One-time secret links against a real SQLite database (the `secret_links` table).

use crate::common::{at, database};
use emberhut_core::OsRandom;
use emberhut_core::secret_link::{LinkToken, SecretName};

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a bad fixture fails the test"
)]
fn names(names: &[&str]) -> Vec<SecretName> {
    names
        .iter()
        .map(|n| SecretName::parse(n).unwrap())
        .collect()
}

#[tokio::test]
async fn a_link_is_found_by_its_token_and_works_once() {
    let (_dir, db) = database().await;
    let links = db.secret_links();
    let token = LinkToken::generate(&OsRandom).unwrap();
    let id = links
        .insert(
            &token.hash(),
            &names(&["slack-token", "other-key"]),
            "To read the channels",
            at(0),
            at(60),
        )
        .await
        .unwrap();
    let found = links.find(&token.hash()).await.unwrap().unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.names, names(&["slack-token", "other-key"]));
    assert_eq!(found.why, "To read the channels");
    assert_eq!(found.expires_at, at(60));
    assert!(found.works_at(at(59)));

    let other = LinkToken::generate(&OsRandom).unwrap();
    assert_eq!(links.find(&other.hash()).await.unwrap(), None);

    assert!(links.take(id).await.unwrap());
    assert!(!links.take(id).await.unwrap(), "only one caller uses it up");
    assert_eq!(links.find(&token.hash()).await.unwrap(), None);
}

#[tokio::test]
async fn expired_links_are_deleted() {
    let (_dir, db) = database().await;
    let links = db.secret_links();
    let old = LinkToken::generate(&OsRandom).unwrap();
    let fresh = LinkToken::generate(&OsRandom).unwrap();
    let one = names(&["one"]);
    links
        .insert(&old.hash(), &one, "old", at(0), at(10))
        .await
        .unwrap();
    links
        .insert(&fresh.hash(), &one, "fresh", at(5), at(30))
        .await
        .unwrap();
    assert_eq!(
        links.delete_expired(at(10)).await.unwrap(),
        0,
        "it works until then"
    );
    assert_eq!(links.delete_expired(at(11)).await.unwrap(), 1);
    assert_eq!(links.find(&old.hash()).await.unwrap(), None);
    assert!(links.find(&fresh.hash()).await.unwrap().is_some());
}
