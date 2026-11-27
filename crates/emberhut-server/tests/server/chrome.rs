//! The owner's Chrome profiles: pairing one with its link over the extension's real WebSocket
//! (served on `127.0.0.1`), its hello, the owner's pause, the browser tools driving it through
//! the extension's relay (the real `DevTools` engine, the extension scripted), the hand-off, a
//! task's own browser (`chrome_tasks`), `chrome_pair`, and the pages and the extension's zip; and
//! the extension's folder on the Mac kept in step with the zip through the Mac helper.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test helper's failure fails the test"
)]

use std::time::Duration;

use emberhut_api::chrome::extension as native;
use emberhut_api::paths;
use emberhut_core::session::Session;
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
use emberhut_server::services::chrome::FOLDER_EVERY;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::common::{Call, TestApp, text};

/// The app on a real socket: its extension WebSocket's address.
async fn served(app: &TestApp) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("ws://{address}{}", paths::CHROME_WS)
}

/// A scripted extension: it answers the relay's calls as Chrome would for one tab, and hands
/// the test everything else it is told.
struct Extension {
    out: mpsc::UnboundedSender<Message>,
    told: mpsc::UnboundedReceiver<Value>,
}

/// The extension's one tab: its address, title and tree.
const TAB_URL: &str = "https://example.com/";

fn ax_tree() -> Value {
    json!({ "nodes": [
        { "nodeId": "1", "role": { "value": "RootWebArea" }, "name": { "value": "Example" }, "childIds": ["2", "3"], "backendDOMNodeId": 1 },
        { "nodeId": "2", "parentId": "1", "role": { "value": "heading" }, "name": { "value": "Hello" }, "backendDOMNodeId": 2 },
        { "nodeId": "3", "parentId": "1", "role": { "value": "link" }, "name": { "value": "More" }, "backendDOMNodeId": 3 }
    ] })
}

/// What Chrome answers a relayed `DevTools` command in the scripted tab.
fn cdp_answer(method: &str, params: &Value) -> Value {
    match method {
        "Runtime.evaluate" => {
            let expression = params["expression"].as_str().unwrap_or_default();
            let value = if expression.contains("location.href") {
                json!([TAB_URL, "Example"])
            } else if expression.contains("document.readyState") {
                json!("complete")
            } else if expression.contains("contentType") {
                json!(false)
            } else if expression.contains("querySelectorAll(\"input\")")
                || expression.contains("createTreeWalker")
            {
                json!([])
            } else {
                Value::Null
            };
            json!({ "result": { "type": "object", "value": value } })
        }
        "Accessibility.getFullAXTree" => ax_tree(),
        "Target.getTargetInfo" => {
            json!({ "targetInfo": { "url": TAB_URL, "title": "Example", "targetId": "T1" } })
        }
        "Page.captureScreenshot" => json!({ "data": "/9j/" }),
        _ => json!({}),
    }
}

impl Extension {
    async fn connect(url: &str) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let (mut sink, mut stream) = socket.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let (tell, told) = mpsc::unbounded_channel::<Value>();
        let answers = out.clone();
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let message = match message {
                    Message::Text(text) => serde_json::from_str::<Value>(text.as_str()).unwrap(),
                    Message::Close(frame) => {
                        let reason = frame.map_or_default(|f| f.reason.to_string());
                        drop(tell.send(json!({ "closed": reason })));
                        break;
                    }
                    _ => continue,
                };
                if let Some(id) = message.get("id").cloned() {
                    let p = &message["params"];
                    let result = match message["method"].as_str().unwrap_or_default() {
                        "chrome.debugger.sendCommand" => {
                            cdp_answer(p[1].as_str().unwrap_or_default(), &p[2])
                        }
                        "chrome.tabs.create" => json!({ "id": 6, "url": p[0]["url"] }),
                        _ => json!({}),
                    };
                    let answer = json!({ "id": id, "result": result });
                    drop(answers.send(Message::text(answer.to_string())));
                    continue;
                }
                if message["method"] == native::START {
                    let created = json!({ "method": "chrome.tabs.onCreated", "params": [{ "id": 5, "url": TAB_URL, "title": "Example" }] });
                    drop(answers.send(Message::text(created.to_string())));
                    let ready = json!({ "method": "extension.initialized", "params": {} });
                    drop(answers.send(Message::text(ready.to_string())));
                }
                drop(tell.send(message));
            }
        });
        Self { out, told }
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "each message, made at its call"
    )]
    fn send(&self, message: Value) {
        self.out.send(Message::text(message.to_string())).unwrap();
    }

    /// The next message the server tells it that `method` names (others are skipped).
    async fn told(&mut self, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = self.told.recv().await.unwrap();
                if message["method"] == method
                    || (method == "closed" && message.get("closed").is_some())
                {
                    return message;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the extension was never told {method}"))
    }

    /// Checks the server tells it nothing `method` names for a moment.
    async fn never_told(&mut self, method: &str) {
        let wait = tokio::time::sleep(Duration::from_millis(300));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                () = &mut wait => return,
                message = self.told.recv() => {
                    let message = message.unwrap();
                    assert_ne!(message["method"], method, "the extension was told {message}");
                }
            }
        }
    }
}

