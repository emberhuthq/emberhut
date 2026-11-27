//! Device sessions: access tokens expire, refresh tokens turn over once, idle devices are
//! signed out.

use emberhut_server::services::auth::AuthError;

use crate::common::{TestApp, device};

#[tokio::test]
async fn an_access_token_works_for_an_hour() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let session = auth.open_session(&device("laptop-1")).await.unwrap();
    let access = session.tokens.access.as_str();
    assert_eq!(
        auth.authenticate(access).await.unwrap(),
        Some(session.device)
    );
    assert_eq!(auth.authenticate("not a token").await.unwrap(), None);

    app.advance_minutes(59);
    assert!(auth.authenticate(access).await.unwrap().is_some());
    app.advance_minutes(1);
    assert_eq!(auth.authenticate(access).await.unwrap(), None);
}

#[tokio::test]
async fn a_refresh_token_turns_over_and_never_renews_after_the_grace() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let first = auth.open_session(&device("laptop-1")).await.unwrap();
    app.advance_minutes(61);

    let second = auth
        .refresh(first.tokens.refresh.as_str(), None)
        .await
        .unwrap();
    assert_eq!(second.device, first.device);
    assert_eq!(
        auth.authenticate(second.tokens.access.as_str())
            .await
            .unwrap(),
        Some(first.device)
    );

    app.advance_minutes(3);
    let reused = auth.refresh(first.tokens.refresh.as_str(), None).await;
    assert!(
        matches!(reused, Err(AuthError::RefreshRejected)),
        "{reused:?}"
    );
    let unknown = auth.refresh("never issued", None).await;
    assert!(
        matches!(unknown, Err(AuthError::RefreshRejected)),
        "{unknown:?}"
    );
}

/// Two refreshes with one token, on two worker threads. Whichever rotates first wins. The other
/// either finds the token already replaced (refused: the store's rotation is conditional on
/// the hash it read, inside one write transaction) or, reading after the rotation, takes it for
/// a retry within the grace and gets a fresh pair in place of the first. Either way exactly one
/// pair works afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_refreshes_with_one_token_leave_one_working_pair() {
    let app = TestApp::new().await;
    let auth = app.state.auth().clone();
    let first = auth.open_session(&device("laptop-1")).await.unwrap();
    let refresh = first.tokens.refresh.as_str().to_owned();
    let other = auth.clone();
    let token = refresh.clone();
    let racing = tokio::spawn(async move { other.refresh(&token, None).await });
    let a = auth.refresh(&refresh, None).await;
    let b = racing.await.unwrap();
    let mut working = 0;
    for session in [a, b].into_iter().flatten() {
        if auth
            .authenticate(session.tokens.access.as_str())
            .await
            .unwrap()
            .is_some()
        {
            working += 1;
        }
    }
    assert_eq!(working, 1);
}

#[tokio::test]
async fn the_old_access_token_works_until_its_own_expiry_after_a_refresh() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let first = auth.open_session(&device("laptop-1")).await.unwrap();
    app.advance_minutes(59);
    let second = auth
        .refresh(first.tokens.refresh.as_str(), None)
        .await
        .unwrap();
    assert_eq!(
        auth.authenticate(first.tokens.access.as_str())
            .await
            .unwrap(),
        Some(first.device),
        "a request sent just before the refresh still passes"
    );
    app.advance_minutes(1);
    assert_eq!(
        auth.authenticate(first.tokens.access.as_str())
            .await
            .unwrap(),
        None,
        "until the old token's own expiry"
    );
    assert!(
        auth.authenticate(second.tokens.access.as_str())
            .await
            .unwrap()
            .is_some()
    );
}

