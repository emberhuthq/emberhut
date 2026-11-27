//! The wire dialect: building a client for a server that spells some wire words differently.
//!
//! The contract ([`emberhut_api`]) spells every wire word one way. A server that spells some of
//! them its own way (a value, an object key, a header name, the start of a text) can still be
//! spoken to by a client built with a dialect file: point `EMBERHUT_WIRE_DIALECT` at it when
//! this crate is built, and every request and answer through [`Client`](crate::Client) is
//! rewritten on its way, so nothing above this module sees the server's spelling.
//!
//! The file is JSON, every part optional, each pair the server's spelling then the contract's:
//!
//! ```json
//! {
//!   "words": [["person", "owner"]],
//!   "keys": [["botAccount", "assistantAccount"]],
//!   "headers": [["x-other-looking", "x-emberhut-looking"]],
//!   "prefixes": [["K-", "T-"], ["other://", "emberhut://"]]
//! }
//! ```
//!
//! The same file and the same rules build the mobile app (`apps/mobile/src/api/dialect.ts`) and
//! the browser extension (`apps/extension/src/dialect.ts`); `tests/fixtures/dialect.json` holds
//! the cases all three must answer alike.
//!
//! - `words`: a string value that is exactly one of them is replaced whole, both ways. Reading
//!   an answer, an object key that is one is renamed too (after `keys`); writing a request,
//!   object keys and query names are renamed by `keys` only, since a field's name may be spelled
//!   as a word (an `owner` field holding the word `owner`).
//! - `keys`: an object key (and a query's name), both ways; reading, `keys` is looked up before
//!   `words`.
//! - `headers`: a header's name, in any case.
//! - `prefixes`: text that starts a token inside a string value that isn't a whole word:
//!   - a token starts at the start of the string or right after a character that can't be part
//!     of one; token characters are letters, digits and `_ - = + / : .` So `(K-1,K-2)`,
//!     `"K-1"` and `[it](other://x)` are rewritten, and `OK-1`, `id=K-1`, `a/K-1` and `a.K-1`
//!     aren't. A link scheme (a prefix ending in `://`) keeps the same rule:
//!     `?next=other://x` stays as it is;
//!   - a prefix ending in `-` (a key's prefix) matches only when a digit (`0`-`9`) follows it,
//!     so `K-12` is rewritten but `K-shirt` and a lone `K-` aren't, either way;
//!   - where several match at one place, the longest wins (the first listed among equals);
//!   - the text is read left to right as it was given: a replacement is never searched again,
//!     and where a token starts is decided by the character before it in the given text.
//!
//! An answer's body is rewritten when it is JSON, and an event stream's `data:` lines when each
//! is JSON; a request's JSON body, its query string's names and values and its header names are
//! rewritten the other way before it is sent. [`Dialect::to_server`] and
//! [`Dialect::from_server`] do the same to one text, such as a link the app passes to or takes
//! from the server some other way, and [`Dialect::json_to_server`] and
//! [`Dialect::json_from_server`] to a whole JSON document, as the transport does to a body.
//!
//! The built dialect is a server's, not the app's: a client applies it only to a server marked
//! as speaking it ([`ClientConfig::with_built_dialect`](crate::ClientConfig::with_built_dialect),
//! on by default), so one app can talk to a server that speaks the dialect and to one that
//! speaks the contract's own words. [`Client::dialect`](crate::Client::dialect) is the one a
//! client applies; the free functions here ([`to_server`] and the rest) apply the built dialect,
//! for a server that speaks it.
//!
//! Without the variable the dialect is empty: nothing is rewritten, and nothing is read or
//! copied to find that out, whatever a server is marked as.

use std::borrow::Cow;
use std::io::{self, BufRead, BufReader, Cursor, Read};

use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use http::uri::PathAndQuery;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::transport::{Answer, Body, Request};

/// The dialect this crate was built with (empty unless `EMBERHUT_WIRE_DIALECT` named a file).
pub(crate) static BUILT: Table = include!(concat!(env!("OUT_DIR"), "/dialect.rs"));