/// Pairs a profile named `label` over the socket at `url`: its token.
async fn pair(app: &TestApp, session: &Session, url: &str, label: &str) -> String {
    let added = app
        .call(
            Call::post(paths::CHROME)
                .token(session)
                .json(&json!({ "kind": "add", "label": label })),
        )
        .await;
    assert_eq!(added.status, 200, "{}", added.text);
    let code = added.json()["pairing"]["code"].as_str().unwrap().to_owned();
    let mut extension = Extension::connect(url).await;
    extension.send(json!({ "method": "pair", "params": { "protocol": native::PROTOCOL, "code": code, "version": "0.6.5", "email": "me@example.com", "os": "mac" } }));
    let paired = extension.told(native::PAIRED).await;
    assert_eq!(paired["params"]["label"], label);
    paired["params"]["token"].as_str().unwrap().to_owned()
}

/// Says hello with `token`: the connected extension.
async fn hello(url: &str, token: &str) -> Extension {
    let mut extension = Extension::connect(url).await;
    extension.send(json!({ "method": "hello", "params": { "protocol": native::PROTOCOL, "token": token, "version": "0.6.5" } }));
    let said = extension.told(native::HELLO).await;
    assert_eq!(said["params"]["name"], "work");
    extension
}

async fn settle_until(app: &TestApp, session: &Session, check: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..50 {
        let state = app
            .call(Call::get(paths::STATE).token(session))
            .await
            .json();
        if check(&state) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the state never got there");
}

#[tokio::test]
pub(crate) async fn a_profile_pairs_with_its_link_into_chrome_devices_and_says_hello_with_its_token()
 {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let added = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "add", "label": "Work" })),
        )
        .await;
    let code = added.json()["pairing"]["code"].as_str().unwrap().to_owned();
    assert!(
        added.json()["pairing"]["link"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/chrome/pair#{code}"))
    );
    assert_eq!(added.json()["chromePairing"][0]["label"], "Work");
    let waiting = app
        .call(
            Call::get(paths::CHROME_PAIR_STATUS)
                .query(&format!("code={code}"))
                .token(&session),
        )
        .await;
    assert_eq!(
        waiting.json(),
        json!({ "state": "waiting", "label": "Work" })
    );

    let mut extension = Extension::connect(&url).await;
    extension.send(json!({ "method": "pair", "params": { "code": code, "version": "0.6.5", "email": "me@example.com" } }));
    let token = extension.told(native::PAIRED).await["params"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let paired = app
        .call(Call::get(paths::CHROME_PAIR_STATUS).query(&format!("code={code}")))
        .await;
    assert_eq!(paired.json()["state"], "paired");
    assert_eq!(paired.json()["email"], "me@example.com");
    let mut used = Extension::connect(&url).await;
    used.send(json!({ "method": "pair", "params": { "code": code } }));
    assert!(
        used.told("closed").await["closed"]
            .as_str()
            .unwrap()
            .contains("used or expired")
    );

    let mut stranger = Extension::connect(&url).await;
    stranger.send(json!({ "method": "hello", "params": { "token": "not-a-token" } }));
    assert!(
        stranger.told("closed").await["closed"]
            .as_str()
            .unwrap()
            .contains("isn't paired")
    );

    let _extension = hello(&url, &token).await;
    let state = settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await;
    assert_eq!(state["chrome"][0]["name"], "work");
    assert_eq!(state["chrome"][0]["version"], "0.6.5");
    assert_eq!(state["chrome"][0]["grant"]["by"], "paired");
    assert_eq!(state["chromeLatest"], "0");
    let rows = app.state.chrome().summary().await.unwrap();
    assert_eq!(rows.len(), 1, "one row in chrome_devices");
}

#[tokio::test]
pub(crate) async fn the_owner_s_pause_holds_the_tools_back_and_the_tools_drive_the_tab_through_the_extension()
 {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair(&app, &session, &url, "Work").await;
    let mut extension = hello(&url, &token).await;
    let id = settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await["chrome"]
        [0]["id"]
        .clone();

    let paused = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "pause", "id": id, "minutes": 5 })),
        )
        .await;
    assert_eq!(paused.status, 200);
    assert_eq!(
        extension.told(native::PAUSE).await["params"]["minutes"],
        5.0
    );
    let until = jiff::Timestamp::now().as_millisecond() + 300_000;
    extension.send(
        json!({ "method": native::STOPPED, "params": { "why": "you paused it", "until": until } }),
    );
    settle_until(&app, &session, |s| {
        s["chrome"][0]["paused"]["why"] == "you paused it"
    })
    .await;
    let held = app
        .call_tool("browser_snapshot", json!({ "browser": "chrome:work" }))
        .await;
    assert!(
        text(&held)
            .starts_with("The owner paused the assistant in their Work Chrome (you paused it)"),
        "{}",
        text(&held)
    );
    extension.send(json!({ "method": native::STOPPED, "params": null }));
    extension.told(native::RESUME).await;
    settle_until(&app, &session, |s| s["chrome"][0]["paused"].is_null()).await;

    let snapshot = app
        .call_tool("browser_snapshot", json!({ "browser": "chrome:work" }))
        .await;
    let tree = text(&snapshot);
    assert!(tree.starts_with("Example · https://example.com/"), "{tree}");
    assert!(
        tree.contains("- heading \"Hello\" [ref=e1]\n- link \"More\" [ref=e2]"),
        "{tree}"
    );

    let handed = app
        .call_tool(
            "browser_handoff",
            json!({ "browser": "chrome:work", "reason": "Sign in", "task_id": 21 }),
        )
        .await;
    assert!(
        text(&handed).starts_with("The owner has their Work Chrome now"),
        "{}",
        text(&handed)
    );
    assert_eq!(
        extension.told(native::HANDOFF).await["params"]["reason"],
        "Sign in"
    );
    assert_eq!(
        app.desk.action(21).as_deref(),
        Some("In your Work Chrome: Sign in")
    );
    let busy = app
        .call_tool("browser_snapshot", json!({ "browser": "chrome:work" }))
        .await;
    assert!(
        text(&busy).starts_with("The owner has their Work Chrome (Sign in)"),
        "{}",
        text(&busy)
    );
    extension.send(json!({ "method": native::HANDBACK, "params": {} }));
    settle_until(&app, &session, |s| s["chrome"][0]["control"] == "agent").await;
    assert!(app.desk.wakes().iter().any(|(kind, said)| kind == "browser"
        && said.contains("handed their Work Chrome back (you asked: Sign in) for task 21")));
    assert_eq!(app.desk.action(21), None);
    let offline = app
        .call_tool("browser_snapshot", json!({ "browser": "chrome:home" }))
        .await;
    assert!(
        text(&offline).starts_with("No Chrome profile \"home\""),
        "{}",
        text(&offline)
    );
}

