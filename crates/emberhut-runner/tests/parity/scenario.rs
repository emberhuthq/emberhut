//! The scenario and the comparison.
#![expect(
    clippy::many_single_char_names,
    reason = "the two sides and their answers, side by side: a and b"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::Uri;
use futures_util::StreamExt as _;
use serde_json::{Map, Value, json};

type Heard = Arc<Mutex<Vec<(String, Value)>>>;

/// One runner under test.
struct Side {
    base: String,
    token: String,
    heard: Heard,
    http: reqwest::Client,
}

async fn listen(at: &str) -> Heard {
    let heard: Heard = Arc::default();
    let keep = Arc::clone(&heard);
    let app = axum::Router::new().fallback(move |uri: Uri, body: Bytes| {
        let keep = Arc::clone(&keep);
        async move {
            keep.lock().unwrap().push((
                uri.path().to_owned(),
                serde_json::from_slice(&body).unwrap_or(Value::Null),
            ));
            if uri.path() == "/api/runner/mcp" {
                r#"{"mcpServers":{}}"#
            } else {
                r#"{"ok":true}"#
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(at).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    heard
}

impl Side {
    async fn new(name: &str) -> Self {
        let var = |k: &str| std::env::var(format!("EMBERHUT_PARITY_{name}{k}")).unwrap();
        Self {
            base: var(""),
            token: var("_TOKEN"),
            heard: listen(&var("_HEARD")).await,
            http: emberhut_runner::client::client().unwrap(),
        }
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        // Once more on a dropped connection: a runner may close an idle kept-alive one just as
        // it is reused.
        for again in [true, false] {
            let mut r = self
                .http
                .request(method.parse().unwrap(), format!("{}{path}", self.base))
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(60));
            if let Some(b) = &body {
                r = r.json(b);
            }
            match r.send().await {
                Ok(r) => return (r.status().as_u16(), r.json().await.unwrap_or(Value::Null)),
                Err(e) if again && e.is_request() => {}
                Err(e) => panic!("{method} {path} on {}: {e}", self.base),
            }
        }
        panic!("the second try returns or panics")
    }

    async fn put(&self, path: &str, bytes: &'static [u8]) -> (u16, Value) {
        let r = self
            .http
            .put(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    /// Waits until the session's turns have all ended.
    async fn settled(&self, id: &str) {
        for _ in 0..240 {
            let (_, t) = self
                .call("GET", &format!("/sessions/{id}/transcript?limit=5"), None)
                .await;
            let status = t["session"]["status"].as_str().unwrap_or_default();
            let busy = t["session"]["running"] == true
                || matches!(status, "running" | "queued")
                || t["session"]["queued"].as_u64().unwrap_or(0) > 0;
            if !busy
                && t["events"]
                    .as_array()
                    .and_then(|e| e.last())
                    .is_some_and(|e| e["kind"] == "end")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("{id} on {} never settled", self.base);
    }

    async fn live(&self, path: &str) -> Vec<Value> {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let mut body = r.bytes_stream();
        let mut text = String::new();
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(until, body.next()).await {
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
        text.split("\n\n")
            .filter_map(|f| f.strip_prefix("data: "))
            .filter_map(|d| serde_json::from_str(d).ok())
            .collect()
    }

    fn events(&self) -> Vec<Value> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == "/api/runner/events")
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// A value's shape: its keys and JSON types, every element of a list merged into one.
fn shape(v: &Value) -> Value {
    match v {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!("int"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(a) => {
            let mut merged = Value::Null;
            for x in a {
                merged = merge(&merged, &shape(x));
            }
            json!([merged])
        }
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), shape(v))).collect()),
    }
}

fn merge(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Null, x) | (x, Value::Null) => x.clone(),
        (Value::Object(x), Value::Object(y)) => {
            let mut m = x.clone();
            for (k, v) in y {
                let merged = m.get(k).map_or_else(|| v.clone(), |old| merge(old, v));
                m.insert(k.clone(), merged);
            }
            Value::Object(m)
        }
        (x, y) if x == y => x.clone(),
        (x, y) => json!(format!(
            "{} | {}",
            x.as_str().unwrap_or("?"),
            y.as_str().unwrap_or("?")
        )),
    }
}

/// Where two shapes differ, as `path: a | b` lines. `null` against a type is a difference too,
/// marked as one the reader may accept.
fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x
                .keys()
                .chain(y.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let p = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => diff(&p, u, v, out),
                    (Some(_), None) => out.push(format!("{p}: only in A")),
                    (None, Some(_)) => out.push(format!("{p}: only in B")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => diff(
            &format!("{path}[]"),
            x.first().unwrap_or(&Value::Null),
            y.first().unwrap_or(&Value::Null),
            out,
        ),
        (x, y) if x == y => {}
        (x, y) if x == "null" || y == "null" => {
            out.push(format!("{path}: {x} | {y} (null on one side)"));
        }
        (x, y) => out.push(format!("{path}: {x} | {y}")),
    }
}

/// The calls, the same on both sides; what each answered, by step.
#[expect(
    clippy::too_many_lines,
    reason = "the scenario, one call after the other"
)]
async fn play(s: &Side, work: &str) -> BTreeMap<String, (u16, Value)> {
    let mut got = BTreeMap::new();
    let enc = |p: &str| form_urlencoded::byte_serialize(p.as_bytes()).collect::<String>();
    got.insert("01 health".into(), s.call("GET", "/health", None).await);
    got.insert("02 accounts".into(), s.call("GET", "/accounts", None).await);
    got.insert("03 folders".into(), s.call("GET", "/folders", None).await);
    got.insert(
        "04 folders inside".into(),
        s.call("GET", &format!("/folders?path={}", enc(work)), None)
            .await,
    );
    got.insert(
        "05 branches".into(),
        s.call("GET", &format!("/branches?cwd={}", enc(work)), None)
            .await,
    );
    got.insert("06 jobs".into(), s.call("GET", "/jobs", None).await);
    got.insert(
        "07 off".into(),
        s.call("POST", "/off", Some(json!({ "mcp": [], "skills": [] })))
            .await,
    );

    // The person's own session, as the app opens it.
    let direct = json!({ "cwd": work, "project": null, "model": "haiku", "effort": "low", "title": "Parity direct",
                         "meta": { "direct": true }, "by": "owner", "prompt": "Reply with the single word OK. No tools." });
    let opened = s.call("POST", "/sessions", Some(direct)).await;
    let d = opened.1["id"].as_str().unwrap().to_owned();
    got.insert("10 open direct".into(), opened);
    s.settled(&d).await;
    // A session the assistant starts for work: no `by`, a task in its meta.
    let work_session = json!({ "title": "Parity work", "cwd": work, "prompt": "Run `echo parity` with the Bash tool, then reply DONE.",
                               "project": null, "model": "haiku", "effort": "low", "meta": { "task": 1 } });
    let opened = s.call("POST", "/sessions", Some(work_session)).await;
    let w = opened.1["id"].as_str().unwrap().to_owned();
    got.insert("11 open work".into(), opened);
    s.settled(&w).await;

    got.insert("12 send by assistant".into(), s.call("POST", &format!("/sessions/{d}/send"), Some(json!({ "prompt": "Reply with the single word AGAIN. No tools.", "by": "assistant" }))).await);
    s.settled(&d).await;
    got.insert(
        "13 settings".into(),
        s.call(
            "POST",
            &format!("/sessions/{d}/settings"),
            Some(json!({ "effort": "medium" })),
        )
        .await,
    );
    got.insert(
        "14 transcript app".into(),
        s.call(
            "GET",
            &format!("/sessions/{d}/transcript?after=&limit="),
            None,
        )
        .await,
    );
    got.insert(
        "15 transcript tool".into(),
        s.call("GET", &format!("/sessions/{w}/transcript?limit=10"), None)
            .await,
    );
    got.insert(
        "16 transcript after".into(),
        s.call(
            "GET",
            &format!("/sessions/{w}/transcript?after=1&limit=3"),
            None,
        )
        .await,
    );
    got.insert(
        "17 session".into(),
        s.call("GET", &format!("/sessions/{d}"), None).await,
    );
    got.insert(
        "18 live".into(),
        (
            200,
            Value::Array(s.live(&format!("/sessions/{w}/live?after=0")).await),
        ),
    );

    // Files with a reply: the session opened without a message, a file put in, then the message.
    let idle = json!({ "cwd": work, "model": "haiku", "effort": "low", "title": "Parity files", "meta": { "direct": true }, "by": "owner" });
    let opened = s.call("POST", "/sessions", Some(idle)).await;
    let f = opened.1["id"].as_str().unwrap().to_owned();
    got.insert("20 open without a message".into(), opened);
    let put = s
        .put(
            &format!("/sessions/{f}/files?name=plan.txt&by=owner&size=8"),
            b"the plan",
        )
        .await;
    let landed = put.1["path"].as_str().unwrap_or_default().to_owned();
    got.insert("21 put file".into(), put);
    let files =
        json!([{ "name": "plan.txt", "url": "/uploads/1a2b3c4d-plan.txt", "kind": "file" }]);
    got.insert(
        "22 send with files".into(),
        s.call(
            "POST",
            &format!("/sessions/{f}/send"),
            Some(json!({ "prompt": "(see the files)", "by": "owner", "files": files })),
        )
        .await,
    );
    s.settled(&f).await;
    got.insert(
        "23 stat file".into(),
        s.call(
            "GET",
            &format!("/files?path={}&by=owner&session={f}&stat=1", enc(&landed)),
            None,
        )
        .await,
    );
    got.insert(
        "24 refused file".into(),
        s.call(
            "GET",
            &format!("/files?path={}&by=owner&stat=1", enc("/etc/shadow")),
            None,
        )
        .await,
    );

    // A long turn interrupted, then a session stopped.
    got.insert("30 send long".into(), s.call("POST", &format!("/sessions/{d}/send"), Some(json!({ "prompt": "Write a 3000-word story about a harbour. No tools.", "by": "owner" }))).await);
    tokio::time::sleep(Duration::from_secs(6)).await;
    got.insert(
        "31 interrupt".into(),
        s.call("POST", &format!("/sessions/{d}/interrupt"), None)
            .await,
    );
    s.settled(&d).await;
    got.insert(
        "32 stop idle".into(),
        s.call("POST", &format!("/sessions/{w}/stop"), None).await,
    );
    got.insert(
        "33 no such session".into(),
        s.call("GET", "/sessions/r-00000000/transcript", None).await,
    );
    got.insert(
        "34 bad effort".into(),
        s.call(
            "POST",
            "/sessions",
            Some(json!({ "cwd": work, "prompt": "x", "effort": "loud" })),
        )
        .await,
    );
    got
}

