//! `GET /api/mac` and `POST /api/mac/lock`: the Mac's row and "Lock now", over a fake helper;
//! and the real helper client against a helper served on `127.0.0.1`.
#![expect(
    clippy::unwrap_used,
    reason = "the fake helper's lock: a poisoned one fails the test that holds it"
)]

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use emberhut_api::mac::{ExtensionFolder, MacStatus, helper as helper_paths};
use emberhut_api::paths;
use emberhut_server::config::AuthMode;
use emberhut_server::services::mac::{HttpMacHelper, MacError, MacHealth, MacHelper};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::common::{Call, TestApp};

/// The helper of the tests: offline until it is given a health; it locks, or refuses with the
/// words it is given.
#[derive(Debug, Default)]
pub(crate) struct FakeMac {
    script: Mutex<Script>,
}

#[derive(Debug, Default)]
struct Script {
    health: Option<MacHealth>,
    refusal: Option<String>,
    locks: usize,
    /// The assistant's Mac tools: each path's answer, and the calls made.
    hands: std::collections::HashMap<String, Result<Value, String>>,
    calls: Vec<(String, Value)>,
    /// The browser extension's folder (none: no helper for it), what an install leaves there,
    /// and the zips installed; whether it is off (it answers nothing about the folder), the
    /// words it refuses installs with, whether installs go unanswered, what an install waits
    /// for before it answers, and how many times the folder was asked about.
    folder: Option<ExtensionFolder>,
    installs_as: Option<String>,
    installed: Vec<Vec<u8>>,
    off: bool,
    install_refusal: Option<String>,
    install_lost: bool,
    install_gate: Option<Arc<Notify>>,
    asked: usize,
}

impl FakeMac {
    /// It answers with `health` from now on.
    pub(crate) fn answers(&self, health: MacHealth) {
        self.script.lock().unwrap().health = Some(health);
    }

    /// It refuses to lock, saying `why`.
    pub(crate) fn refuses(&self, why: &str) {
        self.script.lock().unwrap().refusal = Some(why.to_owned());
    }

    /// The helper answers `path` with `answer` (an object) from now on.
    pub(crate) fn hands_answer(&self, path: &str, answer: Value) {
        self.script
            .lock()
            .unwrap()
            .hands
            .insert(path.to_owned(), Ok(answer));
    }

    /// The helper refuses `path`, saying `why`.
    pub(crate) fn hands_refuse(&self, path: &str, why: &str) {
        self.script
            .lock()
            .unwrap()
            .hands
            .insert(path.to_owned(), Err(why.to_owned()));
    }

    /// The Mac tools' calls so far: their paths and bodies.
    pub(crate) fn hands_calls(&self) -> Vec<(String, Value)> {
        self.script.lock().unwrap().calls.clone()
    }

    /// It keeps the extension's folder, holding `version`; an install leaves `installs_as`
    /// there.
    pub(crate) fn keeps_extension(&self, version: Option<&str>, installs_as: &str) {
        let mut script = self.script.lock().unwrap();
        script.folder = Some(ExtensionFolder {
            version: version.map(str::to_owned),
            path: Some("/Emberhut/Extension".to_owned()),
        });
        script.installs_as = Some(installs_as.to_owned());
    }

    /// The zips installed in the extension's folder so far.
    pub(crate) fn installed(&self) -> Vec<Vec<u8>> {
        self.script.lock().unwrap().installed.clone()
    }

    /// It stops answering about the extension's folder (`true`), or answers again.
    pub(crate) fn turns_off(&self, off: bool) {
        self.script.lock().unwrap().off = off;
    }

    /// It refuses installs from now on, saying `why` (none: it takes them again); the zips sent
    /// still count as installed.
    pub(crate) fn refuses_installs(&self, why: Option<&str>) {
        self.script.lock().unwrap().install_refusal = why.map(str::to_owned);
    }