#[tokio::test]
pub(crate) async fn a_task_s_own_browser_in_chrome_tasks_is_where_its_tools_act() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair(&app, &session, &url, "Work").await;
    let _extension = hello(&url, &token).await;
    settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await;
    let said = app
        .state
        .chrome()
        .set_task_browser(21, "chrome:work")
        .await
        .unwrap();
    assert_eq!(said, "their Work Chrome (chrome:work)");
    let state = settle_until(&app, &session, |s| s["chrome"][0]["tasks"] == json!([21])).await;
    assert_eq!(state["chrome"][0]["tasks"], json!([21]));
    let snapshot = app
        .call_tool("browser_snapshot", json!({ "task_id": 21 }))
        .await;
    assert!(
        text(&snapshot).starts_with("Example · https://example.com/"),
        "{}",
        text(&snapshot)
    );
    assert!(
        app.state
            .chrome()
            .set_task_browser(22, "firefox")
            .await
            .is_err()
    );
    assert_eq!(
        app.state
            .chrome()
            .set_task_browser(22, "assistant")
            .await
            .unwrap(),
        "the server's own browser"
    );
}

#[tokio::test]
async fn chrome_pair_lists_links_renames_and_unpairs() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let none = app.call_tool("chrome_pair", json!({})).await;
    assert!(text(&none).starts_with("No Chrome profile of the owner's is paired yet"));
    let link = app
        .call_tool("chrome_pair", json!({ "profile": "Home" }))
        .await;
    assert!(
        text(&link).starts_with("Pairing link for their new Chrome profile \"Home\": "),
        "{}",
        text(&link)
    );
    let token = pair(&app, &session, &url, "Work").await;
    let mut extension = hello(&url, &token).await;
    settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await;
    let list = app.call_tool("chrome_pair", json!({})).await;
    assert!(
        text(&list).contains("- chrome:work: \"Work\", me@example.com, online, extension 0.6.5"),
        "{}",
        text(&list)
    );
    assert!(
        text(&list).contains("- waiting: \"Home\""),
        "{}",
        text(&list)
    );
    let renamed = app
        .call_tool(
            "chrome_pair",
            json!({ "profile": "chrome:work", "rename": "Office" }),
        )
        .await;
    assert_eq!(
        text(&renamed),
        "Renamed: now \"Office\", browser \"chrome:office\"."
    );
    assert_eq!(
        extension.told(native::NAME).await["params"]["name"],
        "office"
    );
    let gone = app
        .call_tool(
            "chrome_pair",
            json!({ "profile": "office", "unpair": true }),
        )
        .await;
    assert_eq!(text(&gone), "Unpaired their Office Chrome profile.");
    assert!(
        extension.told("closed").await["closed"]
            .as_str()
            .unwrap()
            .contains("unpaired")
    );
    // The socket the server closed is let go of: its goodbye runs.
    settle_until(&app, &session, |s| {
        s["did"].as_array().is_some_and(|did| {
            did.iter().any(|line| {
                line["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("The link to the owner's Office Chrome closed"))
            })
        })
    })
    .await;
}

