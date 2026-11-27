//! `GET /api/connect/probe`: the machine's signed answer, bound to the address and host it was
//! reached at, checked as a device checks it (emberhut-api's `ProbeAnswer::verify`, then the
//! `VerifiedProbe`'s binding) and against the contract's shared vectors
//! (`crates/emberhut-api/tests/fixtures/connect/probes.json`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use emberhut_api::connect::{PROBE, ProbeAnswer, Route, RouteKind, SshForward};
use emberhut_api::error::ErrorBody;
use emberhut_identity::{MachineId, Nonce, ProbeKind, SecretKey};
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
use emberhut_server::identity::Machine;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::common::{Answer, Call, REQUEST_TIMEOUT, TestApp, fixture, test_machine};

/// A fresh nonce's worth of bytes, as a probe sends them.
const NONCE: [u8; 32] = [9; 32];

/// The probe's query for `nonce` and `kind`.
fn query(nonce: Nonce, kind: &str) -> String {
    format!("nonce={nonce}&kind={kind}")
}

/// A direct probe from a device: accepted on `local`, asking for `host`.
fn probe(nonce: Nonce, kind: &str, local: &str, host: &str) -> Call {
    Call::get(PROBE)
        .query(&query(nonce, kind))
        .local(local)
        .header("host", host)
}

/// A route of `kind` at `url`.
fn route(kind: RouteKind, url: &str) -> Route {
    Route {
        kind,
        url: url.to_owned(),
        label: None,
        up: true,
    }
}

/// The vectors' machine: the key `keys.json` names `machine`.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper reads the vectors: a failure there fails the test"
)]
fn vector_machine() -> Machine {
    let keys = fixture("connect/keys.json");
    let entry = keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["name"] == "machine")
        .unwrap();
    let seed = entry["seed"].as_str().unwrap();
    let bytes: Vec<u8> = (0..seed.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&seed[at..at + 2], 16).unwrap())
        .collect();
    Machine::new(
        SecretKey::from_bytes(&bytes.try_into().unwrap()),
        "Vector machine".parse().unwrap(),
    )
}

/// An app whose identity is `machine`, its links pointing at `public_url`.
async fn app_of(machine: Machine, public_url: &str) -> TestApp {
    TestApp::with_dependencies(AuthMode::Enforce, |dependencies, _| Dependencies {
        machine: Some(Arc::new(machine)),
        public_url: public_url.to_owned(),
        ..dependencies
    })
    .await
}

#[tokio::test]
pub(crate) async fn the_machine_signs_each_direct_answer_as_the_vectors_have_it() {
    let app = app_of(vector_machine(), "http://127.0.0.1:18990").await;
    let machine: MachineId = vector_machine().id();
    let vectors = fixture("connect/probes.json");
    let nonce: Nonce = vectors["nonce"].as_str().unwrap().parse().unwrap();
    let mut answers = BTreeMap::new();
    for (name, named) in vectors["answers"].as_object().unwrap() {
        let expected = &named["answer"];
        if expected["via"] != "direct" {
            // Through the relay, the sealed handshake binds the route instead of the probe.
            continue;
        }
        let kind = named["kind"].as_str().unwrap();
        let local = expected["local"].as_str().unwrap();
        let host = expected["host"].as_str().unwrap();
        // No session: the probe comes before a device sends any token.
        let answer = app.call(probe(nonce, kind, local, host)).await;
        assert_eq!(answer.status, 200, "{name}: {}", answer.text);
        assert_eq!(answer.header("access-control-allow-origin"), "*");
        assert_eq!(
            &answer.json(),
            expected,
            "{name}: the vector's answer, signature and all"
        );
        let signed: ProbeAnswer = answer.read_as();
        let kind: ProbeKind = kind.parse().unwrap();
        answers.insert(name.clone(), signed.verify(&machine, nonce, &kind).unwrap());
    }
    // The device's binding check, over the server's own answers.
    for case in vectors["bindings"].as_array().unwrap() {
        let Some(verified) = answers.get(case["answer"].as_str().unwrap()) else {
            continue;
        };
        let route: Route = serde_json::from_value(case["route"].clone()).unwrap();
        assert_eq!(
            Some(verified.binding_matches(&route)),
            case["matches"].as_bool(),
            "{case}"
        );
    }
    for case in vectors["sshBindings"].as_array().unwrap() {
        let Some(verified) = answers.get(case["answer"].as_str().unwrap()) else {
            continue;
        };
        let port = |name: &str| u16::try_from(case["forward"][name].as_u64().unwrap()).unwrap();
        let forward = SshForward {
            local_port: port("localPort"),
            machine_port: port("machinePort"),
        };
        assert_eq!(
            Some(verified.forward_matches(forward)),
            case["matches"].as_bool(),
            "{case}"
        );
    }
}