/// The client that lost the answer to its refresh still holds only the token it sent: within
/// the grace, sending it again gets a fresh pair, and the pair it never received stops working.
#[tokio::test]
async fn a_client_that_lost_the_refresh_answer_gets_a_fresh_pair_within_the_grace() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let first = auth.open_session(&device("laptop-1")).await.unwrap();
    let lost = auth
        .refresh(first.tokens.refresh.as_str(), None)
        .await
        .unwrap();
    app.advance_minutes(1);
    let retried = auth
        .refresh(first.tokens.refresh.as_str(), None)
        .await
        .unwrap();
    assert_eq!(retried.device, first.device);
    assert!(
        auth.authenticate(retried.tokens.access.as_str())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        auth.authenticate(lost.tokens.access.as_str())
            .await
            .unwrap(),
        None,
        "the pair it never received stops"
    );
    let unreceived = auth.refresh(lost.tokens.refresh.as_str(), None).await;
    assert!(matches!(unreceived, Err(AuthError::RefreshRejected)));
    assert!(
        auth.authenticate(first.tokens.access.as_str())
            .await
            .unwrap()
            .is_some(),
        "the access token it still holds works until its own expiry"
    );

    // The grace runs from the turnover, not from the re-issue.
    app.advance_minutes(2);
    let late = auth.refresh(first.tokens.refresh.as_str(), None).await;
    assert!(matches!(late, Err(AuthError::RefreshRejected)), "{late:?}");
    assert_eq!(
        auth.authenticate(retried.tokens.access.as_str())
            .await
            .unwrap(),
        None,
        "a reuse after the grace signs the device out"
    );
}

#[tokio::test]
async fn the_replaced_refresh_token_coming_back_later_signs_the_device_out() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let first = auth.open_session(&device("laptop-1")).await.unwrap();
    let second = auth
        .refresh(first.tokens.refresh.as_str(), None)
        .await
        .unwrap();
    app.advance_minutes(3);
    let stolen = auth.refresh(first.tokens.refresh.as_str(), None).await;
    assert!(
        matches!(stolen, Err(AuthError::RefreshRejected)),
        "{stolen:?}"
    );
    assert_eq!(
        auth.authenticate(second.tokens.access.as_str())
            .await
            .unwrap(),
        None,
        "every token of the device stops"
    );
    let current = auth.refresh(second.tokens.refresh.as_str(), None).await;
    assert!(matches!(current, Err(AuthError::RefreshRejected)));
    assert_eq!(auth.devices().await.unwrap(), []);
}

#[tokio::test]
async fn a_device_idle_for_two_weeks_is_signed_out() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let session = auth.open_session(&device("laptop-1")).await.unwrap();
    assert_eq!(auth.devices().await.unwrap().len(), 1);
    app.advance_minutes(14 * 24 * 60 + 1);
    assert!(
        auth.devices().await.unwrap().is_empty(),
        "an idle device is not listed"
    );
    let late = auth.refresh(session.tokens.refresh.as_str(), None).await;
    assert!(matches!(late, Err(AuthError::RefreshRejected)), "{late:?}");

    // Signed out, not only refused: its session row is gone.
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let row = db
        .sessions()
        .find_by_refresh(&emberhut_core::SecretHash::of(
            session.tokens.refresh.as_str(),
        ))
        .await
        .unwrap();
    assert!(row.is_none(), "the session row is gone");
}

#[tokio::test]
async fn using_a_session_keeps_the_device_from_going_idle() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let mut session = auth.open_session(&device("laptop-1")).await.unwrap();
    for _ in 0..20 {
        app.advance_minutes(24 * 60);
        session = auth
            .refresh(session.tokens.refresh.as_str(), None)
            .await
            .unwrap();
        assert!(
            auth.authenticate(session.tokens.access.as_str())
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn signing_out_stops_the_session_at_once() {
    let app = TestApp::new().await;
    let auth = app.state.auth();
    let session = auth.open_session(&device("laptop-1")).await.unwrap();
    auth.sign_out(session.device).await.unwrap();
    assert_eq!(
        auth.authenticate(session.tokens.access.as_str())
            .await
            .unwrap(),
        None
    );
    let refreshed = auth.refresh(session.tokens.refresh.as_str(), None).await;
    assert!(matches!(refreshed, Err(AuthError::RefreshRejected)));
}