#[tokio::test]
async fn a_first_message_over_a_few_kb_closes_the_socket_before_its_hello() {
    let app = TestApp::new().await;
    let url = served(&app).await;
    let mut big = Extension::connect(&url).await;
    big.send(json!({ "method": "hello", "params": { "token": "x".repeat(16 * 1024) } }));
    assert!(
        big.told("closed").await["closed"]
            .as_str()
            .unwrap()
            .contains("too big")
    );
}

#[tokio::test]
pub(crate) async fn the_app_s_chrome_input_refuses_what_it_can_t_do() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let unknown = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "fly" })),
        )
        .await;
    assert_eq!(unknown.status, 409);
    assert_eq!(unknown.json()["error"], "unknown fly");
    let bad = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "add", "label": "!!" })),
        )
        .await;
    assert_eq!(bad.status, 409);
    let offline = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "pause", "name": "work" })),
        )
        .await;
    assert_eq!(offline.json()["error"], "That Chrome isn't connected");
}

#[tokio::test]
pub(crate) async fn the_pairing_page_the_socket_without_an_upgrade_and_the_extension_s_zip() {
    let app = TestApp::new().await;
    let page = app.call(Call::get(paths::CHROME_PAIR)).await;
    assert_eq!(page.status, 200);
    assert!(page.text.contains("/chrome/pair/status?code="));
    assert!(
        page.text
            .contains("<h1>Pair this Chrome profile with Emberhut</h1>")
    );
    assert!(
        page.text
            .contains(&format!("href=\"{}\"", paths::CHROME_EXTENSION_ZIP))
    );
    let socket = app.call(Call::get(paths::CHROME_WS)).await;
    assert_eq!(socket.status, 400);
    let none = app.call(Call::get(paths::CHROME_EXTENSION_ZIP)).await;
    assert_eq!(none.status, 404);
    std::fs::write(
        app.dir().join("extension.zip"),
        zip_with_manifest(br#"{"version":"0.7.0"}"#),
    )
    .unwrap();
    let zip = app.call(Call::get(paths::CHROME_EXTENSION_ZIP)).await;
    assert_eq!(zip.status, 200);
    assert_eq!(zip.header("content-type"), "application/zip");
    assert_eq!(
        zip.header("content-disposition"),
        "attachment; filename=\"emberhut-extension-0.7.0.zip\""
    );
    let expired = app
        .call(Call::get(paths::CHROME_PAIR_STATUS).query("code=NOPE-NOPE"))
        .await;
    assert_eq!(expired.json(), json!({ "state": "expired" }));
}

/// The extension's manifest, as committed.
fn manifest() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/extension/manifest.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Pairs as the extension does: its pairing says only its code and protocol.
async fn pair_natively(app: &TestApp, session: &Session, url: &str, label: &str) -> String {
    let added = app
        .call(
            Call::post(paths::CHROME)
                .token(session)
                .json(&json!({ "kind": "add", "label": label })),
        )
        .await;
    let code = added.json()["pairing"]["code"].as_str().unwrap().to_owned();
    let mut extension = Extension::connect(url).await;
    // The extension's own pairing says only its code: what it is comes with its first hello.
    extension.send(
        json!({ "method": "pair", "params": { "protocol": native::PROTOCOL, "code": code } }),
    );
    let paired = extension.told(native::PAIRED).await;
    assert_eq!(paired["params"]["label"], label);
    paired["params"]["token"].as_str().unwrap().to_owned()
}

/// Says hello as `version` on the Mac.
async fn hello_natively(url: &str, token: &str, version: &str) -> Extension {
    hello_natively_as(url, token, version, "work").await
}

/// [`hello_natively`] from the profile named `name`.
async fn hello_natively_as(url: &str, token: &str, version: &str, name: &str) -> Extension {
    let mut extension = Extension::connect(url).await;
    extension.send(json!({ "method": "hello", "params": { "protocol": native::PROTOCOL, "token": token, "version": version, "os": "mac" } }));
    let said = extension.told(native::HELLO).await;
    assert_eq!(said["params"]["name"], name);
    extension
}

#[test]
fn the_extension_s_manifest_key_gives_its_fixed_id() {
    let manifest = manifest();
    assert_eq!(manifest["name"], "Emberhut");
    let key = manifest["key"].as_str().unwrap();
    assert_eq!(
        emberhut_core::chrome::extension_id(key).as_deref(),
        Some(native::ID)
    );
}

#[tokio::test]
async fn the_extension_pairs_pauses_hands_off_and_answers_pings() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    extension.send(json!({ "method": "ping", "params": {} }));
    extension.told("pong").await;
    let id = settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await["chrome"]
        [0]["id"]
        .clone();

    // A pause asked in the app goes as the extension's own word, whatever its version.
    let paused = app
        .call(
            Call::post(paths::CHROME)
                .token(&session)
                .json(&json!({ "kind": "pause", "id": id, "minutes": 3 })),
        )
        .await;
    assert_eq!(paused.status, 200);
    assert_eq!(
        extension.told(native::PAUSE).await["params"]["minutes"],
        3.0
    );
    let until = jiff::Timestamp::now().as_millisecond() + 180_000;
    extension.send(
        json!({ "method": native::STOPPED, "params": { "why": "you paused it", "until": until } }),
    );
    settle_until(&app, &session, |s| {
        s["chrome"][0]["paused"]["why"] == "you paused it"
    })
    .await;
    extension.send(json!({ "method": native::STOPPED, "params": null }));
    extension.told(native::RESUME).await;
    settle_until(&app, &session, |s| s["chrome"][0]["paused"].is_null()).await;

    let snapshot = app
        .call_tool("browser_snapshot", json!({ "browser": "chrome:work" }))
        .await;
    assert!(
        text(&snapshot).starts_with("Example · https://example.com/"),
        "{}",
        text(&snapshot)
    );
    let handed = app
        .call_tool(
            "browser_handoff",
            json!({ "browser": "chrome:work", "reason": "Sign in", "task_id": 21 }),
        )
        .await;
    assert!(text(&handed).starts_with("The owner has their Work Chrome now"));
    assert_eq!(
        extension.told(native::HANDOFF).await["params"]["reason"],
        "Sign in"
    );
    let took = app
        .call_tool(
            "browser_takeback",
            json!({ "browser": "chrome:work", "reason": "It can wait" }),
        )
        .await;
    assert!(
        text(&took).starts_with("Took their Work Chrome back"),
        "{}",
        text(&took)
    );
    let back = extension.told(native::HANDBACK).await;
    assert_eq!(back["params"]["by"], native::BY_ASSISTANT);
    app.call_tool(
        "browser_handoff",
        json!({ "browser": "chrome:work", "reason": "Pay", "task_id": 21 }),
    )
    .await;
    extension.told(native::HANDOFF).await;
    extension.send(json!({ "method": native::HANDBACK, "params": {} }));
    settle_until(&app, &session, |s| s["chrome"][0]["control"] == "agent").await;
    assert!(app.desk.wakes().iter().any(|(kind, said)| kind == "browser"
        && said.contains("handed their Work Chrome back (you asked: Pay) for task 21")));
}

