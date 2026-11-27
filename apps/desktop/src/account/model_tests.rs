//! The account entity on GPUI's headless test platform, against a scripted provider and
//! directory: the device grant's sheet, its polls and their end, cancelling, and how the
//! directory's refusals change where sign-in stands.

use std::sync::Arc;
use std::time::Duration;

use emberhut_identity::SecretKey;
use gpui_kit::{Entity, TestAppContext};
use serde_json::json;

use super::*;
use crate::account::http::fake::FakeHttp;
use crate::account::http::{Http, Method};
use crate::account::oidc::discovery_url;
use crate::account::switch::{AccountSettings, BuildValues};
use crate::account::tokens::tests::{id_claims, jwt};
use crate::account::vault::ItemVault;
use crate::account::vault::tests::MemoryItem;
use crate::backend::fake::FakeBackend;

const ISSUER: &str = "https://accounts.example";
const TOKEN: &str = "https://accounts.example/oauth/token";
const DEVICE: &str = "https://accounts.example/oauth/device";
const MACHINES: &str = "https://relay.example/v1/machines";

fn settings() -> AccountSettings {
    AccountSettings::from_build(BuildValues {
        issuer: Some(ISSUER),
        client_id: Some("client-example"),
        directory: Some("https://relay.example"),
        relay_domain: Some("relay.example"),
        associated_domain: None,
    })
    .expect("settings")
}

fn provider() -> Arc<FakeHttp> {
    let http = Arc::new(FakeHttp::default());
    http.answer(
        Method::Get,
        &discovery_url(ISSUER),
        200,
        &json!({"issuer": ISSUER, "authorization_endpoint": "https://accounts.example/a",
            "token_endpoint": TOKEN, "device_authorization_endpoint": DEVICE})
        .to_string(),
    );
    http
}

fn granted(access: &str) -> String {
    json!({"access_token": access, "expires_in": 600, "refresh_token": "r1",
        "id_token": jwt(&id_claims("user_example"))})
    .to_string()
}

fn machines() -> String {
    let key = SecretKey::from_bytes(&[4; 32]);
    json!({"machines": [{"id": key.machine_id().to_string(),
        "publicKey": key.public_key().to_string(), "name": "Main computer",
        "relayUrl": "wss://relay.example/v1/sealed/x", "connected": false,
        "lastSeen": null, "linkedAt": 1_795_000_000_000_u64}]})
    .to_string()
}

/// The account over `http` and `item`, with a store over a fake server at `server`.
fn account(
    http: &Arc<FakeHttp>,
    item: &MemoryItem,
    server: &str,
    cx: &mut TestAppContext,
) -> (Entity<Account>, Arc<FakeBackend>) {
    let backend = Arc::new(FakeBackend::new([]).at(server));
    let shared = Arc::clone(&backend) as Arc<dyn Backend>;
    let store = cx.new(|_| Store::for_tests(Arc::clone(&shared)));
    let session = Arc::new(AccountSession::new(
        settings(),
        Arc::clone(http) as Arc<dyn Http>,
        Box::new(ItemVault::new(item.clone())),
    ));
    let account = cx.new(|cx| Account::new(session, shared, &store, cx));
    cx.run_until_parked();
    (account, backend)
}

fn start_device_grant(http: &FakeHttp) {
    http.answer(
        Method::Post,
        DEVICE,
        200,
        &json!({"device_code": "dc", "user_code": "WDJB-MJHT",
            "verification_uri": "https://accounts.example/device",
            "expires_in": 600, "interval": 5})
        .to_string(),
    );
}

fn phase(account: &Entity<Account>, cx: &TestAppContext) -> Phase {
    account.read_with(cx, |account, _| account.phase().clone())
}