#[tokio::test]
pub(crate) async fn a_probe_signs_its_address_and_host_in_their_normal_form() {
    let app = TestApp::with_machine().await;
    let machine = test_machine().id();
    let nonce = Nonce::from_bytes(NONCE);
    let lan: ProbeKind = "lan".parse().unwrap();

    // An IPv4-mapped local address is its IPv4 address; a name is lowercase, without the
    // scheme's default port.
    let answer: ProbeAnswer = app
        .call(probe(
            nonce,
            "lan",
            "[::ffff:192.0.2.10]:18990",
            "Box.Example:80",
        ))
        .await
        .read_as();
    let verified = answer.verify(&machine, nonce, &lan).unwrap();
    let statement = verified.statement();
    assert_eq!(statement.host().as_str(), "box.example");
    assert_eq!(
        statement.reached().local().unwrap().to_string(),
        "192.0.2.10:18990"
    );
    assert!(verified.binding_matches(&route(RouteKind::Lan, "http://192.0.2.10:18990")));
    assert!(
        !verified.binding_matches(&route(RouteKind::Lan, "http://192.0.2.11:18990")),
        "a remembered address forwarded to this one doesn't answer"
    );
    // Another nonce or kind doesn't verify: the answer is for this probe only.
    assert!(
        answer
            .verify(&machine, Nonce::from_bytes([8; 32]), &lan)
            .is_err()
    );
    assert!(
        answer
            .verify(&machine, nonce, &"vpn".parse().unwrap())
            .is_err()
    );

    // HTTP/2's `:authority` (an absolute target) is the host, before any `Host` header.
    let request = Request::get(format!(
        "http://192.0.2.10:18990{PROBE}?{}",
        query(nonce, "lan")
    ))
    .header("host", "elsewhere.example")
    .extension(axum::extract::ConnectInfo(
        "192.0.2.20:50000".parse::<std::net::SocketAddr>().unwrap(),
    ))
    .extension(emberhut_server::peer::DirectTcp {
        local: Some("192.0.2.10:18990".parse().unwrap()),
    })
    .body(Body::empty())
    .unwrap();
    let answer = Answer::read(app.send(request).await).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let answer: ProbeAnswer = answer.read_as();
    assert_eq!(answer.host.as_str(), "192.0.2.10:18990");

    // Behind a TLS proxy (an `https` public URL), the host is read as `https`'s: 443 is the
    // default, 80 is not.
    let proxied = app_of(test_machine(), "https://box.example").await;
    let public: ProbeKind = "public".parse().unwrap();
    for (sent, signed) in [
        ("box.example:443", "box.example"),
        ("box.example:80", "box.example:80"),
    ] {
        let answer: ProbeAnswer = proxied
            .call(probe(nonce, "public", "127.0.0.1:18990", sent))
            .await
            .read_as();
        let verified = answer.verify(&machine, nonce, &public).unwrap();
        assert_eq!(verified.statement().host().as_str(), signed, "{sent}");
    }
    let answer: ProbeAnswer = proxied
        .call(probe(nonce, "public", "127.0.0.1:18990", "box.example"))
        .await
        .read_as();
    let verified = answer.verify(&machine, nonce, &public).unwrap();
    assert!(verified.binding_matches(&route(RouteKind::Public, "https://box.example")));
}

#[tokio::test]
pub(crate) async fn a_probe_that_does_not_read_is_400() {
    let app = TestApp::with_machine().await;
    let nonce = Nonce::from_bytes(NONCE);
    let refused = |call: Call| {
        let app = &app;
        async move {
            let answer = app.call(call).await;
            assert_eq!(answer.status, 400, "{}", answer.text);
            assert_eq!(answer.header("access-control-allow-origin"), "*");
            assert_ne!(answer.read_as::<ErrorBody>().error, "");
        }
    };
    let local = "192.0.2.10:18990";
    let host = "192.0.2.10:18990";
    for query in [
        String::new(),
        format!("nonce={nonce}"),
        "kind=lan".to_owned(),
        "nonce=AAAA&kind=lan".to_owned(),
        format!("nonce={nonce}=&kind=lan"),
        format!("nonce={nonce}&kind=LAN"),
        format!("nonce={nonce}&kind={}", "a".repeat(33)),
        format!("nonce={nonce}&kind="),
    ] {
        refused(
            Call::get(PROBE)
                .query(&query)
                .local(local)
                .header("host", host),
        )
        .await;
    }
    // A host the probe can't sign: none, an address in a short form, user information.
    refused(Call::get(PROBE).query(&query(nonce, "lan")).local(local)).await;
    for host in [
        "127.1",
        "user@box.example.com",
        "box.example:0",
        "box..example",
    ] {
        refused(probe(nonce, "lan", local, host)).await;
    }
}

#[tokio::test]
pub(crate) async fn a_legacy_host_and_a_stopping_server_sign_no_probe() {
    let nonce = Nonce::from_bytes(NONCE);
    let call = || probe(nonce, "lan", "192.0.2.10:18990", "192.0.2.10:18990");

    let legacy = TestApp::new().await;
    let answer = legacy.call(call()).await;
    assert_eq!(answer.status, 404);
    assert_eq!(answer.header("access-control-allow-origin"), "*");

    let app = TestApp::with_machine().await;
    // A transport that doesn't say where it reached the server can't have its binding signed:
    // a fault, not an answer.
    let unmarked = Call::get(PROBE)
        .query(&query(nonce, "lan"))
        .header("host", "192.0.2.10:18990");
    assert_eq!(app.send(unmarked.request()).await.status(), 500);

    app.state.stopping().cancel();
    let answer = app.call(call()).await;
    assert_eq!(answer.status, 503);
    assert_eq!(answer.header("access-control-allow-origin"), "*");
}

/// Over a real socket: the address signed is the one the connection was accepted on.
#[tokio::test]
async fn over_a_socket_the_probe_is_bound_to_the_listener_s_address() {
    let app = TestApp::with_machine().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    let nonce = Nonce::from_bytes(NONCE);
    let answer = crate::common::http_client()
        .get(format!(
            "http://{address}{PROBE}?{}",
            query(nonce, "loopback")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 200);
    let answer: ProbeAnswer = serde_json::from_str(&answer.text().await.unwrap()).unwrap();
    let verified = answer
        .verify(&test_machine().id(), nonce, &"loopback".parse().unwrap())
        .unwrap();
    assert_eq!(
        verified.statement().reached().local().unwrap().get(),
        address
    );
    assert!(verified.binding_matches(&route(RouteKind::Loopback, &format!("http://{address}"))));
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