#[tokio::test]
async fn a_profile_on_the_mac_behind_the_zip_is_brought_up_through_the_helper() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");

    // A zip whose key isn't the extension's is never installed.
    std::fs::write(
        app.dir().join("extension.zip"),
        zip_with_manifest(br#"{"version":"0.1.5","key":"AAAA"}"#),
    )
    .unwrap();
    let extension = hello_natively(&url, &token, "0.1.0").await;
    said(&app, &session, "the zip served isn't the extension").await;
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    drop(extension);

    let zip = our_zip(&app, "0.2.0");
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    assert_eq!(
        extension.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed(), std::slice::from_ref(&zip));

    // Told once: a profile that comes back still older loads the extension from elsewhere.
    drop(extension);
    let mut again = hello_natively(&url, &token, "0.1.0").await;
    again.never_told(native::UPDATE).await;
    said(&app, &session, "after it was told to reload 0.2.0").await;
    assert_eq!(app.mac.installed().len(), 1);
}

/// The extension's zip at `version`, with its real key, served from now on.
fn our_zip(app: &TestApp, version: &str) -> Vec<u8> {
    let mut manifest = manifest();
    manifest["version"] = json!(version);
    let zip = zip_with_manifest(manifest.to_string().as_bytes());
    // Moved into place whole: a round running meanwhile never reads half a zip.
    let part = app.dir().join("extension.zip.part");
    std::fs::write(&part, &zip).unwrap();
    std::fs::rename(part, app.dir().join("extension.zip")).unwrap();
    zip
}

/// Waits for the activity log to have a line saying `words`.
async fn said(app: &TestApp, session: &Session, words: &str) {
    settle_until(app, session, |s| {
        s["did"].as_array().is_some_and(|did| {
            did.iter()
                .any(|line| line["text"].as_str().is_some_and(|t| t.contains(words)))
        })
    })
    .await;
}

#[tokio::test]
async fn a_helper_that_installs_an_older_version_is_tried_three_times_and_no_reload_is_asked() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.1.0");
    our_zip(&app, "0.2.0");
    for round in 1..=4 {
        let mut extension = hello_natively(&url, &token, "0.1.0").await;
        if round <= 3 {
            said(
                &app,
                &session,
                "the Mac helper's folder holds 0.1.0 after the install",
            )
            .await;
        }
        extension.never_told(native::UPDATE).await;
        assert_eq!(app.mac.installed().len(), round.min(3), "round {round}");
        // A second hello inside 10 minutes tries nothing.
        drop(extension);
        let mut soon = hello_natively(&url, &token, "0.1.0").await;
        soon.never_told(native::UPDATE).await;
        assert_eq!(app.mac.installed().len(), round.min(3), "round {round}");
        drop(soon);
        app.clock.advance(jiff::SignedDuration::from_mins(11));
    }
}