/// No dialect: the contract's own words, for a server that speaks them.
static NONE: Table = Table {
    words: &[],
    keys: &[],
    headers: &[],
    prefixes: &[],
};

/// The largest JSON answer rewritten; a longer one passes unchanged (the client refuses one
/// longer than 64 MiB anyway).
const MAX_BODY: u64 = 64 << 20;

/// The longest event-stream line rewritten; a longer one passes unchanged (the client refuses
/// one longer than 4 MiB anyway).
const MAX_LINE: usize = 4 << 20;

/// A dialect's tables: pairs of the server's spelling and the contract's.
#[derive(Debug)]
pub(crate) struct Table {
    /// Whole string values and object keys.
    pub(crate) words: &'static [(&'static str, &'static str)],
    /// Object keys only.
    pub(crate) keys: &'static [(&'static str, &'static str)],
    /// Header names, in lower case.
    pub(crate) headers: &'static [(&'static str, &'static str)],
    /// Starts of words inside string values.
    pub(crate) prefixes: &'static [(&'static str, &'static str)],
}

/// Which way a text goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    /// From the server's spelling to the contract's (an answer).
    In,
    /// From the contract's spelling to the server's (a request).
    Out,
}

impl Way {
    /// A pair's spelling this way goes from, then the one it goes to.
    fn pick(self, server: &'static str, ours: &'static str) -> (&'static str, &'static str) {
        match self {
            Way::In => (server, ours),
            Way::Out => (ours, server),
        }
    }
}

/// The dialect one server is spoken to in: the one this crate was built with, for a server
/// that speaks it, or none, for a server that speaks the contract's own words.
#[derive(Clone, Copy, Debug)]
pub struct Dialect {
    table: &'static Table,
}

impl Dialect {
    /// The dialect this crate was built with (none, in a build without one).
    #[must_use]
    pub fn built() -> Self {
        Self { table: &BUILT }
    }

    /// No dialect: every word goes as the contract spells it.
    #[must_use]
    pub fn none() -> Self {
        Self { table: &NONE }
    }

    /// The built dialect when `speaks_built` (the server spells the wire words its way), else
    /// none.
    #[must_use]
    pub fn for_server(speaks_built: bool) -> Self {
        if speaks_built {
            Self::built()
        } else {
            Self::none()
        }
    }

    /// Whether it rewrites nothing.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.table.is_empty()
    }

    /// Its tables.
    pub(crate) fn table(self) -> &'static Table {
        self.table
    }

    /// `text` as this dialect's server spells it: a link or a value the app hands the server
    /// outside a request body (unchanged with no dialect).
    #[must_use]
    pub fn to_server(self, text: &str) -> Cow<'_, str> {
        self.table
            .text(text, Way::Out)
            .map_or(Cow::Borrowed(text), Cow::Owned)
    }

    /// `text` from this dialect's server in the contract's spelling: a link or a value the app
    /// takes from the server outside an answer's body (unchanged with no dialect).
    #[must_use]
    pub fn from_server(self, text: &str) -> Cow<'_, str> {
        self.table
            .text(text, Way::In)
            .map_or(Cow::Borrowed(text), Cow::Owned)
    }

    /// `value`, a JSON document taken from this dialect's server some way other than an
    /// answer's body, rewritten in place into the contract's spelling, as the transport does to
    /// a body; whether anything in it changed (never, with no dialect).
    pub fn json_from_server(self, value: &mut Value) -> bool {
        self.table.document(value, Way::In)
    }

    /// `value`, a JSON document for this dialect's server, rewritten in place into its
    /// spelling, as the transport does to a request's body; whether anything in it changed
    /// (never, with no dialect).
    pub fn json_to_server(self, value: &mut Value) -> bool {
        self.table.document(value, Way::Out)
    }
}

