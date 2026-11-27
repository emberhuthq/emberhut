//! `emberhut-server link` and `unlink` against a stand-in for the account's sign-in (OIDC
//! discovery, the device grant, user info) and its directory, on loopback: the code
//! shown, the polling and its slowing down, the signed link posted with the token, and the
//! unlink posted with none.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use emberhut_api::connect::SignedLink;
use emberhut_api::directory::UnlinkRequest;
use emberhut_identity::{AccountId, MachineId, RelayDomain};
use emberhut_server::config::{
    AccountConfig, DatabaseConfig, LinkConfig, MachineKeyConfig, MachineNameConfig, UnlinkConfig,
};
use emberhut_server::link::LinkError;
use emberhut_server::oauth::OauthError;
use serde_json::{Value, json};

use crate::common::TestApp;

/// The stand-in's client id, user code and the token it issues.
const CLIENT_ID: &str = "native-client";
const USER_CODE: &str = "WDJB-MJHT";
const TOKEN: &str = "account-token-for-tests";
const USER: &str = "user_example";

/// What the stand-in saw, and how its token endpoint answers next.
#[derive(Default)]
struct Seen {
    base: String,
    /// The token endpoint's answers, in order; the last repeats.
    token_answers: Vec<&'static str>,
    /// When each poll came.
    polls: Vec<Instant>,
    /// The bodies posted to the directory, with their `Authorization` header.
    posted: Vec<(String, Option<String>, Value)>,
    /// The directory answers a link with another machine than the one that signed it.
    answers_another_machine: bool,
}

type Shared = Arc<Mutex<Seen>>;

fn form(body: &[u8]) -> Vec<(String, String)> {
    form_urlencoded::parse(body).into_owned().collect()
}

fn field<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