#[tokio::test]
async fn a_folder_already_current_installs_nothing_and_its_profile_is_told_once() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.2.0"), "0.2.0");
    our_zip(&app, "0.2.0");
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    assert_eq!(
        extension.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    drop(extension);
    app.clock.advance(jiff::SignedDuration::from_mins(30));
    let mut still = hello_natively(&url, &token, "0.1.0").await;
    still.never_told(native::UPDATE).await;
    said(&app, &session, "loads the extension from another folder").await;
    let state = settle_until(&app, &session, |s| s["chrome"][0]["connected"] == true).await;
    assert_eq!(state["chrome"][0]["outdated"], true);
}

/// The server's own round over the extension's folder, started as `serve` starts it, every
/// [`FOLDER_EVERY`]; it ends when the token is cancelled.
fn keep_folder(app: &TestApp) -> (CancellationToken, JoinHandle<()>) {
    keep_folder_every(app, FOLDER_EVERY)
}

/// [`keep_folder`], a round every `every`.
fn keep_folder_every(app: &TestApp, every: Duration) -> (CancellationToken, JoinHandle<()>) {
    let stop = CancellationToken::new();
    let round = app.state.chrome().clone().keep_folder(every, stop.clone());
    (stop, tokio::spawn(round))
}

/// Rounds this far apart, in real time, for the tests whose rounds write activity lines (a
/// paused clock would jump past the database's own timeouts while it writes them); the tests
/// move the server's clock for the 10 minutes between two tries at one version.
const QUICK: Duration = Duration::from_millis(50);

/// Stops the round, which ends at once.
async fn stop_folder((stop, round): (CancellationToken, JoinHandle<()>)) {
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), round)
        .await
        .unwrap()
        .unwrap();
}

/// Waits until `check` holds: ten seconds at most (as many paused).
async fn until(what: &str, check: impl Fn() -> bool) {
    for _ in 0..1000 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

/// Waits for `n` more questions about the folder: with [`QUICK`] rounds, at least `n - 1` whole
/// rounds that started from here.
async fn rounds(app: &TestApp, n: usize) {
    let from = app.mac.asked();
    until("the rounds", || app.mac.asked() >= from + n).await;
}

/// With time paused: lets every task run until all of them wait (time moves only then).
async fn settle() {
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// How many activity lines say `words`.
async fn lines_saying(app: &TestApp, session: &Session, words: &str) -> usize {
    let state = app
        .call(Call::get(paths::STATE).token(session))
        .await
        .json();
    state["did"].as_array().map_or(0, |did| {
        did.iter()
            .filter(|line| line["text"].as_str().is_some_and(|t| t.contains(words)))
            .count()
    })
}

#[tokio::test]
async fn the_server_fills_an_empty_folder_at_start_and_then_leaves_it_alone() {
    let app = TestApp::new().await;
    app.mac.keeps_extension(None, "0.2.0");
    let zip = our_zip(&app, "0.2.0");
    let round = keep_folder_every(&app, QUICK);
    until("the empty folder is filled", || {
        app.mac.installed() == std::slice::from_ref(&zip)
    })
    .await;
    // The next rounds find the folder current and install nothing.
    rounds(&app, 3).await;
    assert_eq!(app.mac.installed().len(), 1);
    stop_folder(round).await;
}

#[tokio::test]
async fn an_older_folder_is_updated_and_the_mac_s_older_profiles_are_told_to_reload() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    let zip = our_zip(&app, "0.2.0");
    // The profile's own try finds the helper off.
    app.mac.turns_off(true);
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    until("the profile's try", || app.mac.asked() == 1).await;
    extension.never_told(native::UPDATE).await;

    app.mac.turns_off(false);
    let round = keep_folder(&app);
    assert_eq!(
        extension.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed(), std::slice::from_ref(&zip));
    said(
        &app,
        &session,
        "Brought the extension's folder on the Mac to 0.2.0",
    )
    .await;
    stop_folder(round).await;
}

#[tokio::test]
async fn an_offline_helper_installs_nothing_at_start_and_is_asked_again_next_round() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.keeps_extension(None, "0.2.0");
    our_zip(&app, "0.2.0");
    app.mac.turns_off(true);
    // Paused, every FOLDER_EVERY: these rounds write nothing.
    tokio::time::pause();
    let round = keep_folder(&app);
    until("the first round", || app.mac.asked() == 1).await;
    settle().await;
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    tokio::time::advance(FOLDER_EVERY).await;
    until("the next round", || app.mac.asked() == 2).await;
    settle().await;
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    assert!(!round.1.is_finished(), "the round goes on");
    stop_folder(round).await;
    tokio::time::resume();

    // Once it answers, a round fills the folder.
    app.mac.turns_off(false);
    let round = keep_folder_every(&app, QUICK);
    until("the folder is filled", || app.mac.installed().len() == 1).await;
    stop_folder(round).await;
    assert_eq!(lines_saying(&app, &session, "Couldn't bring").await, 0);
}

