//! A refused or failed request: an HTTP status and the words the agent reads.

use serde_json::{Map, Value};

/// A JSON object: every request's body and every answer.
pub type Reply = Map<String, Value>;

/// Why a request was not done, as the server's Mac tools pass it on: `{"error": text}` with
/// `status`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text}")]
pub struct Fail {
    /// The HTTP status: 400 for a request that can't be done, 403 for one that is never done,
    /// 404 for an app that isn't there, 409 for one that clashes with what runs now.
    pub status: u16,
    /// The words the agent reads.
    pub text: String,
}

impl Fail {
    /// A request that can't be done as asked (400).
    #[must_use]
    pub fn bad(text: impl Into<String>) -> Self {
        Self::with(400, text)
    }

    /// A request that is never done here (403).
    #[must_use]
    pub fn never(text: impl Into<String>) -> Self {
        Self::with(403, text)
    }

    /// Something asked for that isn't there (404).
    #[must_use]
    pub fn missing(text: impl Into<String>) -> Self {
        Self::with(404, text)
    }

    /// A request that clashes with what is going on now (409).
    #[must_use]
    pub fn clash(text: impl Into<String>) -> Self {
        Self::with(409, text)
    }

    /// A failure of the Mac itself (500).
    #[must_use]
    pub fn broke(text: impl Into<String>) -> Self {
        Self::with(500, text)
    }

    /// A failure with `status`.
    #[must_use]
    pub fn with(status: u16, text: impl Into<String>) -> Self {
        Self {
            status,
            text: text.into(),
        }
    }

    /// The answer's body: `{"error": text}`.
    #[must_use]
    pub fn body(&self) -> Reply {
        let mut body = Reply::new();
        body.insert("error".to_owned(), Value::String(self.text.clone()));
        body
    }
}

/// An answer with only `text`.
#[must_use]
pub fn said(text: impl Into<String>) -> Reply {
    let mut reply = Reply::new();
    reply.insert("text".to_owned(), Value::String(text.into()));
    reply
}

/// A body's field as the reference helper reads it: a string as it is, a number or a boolean
/// as JSON writes it, nothing (or `null`) as empty.
#[must_use]
pub fn field(body: &Reply, key: &str) -> String {
    match body.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// A body's field as a number, when it is one or a string that reads as one.
#[must_use]
pub fn number(body: &Reply, key: &str) -> Option<f64> {
    match body.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

/// A body's field as a boolean, when it is one.
#[must_use]
pub fn flag(body: &Reply, key: &str) -> Option<bool> {
    body.get(key).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn body(value: Value) -> Reply {
        let Value::Object(map) = value else {
            panic!("an object")
        };
        map
    }

    #[test]
    fn a_fail_answers_its_words_as_error() {
        let fail = Fail::never("Never here.");
        assert_eq!(fail.status, 403);
        assert_eq!(
            Value::Object(fail.body()),
            json!({ "error": "Never here." })
        );
    }

    #[test]
    fn fields_read_as_the_reference_reads_them() {
        let b = body(json!({ "app": "Notes", "pid": 42, "on": true, "none": null, "x": "12.5" }));
        assert_eq!(field(&b, "app"), "Notes");
        assert_eq!(field(&b, "pid"), "42");
        assert_eq!(field(&b, "on"), "true");
        assert_eq!(field(&b, "none"), "");
        assert_eq!(field(&b, "missing"), "");
        assert_eq!(number(&b, "pid"), Some(42.0));
        assert_eq!(number(&b, "x"), Some(12.5));
        assert_eq!(number(&b, "app"), None);
        assert_eq!(flag(&b, "on"), Some(true));
        assert_eq!(flag(&b, "pid"), None);
    }
}
