//! A Chat Completions answer as a Messages API one: the whole message when it isn't streamed,
//! and what both kinds share, the stop reason and the usage.
//!
//! | `finish_reason` | `stop_reason` |
//! |---|---|
//! | `stop` | `end_turn` (`tool_use` when the answer called a tool: some endpoints say `stop` then) |
//! | `length` | `max_tokens` |
//! | `tool_calls`, `function_call` | `tool_use` |
//! | `content_filter` | `refusal` |
//! | none, or another | `end_turn`, or `tool_use` with a call |
//!
//! `stop_sequence` is always `null`: `OpenAI` doesn't say which stop sequence it met.
//! `prompt_tokens` are `input_tokens`, but for those `OpenAI` read from its cache
//! (`prompt_tokens_details.cached_tokens`), which are `cache_read_input_tokens`;
//! `completion_tokens` are `output_tokens`. Reasoning text an endpoint sends
//! (`reasoning_content`) is left out: Anthropic's thinking blocks carry a signature `OpenAI`
//! can't give.

use serde_json::{Value, json};

use super::request::ToolNames;

/// The Messages API's stop reason for `finish` (`OpenAI`'s `finish_reason`), on an answer that
/// called a tool or not.
#[must_use]
pub(crate) fn stop_reason(finish: Option<&str>, called: bool) -> &'static str {
    match finish {
        Some("length") => "max_tokens",
        Some("content_filter") => "refusal",
        Some("tool_calls" | "function_call") => "tool_use",
        _ if called => "tool_use",
        _ => "end_turn",
    }
}

/// `usage` (`OpenAI`'s) as the Messages API counts it; zeros when there is none.
#[must_use]
pub(crate) fn usage(usage: Option<&Value>) -> Value {
    let count = |field: &str| {
        usage
            .and_then(|usage| usage.get(field))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = usage
        .and_then(|usage| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "input_tokens": count("prompt_tokens").saturating_sub(cached),
        "output_tokens": count("completion_tokens"),
        "cache_read_input_tokens": cached,
        "cache_creation_input_tokens": 0,
    })
}

/// A tool call's input from its `arguments` (JSON text); text that isn't a JSON object is kept
/// whole under `arguments`, so the CLI's tool says what is wrong with it.
#[must_use]
pub(crate) fn tool_input(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(input @ Value::Object(_)) => input,
        _ => json!({ "arguments": arguments }),
    }
}

/// The message `reply` (a whole Chat Completions answer) is, with the id `id`, for `model`; the
/// tools' names read back with `names`, and a call without an id given one of `id`'s.
#[must_use]
pub(crate) fn message(reply: &Value, id: &str, model: &str, names: &ToolNames) -> Value {
    let choice = reply.pointer("/choices/0");
    let said = choice.and_then(|choice| choice.get("message"));
    let mut content = Vec::new();
    if let Some(text) = said
        .and_then(|said| said.get("content"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        content.push(json!({ "type": "text", "text": text }));
    }
    let calls = said
        .and_then(|said| said.get("tool_calls"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for (index, call) in calls.iter().enumerate() {
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let call_id = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|call_id| !call_id.is_empty())
            .map_or_else(|| tool_id(id, index), str::to_owned);
        content.push(json!({
            "type": "tool_use",
            "id": call_id,
            "name": names.original(name),
            "input": tool_input(arguments),
        }));
    }
    let finish = choice
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str);
    json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason(finish, !calls.is_empty()),
        "stop_sequence": null,
        "usage": usage(reply.get("usage")),
    })
}

/// The id given the `index`th tool call of message `id`, when the endpoint gave it none.
#[must_use]
pub(crate) fn tool_id(id: &str, index: usize) -> String {
    let id = id.strip_prefix("msg_").unwrap_or(id);
    format!("toolu_{id}_{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reasons_become_stop_reasons() {
        for (finish, called, reason) in [
            (Some("stop"), false, "end_turn"),
            (Some("stop"), true, "tool_use"),
            (Some("length"), false, "max_tokens"),
            (Some("length"), true, "max_tokens"),
            (Some("tool_calls"), true, "tool_use"),
            (Some("function_call"), true, "tool_use"),
            (Some("content_filter"), false, "refusal"),
            (None, false, "end_turn"),
            (None, true, "tool_use"),
        ] {
            assert_eq!(stop_reason(finish, called), reason, "{finish:?} {called}");
        }
    }

    #[test]
    fn a_whole_answer_is_a_message_with_its_text_calls_and_usage() {
        let reply = json!({
            "id": "chatcmpl-1",
            "choices": [{ "index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": "Looking.", "reasoning_content": "hm",
                "tool_calls": [
                    { "id": "call_a", "type": "function",
                        "function": { "name": "Read", "arguments": "{\"path\":\"a.txt\"}" } },
                    { "type": "function", "function": { "name": "Glob", "arguments": "" } },
                    { "id": "call_c", "type": "function",
                        "function": { "name": "Bash", "arguments": "not json" } }] } }],
            "usage": { "prompt_tokens": 120, "completion_tokens": 30,
                "prompt_tokens_details": { "cached_tokens": 100 } },
        });
        let message = message(&reply, "msg_x1", "gpt-test", &ToolNames::default());
        assert_eq!(
            message,
            json!({
                "id": "msg_x1", "type": "message", "role": "assistant", "model": "gpt-test",
                "content": [
                    { "type": "text", "text": "Looking." },
                    { "type": "tool_use", "id": "call_a", "name": "Read",
                        "input": { "path": "a.txt" } },
                    { "type": "tool_use", "id": "toolu_x1_1", "name": "Glob", "input": {} },
                    { "type": "tool_use", "id": "call_c", "name": "Bash",
                        "input": { "arguments": "not json" } },
                ],
                "stop_reason": "tool_use", "stop_sequence": null,
                "usage": { "input_tokens": 20, "output_tokens": 30,
                    "cache_read_input_tokens": 100, "cache_creation_input_tokens": 0 },
            })
        );
    }

    #[test]
    fn an_answer_cut_short_says_max_tokens_and_no_usage_is_zeros() {
        let reply = json!({ "choices": [{ "finish_reason": "length",
            "message": { "role": "assistant", "content": "Half" } }] });
        let message = message(&reply, "msg_2", "m", &ToolNames::default());
        assert_eq!(message["stop_reason"], "max_tokens");
        assert_eq!(
            message["content"],
            json!([{ "type": "text", "text": "Half" }])
        );
        assert_eq!(message["usage"]["input_tokens"], 0);
        assert_eq!(message["usage"]["output_tokens"], 0);
    }
}
