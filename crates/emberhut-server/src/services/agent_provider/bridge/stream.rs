//! A streamed Chat Completions answer (`data: {chunk}` lines, `data: [DONE]` at the end) as the
//! Messages API's events, as they come:
//!
//! - `message_start` (and a `ping`) as soon as the endpoint answers;
//! - a `text` block for each run of `content`, its pieces as `text_delta`s;
//! - a `tool_use` block for each tool call once its name is known, its `arguments` as they
//!   come as `input_json_delta`s (pieces of the input's JSON);
//! - each block's `content_block_stop` when the next begins or the answer ends;
//! - `message_delta`, with the stop reason ([`stop_reason`]) and the usage (the last chunk's,
//!   asked for with `stream_options.include_usage`), then `message_stop`.
//!
//! Blocks follow each other, as the Messages API has them: a call's arguments that come after
//! another block began (an endpoint interleaving two calls) can't be put back in it, so the
//! answer ends with an `error` event. So does an error the endpoint streams, and a stream that
//! ends before the endpoint said it finished.

use serde_json::{Value, json};

use super::reply::{stop_reason, tool_id, usage};
use super::request::ToolNames;

/// One event of the Messages API's stream.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Event {
    /// Its name (`event:`), the same as its data's `type`.
    pub(crate) name: &'static str,
    /// Its data.
    pub(crate) data: Value,
}

impl Event {
    fn new(name: &'static str, mut data: Value) -> Self {
        if let Value::Object(fields) = &mut data {
            fields.insert("type".to_owned(), json!(name));
        }
        Self { name, data }
    }

    /// An `error` event of `kind` saying `message`.
    #[must_use]
    pub(crate) fn error(kind: &str, message: &str) -> Self {
        Self::new(
            "error",
            json!({ "error": { "type": kind, "message": message } }),
        )
    }

    /// A `ping`, which keeps a silent stream open.
    #[must_use]
    pub(crate) fn ping() -> Self {
        Self::new("ping", json!({}))
    }

    /// The event as it is written to the stream.
    #[must_use]
    pub(crate) fn to_sse(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.name, self.data)
    }
}

/// Which tool call a piece of one is: by its index when the endpoint gives one, else by its id,
/// else the one before.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CallKey {
    Index(u64),
    Id(String),
}

/// A tool call as it comes.
#[derive(Debug)]
struct Call {
    key: CallKey,
    id: String,
    name: Option<String>,
    /// Arguments that came before its name.
    pending: String,
    /// Its block, once it has begun.
    block: Option<usize>,
}

/// The block open now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Text(usize),
    /// A call (its place in the calls) and its block.
    Call(usize, usize),
}

/// Translates one streamed answer.
#[derive(Debug)]
pub(crate) struct Translator {
    id: String,
    model: String,
    names: ToolNames,
    buffer: Vec<u8>,
    blocks: usize,
    open: Option<Open>,
    calls: Vec<Call>,
    finish: Option<String>,
    usage: Option<Value>,
    done: bool,
}

impl Translator {
    /// A translator for the message `id`, on `model`, reading tools' names back with `names`.
    #[must_use]
    pub(crate) fn new(id: String, model: String, names: ToolNames) -> Self {
        Self {
            id,
            model,
            names,
            buffer: Vec::new(),
            blocks: 0,
            open: None,
            calls: Vec::new(),
            finish: None,
            usage: None,
            done: false,
        }
    }

    /// Whether the answer is over (ended, or failed): nothing more is read.
    #[must_use]
    pub(crate) fn done(&self) -> bool {
        self.done
    }