/// `text` as a server that speaks the built dialect spells it ([`Dialect::to_server`]; a
/// client's own is [`Client::dialect`](crate::Client::dialect)).
#[must_use]
pub fn to_server(text: &str) -> Cow<'_, str> {
    Dialect::built().to_server(text)
}

/// `text` from a server that speaks the built dialect, in the contract's spelling
/// ([`Dialect::from_server`]).
#[must_use]
pub fn from_server(text: &str) -> Cow<'_, str> {
    Dialect::built().from_server(text)
}

/// `value`, from a server that speaks the built dialect, in the contract's spelling
/// ([`Dialect::json_from_server`]).
pub fn json_from_server(value: &mut Value) -> bool {
    Dialect::built().json_from_server(value)
}

/// `value`, for a server that speaks the built dialect, in its spelling
/// ([`Dialect::json_to_server`]).
pub fn json_to_server(value: &mut Value) -> bool {
    Dialect::built().json_to_server(value)
}

impl Table {
    /// Whether it rewrites nothing.
    fn is_empty(&self) -> bool {
        self.words.is_empty()
            && self.keys.is_empty()
            && self.headers.is_empty()
            && self.prefixes.is_empty()
    }

    /// `request` in the server's spelling: its query's names and values, its header names and
    /// its JSON body.
    pub(crate) fn request<'a>(&self, request: Request<'a>) -> Request<'a> {
        if self.is_empty() {
            return request;
        }
        let (method, path, headers, body) = request.into_parts();
        let json = is_json(&headers);
        let path = self.path(path);
        let headers = self.header_names(headers, Way::Out);
        let body = match body {
            Body::Bytes(bytes) if json => match self.json(&bytes, Way::Out) {
                Some(rewritten) => Body::Bytes(Cow::Owned(rewritten)),
                None => Body::Bytes(bytes),
            },
            body => body,
        };
        Request::new(method, path)
            .with_headers(headers)
            .with_body(body)
    }

    /// `answer` in the contract's spelling: its header names, and its body when it is JSON or
    /// an event stream.
    pub(crate) fn answer(&'static self, answer: Answer) -> Answer {
        if self.is_empty() {
            return answer;
        }
        let (status, headers, body) = answer.into_parts();
        let mut headers = self.header_names(headers, Way::In);
        let mut body = body;
        let body: Box<dyn Read + Send> = if is_json(&headers) {
            let mut bytes = Vec::new();
            match body.by_ref().take(MAX_BODY + 1).read_to_end(&mut bytes) {
                // Too long to rewrite: what was read, then the rest, unchanged.
                Ok(_) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BODY => {
                    Box::new(Cursor::new(bytes).chain(body))
                }
                Ok(_) => {
                    if let Some(rewritten) = self.json(&bytes, Way::In) {
                        bytes = rewritten;
                        headers.insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
                    }
                    Box::new(Cursor::new(bytes))
                }
                // What was read, then the failure, as the body would have given them.
                Err(error) => Box::new(Cursor::new(bytes).chain(Failed(Some(error)))),
            }
        } else if is_event_stream(&headers) {
            Box::new(EventLines {
                table: self,
                reader: BufReader::new(body),
                line: Vec::new(),
                out: Cursor::new(Vec::new()),
            })
        } else {
            body
        };
        Answer::from_parts(status, headers, body)
    }

    /// `bytes`, a JSON document, rewritten; none when it isn't JSON or nothing in it changed.
    fn json(&self, bytes: &[u8], way: Way) -> Option<Vec<u8>> {
        let mut value: Value = serde_json::from_slice(bytes).ok()?;
        self.document(&mut value, way)
            .then(|| serde_json::to_vec(&value).ok())
            .flatten()
    }

    /// Rewrites `value` and everything in it; whether anything changed.
    fn document(&self, value: &mut Value, way: Way) -> bool {
        if self.is_empty() {
            return false;
        }
        self.value(value, way)
    }

    /// Rewrites `value` and everything in it; whether anything changed.
    fn value(&self, value: &mut Value, way: Way) -> bool {
        match value {
            Value::String(text) => match self.text(text, way) {
                Some(rewritten) => {
                    *text = rewritten;
                    true
                }
                None => false,
            },
            Value::Array(items) => items
                .iter_mut()
                .fold(false, |changed, item| self.value(item, way) | changed),
            Value::Object(object) => {
                let mut changed = false;
                let renamed: Vec<(String, String)> = object
                    .keys()
                    .filter_map(|key| Some((key.clone(), self.key(key, way)?.to_owned())))
                    .collect();
                for (from, to) in renamed {
                    if let Some(item) = object.remove(&from) {
                        object.insert(to, item);
                        changed = true;
                    }
                }
                object
                    .values_mut()
                    .fold(changed, |changed, item| self.value(item, way) | changed)
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    /// An object key's other spelling; none when it has none. Coming in, a key is read by
    /// `keys`, then `words`; going out, by `keys` only, since a field's name may be spelled as
    /// a word (an `owner` field holding the word `owner`).
    fn key(&self, key: &str, way: Way) -> Option<&'static str> {
        match way {
            Way::In => whole(self.keys, key, way).or_else(|| whole(self.words, key, way)),
            Way::Out => whole(self.keys, key, way),
        }
    }

    /// A string value rewritten: a whole word, else every prefix that starts a token in it
    /// (see the module's rules); none when nothing in it changes.
    fn text(&self, text: &str, way: Way) -> Option<String> {
        if let Some(word) = whole(self.words, text, way) {
            return Some(word.to_owned());
        }
        if self.prefixes.is_empty() {
            return None;
        }
        let mut out: Option<String> = None;
        let mut copied = 0;
        let mut at = 0;
        let mut before: Option<char> = None;
        while at < text.len() {
            let rest = &text[at..];
            let found = if before.is_some_and(in_token) {
                None
            } else {
                self.prefix(rest, way)
            };
            if let Some((from, to)) = found {
                let out = out.get_or_insert_with(String::new);
                out.push_str(&text[copied..at]);
                out.push_str(to);
                at += from.len();
                copied = at;
                before = from.chars().next_back();
            } else {
                let c = rest.chars().next().unwrap_or_default();
                at += c.len_utf8();
                before = Some(c);
            }
        }
        let mut out = out?;
        out.push_str(&text[copied..]);
        Some(out)
    }

    /// The prefix that `rest` starts with and its other spelling: the longest that matches (the
    /// first listed among equals); one ending in `-` only before a digit.
    fn prefix(&self, rest: &str, way: Way) -> Option<(&'static str, &'static str)> {
        let mut best: Option<(&'static str, &'static str)> = None;
        for &(server, ours) in self.prefixes {
            let (from, to) = way.pick(server, ours);
            let fits = rest.strip_prefix(from).is_some_and(|after| {
                !from.ends_with('-') || after.starts_with(|c: char| c.is_ascii_digit())
            });
            if fits && best.is_none_or(|(longest, _)| from.len() > longest.len()) {
                best = Some((from, to));
            }
        }
        best
    }

    /// `path` with its query's names and values rewritten.
    fn path(&self, path: PathAndQuery) -> PathAndQuery {
        let Some(query) = path.query() else {
            return path;
        };
        let mut changed = false;
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| {
                let (name, value) = match pair.split_once('=') {
                    Some((name, value)) => (name, Some(value)),
                    None => (pair, None),
                };
                let new_name =
                    rewrite_encoded(name, |name| self.key(name, Way::Out).map(str::to_owned));
                let new_value =
                    value.and_then(|value| rewrite_encoded(value, |v| self.text(v, Way::Out)));
                if new_name.is_none() && new_value.is_none() {
                    return pair.to_owned();
                }
                changed = true;
                let name = new_name.unwrap_or_else(|| name.to_owned());
                match (new_value, value) {
                    (Some(value), _) => format!("{name}={value}"),
                    (None, Some(value)) => format!("{name}={value}"),
                    (None, None) => name,
                }
            })
            .collect();
        if !changed {
            return path;
        }
        let target = format!("{}?{}", path.path(), pairs.join("&"));
        PathAndQuery::try_from(target).unwrap_or(path)
    }

    /// `headers` with their names rewritten.
    fn header_names(&self, headers: HeaderMap, way: Way) -> HeaderMap {
        if self.headers.is_empty() {
            return headers;
        }
        let mut out = HeaderMap::with_capacity(headers.len());
        let mut name = None;
        for (next, value) in headers {
            if let Some(next) = next {
                name = Some(
                    whole(self.headers, next.as_str(), way)
                        .and_then(|other| HeaderName::from_bytes(other.as_bytes()).ok())
                        .unwrap_or(next),
                );
            }
            if let Some(name) = &name {
                out.append(name.clone(), value);
            }
        }
        out
    }
}

