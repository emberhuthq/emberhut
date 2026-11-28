//! The account's rules against a scripted provider and directory, and a keychain in memory.

use std::sync::{Arc, Mutex};

use emberhut_api::connect::SignedLink;
use emberhut_identity::{MachineName, RelayDomain, SecretKey, UnixMillis};
use serde_json::json;

use super::*;
use crate::account::http::Method;
use crate::account::http::fake::{FakeHttp, response};
use crate::account::switch::BuildValues;
use crate::account::tokens::tests::{id_claims, jwt};
use crate::account::vault::tests::MemoryItem;
use crate::account::vault::{ItemVault, SecretItem};
use crate::backend::fake::FakeBackend;

const ISSUER: &str = "https://accounts.example";
const TOKEN: &str = "https://accounts.example/oauth/token";
const DEVICE: &str = "https://accounts.example/oauth/device";
const USERINFO: &str = "https://accounts.example/oauth/userinfo";
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

/// A session over a fresh fake provider and directory, its keychain item, and the fake.
fn session() -> (AccountSession, Arc<FakeHttp>, MemoryItem) {
    let http = Arc::new(FakeHttp::default());
    http.answer(
        Method::Get,
        &discovery_url(ISSUER),
        200,
        &json!({
            "issuer": ISSUER,
            "authorization_endpoint": "https://accounts.example/oauth/authorize",
            "token_endpoint": TOKEN,
            "device_authorization_endpoint": DEVICE,
            "userinfo_endpoint": USERINFO,
        })
        .to_string(),
    );
    let item = MemoryItem::default();
    let shared = Arc::clone(&http) as Arc<dyn Http>;
    let session = AccountSession::new(settings(), shared, Box::new(ItemVault::new(item.clone())));
    (session, http, item)
}

fn granted(access: &str, refresh: Option<&str>, expires_in: u64) -> String {
    json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "refresh_token": refresh,
        "id_token": jwt(&id_claims("user_example")),
    })
    .to_string()
}