    /// Its installs go unanswered from now on, as when they take too long (the zips sent still
    /// count as installed, and the folder stays as it was).
    pub(crate) fn loses_installs(&self) {
        self.script.lock().unwrap().install_lost = true;
    }

    /// Each install from now on holds its answer (and leaves the folder as it was) until the
    /// gate returned is opened once for it (`notify_one`).
    pub(crate) fn holds_installs(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        self.script.lock().unwrap().install_gate = Some(Arc::clone(&gate));
        gate
    }

    /// The version the extension's folder holds.
    pub(crate) fn folder_version(&self) -> Option<String> {
        let script = self.script.lock().unwrap();
        script
            .folder
            .as_ref()
            .and_then(|folder| folder.version.clone())
    }

    /// How many times it was asked what the extension's folder holds.
    pub(crate) fn asked(&self) -> usize {
        self.script.lock().unwrap().asked
    }

    /// How many times it locked the screen.
    pub(crate) fn locks(&self) -> usize {
        self.script.lock().unwrap().locks
    }
}

impl MacHelper for FakeMac {
    fn extension(&self) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        let mut script = self.script.lock().unwrap();
        script.asked += 1;
        let folder = if script.off {
            Err(MacError::Unreachable("the helper is off".into()))
        } else {
            script.folder.clone().ok_or(MacError::NotSetUp)
        };
        Box::pin(async move { folder })
    }

    fn install_extension(&self, zip: Vec<u8>) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        Box::pin(async move {
            let gate = {
                let mut script = self.script.lock().unwrap();
                if script.off {
                    return Err(MacError::Unreachable("the helper is off".into()));
                }
                script.installed.push(zip);
                script.install_gate.clone()
            };
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let mut script = self.script.lock().unwrap();
            if script.install_lost {
                return Err(MacError::Unreachable("the install took too long".into()));
            }
            if let Some(why) = script.install_refusal.clone() {
                return Err(MacError::Refused(why));
            }
            let version = script.installs_as.clone();
            let folder = script.folder.as_mut().map(|folder| {
                folder.version.clone_from(&version);
                folder.clone()
            });
            folder.ok_or(MacError::NotSetUp)
        })
    }

    fn hands(
        &self,
        path: &str,
        body: Value,
        _wait: std::time::Duration,
    ) -> BoxFuture<'_, Result<serde_json::Map<String, Value>, MacError>> {
        let mut script = self.script.lock().unwrap();
        script.calls.push((path.to_owned(), body));
        let answer = match script.hands.get(path).cloned() {
            Some(Ok(Value::Object(answer))) => Ok(answer),
            Some(Ok(_)) => Ok(serde_json::Map::new()),
            Some(Err(why)) => Err(MacError::Refused(why)),
            None => Err(MacError::Unreachable("the helper is off".into())),
        };
        Box::pin(async move { answer })
    }

    fn health(&self) -> BoxFuture<'_, Result<MacHealth, MacError>> {
        let health = self.script.lock().unwrap().health;
        Box::pin(
            async move { health.ok_or_else(|| MacError::Unreachable("the helper is off".into())) },
        )
    }

    fn lock(&self) -> BoxFuture<'_, Result<(), MacError>> {
        let mut script = self.script.lock().unwrap();
        let done = if let Some(why) = script.refusal.clone() {
            Err(MacError::Refused(why))
        } else {
            script.locks += 1;
            Ok(())
        };
        Box::pin(async move { done })
    }
}

#[tokio::test]
pub(crate) async fn the_mac_row_is_offline_until_the_helper_answers() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;

    let offline = app.call(Call::get(paths::MAC).token(&session)).await;
    assert_eq!(offline.status, 200);
    assert_eq!(offline.json(), json!({ "online": false }));

    app.mac.answers(MacHealth {
        locked: true,
        idle: Some(312.5),
    });
    let online = app.call(Call::get(paths::MAC).token(&session)).await;
    assert_eq!(
        online.json(),
        json!({ "online": true, "locked": true, "idle": 312.5 })
    );
    assert_eq!(
        online.read_as::<MacStatus>(),
        MacStatus {
            online: true,
            locked: Some(true),
            idle: Some(312.5)
        }
    );

    app.mac.answers(MacHealth {
        locked: false,
        idle: None,
    });
    let quiet = app.call(Call::get(paths::MAC).token(&session)).await;
    assert_eq!(
        quiet.json(),
        json!({ "online": true, "locked": false, "idle": null })
    );
}

