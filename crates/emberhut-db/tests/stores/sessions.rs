//! Devices and sessions against a real SQLite database.

use crate::common::{at, database, reopen, tokens};
use emberhut_core::session::RefreshMatch;
use emberhut_core::{DeviceDetails, SecretHash};

fn device(install: Option<&str>) -> DeviceDetails {
    DeviceDetails::new(Some("Laptop"), Some("Model 1"), install)
}

#[tokio::test]
async fn an_opened_session_is_found_by_either_token() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let issued = tokens(0);
    let id = sessions.open(&device(None), &issued, at(0)).await.unwrap();

    let access = sessions
        .find_by_access(&issued.access.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(access.device, id);
    assert_eq!(access.expires_at, issued.access_expires_at);
    assert_eq!(access.device_seen_at, at(0));

    let refresh = sessions
        .find_by_refresh(&issued.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refresh.device, id);
    assert_eq!(refresh.device_install, None);

    assert_eq!(
        sessions
            .find_by_access(&SecretHash::of("not a token"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        sessions
            .find_by_access(&issued.refresh.hash())
            .await
            .unwrap(),
        None,
        "a refresh token is not an access token"
    );
}

#[tokio::test]
async fn rotating_replaces_both_tokens_once_and_keeps_the_old_access_token_until_it_expires() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let first = tokens(0);
    let id = sessions.open(&device(None), &first, at(0)).await.unwrap();

    let second = tokens(30);
    let rotated = sessions
        .rotate(id, &first.refresh.hash(), &second, None, at(30))
        .await
        .unwrap();
    assert!(rotated);
    let previous = sessions
        .find_by_access(&first.access.hash())
        .await
        .unwrap()
        .expect("the previous access token is still known");
    assert_eq!(previous.device, id);
    assert_eq!(
        previous.expires_at, first.access_expires_at,
        "with its own expiry"
    );
    let reused = sessions
        .find_by_refresh(&first.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    match reused.matched {
        RefreshMatch::Previous {
            rotated_at,
            current,
        } => {
            assert_eq!(rotated_at, at(30));
            assert_eq!(current.as_bytes(), second.refresh.hash().as_bytes());
        }
        RefreshMatch::Current => panic!("the replaced token is the previous one"),
    }
    let current = sessions
        .find_by_access(&second.access.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.device_seen_at, at(30));

    let again = sessions
        .rotate(id, &first.refresh.hash(), &tokens(31), None, at(31))
        .await
        .unwrap();
    assert!(!again, "an old refresh token never works twice");

    let third = tokens(40);
    assert!(
        sessions
            .rotate(id, &second.refresh.hash(), &third, None, at(40))
            .await
            .unwrap()
    );
    assert!(
        sessions
            .find_by_access(&first.access.hash())
            .await
            .unwrap()
            .is_none(),
        "only the one previous pair is kept"
    );
    assert!(
        sessions
            .find_by_access(&second.access.hash())
            .await
            .unwrap()
            .is_some(),
        "a refused rotation changes nothing"
    );
}

#[tokio::test]
async fn rotating_claims_an_install_id_no_other_device_has() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let other = tokens(0);
    sessions
        .open(&device(Some("taken")), &other, at(0))
        .await
        .unwrap();
    let first = tokens(0);
    let id = sessions.open(&device(None), &first, at(0)).await.unwrap();

    let second = tokens(1);
    assert!(
        sessions
            .rotate(id, &first.refresh.hash(), &second, Some("taken"), at(1))
            .await
            .unwrap()
    );
    let record = sessions
        .find_by_refresh(&second.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.device_install, None, "another device has that one");

    let third = tokens(2);
    assert!(
        sessions
            .rotate(id, &second.refresh.hash(), &third, Some("mine"), at(2))
            .await
            .unwrap()
    );
    let record = sessions
        .find_by_refresh(&third.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.device_install.as_deref(), Some("mine"));
}

#[tokio::test]
async fn the_same_install_signing_in_again_keeps_its_row() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let first = tokens(0);
    let id = sessions
        .open(&device(Some("install-1")), &first, at(0))
        .await
        .unwrap();
    sessions.sign_out(id, at(1)).await.unwrap();

    let second = tokens(2);
    let again = sessions
        .open(&device(Some("install-1")), &second, at(2))
        .await
        .unwrap();
    assert_eq!(again, id);
    assert!(
        sessions
            .find_by_access(&first.access.hash())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .find_by_access(&second.access.hash())
            .await
            .unwrap()
            .is_some(),
        "signing in again undoes the sign-out"
    );

    let elsewhere = sessions
        .open(&device(Some("install-2")), &tokens(3), at(3))
        .await
        .unwrap();
    assert_ne!(elsewhere, id);
}

#[tokio::test]
async fn signing_out_stops_both_tokens() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let issued = tokens(0);
    let id = sessions.open(&device(None), &issued, at(0)).await.unwrap();
    sessions.sign_out(id, at(1)).await.unwrap();
    assert!(
        sessions
            .find_by_access(&issued.access.hash())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .find_by_refresh(&issued.refresh.hash())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn marking_seen_moves_the_last_seen_time() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let issued = tokens(0);
    let id = sessions.open(&device(None), &issued, at(0)).await.unwrap();
    sessions.mark_seen(id, at(5)).await.unwrap();
    let record = sessions
        .find_by_access(&issued.access.hash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.device_seen_at, at(5));
}