/// Whether `c` can be part of a token: a letter, a digit or one of `_ - = + / : .`.
fn in_token(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '=' | '+' | '/' | ':' | '.')
}

/// `text`'s other spelling in `pairs`, whole; none when it has none.
fn whole(
    pairs: &'static [(&'static str, &'static str)],
    text: &str,
    way: Way,
) -> Option<&'static str> {
    pairs.iter().find_map(|&(server, ours)| {
        let (from, to) = way.pick(server, ours);
        (from == text).then_some(to)
    })
}

/// A query's name or value, percent-decoded, given to `rewrite`, and encoded again when it
/// changed; none when it didn't (or isn't UTF-8 once decoded).
fn rewrite_encoded(encoded: &str, rewrite: impl Fn(&str) -> Option<String>) -> Option<String> {
    let decoded = percent_decode(encoded)?;
    rewrite(&decoded).map(|text| percent_encode(&text))
}

/// Whether the headers say the body is JSON.
fn is_json(headers: &HeaderMap) -> bool {
    media_type(headers).is_some_and(|media| media == "application/json" || media.ends_with("+json"))
}

/// Whether the headers say the body is an event stream.
fn is_event_stream(headers: &HeaderMap) -> bool {
    media_type(headers).is_some_and(|media| media == "text/event-stream")
}