#[tokio::test]
async fn a_refusing_helper_is_retried_only_within_the_limit_and_its_refusal_said_once() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.keeps_extension(None, "0.2.0");
    app.mac.refuses_installs(Some("the disk is full"));
    our_zip(&app, "0.2.0");
    let round = keep_folder_every(&app, QUICK);
    // However many rounds run, a try at most every 10 minutes, and 3 in all.
    for (step, installs) in [1, 1, 2, 2, 3, 3, 3, 3].into_iter().enumerate() {
        if step > 0 {
            app.clock.advance(jiff::SignedDuration::from_mins(5));
        }
        until("the try", || app.mac.installed().len() >= installs).await;
        rounds(&app, 3).await;
        assert_eq!(
            app.mac.installed().len(),
            installs,
            "after {} minutes",
            step * 5
        );
    }
    stop_folder(round).await;
    said(&app, &session, "the disk is full").await;
    assert_eq!(lines_saying(&app, &session, "the disk is full").await, 1);
    // Out of tries: said once too, so a stuck version is never silent.
    assert_eq!(
        lines_saying(&app, &session, "0.2.0 was tried 3 times already").await,
        1
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_build_served_mid_run_gets_its_own_tries_and_is_installed() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    app.mac.refuses_installs(Some("the disk is full"));
    our_zip(&app, "0.2.0");
    let round = keep_folder_every(&app, QUICK);
    for installs in 1..=3 {
        until("the try", || app.mac.installed().len() == installs).await;
        app.clock.advance(jiff::SignedDuration::from_mins(10));
    }
    rounds(&app, 3).await;
    assert_eq!(app.mac.installed().len(), 3, "0.2.0 is out of tries");
    // A version out of tries isn't read again: an unreadable zip goes unnoticed.
    let path = app.dir().join("extension.zip");
    set_mode(&path, 0o000);
    rounds(&app, 3).await;
    set_mode(&path, 0o644);

    app.mac.refuses_installs(None);
    app.mac.keeps_extension(Some("0.1.0"), "0.10.0");
    let zip = our_zip(&app, "0.10.0");
    until("the new build is installed", || {
        app.mac.folder_version().as_deref() == Some("0.10.0")
    })
    .await;
    stop_folder(round).await;
    assert_eq!(app.mac.installed().len(), 4);
    assert_eq!(app.mac.installed().last(), Some(&zip));
    assert_eq!(lines_saying(&app, &session, "couldn't be read").await, 0);
}

/// Sets `path`'s permission bits.
#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn an_install_the_helper_never_answers_uses_a_try_and_is_said_once() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    app.mac.loses_installs();
    our_zip(&app, "0.2.0");
    let round = keep_folder(&app);
    said(&app, &session, "didn't answer the install").await;
    stop_folder(round).await;
    // At the same time the version waits; 10 minutes on it is tried again, and not said again.
    let round = keep_folder(&app);
    until("the round", || app.mac.asked() == 2).await;
    stop_folder(round).await;
    assert_eq!(app.mac.installed().len(), 1);
    app.clock.advance(jiff::SignedDuration::from_mins(10));
    let round = keep_folder(&app);
    until("the second install", || app.mac.installed().len() == 2).await;
    // Time for the round to say it, were it to say it again.
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop_folder(round).await;
    assert_eq!(
        lines_saying(&app, &session, "didn't answer the install").await,
        1
    );
}

#[tokio::test]
async fn a_profile_s_try_waits_while_the_round_installs_and_is_told_when_it_lands() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    let zip = our_zip(&app, "0.2.0");
    let gate = app.mac.holds_installs();
    let round = keep_folder(&app);
    until("the round's install", || app.mac.installed().len() == 1).await;

    // The profile's try finds the folder still older and the version's try under way.
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    until("the profile's try", || app.mac.asked() == 2).await;
    extension.never_told(native::UPDATE).await;
    assert_eq!(app.mac.installed(), std::slice::from_ref(&zip));

    gate.notify_one();
    assert_eq!(
        extension.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed().len(), 1);
    stop_folder(round).await;
}

#[tokio::test]
async fn a_profile_finding_another_s_install_under_way_is_told_when_it_lands() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let work = pair_natively(&app, &session, &url, "Work").await;
    let home = pair_natively(&app, &session, &url, "Home").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    our_zip(&app, "0.2.0");
    let gate = app.mac.holds_installs();
    let mut first = hello_natively(&url, &work, "0.1.0").await;
    until("the first profile's install", || {
        app.mac.installed().len() == 1
    })
    .await;
    let mut second = hello_natively_as(&url, &home, "0.1.0", "home").await;
    until("the second profile's try", || app.mac.asked() == 2).await;
    second.never_told(native::UPDATE).await;

    gate.notify_one();
    assert_eq!(
        first.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(
        second.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed().len(), 1);
}