async fn stand_in(
    State(seen): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let base = seen.lock().unwrap().base.clone();
    let auth = headers
        .get("authorization")
        .map(|value| value.to_str().unwrap().to_owned());
    match (method.as_str(), uri.path()) {
        ("GET", "/.well-known/openid-configuration") => axum::Json(json!({
            "issuer": base,
            "device_authorization_endpoint": format!("{base}/oauth/device"),
            "token_endpoint": format!("{base}/oauth/token"),
            "userinfo_endpoint": format!("{base}/oauth/userinfo"),
            "grant_types_supported": ["urn:ietf:params:oauth:grant-type:device_code"],
        }))
        .into_response(),
        ("POST", "/oauth/device") => {
            let form = form(&body);
            assert_eq!(field(&form, "client_id"), Some(CLIENT_ID));
            assert_eq!(field(&form, "scope"), Some("openid"));
            axum::Json(json!({
                "device_code": "device-code-for-tests",
                "user_code": USER_CODE,
                "verification_uri": format!("{base}/activate"),
                "verification_uri_complete": format!("{base}/activate?user_code={USER_CODE}"),
                "expires_in": 600,
                "interval": 0,
            }))
            .into_response()
        }
        ("POST", "/oauth/token") => {
            let form = form(&body);
            assert_eq!(
                field(&form, "grant_type"),
                Some("urn:ietf:params:oauth:grant-type:device_code")
            );
            assert_eq!(field(&form, "device_code"), Some("device-code-for-tests"));
            assert_eq!(field(&form, "client_id"), Some(CLIENT_ID));
            let mut seen = seen.lock().unwrap();
            seen.polls.push(Instant::now());
            let answer = if seen.token_answers.len() > 1 {
                seen.token_answers.remove(0)
            } else {
                seen.token_answers[0]
            };
            if answer == "unavailable" {
                return (StatusCode::SERVICE_UNAVAILABLE, "try again later").into_response();
            }
            if answer == "token" {
                axum::Json(json!({ "access_token": TOKEN, "token_type": "Bearer" })).into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({ "error": answer })),
                )
                    .into_response()
            }
        }
        ("GET", "/oauth/userinfo") => {
            assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
            axum::Json(json!({ "sub": USER })).into_response()
        }
        ("POST", path) if path.starts_with("/directory/v1/machines") => {
            let posted: Value = serde_json::from_slice(&body).unwrap();
            let another = {
                let mut seen = seen.lock().unwrap();
                seen.posted.push((path.to_owned(), auth, posted.clone()));
                seen.answers_another_machine
            };
            if path.ends_with("/unlink") {
                StatusCode::NO_CONTENT.into_response()
            } else {
                let link: SignedLink = serde_json::from_value(posted).unwrap();
                let key = if another {
                    emberhut_identity::SecretKey::from_bytes(&[9; 32]).public_key()
                } else {
                    link.public_key
                };
                let id = key.machine_id();
                axum::Json(json!({
                    "id": id.to_string(),
                    "publicKey": key.to_string(),
                    "name": link.name.as_str(),
                    "relayUrl": format!("wss://relay.example/v1/sealed/{id}"),
                    "connected": false,
                    "lastSeen": null,
                    "linkedAt": u64::from(link.time),
                }))
                .into_response()
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The stand-in, on loopback, answering polls with `token_answers`.
async fn account(token_answers: Vec<&'static str>) -> Shared {
    let seen = Shared::default();
    let app = Router::new()
        .fallback(stand_in)
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    {
        let mut seen = seen.lock().unwrap();
        seen.base = format!("http://{address}");
        seen.token_answers = token_answers;
    }
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    seen
}

fn settings(seen: &Shared) -> AccountConfig {
    let base = seen.lock().unwrap().base.clone();
    AccountConfig {
        issuer: Some(base.parse().unwrap()),
        client_id: Some(CLIENT_ID.to_owned()),
        relay_domain: Some("relay.example".parse().unwrap()),
        directory_url: Some(format!("{base}/directory").parse().unwrap()),
    }
}

fn link_config(app: &TestApp, account: AccountConfig) -> LinkConfig {
    LinkConfig {
        database: DatabaseConfig {
            url: app.url.clone(),
        },
        machine_key: MachineKeyConfig::default(),
        machine_name: MachineNameConfig {
            name: Some("Desk".parse().unwrap()),
        },
        account,
    }
}

#[tokio::test]
async fn link_signs_in_with_the_device_grant_and_posts_a_signed_link() {
    let app = TestApp::new().await;
    app.serve_once().await;
    let seen = account(vec![
        "authorization_pending",
        "unavailable",
        "slow_down",
        "token",
    ])
    .await;
    let mut out = Vec::new();
    emberhut_server::link::link(&link_config(&app, settings(&seen)), &mut out)
        .await
        .unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert!(printed.contains(USER_CODE), "{printed}");
    assert!(printed.contains("/activate"), "{printed}");
    assert!(printed.contains("Linked \"Desk\""), "{printed}");
    assert!(!printed.contains(TOKEN), "the token is never shown");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.polls.len(), 4, "a failed poll is tried again");
    assert!(
        seen.polls[1] - seen.polls[0] >= Duration::from_secs(1),
        "an interval of 0 is taken as a second"
    );
    assert!(
        seen.polls[3] - seen.polls[2] >= Duration::from_secs(6),
        "slow_down makes the next poll five seconds later"
    );
    let [(path, auth, body)] = seen.posted.as_slice() else {
        panic!("one link posted: {:?}", seen.posted.len());
    };
    assert_eq!(path, "/directory/v1/machines");
    assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    let link: SignedLink = serde_json::from_value(body.clone()).unwrap();
    let id = link
        .verify(
            &AccountId::new(USER).unwrap(),
            &"relay.example".parse::<RelayDomain>().unwrap(),
        )
        .unwrap();
    assert_eq!(link.name.as_str(), "Desk");
    let machine = emberhut_server::machine_id(&emberhut_server::config::IdConfig {
        database: DatabaseConfig {
            url: app.url.clone(),
        },
        machine_key: MachineKeyConfig::default(),
    })
    .unwrap();
    assert_eq!(id, machine, "signed with the machine's own key");
}

#[tokio::test]
async fn a_declined_sign_in_links_nothing() {
    let app = TestApp::new().await;
    app.serve_once().await;
    let seen = account(vec!["authorization_pending", "access_denied"]).await;
    let refused = emberhut_server::link::link(&link_config(&app, settings(&seen)), &mut Vec::new())
        .await
        .unwrap_err();
    assert!(
        matches!(refused, LinkError::SignIn(OauthError::Denied)),
        "{refused:?}"
    );
    assert_eq!(seen.lock().unwrap().posted, Vec::new(), "nothing is posted");

    let seen = account(vec!["expired_token"]).await;
    let expired = emberhut_server::link::link(&link_config(&app, settings(&seen)), &mut Vec::new())
        .await
        .unwrap_err();
    assert!(
        matches!(expired, LinkError::SignIn(OauthError::Expired)),
        "{expired:?}"
    );
}

#[tokio::test]
async fn a_sign_in_that_keeps_failing_gives_up_after_a_few_tries() {
    let app = TestApp::new().await;
    app.serve_once().await;
    let seen = account(vec!["unavailable"]).await;
    let failed = emberhut_server::link::link(&link_config(&app, settings(&seen)), &mut Vec::new())
        .await
        .unwrap_err();
    assert!(
        matches!(failed, LinkError::SignIn(OauthError::Unexpected { .. })),
        "{failed:?}"
    );
    assert_eq!(
        seen.lock().unwrap().polls.len(),
        4,
        "the first try and three more"
    );
}

#[tokio::test]
async fn a_directory_answer_for_another_machine_is_an_error() {
    let app = TestApp::new().await;
    app.serve_once().await;
    let seen = account(vec!["token"]).await;
    seen.lock().unwrap().answers_another_machine = true;
    let wrong = emberhut_server::link::link(&link_config(&app, settings(&seen)), &mut Vec::new())
        .await
        .unwrap_err();
    assert!(matches!(wrong, LinkError::OtherMachine(_)), "{wrong:?}");
}

#[tokio::test]
async fn without_the_account_s_settings_link_says_the_account_is_off() {
    let app = TestApp::new().await;
    let off = emberhut_server::link::link(
        &link_config(&app, AccountConfig::default()),
        &mut Vec::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(off, LinkError::AccountOff { .. }), "{off:?}");
    assert!(off.to_string().contains("--account-issuer"), "{off}");
    assert!(
        !app.dir().join("machine.key").exists(),
        "nothing is made before the settings are checked"
    );
}

#[tokio::test]
async fn unlink_posts_the_machine_s_own_statement_with_no_token() {
    let app = TestApp::new().await;
    app.serve_once().await;
    let seen = account(vec!["token"]).await;
    let config = UnlinkConfig {
        database: DatabaseConfig {
            url: app.url.clone(),
        },
        machine_key: MachineKeyConfig::default(),
        account: settings(&seen),
    };
    let mut out = Vec::new();
    emberhut_server::link::unlink(&config, &mut out)
        .await
        .unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.polls.is_empty(), "no sign-in");
    let [(path, auth, body)] = seen.posted.as_slice() else {
        panic!("one unlink posted");
    };
    assert_eq!(*auth, None, "no account token");
    let id: MachineId = path
        .strip_prefix("/directory/v1/machines/")
        .and_then(|rest| rest.strip_suffix("/unlink"))
        .unwrap()
        .parse()
        .unwrap();
    let request: UnlinkRequest = serde_json::from_value(body.clone()).unwrap();
    request
        .verify(&id, &"relay.example".parse().unwrap())
        .unwrap();
    assert!(String::from_utf8(out).unwrap().contains(&id.to_string()));
}