/// The `Content-Type`'s media type, in lower case, without parameters.
fn media_type(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(CONTENT_TYPE)?.to_str().ok()?;
    let media = value.split(';').next().unwrap_or_default().trim();
    Some(media.to_ascii_lowercase())
}

/// `text` with its `%XX` escapes (and `+` for a space) undone; none when that isn't UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = text.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// `text` percent-encoded for a query, every byte but the unreserved ones escaped.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
    }
    out
}

/// An event stream with each `data:` line that holds JSON rewritten, line by line as it
/// arrives. A line cut short by a failed read is kept, and finished by the next read.
struct EventLines {
    table: &'static Table,
    reader: BufReader<Box<dyn Read + Send>>,
    /// The line being read.
    line: Vec<u8>,
    /// The last whole line, rewritten, as it is read out.
    out: Cursor<Vec<u8>>,
}

impl EventLines {
    /// `line` (its line end included) with its data rewritten when it is a `data:` line of JSON.
    fn rewrite(&self, line: Vec<u8>) -> Vec<u8> {
        if line.len() > MAX_LINE {
            return line;
        }
        let Some(data) = line.strip_prefix(b"data:") else {
            return line;
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        let body_end = data.len()
            - data
                .iter()
                .rev()
                .take_while(|&&b| b == b'\n' || b == b'\r')
                .count();
        let (json, end) = data.split_at(body_end);
        match self.table.json(json, Way::In) {
            Some(mut rewritten) => {
                rewritten.splice(0..0, b"data: ".iter().copied());
                rewritten.extend_from_slice(end);
                rewritten
            }
            None => line,
        }
    }
}

impl Read for EventLines {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let read = self.out.read(buf)?;
            if read > 0 || buf.is_empty() {
                return Ok(read);
            }
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                if self.line.is_empty() {
                    return Ok(0);
                }
                let line = std::mem::take(&mut self.line);
                self.out = Cursor::new(self.rewrite(line));
                continue;
            }
            if let Some(end) = available.iter().position(|&b| b == b'\n') {
                self.line.extend_from_slice(&available[..=end]);
                self.reader.consume(end + 1);
                let line = std::mem::take(&mut self.line);
                self.out = Cursor::new(self.rewrite(line));
            } else {
                let taken = available.len();
                self.line.extend_from_slice(available);
                self.reader.consume(taken);
                if self.line.len() > MAX_LINE {
                    // Too long to rewrite: passed on as it is, its rest after it.
                    self.out = Cursor::new(std::mem::take(&mut self.line));
                }
            }
        }
    }
}