#[tokio::test]
async fn the_round_tells_an_older_profile_once_the_folder_is_current() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    our_zip(&app, "0.2.0");
    app.mac.turns_off(true);
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    until("the profile's try", || app.mac.asked() == 1).await;

    // The folder got to 0.2.0 some other way: the round installs nothing, and tells.
    app.mac.keeps_extension(Some("0.2.0"), "0.2.0");
    app.mac.turns_off(false);
    let round = keep_folder(&app);
    assert_eq!(
        extension.told(native::UPDATE).await["params"]["version"],
        "0.2.0"
    );
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    stop_folder(round).await;
}

#[tokio::test]
async fn the_round_ends_without_a_zip_or_without_a_helper() {
    let no_zip = TestApp::with_dependencies(AuthMode::Enforce, |dependencies, _| Dependencies {
        chrome_extension_zip: None,
        ..dependencies
    })
    .await;
    no_zip.mac.keeps_extension(None, "0.2.0");
    let (_stop, round) = keep_folder(&no_zip);
    tokio::time::timeout(Duration::from_secs(5), round)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(no_zip.mac.asked(), 0);

    // The test helper keeps no folder unless told to: it says none is set up.
    let no_helper = TestApp::new().await;
    our_zip(&no_helper, "0.2.0");
    let (_stop, round) = keep_folder(&no_helper);
    tokio::time::timeout(Duration::from_secs(5), round)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(no_helper.mac.asked(), 1);
}

#[tokio::test]
async fn stopping_the_round_during_an_install_leaves_the_folder_alone() {
    let app = TestApp::new().await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    our_zip(&app, "0.2.0");
    let gate = app.mac.holds_installs();
    let round = keep_folder(&app);
    until("the round's install", || app.mac.installed().len() == 1).await;
    stop_folder(round).await;
    gate.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(app.mac.folder_version().as_deref(), Some("0.1.0"));
}

#[tokio::test]
async fn the_round_never_installs_a_zip_whose_key_gives_another_id() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.mac.keeps_extension(None, "0.2.0");
    std::fs::write(
        app.dir().join("extension.zip"),
        zip_with_manifest(br#"{"version":"0.2.0","key":"AAAA"}"#),
    )
    .unwrap();
    let round = keep_folder(&app);
    said(&app, &session, "the zip served isn't the extension").await;
    assert_eq!(app.mac.installed(), Vec::<Vec<u8>>::new());
    stop_folder(round).await;
}

#[tokio::test]
async fn the_round_and_a_profile_s_tries_share_one_limit() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let url = served(&app).await;
    let token = pair_natively(&app, &session, &url, "Work").await;
    app.mac.keeps_extension(Some("0.1.0"), "0.2.0");
    app.mac.refuses_installs(Some("the disk is full"));
    our_zip(&app, "0.2.0");

    // The round's try, then the profile's at the same time: one install.
    let round = keep_folder(&app);
    until("the round's install", || app.mac.installed().len() == 1).await;
    stop_folder(round).await;
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    until("the profile's try", || app.mac.asked() >= 2).await;
    extension.never_told(native::UPDATE).await;
    assert_eq!(app.mac.installed().len(), 1);
    drop(extension);

    // The profile's try 10 minutes on, then the round's at the same time: one install.
    app.clock.advance(jiff::SignedDuration::from_mins(11));
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    until("the profile's install", || app.mac.installed().len() == 2).await;
    extension.never_told(native::UPDATE).await;
    let asked = app.mac.asked();
    let round = keep_folder(&app);
    until("the round", || app.mac.asked() > asked).await;
    extension.never_told(native::UPDATE).await;
    stop_folder(round).await;
    assert_eq!(app.mac.installed().len(), 2);

    // The third and last, whichever asks.
    app.clock.advance(jiff::SignedDuration::from_mins(11));
    let round = keep_folder(&app);
    until("the round's install", || app.mac.installed().len() == 3).await;
    stop_folder(round).await;
    app.clock.advance(jiff::SignedDuration::from_mins(11));
    drop(extension);
    let mut extension = hello_natively(&url, &token, "0.1.0").await;
    extension.never_told(native::UPDATE).await;
    let round = keep_folder(&app);
    extension.never_told(native::UPDATE).await;
    stop_folder(round).await;
    assert_eq!(app.mac.installed().len(), 3);
}

/// A zip holding `manifest.json`, stored.
pub(crate) fn zip_with_manifest(manifest: &[u8]) -> Vec<u8> {
    let name = b"manifest.json";
    let size = u32::try_from(manifest.len()).unwrap();
    let name_len = u16::try_from(name.len()).unwrap();
    let mut zip = Vec::new();
    zip.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(name);
    zip.extend_from_slice(manifest);
    let central = u32::try_from(zip.len()).unwrap();
    zip.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&[0; 12]);
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(name);
    let central_len = u32::try_from(zip.len()).unwrap() - central;
    zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    zip.extend_from_slice(&central_len.to_le_bytes());
    zip.extend_from_slice(&central.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);
    zip
}