#[gpui_kit::test]
fn the_device_grant_shows_its_code_and_signs_in_once_approved(cx: &mut TestAppContext) {
    let http = provider();
    let item = MemoryItem::default();
    let (account, _) = account(&http, &item, "http://127.0.0.1:8791", cx);
    assert_eq!(phase(&account, cx), Phase::SignedOut);

    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    assert_eq!(
        phase(&account, cx),
        Phase::SigningIn(SignIn::Code {
            user_code: "WDJB-MJHT".into(),
            page: "https://accounts.example/device".into()
        })
    );
    assert_eq!(http.sent_to(TOKEN), [], "no poll before the interval");

    http.answer(
        Method::Post,
        TOKEN,
        400,
        r#"{"error":"authorization_pending"}"#,
    );
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(http.sent_to(TOKEN).len(), 1);

    http.answer(Method::Post, TOKEN, 400, r#"{"error":"slow_down"}"#);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(http.sent_to(TOKEN).len(), 2);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(http.sent_to(TOKEN).len(), 2, "slower after slow_down");

    http.answer(Method::Post, TOKEN, 200, &granted("a1"));
    http.answer(Method::Get, MACHINES, 200, &machines());
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(phase(&account, cx), Phase::SignedIn);
    account.read_with(cx, |account, _| {
        assert_eq!(account.machines().map(<[_]>::len), Some(1));
    });
    assert!(item.held().is_some(), "kept in the Keychain item");
}

#[gpui_kit::test]
fn a_cancelled_sign_in_polls_no_more(cx: &mut TestAppContext) {
    let http = provider();
    let (account, _) = account(&http, &MemoryItem::default(), "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    account.update(cx, Account::cancel_sign_in);
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(phase(&account, cx), Phase::SignedOut);
    assert_eq!(http.sent_to(TOKEN), []);
}

#[gpui_kit::test]
fn a_declined_code_signs_nobody_in(cx: &mut TestAppContext) {
    let http = provider();
    let (account, _) = account(&http, &MemoryItem::default(), "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    http.answer(Method::Post, TOKEN, 400, r#"{"error":"access_denied"}"#);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(phase(&account, cx), Phase::SignedOut);
    account.read_with(cx, |account, _| {
        assert_eq!(
            account.note(),
            Some("Sign-in was declined on the sign-in page.")
        );
    });
}

/// An account signed in from the Keychain item, whose first list of machines `first` answers.
fn kept_account(
    http: &Arc<FakeHttp>,
    status: u16,
    body: &str,
    cx: &mut TestAppContext,
) -> (Entity<Account>, MemoryItem, Arc<FakeBackend>) {
    let item = MemoryItem::default();
    item.put(
        json!({"version": 1, "account": "user_example", "access": "a1", "refresh": "r1",
            "expiresAt": 4_000_000_000_u64})
        .to_string()
        .as_bytes(),
    );
    http.answer(Method::Get, MACHINES, status, body);
    let (account, backend) = account(http, &item, "http://127.0.0.1:8791", cx);
    (account, item, backend)
}

#[gpui_kit::test]
fn a_busy_directory_keeps_the_account_signed_in(cx: &mut TestAppContext) {
    let http = provider();
    let (account, item, _) = kept_account(
        &http,
        503,
        r#"{"error":"Try later.","code":"unavailable"}"#,
        cx,
    );
    assert_eq!(phase(&account, cx), Phase::SignedIn);
    account.read_with(cx, |account, _| {
        assert_eq!(
            account.note(),
            Some("The account is busy right now. Try again in a minute.")
        );
    });
    assert!(item.held().is_some());
}

#[gpui_kit::test]
fn a_sign_in_refusal_that_renewing_cant_fix_signs_the_account_out(cx: &mut TestAppContext) {
    let http = provider();
    http.answer(Method::Post, TOKEN, 400, r#"{"error":"invalid_grant"}"#);
    let (account, item, backend) = kept_account(
        &http,
        401,
        r#"{"error":"Sign in.","code":"sign-in","signIn":true}"#,
        cx,
    );
    assert_eq!(phase(&account, cx), Phase::SignedOut);
    assert_eq!(item.held(), None);
    assert_eq!(
        backend.calls(),
        [],
        "the device's own session is left alone"
    );
}

#[gpui_kit::test]
fn adding_this_computer_links_it_and_lists_it(cx: &mut TestAppContext) {
    let http = provider();
    let (account, _, backend) = kept_account(&http, 200, r#"{"machines": []}"#, cx);
    backend
        .account
        .push_link_statement(Ok(emberhut_api::connect::SignedLink::sign(
            &SecretKey::from_bytes(&[4; 32]),
            emberhut_identity::MachineName::cut("Main computer").unwrap(),
            emberhut_identity::AccountId::new("user_example").unwrap(),
            "relay.example".parse().unwrap(),
            emberhut_identity::UnixMillis::new(1_795_561_073_000).unwrap(),
        )));
    let one =
        serde_json::from_str::<serde_json::Value>(&machines()).unwrap()["machines"][0].to_string();
    http.answer(Method::Post, MACHINES, 200, &one);
    http.answer(Method::Get, MACHINES, 200, &machines());

    account.update(cx, Account::add_this_computer);
    cx.run_until_parked();

    account.read_with(cx, |account, _| {
        assert_eq!(account.note(), Some("Added Main computer to your account."));
        assert_eq!(account.machines().map(<[_]>::len), Some(1));
        assert!(!account.is_linking());
    });
}

#[gpui_kit::test]
fn a_server_elsewhere_cant_add_this_computer(cx: &mut TestAppContext) {
    let http = provider();
    http.answer(Method::Get, MACHINES, 200, r#"{"machines": []}"#);
    let item = MemoryItem::default();
    let (account, _) = account(&http, &item, "https://emberhut.example.com", cx);
    account.read_with(cx, |account, _| {
        assert!(
            account
                .cant_add_this_computer()
                .is_some_and(|why| why.contains("https://emberhut.example.com"))
        );
    });
}

/// The machines' list as the directory gives it, read.
fn machine_list() -> Vec<LinkedMachine> {
    serde_json::from_str::<emberhut_api::directory::MachineList>(&machines())
        .unwrap()
        .machines
}

#[gpui_kit::test]
fn answers_that_come_after_a_sign_out_are_dropped(cx: &mut TestAppContext) {
    let http = provider();
    let (account, _, _) = kept_account(&http, 200, r#"{"machines": []}"#, cx);
    let before = account.read_with(cx, |account, _| account.attempt);
    account.update(cx, Account::sign_out);

    account.update(cx, |account, cx| {
        account.fetched(before, Ok(machine_list()), cx);
        account.linked(before, Ok(machine_list().remove(0)), cx);
        let id = *machine_list()[0].machine.id();
        account.unlinked(before, &id, "Main computer", Ok(()), cx);
        account.sign_in_ended(before, Ok(()), cx);
    });

    account.read_with(cx, |account, _| {
        assert_eq!(account.phase(), &Phase::SignedOut);
        assert_eq!(account.machines(), None);
        assert_eq!(
            account.note(),
            Some("Signed out of your account. This device stays signed in to its machines.")
        );
    });
}

#[gpui_kit::test]
fn a_sign_in_cancelled_while_its_tokens_came_keeps_nothing(cx: &mut TestAppContext) {
    let http = provider();
    let item = MemoryItem::default();
    let (account, _) = account(&http, &item, "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    let attempt = account.read_with(cx, |account, _| account.attempt);
    account.update(cx, Account::cancel_sign_in);
    account.update(cx, |account, cx| account.sign_in_ended(attempt, Ok(()), cx));
    assert_eq!(phase(&account, cx), Phase::SignedOut);
    assert_eq!(item.held(), None);
}

#[gpui_kit::test]
fn a_sign_out_while_the_code_is_approved_keeps_nothing(cx: &mut TestAppContext) {
    let http = provider();
    let item = MemoryItem::default();
    let (account, _) = account(&http, &item, "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    let session = account.read_with(cx, |account, _| Arc::clone(&account.session));
    // The sign-in this poll belongs to is overtaken while the poll is out.
    http.meanwhile(Method::Post, TOKEN, move || {
        session.sign_out();
        crate::account::http::fake::response(200, &granted("a1"))
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(phase(&account, cx), Phase::SignedOut);
    assert_eq!(item.held(), None);
}

#[gpui_kit::test]
fn signing_out_of_the_account_touches_no_device_session(cx: &mut TestAppContext) {
    let http = provider();
    let (account, item, backend) = kept_account(&http, 200, r#"{"machines": []}"#, cx);
    let signed_in_before = backend.is_signed_in();
    account.update(cx, Account::sign_out);
    cx.run_until_parked();
    assert_eq!(item.held(), None, "the account's item is gone");
    assert_eq!(backend.calls(), [], "nothing was asked of the server");
    assert_eq!(backend.is_signed_in(), signed_in_before);
}

#[gpui_kit::test]
fn cancel_drops_the_sign_in_in_the_session_too(cx: &mut TestAppContext) {
    let http = provider();
    let (account, _) = account(&http, &MemoryItem::default(), "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    let session = account.read_with(cx, |account, _| Arc::clone(&account.session));
    let before = session.generation();
    account.update(cx, Account::cancel_sign_in);
    assert!(
        session.generation() > before,
        "a request still out for this sign-in can't keep its tokens"
    );
}

#[gpui_kit::test]
fn a_cancel_while_the_accounts_id_is_asked_keeps_nothing(cx: &mut TestAppContext) {
    const USERINFO: &str = "https://accounts.example/oauth/userinfo";
    let http = Arc::new(FakeHttp::default());
    http.answer(
        Method::Get,
        &discovery_url(ISSUER),
        200,
        &json!({"issuer": ISSUER, "authorization_endpoint": "https://accounts.example/a",
            "token_endpoint": TOKEN, "device_authorization_endpoint": DEVICE,
            "userinfo_endpoint": USERINFO})
        .to_string(),
    );
    let item = MemoryItem::default();
    let (account, _) = account(&http, &item, "http://127.0.0.1:8791", cx);
    start_device_grant(&http);
    account.update(cx, Account::sign_in);
    cx.run_until_parked();
    // Approved, with no ID token: the account's id is asked for, and the sign-in is
    // cancelled while that request is out (what Cancel does to the session).
    http.answer(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token": "a1", "expires_in": 600, "refresh_token": "r1"}"#,
    );
    let session = account.read_with(cx, |account, _| Arc::clone(&account.session));
    http.meanwhile(Method::Get, USERINFO, move || {
        session.abandon();
        crate::account::http::fake::response(200, r#"{"sub": "user_example"}"#)
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_ne!(phase(&account, cx), Phase::SignedIn);
    assert_eq!(item.held(), None);
    account.read_with(cx, |account, _| assert!(!account.session.is_signed_in()));
}