/// A body's failure, given once after what was read before it.
struct Failed(Option<io::Error>);

impl Read for Failed {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0.take().map_or(Ok(0), Err)
    }
}

#[cfg(test)]
mod tests {
    use http::header::ETAG;
    use http::{Method, StatusCode};
    use serde_json::json;

    use super::*;

    /// A made-up dialect: a server that says `person` for the owner, `botAccount` for the
    /// assistant's account, its own header, `K-` task keys and `other://` links.
    static OTHER: Table = Table {
        words: &[("person", "owner")],
        keys: &[("botAccount", "assistantAccount")],
        headers: &[("x-other-looking", "x-emberhut-looking")],
        prefixes: &[("K-", "T-"), ("other://", "emberhut://")],
    };

    static EMPTY: Table = Table {
        words: &[],
        keys: &[],
        headers: &[],
        prefixes: &[],
    };

    fn served(content_type: &'static str, body: &'static [u8]) -> Answer {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers.insert(
            HeaderName::from_static("x-other-looking"),
            HeaderValue::from_static("1"),
        );
        headers.insert(ETAG, HeaderValue::from_static("\"e\""));
        Answer::new(StatusCode::OK, headers, body)
    }

    fn body(answer: Answer) -> String {
        let mut text = String::new();
        answer.into_body().read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn the_built_dialect_is_empty_without_a_file() {
        if option_env!("EMBERHUT_WIRE_DIALECT").is_none_or(str::is_empty) {
            assert!(BUILT.is_empty());
            assert_eq!(to_server("K-1"), "K-1");
            assert!(matches!(from_server("person"), Cow::Borrowed("person")));
            let mut value = json!({"person": "K-1"});
            assert!(!json_from_server(&mut value));
            assert!(!json_to_server(&mut value));
            assert_eq!(value, json!({"person": "K-1"}));
        }
    }

    #[test]
    fn a_server_that_speaks_the_contract_gets_no_dialect() {
        let none = Dialect::for_server(false);
        assert!(none.is_empty());
        assert!(std::ptr::eq(none.table(), &raw const NONE));
        assert!(std::ptr::eq(
            Dialect::for_server(true).table(),
            &raw const BUILT
        ));
        let other = Dialect { table: &OTHER };
        assert_eq!(other.to_server("T-1 owner"), "K-1 owner");
        assert_eq!(none.to_server("T-1"), "T-1");
        assert_eq!(other.from_server("person"), "owner");
        assert_eq!(none.from_server("person"), "person");
        let mut value = json!({"botAccount": "person"});
        assert!(!none.json_from_server(&mut value));
        assert!(other.json_from_server(&mut value));
        assert_eq!(value, json!({"assistantAccount": "owner"}));
        assert!(other.json_to_server(&mut value));
        assert_eq!(value, json!({"botAccount": "person"}));
    }

    /// The dialect in a shared case file, for the life of the tests.
    fn table_of(dialect: &Value) -> &'static Table {
        let pairs = |part: &str| -> &'static [(&'static str, &'static str)] {
            let leak = |side: &Value| -> &'static str {
                Box::leak(side.as_str().unwrap().to_owned().into_boxed_str())
            };
            let pairs: Vec<_> = dialect[part]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (leak(&pair[0]), leak(&pair[1])))
                .collect();
            Box::leak(pairs.into_boxed_slice())
        };
        Box::leak(Box::new(Table {
            words: pairs("words"),
            keys: pairs("keys"),
            headers: pairs("headers"),
            prefixes: pairs("prefixes"),
        }))
    }

    /// The cases the mobile app and the extension answer too (`tests/fixtures/dialect.json`).
    #[test]
    fn the_shared_cases_get_their_answers() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/fixtures/dialect.json")).unwrap();
        let table = table_of(&cases["dialect"]);
        for (part, way) in [("read", Way::In), ("write", Way::Out)] {
            for case in cases["texts"][part].as_array().unwrap() {
                let (given, expected) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
                let rewritten = table.text(given, way);
                assert_eq!(rewritten.is_some(), given != expected, "{part} {given:?}");
                assert_eq!(
                    rewritten.as_deref().unwrap_or(given),
                    expected,
                    "{part} {given:?}"
                );
            }
            for case in cases["documents"][part].as_array().unwrap() {
                let mut value = case[0].clone();
                let changed = table.document(&mut value, way);
                assert_eq!(value, case[1], "{part} {}", case[0]);
                assert_eq!(changed, case[0] != case[1], "{part} {}", case[0]);
            }
        }
    }

    #[test]
    fn prefixes_keep_to_tokens_digits_and_the_longest() {
        static PREFIXES: Table = Table {
            words: &[],
            keys: &[],
            headers: &[],
            prefixes: &[("a://", "b://"), ("a://x/", "b://y/"), ("K-", "T-")],
        };
        let text = |text: &str, way| PREFIXES.text(text, way);
        assert_eq!(text("a://x/1", Way::In).as_deref(), Some("b://y/1"));
        assert_eq!(text("b://y/1", Way::Out).as_deref(), Some("a://x/1"));
        assert_eq!(text("a://z", Way::In).as_deref(), Some("b://z"));
        assert_eq!(text("u=a://x/1 /K-1 .K-1", Way::In), None);
        assert_eq!(text("K-shirt T-shirt", Way::In), None);
        assert_eq!(text("T-shirt", Way::Out), None);
    }

    #[test]
    fn keys_come_before_words_reading_an_answer() {
        static BOTH: Table = Table {
            words: &[("person", "owner")],
            keys: &[("person", "chief")],
            headers: &[],
            prefixes: &[],
        };
        let mut value = json!({"person": "person"});
        assert!(BOTH.document(&mut value, Way::In));
        assert_eq!(value, json!({"chief": "owner"}));
        let mut value = json!({"chief": "owner", "owner": 1});
        assert!(BOTH.document(&mut value, Way::Out));
        assert_eq!(value, json!({"person": "person", "owner": 1}));
    }

    #[test]
    fn a_json_answer_reads_in_the_contracts_words() {
        let answer = OTHER.answer(served(
            "application/json; charset=utf-8",
            br#"{"by": "person", "botAccount": {"on": "a"}, "person": 1,
                "note": "See K-12 and [it](other://task/12); OK-1 and XK-2 stay",
                "list": ["person", "personal", 3, null]}"#,
        ));
        assert!(answer.headers().contains_key("x-emberhut-looking"));
        assert!(!answer.headers().contains_key("x-other-looking"));
        assert_eq!(answer.headers()[ETAG], "\"e\"");
        let length: usize = answer.headers()[CONTENT_LENGTH]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let text = body(answer);
        assert_eq!(text.len(), length);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            json!({"by": "owner", "assistantAccount": {"on": "a"}, "owner": 1,
                   "note": "See T-12 and [it](emberhut://task/12); OK-1 and XK-2 stay",
                   "list": ["owner", "personal", 3, null]})
        );
    }

    #[test]
    fn an_event_streams_data_lines_read_in_the_contracts_words() {
        let stream = b": ping\n\nevent: step\ndata: {\"text\":\"K-3\",\"who\":\"person\"}\r\n\
                       data: not json person\n\ndata:{\"botAccount\":1}";
        let answer = OTHER.answer(served("text/event-stream", stream));
        assert_eq!(
            body(answer),
            ": ping\n\nevent: step\ndata: {\"text\":\"T-3\",\"who\":\"owner\"}\r\n\
             data: not json person\n\ndata: {\"assistantAccount\":1}"
        );
    }

    #[test]
    fn other_bodies_pass_as_they_are() {
        let answer = OTHER.answer(served("application/octet-stream", b"person K-1"));
        assert_eq!(body(answer), "person K-1");
        let answer = OTHER.answer(served("application/json", b"not json: person"));
        assert_eq!(body(answer), "not json: person");
    }

    #[test]
    fn a_request_goes_out_in_the_servers_words() {
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static(
                "/api/tasks?q=T-4&owner=owner&archived=1&flag&link=emberhut%3A%2F%2Ftask%2F2",
            ),
        )
        .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .with_header(
            HeaderName::from_static("x-emberhut-looking"),
            HeaderValue::from_static("1"),
        )
        .with_body(Body::Bytes(Cow::Borrowed(
            br#"{"owner":"owner","assistantAccount":"a","text":"about T-4"}"#,
        )));
        let (_, path, headers, body) = OTHER.request(request).into_parts();
        assert_eq!(
            path.as_str(),
            "/api/tasks?q=K-4&owner=person&archived=1&flag&link=other%3A%2F%2Ftask%2F2"
        );
        assert!(headers.contains_key("x-other-looking"));
        assert!(!headers.contains_key("x-emberhut-looking"));
        let Body::Bytes(bytes) = body else {
            panic!("a JSON body")
        };
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value,
            json!({"owner": "person", "botAccount": "a", "text": "about K-4"})
        );
    }

    #[test]
    fn a_request_with_nothing_to_rewrite_keeps_its_query_as_written() {
        let request = Request::new(
            Method::GET,
            PathAndQuery::from_static("/api/state?room=1&x=a%20b"),
        );
        let (_, path, _, _) = OTHER.request(request).into_parts();
        assert_eq!(path.as_str(), "/api/state?room=1&x=a%20b");
    }

    #[test]
    fn an_empty_dialect_changes_nothing() {
        let answer = EMPTY.answer(served("application/json", br#"{"owner":"person"}"#));
        assert!(answer.headers().contains_key("x-other-looking"));
        assert!(!answer.headers().contains_key(CONTENT_LENGTH));
        assert_eq!(body(answer), r#"{"owner":"person"}"#);
        assert_eq!(EMPTY.text("K-1 person", Way::In), None);
    }

    #[test]
    fn texts_go_both_ways() {
        let text = |text: &str, way| OTHER.text(text, way);
        assert_eq!(text("person", Way::In).as_deref(), Some("owner"));
        assert_eq!(text("owner", Way::Out).as_deref(), Some("person"));
        assert_eq!(text("persons", Way::In), None);
        assert_eq!(
            text("other://task/7", Way::In).as_deref(),
            Some("emberhut://task/7")
        );
        assert_eq!(
            text("emberhut://task/7", Way::Out).as_deref(),
            Some("other://task/7")
        );
        assert_eq!(text("(K-1,K-2)", Way::In).as_deref(), Some("(T-1,T-2)"));
        assert_eq!(text("ünK-1 _K-2", Way::In), None);
    }

    #[test]
    fn a_long_event_line_passes_unchanged() {
        let line = format!(
            "data: {{\"who\":\"person\",\"pad\":\"{}\"}}\n",
            "x".repeat(MAX_LINE)
        );
        let stream: &'static [u8] = Box::leak(line.clone().into_bytes().into_boxed_slice());
        assert_eq!(
            body(OTHER.answer(served("text/event-stream", stream))),
            line
        );
    }
}