/// The session signed in with access token `access` (good for 10 minutes) and refresh `r1`.
fn signed_in(access: &str) -> (AccountSession, Arc<FakeHttp>, MemoryItem) {
    let (session, http, item) = session();
    let answer: TokenAnswer = serde_json::from_str(&granted(access, Some("r1"), 600)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    (session, http, item)
}

fn machine_list() -> String {
    let key = SecretKey::from_bytes(&[4; 32]);
    json!({"machines": [{
        "id": key.machine_id().to_string(),
        "publicKey": key.public_key().to_string(),
        "name": "Main computer",
        "relayUrl": "wss://relay.example/v1/sealed/x",
        "connected": true,
        "lastSeen": 1_795_561_073_000_u64,
        "linkedAt": 1_795_000_000_000_u64,
    }]})
    .to_string()
}

fn directory_error(code: &str) -> String {
    json!({"error": "The directory's sentence.", "code": code, "signIn": code == "sign-in"})
        .to_string()
}

#[test]
fn the_device_grant_starts_at_the_discovered_endpoint() {
    let (session, http, _) = session();
    http.answer(
        Method::Post,
        DEVICE,
        200,
        &json!({"device_code": "dc", "user_code": "WDJB-MJHT",
            "verification_uri": "https://accounts.example/device",
            "verification_uri_complete": "https://accounts.example/device?code=WDJB-MJHT",
            "expires_in": 600, "interval": 5})
        .to_string(),
    );
    let started = session.start_device_grant().unwrap();
    assert_eq!(started.user_code, "WDJB-MJHT");
    assert_eq!(
        started.page(),
        "https://accounts.example/device?code=WDJB-MJHT"
    );
    let sent = http.sent_to(DEVICE);
    assert_eq!(sent[0].field("scope"), Some("openid offline_access"));
    assert_eq!(sent[0].field("client_id"), Some("client-example"));

    http.answer(
        Method::Post,
        TOKEN,
        400,
        r#"{"error":"authorization_pending"}"#,
    );
    assert_eq!(session.poll_device_grant(&started), Poll::Pending);
    http.answer(Method::Post, TOKEN, 400, r#"{"error":"slow_down"}"#);
    assert_eq!(session.poll_device_grant(&started), Poll::SlowDown);
    http.fail(Method::Post, TOKEN);
    assert_eq!(session.poll_device_grant(&started), Poll::Unanswered);
    http.answer(Method::Post, TOKEN, 200, &granted("a1", Some("r1"), 600));
    let Poll::Granted(answer) = session.poll_device_grant(&started) else {
        panic!("tokens")
    };
    session.signed_in(answer, session.generation()).unwrap();
    assert!(session.is_signed_in());
    assert_eq!(
        http.sent_to(TOKEN)[0].field("device_code"),
        Some("dc"),
        "polled with the device code"
    );
}

#[test]
fn a_provider_without_the_device_grant_says_so() {
    let http = Arc::new(FakeHttp::default());
    http.answer(
        Method::Get,
        &discovery_url(ISSUER),
        200,
        &json!({"issuer": ISSUER, "authorization_endpoint": "https://accounts.example/a",
            "token_endpoint": TOKEN})
        .to_string(),
    );
    let session = AccountSession::new(
        settings(),
        http,
        Box::new(ItemVault::new(MemoryItem::default())),
    );
    assert!(matches!(
        session.start_device_grant(),
        Err(AccountError::NoDeviceGrant)
    ));
}

#[test]
fn signing_in_keeps_the_tokens_in_the_keychain_item_and_a_new_launch_reads_them() {
    let (session, http, item) = signed_in("a1");
    let kept: AccountTokens = serde_json::from_slice(&item.held().unwrap()).unwrap();
    assert_eq!(kept.account.as_str(), "user_example");
    assert_eq!(kept.refresh.as_deref(), Some("r1"));
    assert_eq!(session.unkept(), None);

    let again = AccountSession::new(settings(), http, Box::new(ItemVault::new(item.clone())));
    assert!(again.restore().unwrap());
    let generation = again.sign_out();
    assert!(item.held().is_some(), "the Keychain is left to forget_kept");
    again.forget_kept(generation);
    assert_eq!(item.held(), None, "signing out removes the item");
    assert!(!again.is_signed_in());
}

#[test]
fn without_an_id_token_the_account_comes_from_user_info() {
    let (session, http, _) = session();
    http.answer(Method::Get, USERINFO, 200, r#"{"sub":"user_other"}"#);
    let answer: TokenAnswer =
        serde_json::from_value(json!({"access_token": "a1", "expires_in": 600})).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    assert_eq!(
        http.sent_to(USERINFO)[0].bearer.as_deref(),
        Some("a1"),
        "asked with the new token"
    );
    assert!(session.is_signed_in());
}

#[test]
fn the_machines_are_listed_with_the_access_token() {
    let (session, http, _) = signed_in("a1");
    http.answer(Method::Get, MACHINES, 200, &machine_list());
    let machines = session.machines().unwrap();
    assert_eq!(machines[0].name.as_str(), "Main computer");
    assert!(machines[0].connected);
    assert_eq!(http.sent_to(MACHINES)[0].bearer.as_deref(), Some("a1"));
}

#[test]
fn an_expired_token_is_renewed_before_the_call() {
    let (session, http, item) = session();
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 30)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    http.answer(Method::Post, TOKEN, 200, &granted("a2", Some("r2"), 600));
    http.answer(Method::Get, MACHINES, 200, &machine_list());

    session.machines().unwrap();

    let renewal = &http.sent_to(TOKEN)[0];
    assert_eq!(renewal.field("grant_type"), Some("refresh_token"));
    assert_eq!(renewal.field("refresh_token"), Some("r1"));
    assert_eq!(http.sent_to(MACHINES)[0].bearer.as_deref(), Some("a2"));
    let kept: AccountTokens = serde_json::from_slice(&item.held().unwrap()).unwrap();
    assert_eq!(
        kept.refresh.as_deref(),
        Some("r2"),
        "the turned-over token is kept"
    );
}

#[test]
fn a_401_renews_once_and_a_second_signs_the_account_out() {
    let (session, http, item) = signed_in("a1");
    http.answer(Method::Get, MACHINES, 401, &directory_error("sign-in"));
    http.answer(Method::Post, TOKEN, 200, &granted("a2", None, 600));
    http.answer(Method::Get, MACHINES, 200, &machine_list());
    assert!(session.machines().is_ok(), "the renewed token works");

    http.answer(Method::Get, MACHINES, 401, &directory_error("sign-in"));
    http.answer(Method::Post, TOKEN, 200, &granted("a3", None, 600));
    http.answer(Method::Get, MACHINES, 401, &directory_error("sign-in"));
    let error = session.machines().unwrap_err();
    assert!(error.is_signed_out(), "{error:?}");
    assert!(!session.is_signed_in());
    assert_eq!(item.held(), None);
}

#[test]
fn a_refused_refresh_token_signs_the_account_out() {
    let (session, http, _) = signed_in("a1");
    http.answer(Method::Get, MACHINES, 401, &directory_error("sign-in"));
    http.answer(Method::Post, TOKEN, 400, r#"{"error":"invalid_grant"}"#);
    assert!(session.machines().unwrap_err().is_signed_out());
    assert!(!session.is_signed_in());
}

#[test]
fn busy_or_out_is_try_later_and_never_signs_out() {
    let (session, http, item) = signed_in("a1");
    http.respond(
        Method::Get,
        MACHINES,
        Response {
            status: 503,
            retry_after: Some(Duration::from_secs(30)),
            body: directory_error("unavailable").into_bytes(),
        },
    );
    let error = session.machines().unwrap_err();
    assert!(matches!(error, AccountError::Later(Some(_))), "{error:?}");
    assert_eq!(
        error.to_string(),
        "The account is busy right now. Try again in 30 seconds."
    );
    http.answer(Method::Get, MACHINES, 429, &directory_error("limit"));
    assert!(matches!(session.machines(), Err(AccountError::Later(None))));
    http.fail(Method::Get, MACHINES);
    assert!(matches!(
        session.machines(),
        Err(AccountError::Unreachable(_))
    ));
    assert!(session.is_signed_in());
    assert!(item.held().is_some());
}

#[test]
fn a_renewal_with_no_answer_keeps_the_account() {
    let (session, http, _) = session();
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 30)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    http.answer(Method::Post, TOKEN, 503, "busy");
    assert!(matches!(session.machines(), Err(AccountError::Later(_))));
    http.fail(Method::Post, TOKEN);
    assert!(matches!(
        session.machines(),
        Err(AccountError::Unreachable(_))
    ));
    assert!(session.is_signed_in());
}

#[test]
fn unlinking_deletes_the_machine_from_the_account() {
    let (session, http, _) = signed_in("a1");
    let id = SecretKey::from_bytes(&[4; 32]).machine_id();
    let url = format!("{MACHINES}/{id}");
    http.answer(Method::Delete, &url, 204, "");
    session.unlink(&id).unwrap();
    http.answer(Method::Delete, &url, 404, &directory_error("not-found"));
    assert_eq!(
        session.unlink(&id).unwrap_err().to_string(),
        "That hut isn't in this account any more."
    );
}

/// A link as the server on this computer signs it for the signed-in account.
fn signed_link() -> SignedLink {
    SignedLink::sign(
        &SecretKey::from_bytes(&[4; 32]),
        MachineName::cut("Main computer").unwrap(),
        AccountId::new("user_example").unwrap(),
        "relay.example".parse::<RelayDomain>().unwrap(),
        UnixMillis::new(1_795_561_073_000).unwrap(),
    )
}

fn loopback_backend() -> FakeBackend {
    FakeBackend::new([]).at("http://127.0.0.1:8791")
}

#[test]
fn adding_this_computer_posts_the_servers_link_with_the_account_token() {
    let (session, http, _) = signed_in("a1");
    let backend = loopback_backend();
    backend.account.push_link_statement(Ok(signed_link()));
    let one = serde_json::from_str::<serde_json::Value>(&machine_list()).unwrap()["machines"][0]
        .to_string();
    http.answer(Method::Post, MACHINES, 200, &one);

    let linked = session.link_this_computer(&backend).unwrap();

    assert_eq!(linked.name.as_str(), "Main computer");
    assert_eq!(
        backend.calls(),
        [backend::fake::Call::LinkStatement(LinkStatementRequest {
            account: AccountId::new("user_example").unwrap(),
            relay_domain: "relay.example".parse().unwrap(),
        })]
    );
    let posted = &http.sent_to(MACHINES)[0];
    assert_eq!(posted.bearer.as_deref(), Some("a1"));
    let crate::account::http::Body::Json(body) = &posted.body else {
        panic!("a JSON body")
    };
    assert_eq!(
        serde_json::from_slice::<SignedLink>(body).unwrap(),
        signed_link(),
        "the link as the server signed it"
    );
}

#[test]
fn adding_this_computer_shows_the_directorys_refusal_in_words() {
    for (code, words) in [
        (
            "linked-elsewhere",
            "This hut is linked to another account. Unlink it there first.",
        ),
        (
            "account-full",
            "This account has all the huts it can hold. Unlink one first.",
        ),
    ] {
        let (session, http, _) = signed_in("a1");
        let backend = loopback_backend();
        backend.account.push_link_statement(Ok(signed_link()));
        http.answer(Method::Post, MACHINES, 409, &directory_error(code));
        let error = session.link_this_computer(&backend).unwrap_err();
        assert_eq!(error.to_string(), words);
        assert!(session.is_signed_in());
    }
}

#[test]
fn adding_a_computer_needs_the_server_on_this_computer() {
    let (session, http, _) = signed_in("a1");
    let backend = FakeBackend::new([]);
    let error = session.link_this_computer(&backend).unwrap_err();
    assert!(
        matches!(error, AccountError::NotThisComputer(_)),
        "{error:?}"
    );
    assert_eq!(backend.calls(), [], "the server isn't asked");
    assert_eq!(http.sent_to(MACHINES), []);

    let backend = loopback_backend();
    backend
        .account
        .push_link_statement(Err(backend::Error::Refused {
            status: Some(403),
            message: Some("Only from this computer.".into()),
        }));
    assert_eq!(
        session
            .link_this_computer(&backend)
            .unwrap_err()
            .to_string(),
        "The server on this computer didn't sign the link: Only from this computer."
    );
}

#[test]
fn a_keychain_that_cant_keep_the_tokens_keeps_them_for_this_launch() {
    #[derive(Debug)]
    struct Refusing;
    impl Vault for Refusing {
        fn load(&self) -> Result<Option<AccountTokens>, VaultError> {
            Ok(None)
        }
        fn save(&self, _: &AccountTokens) -> Result<(), VaultError> {
            Err(VaultError::Unsupported)
        }
        fn clear(&self) -> Result<(), VaultError> {
            Ok(())
        }
    }
    let (with_item, http, _) = session();
    drop(with_item);
    let session = AccountSession::new(settings(), http, Box::new(Refusing));
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 600)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    assert!(session.is_signed_in());
    assert_eq!(
        session.unkept().as_deref(),
        Some("keeping the account's sign-in isn't supported on this system yet")
    );
}

/// A session signed in with an access token that has run out, behind an `Arc`.
fn stale() -> (Arc<AccountSession>, Arc<FakeHttp>, MemoryItem) {
    let (session, http, item) = session();
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 30)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    (Arc::new(session), http, item)
}