#[tokio::test]
pub(crate) async fn lock_now_locks_the_mac() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let locked = app.call(Call::post(paths::MAC_LOCK).token(&session)).await;
    assert_eq!(locked.status, 200);
    assert_eq!(locked.json(), json!({ "ok": true }));
    assert_eq!(app.mac.locks(), 1);
}

#[tokio::test]
pub(crate) async fn a_lock_the_helper_refuses_is_400_with_its_words() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.refuses("The screen can't lock during a call");
    let refused = app.call(Call::post(paths::MAC_LOCK).token(&session)).await;
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.json(),
        json!({ "error": "The screen can't lock during a call" })
    );
    assert_eq!(app.mac.locks(), 0);
}

#[tokio::test]
pub(crate) async fn the_mac_needs_a_session_when_sign_in_is_enforced() {
    let app = TestApp::new().await;
    app.mac.answers(MacHealth {
        locked: false,
        idle: Some(1.0),
    });
    for call in [Call::get(paths::MAC), Call::post(paths::MAC_LOCK)] {
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(refused.json()["signIn"], true);
    }
    assert_eq!(app.mac.locks(), 0);

    let report = TestApp::with_mode(AuthMode::Report).await;
    let row = report.call(Call::get(paths::MAC)).await;
    assert_eq!(row.status, 200, "report mode lets it through, noted");
}

/// What a helper served here was sent: (path, authorization, body).
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A helper on `127.0.0.1`: `/health` answers `health`; `/lock` refuses with 409 when
/// `refuse` is set.
async fn serve_helper(health: Value, refuse: bool) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let note = |seen: &Seen, path: &str, headers: &HeaderMap, body: Value| {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        seen.lock().unwrap().push((path.to_owned(), auth, body));
    };
    let on_health = {
        let seen = Arc::clone(&seen);
        move |headers: HeaderMap, Json(body): Json<Value>| async move {
            note(&seen, "/health", &headers, body);
            Json(health)
        }
    };
    let on_lock = {
        let seen = Arc::clone(&seen);
        move |headers: HeaderMap, Json(body): Json<Value>| async move {
            note(&seen, "/lock", &headers, body);
            if refuse {
                (
                    StatusCode::CONFLICT,
                    Json(json!({ "error": "A call is on" })),
                )
            } else {
                (StatusCode::OK, Json(json!({ "text": "Locked" })))
            }
        }
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "a stand-in helper for the test, not a route of the server's document"
    )]
    let helper = axum::Router::new()
        .route("/health", axum::routing::post(on_health))
        .route("/lock", axum::routing::post(on_lock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, helper).await });
    (format!("http://{address}"), seen)
}

#[tokio::test]
async fn the_http_helper_sends_its_token_and_reads_the_health() {
    let (url, seen) = serve_helper(json!({ "locked": 1, "idle": 42, "name": "Mac" }), false).await;
    let helper = HttpMacHelper::new(url.parse().unwrap(), "helper-token".to_owned()).unwrap();
    let health = helper.health().await.unwrap();
    assert_eq!(
        health,
        MacHealth {
            locked: true,
            idle: Some(42.0)
        }
    );
    helper.lock().await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (
                "/health".to_owned(),
                "Bearer helper-token".to_owned(),
                json!({})
            ),
            (
                "/lock".to_owned(),
                "Bearer helper-token".to_owned(),
                json!({})
            ),
        ]
    );
}