    /// The events that open the message.
    #[must_use]
    pub(crate) fn start(&self) -> Vec<Event> {
        vec![
            Event::new(
                "message_start",
                json!({ "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": usage(None),
                } }),
            ),
            Event::ping(),
        ]
    }

    /// The events `bytes` (the next of the stream, cut anywhere) make.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        self.buffer.extend_from_slice(bytes);
        while !self.done
            && let Some(end) = self.buffer.iter().position(|&byte| byte == b'\n')
        {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\r', '\n']), &mut events);
        }
        events
    }

    /// The events that end the message once the stream has ended: its last blocks, the stop
    /// reason and usage; an error when the endpoint never said it finished.
    pub(crate) fn end(&mut self) -> Vec<Event> {
        if self.done {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.buffer);
        let mut events = Vec::new();
        if !rest.is_empty() {
            let line = String::from_utf8_lossy(&rest).into_owned();
            self.line(line.trim_end_matches(['\r', '\n']), &mut events);
        }
        if self.done {
            return events;
        }
        if self.finish.is_none() {
            events.push(self.fail(
                "api_error",
                "The model's endpoint stopped sending before it finished its answer.",
            ));
            return events;
        }
        self.close(&mut events);
        events
    }

    /// The `error` event that ends the message when the endpoint can't be read on (a dropped
    /// connection, no word in time).
    pub(crate) fn fail(&mut self, kind: &str, message: &str) -> Event {
        self.done = true;
        Event::error(kind, message)
    }

    fn line(&mut self, line: &str, events: &mut Vec<Event>) {
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            return;
        };
        if data == "[DONE]" {
            self.close(events);
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(error) = chunk.get("error").filter(|error| !error.is_null()) {
            let said = error
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| error.to_string(), str::to_owned);
            let kind = if said.to_lowercase().contains("rate limit") {
                "rate_limit_error"
            } else {
                "api_error"
            };
            let message = format!("The model's endpoint sent an error: {said}");
            events.push(self.fail(kind, &message));
            return;
        }
        if let Some(counted) = chunk.get("usage").filter(|usage| usage.is_object()) {
            self.usage = Some(counted.clone());
        }
        let Some(choice) = chunk.pointer("/choices/0") else {
            return;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                self.text(text, events);
            }
            for piece in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
            {
                self.call(piece, events);
                if self.done {
                    return;
                }
            }
        }
        if let Some(finish) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(finish.to_owned());
        }
    }

    fn text(&mut self, text: &str, events: &mut Vec<Event>) {
        let index = if let Some(Open::Text(index)) = self.open {
            index
        } else {
            let index = self.begin(events);
            self.open = Some(Open::Text(index));
            events.push(Event::new(
                "content_block_start",
                json!({ "index": index, "content_block": { "type": "text", "text": "" } }),
            ));
            index
        };
        events.push(Event::new(
            "content_block_delta",
            json!({ "index": index, "delta": { "type": "text_delta", "text": text } }),
        ));
    }

    fn call(&mut self, piece: &Value, events: &mut Vec<Event>) {
        let id = piece
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let key = match (piece.get("index").and_then(Value::as_u64), id) {
            (Some(index), _) => Some(CallKey::Index(index)),
            (None, Some(id)) => Some(CallKey::Id(id.to_owned())),
            (None, None) => None,
        };
        let known = match &key {
            Some(key) => self.calls.iter().position(|call| call.key == *key),
            None => self.calls.len().checked_sub(1),
        };
        let at = if let Some(at) = known {
            at
        } else {
            let index = self.calls.len();
            self.calls.push(Call {
                key: key.unwrap_or(CallKey::Index(u64::MAX)),
                id: id.map_or_else(|| tool_id(&self.id, index), str::to_owned),
                name: None,
                pending: String::new(),
                block: None,
            });
            index
        };
        let name = piece.pointer("/function/name").and_then(Value::as_str);
        let arguments = piece
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let call = &mut self.calls[at];
        if call.name.is_none()
            && let Some(name) = name.filter(|name| !name.is_empty())
        {
            call.name = Some(name.to_owned());
        }
        match (call.block, self.open) {
            (Some(block), Some(Open::Call(open, _))) if open == at => {
                if !arguments.is_empty() {
                    events.push(input_delta(block, arguments));
                }
            }
            (Some(_), _) => {
                let message = "The model's endpoint interleaved two tool calls, which the \
                               OpenAI bridge can't pass on.";
                events.push(self.fail("api_error", message));
            }
            (None, _) => {
                call.pending.push_str(arguments);
                if call.name.is_some() {
                    self.begin_call(at, events);
                }
            }
        }
    }

    /// Begins call `at`'s block, with the arguments that came before its name.
    fn begin_call(&mut self, at: usize, events: &mut Vec<Event>) {
        let index = self.begin(events);
        self.open = Some(Open::Call(at, index));
        let call = &mut self.calls[at];
        call.block = Some(index);
        let name = self
            .names
            .original(call.name.as_deref().unwrap_or_default());
        events.push(Event::new(
            "content_block_start",
            json!({ "index": index, "content_block": {
                "type": "tool_use", "id": call.id, "name": name, "input": {},
            } }),
        ));
        let pending = std::mem::take(&mut call.pending);
        if !pending.is_empty() {
            events.push(input_delta(index, &pending));
        }
    }

    /// Stops the open block, and gives the next one's index.
    fn begin(&mut self, events: &mut Vec<Event>) -> usize {
        self.stop_open(events);
        let index = self.blocks;
        self.blocks += 1;
        index
    }

    fn stop_open(&mut self, events: &mut Vec<Event>) {
        if let Some(Open::Text(index) | Open::Call(_, index)) = self.open.take() {
            events.push(Event::new("content_block_stop", json!({ "index": index })));
        }
    }

    /// Ends the message: its last block, the stop reason and the usage.
    fn close(&mut self, events: &mut Vec<Event>) {
        if self.calls.iter().any(|call| call.name.is_none()) {
            let message = "The model's endpoint sent a tool call without a name.";
            events.push(self.fail("api_error", message));
            return;
        }
        self.stop_open(events);
        self.done = true;
        let reason = stop_reason(self.finish.as_deref(), !self.calls.is_empty());
        events.push(Event::new(
            "message_delta",
            json!({
                "delta": { "stop_reason": reason, "stop_sequence": null },
                "usage": usage(self.usage.as_ref()),
            }),
        ));
        events.push(Event::new("message_stop", json!({})));
    }
}

