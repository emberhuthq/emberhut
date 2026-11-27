//! A tiny HTTP/1.1 server on `127.0.0.1:0` for the client's tests: it answers a fixed number
//! of requests with what a handler says and records each one.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::config::ServerUrl;

/// A request as the server saw it.
#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    /// `GET`, `POST`, …
    pub(crate) method: String,
    /// The path with its query, e.g. `/api/state?room=1`.
    pub(crate) target: String,
    /// Header names in lower case.
    pub(crate) headers: HashMap<String, String>,
    /// The body.
    pub(crate) body: String,
}

impl Recorded {
    /// The body as JSON.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("a JSON body")
    }
}

/// What the server answers.
#[derive(Clone, Debug)]
pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    /// A JSON reply.
    pub(crate) fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string(),
        }
    }

    /// A plain reply.
    pub(crate) fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }

    /// The same reply with one more header.
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// The running server.
#[derive(Debug)]
pub(crate) struct TestServer {
    url: ServerUrl,
    requests: Arc<Mutex<Vec<Recorded>>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Answers `count` requests with `handler`, then stops.
    pub(crate) fn start(
        count: usize,
        handler: impl Fn(&Recorded) -> Reply + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let port = listener.local_addr().expect("its address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming().take(count) {
                let mut stream = stream.expect("a connection");
                let request = read_request(&mut stream);
                let reply = handler(&request);
                seen.lock().expect("not poisoned").push(request);
                write_reply(&mut stream, &reply);
            }
        });
        let url = ServerUrl::parse(&format!("http://127.0.0.1:{port}")).expect("an address");
        Self {
            url,
            requests,
            thread: Some(thread),
        }
    }

    /// Its address.
    pub(crate) fn url(&self) -> ServerUrl {
        self.url.clone()
    }

    /// The requests it answered, waiting until it answered all it was told to.
    pub(crate) fn finish(mut self) -> Vec<Recorded> {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the server thread");
        }
        self.requests.lock().expect("not poisoned").clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Recorded {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("a request line");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).expect("a header line");
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("the body");
    Recorded {
        method,
        target,
        headers,
        body: String::from_utf8(body).expect("UTF-8"),
    }
}

fn write_reply(stream: &mut TcpStream, reply: &Reply) {
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        write!(head, "{name}: {value}\r\n").expect("writing to a String");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("write the head");
    stream
        .write_all(reply.body.as_bytes())
        .expect("write the body");
}