#[tokio::test]
async fn signing_out_stops_the_previous_access_token_too() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let first = tokens(0);
    let id = sessions.open(&device(None), &first, at(0)).await.unwrap();
    assert!(
        sessions
            .rotate(id, &first.refresh.hash(), &tokens(30), None, at(30))
            .await
            .unwrap()
    );
    sessions.sign_out(id, at(31)).await.unwrap();
    assert!(
        sessions
            .find_by_access(&first.access.hash())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_device_list_leaves_out_signed_out_and_idle_devices() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let old = sessions
        .open(&device(Some("old")), &tokens(0), at(0))
        .await
        .unwrap();
    let gone = sessions
        .open(&device(Some("gone")), &tokens(0), at(0))
        .await
        .unwrap();
    let recent = sessions
        .open(&device(Some("recent")), &tokens(0), at(0))
        .await
        .unwrap();
    sessions.sign_out(gone, at(1)).await.unwrap();
    let two_weeks = 14 * 24 * 60;
    sessions.mark_seen(recent, at(two_weeks)).await.unwrap();

    let listed = |devices: Vec<emberhut_core::session::DeviceSummary>| {
        devices.into_iter().map(|d| d.id).collect::<Vec<_>>()
    };
    assert_eq!(
        listed(sessions.signed_in(at(two_weeks)).await.unwrap()),
        [recent, old]
    );
    assert_eq!(
        listed(sessions.signed_in(at(two_weeks + 1)).await.unwrap()),
        [recent],
        "the device not seen for two weeks is idle"
    );
}

/// Two pools on one file, as a server and `emberhut-server pair` are: every write lands, none
/// fails with SQLite's "database is locked".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pools_write_one_file_without_busy_errors() {
    let (dir, first) = database().await;
    let second = reopen(&dir).await;
    let mut writers = tokio::task::JoinSet::new();
    for n in 0..20 {
        let db = if n % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        writers.spawn(async move {
            let install = format!("install-{n}");
            db.sessions()
                .open(&device(Some(&install)), &tokens(0), at(0))
                .await
        });
    }
    let mut opened = 0;
    while let Some(result) = writers.join_next().await {
        result.unwrap().unwrap();
        opened += 1;
    }
    assert_eq!(opened, 20);
    assert_eq!(first.sessions().signed_in(at(1)).await.unwrap().len(), 20);
}

#[tokio::test]
async fn reissuing_from_the_previous_token_replaces_only_the_current_pair() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let first = tokens(0);
    let id = sessions.open(&device(None), &first, at(0)).await.unwrap();
    let lost = tokens(1);
    assert!(
        sessions
            .rotate(id, &first.refresh.hash(), &lost, None, at(1))
            .await
            .unwrap()
    );
    let fresh = tokens(2);
    assert!(
        sessions
            .reissue(
                id,
                &first.refresh.hash(),
                &lost.refresh.hash(),
                &fresh,
                at(2)
            )
            .await
            .unwrap()
    );
    assert!(
        sessions
            .find_by_access(&lost.access.hash())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sessions
            .find_by_access(&fresh.access.hash())
            .await
            .unwrap()
            .is_some()
    );
    let previous = sessions
        .find_by_refresh(&first.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    let RefreshMatch::Previous { rotated_at, .. } = previous.matched else {
        panic!("the replaced token is the previous one");
    };
    assert_eq!(rotated_at, at(1), "the turnover time stays");
    assert!(
        !sessions
            .reissue(
                id,
                &lost.refresh.hash(),
                &fresh.refresh.hash(),
                &tokens(3),
                at(3)
            )
            .await
            .unwrap(),
        "only from the previous token"
    );
}

/// Two re-issues from one read of the session: the first replaces the pair it read, and the
/// second, whose pair is gone, is refused instead of handing out tokens that are already dead.
#[tokio::test]
async fn of_two_reissues_from_one_read_only_the_first_succeeds() {
    let (_dir, db) = database().await;
    let sessions = db.sessions();
    let first = tokens(0);
    let id = sessions.open(&device(None), &first, at(0)).await.unwrap();
    let lost = tokens(1);
    assert!(
        sessions
            .rotate(id, &first.refresh.hash(), &lost, None, at(1))
            .await
            .unwrap()
    );
    let read = sessions
        .find_by_refresh(&first.refresh.hash())
        .await
        .unwrap()
        .unwrap();
    let RefreshMatch::Previous { current, .. } = read.matched else {
        panic!("the replaced token is the previous one");
    };
    let a = tokens(2);
    let b = tokens(2);
    assert!(
        sessions
            .reissue(id, &first.refresh.hash(), &current, &a, at(2))
            .await
            .unwrap()
    );
    assert!(
        !sessions
            .reissue(id, &first.refresh.hash(), &current, &b, at(2))
            .await
            .unwrap()
    );
    assert!(
        sessions
            .find_by_access(&a.access.hash())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        sessions
            .find_by_access(&b.access.hash())
            .await
            .unwrap()
            .is_none()
    );
}