fn input_delta(index: usize, json: &str) -> Event {
    Event::new(
        "content_block_delta",
        json!({ "index": index, "delta": { "type": "input_json_delta", "partial_json": json } }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk with `delta` and, maybe, a finish reason.
    fn chunk(delta: &Value, finish: Option<&str>) -> String {
        let chunk = json!({ "id": "chatcmpl-1", "object": "chat.completion.chunk",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] });
        format!("data: {chunk}\n\n")
    }

    fn usage_chunk(prompt: u64, completion: u64) -> String {
        let chunk = json!({ "choices": [],
            "usage": { "prompt_tokens": prompt, "completion_tokens": completion } });
        format!("data: {chunk}\n\n")
    }

    fn translator() -> Translator {
        Translator::new(
            "msg_t".to_owned(),
            "gpt-test".to_owned(),
            ToolNames::default(),
        )
    }

    /// The events `stream` makes, fed in pieces of `step` bytes, then ended.
    fn events(stream: &str, step: usize) -> Vec<Event> {
        let mut translator = translator();
        let mut events = translator.start();
        for piece in stream.as_bytes().chunks(step) {
            events.extend(translator.feed(piece));
        }
        events.extend(translator.end());
        events
    }

    /// Events as their name and their data without its `type`, to compare whole. Data is
    /// compared as JSON values, so the order of an object's keys doesn't matter.
    fn lines(events: &[Event]) -> Vec<(String, Value)> {
        events
            .iter()
            .map(|event| {
                let mut data = event.data.clone();
                if let Value::Object(fields) = &mut data {
                    assert_eq!(fields.remove("type"), Some(json!(event.name)));
                }
                (event.name.to_owned(), data)
            })
            .collect()
    }

    /// `name data` lines, as [`lines`] gives them.
    fn parsed(expected: &[&str]) -> Vec<(String, Value)> {
        expected
            .iter()
            .map(|line| {
                let (name, data) = line.split_once(' ').expect("a name and its data");
                let data = serde_json::from_str(data).expect("the data is JSON");
                (name.to_owned(), data)
            })
            .collect()
    }

    /// The text and each call's whole input, as the CLI would piece them together.
    fn assembled(events: &[Event]) -> (String, Vec<(String, String, Value)>) {
        let mut text = String::new();
        let mut calls: Vec<(String, String, String)> = Vec::new();
        for event in events {
            match (event.name, event.data["delta"]["type"].as_str()) {
                ("content_block_start", _) if event.data["content_block"]["type"] == "tool_use" => {
                    let block = &event.data["content_block"];
                    calls.push((
                        block["id"].as_str().unwrap().to_owned(),
                        block["name"].as_str().unwrap().to_owned(),
                        String::new(),
                    ));
                }
                ("content_block_delta", Some("text_delta")) => {
                    text.push_str(event.data["delta"]["text"].as_str().unwrap());
                }
                ("content_block_delta", Some("input_json_delta")) => {
                    let piece = event.data["delta"]["partial_json"].as_str().unwrap();
                    calls.last_mut().unwrap().2.push_str(piece);
                }
                _ => {}
            }
        }
        let calls = calls
            .into_iter()
            .map(|(id, name, json)| (id, name, serde_json::from_str(&json).unwrap()))
            .collect();
        (text, calls)
    }

    #[test]
    fn text_streams_as_one_text_block() {
        let stream = [
            chunk(&json!({ "role": "assistant", "content": "" }), None),
            chunk(&json!({ "content": "Hel" }), None),
            chunk(&json!({ "content": "lo." }), None),
            chunk(&json!({}), Some("stop")),
            usage_chunk(11, 2),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat();
        let expected = parsed(&[
            r#"message_start {"message":{"content":[],"id":"msg_t","model":"gpt-test","role":"assistant","stop_reason":null,"stop_sequence":null,"type":"message","usage":{"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"input_tokens":0,"output_tokens":0}}}"#,
            "ping {}",
            r#"content_block_start {"content_block":{"text":"","type":"text"},"index":0}"#,
            r#"content_block_delta {"delta":{"text":"Hel","type":"text_delta"},"index":0}"#,
            r#"content_block_delta {"delta":{"text":"lo.","type":"text_delta"},"index":0}"#,
            r#"content_block_stop {"index":0}"#,
            r#"message_delta {"delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"input_tokens":11,"output_tokens":2}}"#,
            "message_stop {}",
        ]);
        // Whole, and cut into pieces of every size a network might.
        for step in [stream.len(), 1, 7, 64] {
            assert_eq!(lines(&events(&stream, step)), expected, "step {step}");
        }
    }

    #[test]
    fn a_tool_call_s_arguments_split_over_many_chunks_stream_as_its_input() {
        let arguments = r#"{"command":"ls -la","description":"List the files"}"#;
        let mut stream = vec![chunk(&json!({ "content": "Let me look." }), None)];
        stream.push(chunk(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_1", "type": "function",
                "function": { "name": "Bash", "arguments": "" } }] }),
            None,
        ));
        for piece in arguments.as_bytes().chunks(5) {
            let piece = std::str::from_utf8(piece).unwrap();
            stream.push(chunk(
                &json!({ "tool_calls": [{ "index": 0, "function": { "arguments": piece } }] }),
                None,
            ));
        }
        stream.push(chunk(&json!({}), Some("tool_calls")));
        stream.push("data: [DONE]\n\n".to_owned());
        let events = events(&stream.concat(), 13);
        let (text, calls) = assembled(&events);
        assert_eq!(text, "Let me look.");
        assert_eq!(
            calls,
            [(
                "call_1".to_owned(),
                "Bash".to_owned(),
                serde_json::from_str::<Value>(arguments).unwrap()
            )]
        );
        let deltas = events
            .iter()
            .filter(|event| event.data["delta"]["type"] == "input_json_delta")
            .count();
        assert_eq!(deltas, arguments.len().div_ceil(5), "piece by piece");
        let names: Vec<&str> = events.iter().map(|event| event.name).collect();
        assert_eq!(
            names[2..5],
            [
                "content_block_start",
                "content_block_delta",
                "content_block_stop"
            ],
            "the text block ends before the call's begins"
        );
        assert_eq!(events[5].data["index"], 1);
        assert_eq!(events[5].data["content_block"]["input"], json!({}));
        let delta = &events[events.len() - 2];
        assert_eq!(delta.data["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn two_parallel_tool_calls_are_two_blocks_in_order() {
        let stream = [
            chunk(
                &json!({ "tool_calls": [{ "index": 0, "id": "call_a",
                    "function": { "name": "Read", "arguments": "{\"path\":" } }] }),
                None,
            ),
            chunk(
                &json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "\"a.txt\"}" } }] }),
                None,
            ),
            // The second call's name comes after its first arguments.
            chunk(
                &json!({ "tool_calls": [{ "index": 1, "id": "call_b",
                    "function": { "arguments": "{\"path\":" } }] }),
                None,
            ),
            chunk(
                &json!({ "tool_calls": [{ "index": 1,
                    "function": { "name": "Read", "arguments": "\"b.txt\"}" } }] }),
                None,
            ),
            chunk(&json!({}), Some("tool_calls")),
            usage_chunk(50, 20),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat();
        let events = events(&stream, 9);
        let (text, calls) = assembled(&events);
        assert_eq!(text, "");
        assert_eq!(
            calls,
            [
                (
                    "call_a".to_owned(),
                    "Read".to_owned(),
                    json!({ "path": "a.txt" })
                ),
                (
                    "call_b".to_owned(),
                    "Read".to_owned(),
                    json!({ "path": "b.txt" })
                ),
            ]
        );
        let starts: Vec<&Value> = events
            .iter()
            .filter(|event| event.name == "content_block_start")
            .map(|event| &event.data["index"])
            .collect();
        assert_eq!(starts, [&json!(0), &json!(1)]);
        let stops: Vec<&Value> = events
            .iter()
            .filter(|event| event.name == "content_block_stop")
            .map(|event| &event.data["index"])
            .collect();
        assert_eq!(stops, [&json!(0), &json!(1)]);
        let delta = &events[events.len() - 2].data;
        assert_eq!(delta["delta"]["stop_reason"], "tool_use");
        assert_eq!(delta["usage"]["input_tokens"], 50);
        assert_eq!(delta["usage"]["output_tokens"], 20);
    }

    #[test]
    fn a_length_finish_is_max_tokens() {
        let stream = [
            chunk(&json!({ "content": "Once upon" }), None),
            chunk(&json!({}), Some("length")),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat();
        let events = events(&stream, 4);
        let delta = &events[events.len() - 2];
        assert_eq!(delta.name, "message_delta");
        assert_eq!(delta.data["delta"]["stop_reason"], "max_tokens");
        assert_eq!(events.last().unwrap().name, "message_stop");
    }

    #[test]
    fn a_whole_call_in_one_chunk_without_an_index_or_done_still_ends_well() {
        // As some local servers stream: the call whole, no index, no `[DONE]`.
        let stream = [
            chunk(
                &json!({ "tool_calls": [{ "id": "call_z",
                    "function": { "name": "Glob", "arguments": "{\"pattern\":\"*.rs\"}" } }] }),
                None,
            ),
            chunk(&json!({}), Some("stop")),
        ]
        .concat();
        let events = events(&stream, 1000);
        let (_, calls) = assembled(&events);
        assert_eq!(
            calls,
            [(
                "call_z".to_owned(),
                "Glob".to_owned(),
                json!({ "pattern": "*.rs" })
            )]
        );
        assert_eq!(
            events[events.len() - 2].data["delta"]["stop_reason"],
            "tool_use"
        );
    }

    #[test]
    fn an_error_an_interleaving_or_an_early_end_is_an_error_event() {
        let failed = events(
            &format!(
                "{}data: {}\n\n",
                chunk(&json!({ "content": "Hi" }), None),
                json!({ "error": { "message": "Rate limit reached", "type": "requests" } })
            ),
            100,
        );
        let last = failed.last().unwrap();
        assert_eq!(last.name, "error");
        assert_eq!(last.data["error"]["type"], "rate_limit_error");
        assert!(
            last.data["error"]["message"]
                .as_str()
                .unwrap()
                .ends_with("Rate limit reached")
        );

        let interleaved = [
            chunk(
                &json!({ "tool_calls": [{ "index": 0, "id": "a", "function": { "name": "A" } }] }),
                None,
            ),
            chunk(
                &json!({ "tool_calls": [{ "index": 1, "id": "b", "function": { "name": "B" } }] }),
                None,
            ),
            chunk(
                &json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{}" } }] }),
                None,
            ),
            chunk(&json!({}), Some("tool_calls")),
        ]
        .concat();
        let events_of = events(&interleaved, 100);
        assert_eq!(events_of.last().unwrap().name, "error");
        assert!(
            events_of.iter().all(|event| event.name != "message_stop"),
            "no answer passes as whole"
        );

        let cut = events(&chunk(&json!({ "content": "Hal" }), None), 100);
        assert_eq!(cut.last().unwrap().name, "error");
        assert_eq!(cut.last().unwrap().data["error"]["type"], "api_error");
    }

    #[test]
    fn an_event_is_written_with_its_name_and_data() {
        let sse = Event::error("api_error", "no").to_sse();
        let data = sse
            .strip_prefix("event: error\ndata: ")
            .and_then(|rest| rest.strip_suffix("\n\n"))
            .expect("one named event with one data line");
        assert!(!data.contains('\n'), "the data is one line");
        assert_eq!(
            serde_json::from_str::<Value>(data).expect("the data is JSON"),
            json!({ "type": "error", "error": { "type": "api_error", "message": "no" } })
        );
    }
}
