//! Runners the test scripts: their names, places and tokens, what each answers, and every
//! request the server made of them.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a poisoned lock fails the test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use axum::body::Bytes;
use emberhut_server::services::runner_api::{ByteStream, RunnerApi, RunnerError, Verb};
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

/// A request the server made of a runner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) machine: String,
    pub(crate) verb: &'static str,
    pub(crate) path: String,
    pub(crate) body: Option<Value>,
}

#[derive(Debug, Default)]
struct Script {
    runners: Vec<(String, String, String)>,
    answers: Vec<(String, String, Result<Value, RunnerError>)>,
    requests: Vec<Request>,
    live: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
    uploads: Vec<(String, String, Vec<u8>)>,
    down: BTreeSet<String>,
}

/// The fake runners. A request no answer was scripted for gets `{}`.
#[derive(Debug, Default)]
pub(crate) struct FakeRunner {
    script: Mutex<Script>,
}

impl FakeRunner {
    /// These runners, each `(name, place, token)`.
    pub(crate) fn on(&self, runners: &[(&str, &str, &str)]) {
        self.script.lock().unwrap().runners = runners
            .iter()
            .map(|&(name, place, token)| (name.to_owned(), place.to_owned(), token.to_owned()))
            .collect();
    }

    /// `machine` answers a request whose path starts with `prefix` with `answer` (the newest
    /// scripted answer wins).
    pub(crate) fn answer(&self, machine: &str, prefix: &str, answer: Value) {
        self.script.lock().unwrap().answers.push((
            machine.to_owned(),
            prefix.to_owned(),
            Ok(answer),
        ));
    }

    /// `machine` refuses a request whose path starts with `prefix`, saying `why`.
    pub(crate) fn refuse(&self, machine: &str, prefix: &str, why: &str) {
        self.script.lock().unwrap().answers.push((
            machine.to_owned(),
            prefix.to_owned(),
            Err(RunnerError::Refused(why.to_owned())),
        ));
    }

    /// A live stream sends these chunks, then ends.
    pub(crate) fn live(&self, chunks: &[&str]) {
        self.script.lock().unwrap().live = chunks.iter().map(|&c| c.to_owned()).collect();
    }

    /// `GET /files` (without `stat`) on any runner sends these bytes.
    pub(crate) fn file(&self, path: &str, bytes: &[u8]) {
        self.script
            .lock()
            .unwrap()
            .files
            .insert(path.to_owned(), bytes.to_vec());
    }

    /// `machine`'s health fails.
    pub(crate) fn take_down(&self, machine: &str) {
        self.script.lock().unwrap().down.insert(machine.to_owned());
    }

    /// Every request so far.
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.script.lock().unwrap().requests.clone()
    }

    /// The requests whose path starts with `prefix`.
    pub(crate) fn requests_to(&self, prefix: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|request| request.path.starts_with(prefix))
            .collect()
    }

    /// Every file put into a session: (machine, path, bytes).
    pub(crate) fn uploads(&self) -> Vec<(String, String, Vec<u8>)> {
        self.script.lock().unwrap().uploads.clone()
    }

    fn known(&self, machine: &str) -> Result<(), RunnerError> {
        if self
            .script
            .lock()
            .unwrap()
            .runners
            .iter()
            .any(|(name, _, _)| name == machine)
        {
            Ok(())
        } else {
            Err(RunnerError::Unknown(machine.to_owned()))
        }
    }

    fn take(
        &self,
        machine: &str,
        verb: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, RunnerError> {
        self.known(machine)?;
        let mut script = self.script.lock().unwrap();
        script.requests.push(Request {
            machine: machine.to_owned(),
            verb,
            path: path.to_owned(),
            body,
        });
        script
            .answers
            .iter()
            .rev()
            .find(|(name, prefix, _)| name == machine && path.starts_with(prefix.as_str()))
            .map_or_else(|| Ok(json!({})), |(_, _, answer)| answer.clone())
    }
}

impl RunnerApi for FakeRunner {
    fn machines(&self) -> Vec<String> {
        self.script
            .lock()
            .unwrap()
            .runners
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect()
    }

    fn place(&self, machine: &str) -> String {
        self.script
            .lock()
            .unwrap()
            .runners
            .iter()
            .find(|(name, _, _)| name == machine)
            .map_or_else(|| machine.to_owned(), |(_, place, _)| place.clone())
    }

    fn address(&self, machine: &str) -> Option<String> {
        self.known(machine)
            .ok()
            .map(|()| format!("https://{machine}.example.com"))
    }

    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
        let tokens = self
            .script
            .lock()
            .unwrap()
            .runners
            .iter()
            .map(|(name, _, token)| (name.clone(), token.clone()))
            .collect();
        Box::pin(async move { tokens })
    }

    fn call<'a>(
        &'a self,
        machine: &'a str,
        verb: Verb,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        let verb = match verb {
            Verb::Get => "GET",
            Verb::Post => "POST",
        };
        Box::pin(async move { self.take(machine, verb, path, body) })
    }

    fn stream<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<ByteStream, RunnerError>> {
        Box::pin(async move {
            self.take(machine, "GET", path, None)?;
            let chunks = self.script.lock().unwrap().live.clone();
            Ok(
                futures_util::stream::iter(chunks.into_iter().map(|chunk| Ok(Bytes::from(chunk))))
                    .boxed(),
            )
        })
    }

    fn upload<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        file: &'a Path,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        Box::pin(async move {
            let bytes =
                std::fs::read(file).map_err(|error| RunnerError::Local(error.to_string()))?;
            let answer = self.take(machine, "PUT", path, None)?;
            let size = bytes.len();
            self.script
                .lock()
                .unwrap()
                .uploads
                .push((machine.to_owned(), path.to_owned(), bytes));
            if answer.get("path").is_some() {
                return Ok(answer);
            }
            Ok(json!({ "path": "/work/inbox/landed", "size": size }))
        })
    }

    fn download<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        to: &'a Path,
        limit: u64,
    ) -> BoxFuture<'a, Result<u64, RunnerError>> {
        Box::pin(async move {
            self.take(machine, "GET", path, None)?;
            let wanted = form_value(path, "path").unwrap_or_default();
            let bytes = self
                .script
                .lock()
                .unwrap()
                .files
                .get(&wanted)
                .cloned()
                .ok_or_else(|| RunnerError::Refused("no such file".to_owned()))?;
            if bytes.len() as u64 > limit {
                return Err(RunnerError::Local("over the limit".to_owned()));
            }
            std::fs::write(to, &bytes).map_err(|error| RunnerError::Local(error.to_string()))?;
            Ok(bytes.len() as u64)
        })
    }

    fn health<'a>(&'a self, machine: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.script.lock().unwrap().down.contains(machine) {
                Err("not reachable (could not connect)".to_owned())
            } else {
                Ok(())
            }
        })
    }
}

/// The value of `key` in `path`'s query.
pub(crate) fn form_value(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}