#[test]
fn a_renewal_that_ends_after_a_sign_out_keeps_nothing() {
    let (session, http, item) = stale();
    let signing_out = Arc::clone(&session);
    http.meanwhile(Method::Post, TOKEN, move || {
        let generation = signing_out.sign_out();
        signing_out.forget_kept(generation);
        response(200, &granted("a2", Some("r2"), 600))
    });

    let error = session.machines().unwrap_err();

    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(!session.is_signed_in());
    assert_eq!(
        item.held(),
        None,
        "the renewed tokens never reach the Keychain"
    );
    assert_eq!(http.sent_to(MACHINES), []);
}

#[test]
fn a_refusal_that_ends_after_a_new_sign_in_signs_nobody_out() {
    let (session, http, item) = stale();
    let again = Arc::clone(&session);
    http.meanwhile(Method::Post, TOKEN, move || {
        again.sign_out();
        let answer: TokenAnswer = serde_json::from_str(&granted("b1", Some("rb"), 600)).unwrap();
        again.signed_in(answer, again.generation()).unwrap();
        response(400, r#"{"error":"invalid_grant"}"#)
    });

    let error = session.machines().unwrap_err();
    assert!(matches!(error, AccountError::Superseded), "{error:?}");

    assert!(session.is_signed_in(), "the new sign-in stands");
    let kept: AccountTokens = serde_json::from_slice(&item.held().unwrap()).unwrap();
    assert_eq!(kept.access, "b1");
}

#[test]
fn a_second_401_after_a_new_sign_in_signs_nobody_out() {
    let (session, http, _) = signed_in("a1");
    let session = Arc::new(session);
    http.answer(Method::Get, MACHINES, 401, &directory_error("sign-in"));
    http.answer(Method::Post, TOKEN, 200, &granted("a2", None, 600));
    let again = Arc::clone(&session);
    http.meanwhile(Method::Get, MACHINES, move || {
        again.sign_out();
        let answer: TokenAnswer = serde_json::from_str(&granted("b1", Some("rb"), 600)).unwrap();
        again.signed_in(answer, again.generation()).unwrap();
        response(401, &directory_error("sign-in"))
    });

    let error = session.machines().unwrap_err();
    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(session.is_signed_in(), "the sign-in made meanwhile stands");
}

#[test]
fn a_sign_in_that_finishes_after_a_sign_out_keeps_nothing() {
    let (session, _, item) = session();
    let since = session.generation();
    let generation = session.sign_out();
    session.forget_kept(generation);
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 600)).unwrap();

    let error = session.signed_in(answer, since).unwrap_err();

    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(!session.is_signed_in());
    assert_eq!(item.held(), None);
}