#[tokio::test]
async fn the_http_helper_passes_on_a_refusal_and_an_absence() {
    let (url, _seen) = serve_helper(json!({}), true).await;
    let helper = HttpMacHelper::new(url.parse().unwrap(), "t".to_owned()).unwrap();
    let refused = helper.lock().await.unwrap_err();
    assert_eq!(refused.to_string(), "A call is on");

    // A port nothing listens on: bound, then let go.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let gone =
        HttpMacHelper::new(format!("http://{address}").parse().unwrap(), "t".to_owned()).unwrap();
    assert!(matches!(gone.health().await, Err(MacError::Unreachable(_))));
    assert!(matches!(gone.lock().await, Err(MacError::Unreachable(_))));
}

/// What the extension's helper served here was sent: (path, authorization, content type, size).
type SeenZips = Arc<Mutex<Vec<(String, String, String, usize)>>>;

/// A helper on `127.0.0.1` that keeps the extension's folder: `GET /extension` answers its
/// version; `POST /extension/install` takes a zip (refusing one that isn't sent as a zip) and
/// answers the version it unpacked.
async fn serve_extension_helper() -> (String, SeenZips) {
    let seen: SeenZips = Arc::default();
    let header = |headers: &HeaderMap, name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let on_folder = {
        let seen = Arc::clone(&seen);
        move |headers: HeaderMap| async move {
            let auth = header(&headers, "authorization");
            seen.lock()
                .unwrap()
                .push((helper_paths::EXTENSION.to_owned(), auth, String::new(), 0));
            Json(json!({ "version": "0.1.0", "path": "/Emberhut/Extension" }))
        }
    };
    let on_install = {
        let seen = Arc::clone(&seen);
        move |headers: HeaderMap, body: axum::body::Bytes| async move {
            let (auth, kind) = (
                header(&headers, "authorization"),
                header(&headers, "content-type"),
            );
            seen.lock().unwrap().push((
                helper_paths::EXTENSION_INSTALL.to_owned(),
                auth,
                kind.clone(),
                body.len(),
            ));
            if kind == "application/zip" {
                (
                    StatusCode::OK,
                    Json(json!({ "version": "0.2.0", "path": "/Emberhut/Extension" })),
                )
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "Not a zip" })),
                )
            }
        }
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "a stand-in helper for the test, not a route of the server's document"
    )]
    let helper = axum::Router::new()
        .route(helper_paths::EXTENSION, axum::routing::get(on_folder))
        .route(
            helper_paths::EXTENSION_INSTALL,
            axum::routing::post(on_install),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, helper).await });
    (format!("http://{address}"), seen)
}

#[tokio::test]
async fn the_http_helper_reads_the_extension_s_folder_and_sends_it_the_zip() {
    let (url, seen) = serve_extension_helper().await;
    let helper = HttpMacHelper::new(url.parse().unwrap(), "helper-token".to_owned()).unwrap();
    let folder = helper.extension().await.unwrap();
    assert_eq!(folder.version.as_deref(), Some("0.1.0"));
    let installed = helper.install_extension(vec![7; 1024]).await.unwrap();
    assert_eq!(
        installed,
        ExtensionFolder {
            version: Some("0.2.0".to_owned()),
            path: Some("/Emberhut/Extension".to_owned())
        }
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (
                "/extension".to_owned(),
                "Bearer helper-token".to_owned(),
                String::new(),
                0
            ),
            (
                "/extension/install".to_owned(),
                "Bearer helper-token".to_owned(),
                "application/zip".to_owned(),
                1024
            ),
        ]
    );
    let big = helper
        .install_extension(vec![0; helper_paths::EXTENSION_ZIP_MAX + 1])
        .await
        .unwrap_err();
    assert!(big.to_string().contains("over 20 MB"), "{big}");
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "a zip over the cap is never sent"
    );
    let none = emberhut_server::services::mac::NoMacHelper;
    assert!(matches!(none.extension().await, Err(MacError::NotSetUp)));
}