#[tokio::test]
#[ignore = "needs two running runners and a real agent; see the module's docs"]
async fn two_runners_answer_and_report_in_the_same_shapes() {
    let work = std::env::var("EMBERHUT_PARITY_WORK").unwrap();
    let (a, b) = (Side::new("A").await, Side::new("B").await);
    let (got_a, got_b) = tokio::join!(play(&a, &work), play(&b, &work));
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut report = Map::new();
    let mut statuses = Vec::new();
    for (step, (sa, va)) in &got_a {
        let (sb, vb) = &got_b[step];
        let mut d = Vec::new();
        diff("", &shape(va), &shape(vb), &mut d);
        if sa != sb {
            statuses.push(format!("{step}: {sa} | {sb}"));
        }
        report.insert(
            step.clone(),
            json!({ "status": [sa, sb], "differences": d }),
        );
    }
    // The events: their kinds in order, and each kind's shape.
    let kinds = |s: &Side| -> Vec<String> {
        s.events()
            .iter()
            .map(|e| e["kind"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    let by_kind = |s: &Side| -> BTreeMap<String, Value> {
        let mut m = BTreeMap::new();
        for e in s.events() {
            let k = e["kind"].as_str().unwrap_or_default().to_owned();
            let merged = merge(m.get(&k).unwrap_or(&Value::Null), &shape(&e));
            m.insert(k, merged);
        }
        m
    };
    let (ka, kb) = (by_kind(&a), by_kind(&b));
    for k in ka
        .keys()
        .chain(kb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let mut d = Vec::new();
        diff(
            "",
            ka.get(k).unwrap_or(&Value::Null),
            kb.get(k).unwrap_or(&Value::Null),
            &mut d,
        );
        report.insert(format!("event {k}"), json!({ "differences": d }));
    }
    report.insert(
        "event kinds".into(),
        json!({ "A": kinds(&a), "B": kinds(&b) }),
    );
    let samples = |s: &Side| {
        s.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == "/api/runner/machine")
            .map(|(_, v)| shape(v))
            .fold(Value::Null, |m, x| merge(&m, &x))
    };
    let mut d = Vec::new();
    diff("", &samples(&a), &samples(&b), &mut d);
    report.insert("machine sample".into(), json!({ "differences": d }));

    let text = serde_json::to_string_pretty(&Value::Object(report)).unwrap();
    if let Ok(file) = std::env::var("EMBERHUT_PARITY_REPORT") {
        std::fs::write(file, &text).unwrap();
    }
    println!("{text}");
    assert!(
        statuses.is_empty(),
        "the runners answered with different statuses: {statuses:#?}"
    );
}