#[test]
fn a_sign_out_left_to_clear_the_keychain_late_spares_a_newer_sign_in() {
    let (session, _, item) = signed_in("a1");
    let generation = session.sign_out();
    let answer: TokenAnswer = serde_json::from_str(&granted("b1", Some("rb"), 600)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    session.forget_kept(generation);
    assert!(item.held().is_some());
}

#[test]
fn two_calls_with_a_stale_token_renew_it_once() {
    let (session, http, _) = stale();
    http.meanwhile(Method::Post, TOKEN, || {
        std::thread::sleep(Duration::from_millis(100));
        response(200, &granted("a2", Some("r2"), 600))
    });
    http.answer(Method::Get, MACHINES, 200, &machine_list());
    http.answer(Method::Get, MACHINES, 200, &machine_list());

    let calls: Vec<_> = (0..2)
        .map(|_| {
            let session = Arc::clone(&session);
            std::thread::spawn(move || session.machines())
        })
        .collect();
    for call in calls {
        call.join().unwrap().unwrap();
    }

    assert_eq!(http.sent_to(TOKEN).len(), 1, "one renewal for both");
    let tokens: Vec<_> = http
        .sent_to(MACHINES)
        .into_iter()
        .map(|request| request.bearer)
        .collect();
    assert_eq!(tokens, [Some("a2".to_owned()), Some("a2".to_owned())]);
}

#[test]
fn only_invalid_grant_signs_out_and_a_busy_provider_is_later() {
    let (session, http, item) = stale();
    http.answer(Method::Post, TOKEN, 400, r#"{"error":"invalid_client"}"#);
    assert!(matches!(session.machines(), Err(AccountError::Grant(_))));
    http.answer(Method::Post, TOKEN, 429, r#"{"error":"invalid_grant"}"#);
    assert!(
        matches!(session.machines(), Err(AccountError::Later(_))),
        "429 is read by its status"
    );
    assert!(session.is_signed_in());
    assert!(item.held().is_some());
}

/// A session whose provider has `flow`'s callback ready, and the callback with `state`.
fn code_flow_session() -> (AccountSession, Arc<FakeHttp>, CodeFlow, String) {
    let (session, http, _) = session();
    let flow = CodeFlow::new().unwrap();
    let page = session.authorize_url(&flow).unwrap();
    let state = form_urlencoded::parse(page.split_once('?').unwrap().1.as_bytes())
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    (session, http, flow, state)
}

#[test]
fn a_callback_with_another_state_never_reaches_the_token_endpoint() {
    let (session, http, flow, _) = code_flow_session();
    let callback = format!("{}?code=c1&state=forged", crate::account::oidc::CALLBACK);
    let error = session
        .finish_code_flow(&flow, &callback, session.generation())
        .unwrap_err();
    assert!(matches!(error, AccountError::Callback(_)), "{error:?}");
    assert_eq!(http.sent_to(TOKEN), []);
    assert!(!session.is_signed_in());
}

#[test]
fn the_codes_exchange_carries_the_verifier_and_the_redirect() {
    let (session, http, flow, state) = code_flow_session();
    http.answer(Method::Post, TOKEN, 200, &granted("a1", Some("r1"), 600));
    let callback = format!("{}?code=c1&state={state}", crate::account::oidc::CALLBACK);

    session
        .finish_code_flow(&flow, &callback, session.generation())
        .unwrap();

    let exchange = &http.sent_to(TOKEN)[0];
    assert_eq!(exchange.field("grant_type"), Some("authorization_code"));
    assert_eq!(exchange.field("code"), Some("c1"));
    assert_eq!(exchange.field("code_verifier"), Some(flow.verifier()));
    assert_eq!(
        exchange.field("redirect_uri"),
        Some("https://emberhut.com/oauth/callback")
    );
    assert!(session.is_signed_in());
}

#[test]
fn a_sign_in_page_that_isnt_https_is_refused() {
    let (session, http, _) = session();
    http.answer(
        Method::Post,
        DEVICE,
        200,
        &json!({"device_code": "dc", "user_code": "WDJB-MJHT",
            "verification_uri": "http://accounts.example/device", "expires_in": 600})
        .to_string(),
    );
    assert!(matches!(
        session.start_device_grant(),
        Err(AccountError::InsecurePage)
    ));
    http.answer(
        Method::Post,
        DEVICE,
        200,
        &json!({"device_code": "dc", "user_code": "WDJB-MJHT",
            "verification_uri": "https://accounts.example/device",
            "verification_uri_complete": "javascript:alert(1)", "expires_in": 600})
        .to_string(),
    );
    let started = session.start_device_grant().unwrap();
    assert_eq!(started.page(), "https://accounts.example/device");
}

#[test]
fn a_link_the_server_signed_for_another_account_isnt_posted() {
    let (session, http, _) = signed_in("a1");
    let backend = loopback_backend();
    backend.account.push_link_statement(Ok(SignedLink::sign(
        &SecretKey::from_bytes(&[4; 32]),
        MachineName::cut("Main computer").unwrap(),
        AccountId::new("user_other").unwrap(),
        "relay.example".parse::<RelayDomain>().unwrap(),
        UnixMillis::new(1_795_561_073_000).unwrap(),
    )));
    let error = session.link_this_computer(&backend).unwrap_err();
    assert!(matches!(error, AccountError::Server(_)), "{error:?}");
    assert_eq!(http.sent_to(MACHINES), []);
}

#[test]
fn a_code_exchange_abandoned_while_it_is_out_keeps_nothing() {
    let (session, http, flow, state) = code_flow_session();
    let session = Arc::new(session);
    let cancelling = Arc::clone(&session);
    http.meanwhile(Method::Post, TOKEN, move || {
        cancelling.abandon();
        response(200, &granted("a1", Some("r1"), 600))
    });
    let callback = format!("{}?code=c1&state={state}", crate::account::oidc::CALLBACK);

    let error = session
        .finish_code_flow(&flow, &callback, session.generation())
        .unwrap_err();

    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(!session.is_signed_in());
}

/// What runs in the middle of a keychain write, once.
type During = Option<Box<dyn FnOnce() + Send>>;

/// A keychain item that runs `during` in the middle of its first write.
#[derive(Clone, Default)]
struct InterruptedItem {
    item: MemoryItem,
    during: Arc<Mutex<During>>,
}

impl SecretItem for InterruptedItem {
    fn read(&self) -> Result<Option<Vec<u8>>, VaultError> {
        self.item.read()
    }

    fn write(&self, bytes: &[u8]) -> Result<(), VaultError> {
        self.item.write(bytes)?;
        let during = self.during.lock().unwrap().take();
        if let Some(during) = during {
            during();
        }
        Ok(())
    }

    fn delete(&self) -> Result<(), VaultError> {
        self.item.delete()
    }
}

/// A session over `item` whose first keychain write runs `during` with the session.
fn interrupted(
    item: &InterruptedItem,
    during: impl FnOnce(&AccountSession) + Send + 'static,
) -> (Arc<AccountSession>, Arc<FakeHttp>) {
    let (_, http, _) = session();
    let session = Arc::new(AccountSession::new(
        settings(),
        Arc::clone(&http) as Arc<dyn Http>,
        Box::new(ItemVault::new(item.clone())),
    ));
    let weak = Arc::downgrade(&session);
    *item.during.lock().unwrap() = Some(Box::new(move || {
        if let Some(session) = weak.upgrade() {
            during(&session);
        }
    }));
    (session, http)
}

#[test]
fn a_cancel_during_the_keychain_write_leaves_nothing_there() {
    let item = InterruptedItem::default();
    let (session, _) = interrupted(&item, AccountSession::abandon);
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 600)).unwrap();

    let error = session.signed_in(answer, session.generation()).unwrap_err();

    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(!session.is_signed_in());
    assert_eq!(
        item.item.held(),
        None,
        "the abandoned sign-in isn't in the Keychain"
    );
}

#[test]
fn a_sign_out_during_a_renewals_keychain_write_leaves_nothing_there() {
    let item = InterruptedItem::default();
    let (session, http) = interrupted(&item, |_| {});
    let answer: TokenAnswer = serde_json::from_str(&granted("a1", Some("r1"), 30)).unwrap();
    session.signed_in(answer, session.generation()).unwrap();
    let signing_out = Arc::downgrade(&session);
    *item.during.lock().unwrap() = Some(Box::new(move || {
        if let Some(session) = signing_out.upgrade() {
            session.sign_out();
        }
    }));
    http.answer(Method::Post, TOKEN, 200, &granted("a2", Some("r2"), 600));

    let error = session.machines().unwrap_err();

    assert!(matches!(error, AccountError::Superseded), "{error:?}");
    assert!(!session.is_signed_in());
    assert_eq!(
        item.item.held(),
        None,
        "the renewed tokens aren't in the Keychain"
    );
}
