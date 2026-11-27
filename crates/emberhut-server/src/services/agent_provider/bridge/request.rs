//! A Messages API request, as the agent CLI sends it, and its translation into a Chat
//! Completions request.
//!
//! | Anthropic | `OpenAI` |
//! |---|---|
//! | `system` (a string, or text blocks) | a first `system` message |
//! | a user's `text` and `image` blocks | a `user` message: its text, or parts (`text`, `image_url` with a `data:` URL or the image's URL) |
//! | a user's `tool_result` blocks | `tool` messages first, by `tool_call_id`; `is_error` puts `Error: ` before the text; the result's images follow in the user message |
//! | an assistant's `text` and `tool_use` blocks | an `assistant` message with `content` and `tool_calls` (the input as JSON text) |
//! | a `document` | its text when it is plain text, a note that it can't be read otherwise |
//! | `tools` | `function` tools, `input_schema` as `parameters` (Anthropic's server tools are left out) |
//! | `tool_choice` `auto`, `any`, `tool`, `none` | `auto`, `required`, the named function, `none`; `disable_parallel_tool_use` as `parallel_tool_calls: false` |
//! | `max_tokens` | `max_completion_tokens` for `OpenAI`, `max_tokens` for any other endpoint |
//! | `temperature`, `top_p`, `stop_sequences` | `temperature`, `top_p`, `stop` (its first four) |
//! | `stream` | `stream`, with `stream_options.include_usage` |
//!
//! Left out, as `OpenAI` has nothing for them: `thinking` and `redacted_thinking` blocks and the
//! `thinking` setting, `cache_control`, `top_k`, `metadata`, server tools and their results, and
//! every field the bridge doesn't know. A tool's name that `OpenAI` doesn't take (over 64
//! characters, or with other characters than letters, digits, `_` and `-`) is sent as a shorter
//! one of its own and read back ([`ToolNames`]).

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::Failure;

/// The most stop sequences `OpenAI` takes.
const MOST_STOPS: usize = 4;

/// The longest tool name `OpenAI` takes.
const NAME_LENGTH: usize = 64;

/// A Messages API request: what the bridge reads of it.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MessagesRequest {
    /// The model the CLI asks for.
    pub(crate) model: String,
    /// The most tokens the answer may take.
    #[serde(default)]
    pub(crate) max_tokens: Option<u64>,
    /// The system prompt.
    #[serde(default)]
    pub(crate) system: Option<Content>,
    /// The conversation.
    pub(crate) messages: Vec<Message>,
    /// The tools the model may call.
    #[serde(default)]
    pub(crate) tools: Vec<Tool>,
    /// How the model picks a tool.
    #[serde(default)]
    pub(crate) tool_choice: Option<ToolChoice>,
    /// Sampling temperature.
    #[serde(default)]
    pub(crate) temperature: Option<f64>,
    /// Nucleus sampling.
    #[serde(default)]
    pub(crate) top_p: Option<f64>,
    /// Where the answer stops.
    #[serde(default)]
    pub(crate) stop_sequences: Vec<String>,
    /// Whether the answer is streamed.
    #[serde(default)]
    pub(crate) stream: bool,
}

/// One message of the conversation.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Message {
    role: Role,
    content: Content,
}

/// Who said a message.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Assistant,
}

/// A message's (or the system prompt's) content: a string, or blocks.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Content {
    /// Plain text.
    Text(String),
    /// Blocks.
    Blocks(Vec<Block>),
}

/// A content block. Those the bridge doesn't translate are [`Block::Other`].
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Block {
    /// Text.
    Text {
        /// It.
        text: String,
    },
    /// An image.
    Image {
        /// Where it is.
        source: Source,
    },
    /// A document (a PDF, or plain text).
    Document {
        /// Where it is.
        source: Source,
        /// Its title, if given.
        #[serde(default)]
        title: Option<String>,
    },
    /// A call the assistant made.
    ToolUse {
        /// The call's id.
        id: String,
        /// The tool's name.
        name: String,
        /// Its input.
        #[serde(default)]
        input: Value,
    },
    /// A call's result.
    ToolResult {
        /// The call's id.
        tool_use_id: String,
        /// What it gave.
        #[serde(default)]
        content: Option<Content>,
        /// Whether it failed.
        #[serde(default)]
        is_error: bool,
    },
    /// Anything else: thinking, server tools' blocks, and those to come.
    #[serde(other)]
    Other,
}

/// Where an image's or a document's data is.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Source {
    /// In the request, base64-encoded.
    Base64 {
        /// Its type (`image/png`, `application/pdf`).
        media_type: String,
        /// The data.
        data: String,
    },
    /// At a URL.
    Url {
        /// The URL.
        url: String,
    },
    /// Plain text (a document's).
    Text {
        /// The text.
        data: String,
    },
    /// Anything else (a file id, a document's blocks).
    #[serde(other)]
    Other,
}

/// A tool the model may call.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Tool {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    input_schema: Option<Value>,
    /// Set for Anthropic's own tools (`web_search_20250305`, ...); `custom` or none for one
    /// described by its schema.
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// How the model picks a tool.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToolChoice {
    /// As it likes.
    Auto {
        /// One call at most.
        #[serde(default)]
        disable_parallel_tool_use: bool,
    },
    /// Some tool.
    Any {
        /// One call at most.
        #[serde(default)]
        disable_parallel_tool_use: bool,
    },
    /// This tool.
    Tool {
        /// Its name.
        name: String,
        /// One call at most.
        #[serde(default)]
        disable_parallel_tool_use: bool,
    },
    /// None.
    None,
}

/// Which Chat Completions API the endpoint speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// `OpenAI`'s own, which takes `max_completion_tokens` (its reasoning models refuse
    /// `max_tokens`).
    OpenAi,
    /// Another endpoint's (vLLM, LM Studio, Groq, Together...), which all take `max_tokens`.
    Compatible,
}

/// The tools' names as `OpenAI` gets them, and back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolNames {
    /// The name sent, for each name changed.
    back: HashMap<String, String>,
}

impl ToolNames {
    /// The names in `request`: its tools', and those its conversation called.
    #[must_use]
    pub(crate) fn of(request: &MessagesRequest) -> Self {
        let called = request
            .messages
            .iter()
            .flat_map(|message| match &message.content {
                Content::Blocks(blocks) => blocks.as_slice(),
                Content::Text(_) => &[],
            });
        let called = called.filter_map(|block| match block {
            Block::ToolUse { name, .. } => Some(name.as_str()),
            _ => None,
        });
        let back = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .chain(called)
            .filter_map(|name| wire_name(name).map(|sent| (sent, name.to_owned())))
            .collect();
        Self { back }
    }

    /// `name` as it is sent.
    #[must_use]
    pub(crate) fn sent(name: &str) -> String {
        wire_name(name).unwrap_or_else(|| name.to_owned())
    }

    /// The tool `sent` names, as the CLI knows it.
    #[must_use]
    pub(crate) fn original(&self, sent: &str) -> String {
        self.back
            .get(sent)
            .cloned()
            .unwrap_or_else(|| sent.to_owned())
    }
}

/// `name` as `OpenAI` takes it, when it doesn't as it is: its other characters as `_`, cut to
/// fit with a hash of the whole name, so two long names stay apart.
fn wire_name(name: &str) -> Option<String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if !name.is_empty() && name.len() <= NAME_LENGTH && name.chars().all(allowed) {
        return None;
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, name.as_bytes());
    // URL-safe base64 is made of the characters a name takes.
    let hash = URL_SAFE_NO_PAD.encode(&digest.as_ref()[..6]);
    let kept: String = name
        .chars()
        .map(|c| if allowed(c) { c } else { '_' })
        .take(NAME_LENGTH - hash.len() - 1)
        .collect();
    Some(format!("{kept}_{hash}"))
}

/// `request` as a Chat Completions request for `model` at an endpoint speaking `dialect`.
///
/// # Errors
///
/// [`Failure`] (`invalid_request_error`) for an image the bridge can't pass on.
pub(crate) fn translate(
    request: &MessagesRequest,
    model: &str,
    dialect: Dialect,
) -> Result<Value, Failure> {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_ref().map(plain_text)
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        match message.role {
            Role::User => user(&message.content, &mut messages)?,
            Role::Assistant => assistant(&message.content, &mut messages),
        }
    }
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(messages));
    let tools: Vec<Value> = request.tools.iter().filter_map(function).collect();
    if !tools.is_empty() {
        body.insert("tools".to_owned(), Value::Array(tools));
        if let Some(choice) = &request.tool_choice {
            let (choice, single) = tool_choice(choice);
            body.insert("tool_choice".to_owned(), choice);
            if single {
                body.insert("parallel_tool_calls".to_owned(), json!(false));
            }
        }
    }
    if let Some(max) = request.max_tokens {
        let field = match dialect {
            Dialect::OpenAi => "max_completion_tokens",
            Dialect::Compatible => "max_tokens",
        };
        body.insert(field.to_owned(), json!(max));
    }
    if let Some(temperature) = request.temperature {
        body.insert("temperature".to_owned(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        body.insert("top_p".to_owned(), json!(top_p));
    }
    if !request.stop_sequences.is_empty() {
        let stops: Vec<&String> = request.stop_sequences.iter().take(MOST_STOPS).collect();
        body.insert("stop".to_owned(), json!(stops));
    }
    if request.stream {
        body.insert("stream".to_owned(), json!(true));
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    Ok(Value::Object(body))
}

/// A rough count of `body`'s input tokens, a token for every four characters of its messages
/// and tools: what `count_tokens` answers, as the endpoint has no such route.
#[must_use]
pub(crate) fn estimate_tokens(body: &Value) -> u64 {
    let characters = ["messages", "tools"]
        .iter()
        .filter_map(|field| body.get(field))
        .map(|value| value.to_string().chars().count())
        .sum::<usize>();
    u64::try_from(characters.div_ceil(4)).unwrap_or(u64::MAX)
}

/// The text of `content`: its text blocks, a blank line between them.
fn plain_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// A user's message: its tool results as `tool` messages first (they answer the assistant's
/// calls just before), then the rest, with the results' images.
fn user(content: &Content, out: &mut Vec<Value>) -> Result<(), Failure> {
    let blocks = match content {
        Content::Text(text) => {
            out.push(json!({ "role": "user", "content": text }));
            return Ok(());
        }
        Content::Blocks(blocks) => blocks,
    };
    let mut results = Vec::new();
    let mut parts = Vec::new();
    let mut shown = Vec::new();
    for block in blocks {
        match block {
            Block::Text { text } => parts.push(text_part(text)),
            Block::Image { source } => parts.push(image_part(source)?),
            Block::Document { source, title } => {
                parts.push(text_part(&document(source, title.as_deref())));
            }
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let (text, images) = tool_result(content.as_ref())?;
                let text = if *is_error {
                    format!("Error: {text}")
                } else {
                    text
                };
                results
                    .push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": text }));
                if !images.is_empty() {
                    shown.push(text_part(&format!(
                        "The result of tool call {tool_use_id} came with {}:",
                        if images.len() == 1 {
                            "an image"
                        } else {
                            "images"
                        }
                    )));
                    shown.extend(images);
                }
            }
            Block::ToolUse { .. } | Block::Other => {}
        }
    }
    out.extend(results);
    shown.extend(parts);
    if shown.is_empty() {
        return Ok(());
    }
    let only_text = shown.iter().all(|part| part["type"] == "text");
    let content = if only_text {
        let texts: Vec<&str> = shown
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect();
        json!(texts.join("\n\n"))
    } else {
        Value::Array(shown)
    };
    out.push(json!({ "role": "user", "content": content }));
    Ok(())
}

/// An assistant's message: its text, and its tool calls. One with neither (thinking only) is
/// left out: `OpenAI` takes no empty assistant message.
fn assistant(content: &Content, out: &mut Vec<Value>) {
    let blocks = match content {
        Content::Text(text) => {
            out.push(json!({ "role": "assistant", "content": text }));
            return;
        }
        Content::Blocks(blocks) => blocks,
    };
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for block in blocks {
        match block {
            Block::Text { text } if !text.is_empty() => texts.push(text.as_str()),
            Block::ToolUse { id, name, input } => {
                let arguments = if input.is_null() {
                    "{}".to_owned()
                } else {
                    input.to_string()
                };
                calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": ToolNames::sent(name), "arguments": arguments },
                }));
            }
            _ => {}
        }
    }
    if texts.is_empty() && calls.is_empty() {
        return;
    }
    let mut message = Map::new();
    message.insert("role".to_owned(), json!("assistant"));
    let text = (!texts.is_empty()).then(|| texts.join("\n\n"));
    message.insert("content".to_owned(), json!(text));
    if !calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(calls));
    }
    out.push(Value::Object(message));
}

/// A tool result's text, and its images as parts.
fn tool_result(content: Option<&Content>) -> Result<(String, Vec<Value>), Failure> {
    let Some(content) = content else {
        return Ok((String::new(), Vec::new()));
    };
    let blocks = match content {
        Content::Text(text) => return Ok((text.clone(), Vec::new())),
        Content::Blocks(blocks) => blocks,
    };
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for block in blocks {
        match block {
            Block::Text { text } => texts.push(text.clone()),
            Block::Image { source } => images.push(image_part(source)?),
            Block::Document { source, title } => texts.push(document(source, title.as_deref())),
            _ => {}
        }
    }
    Ok((texts.join("\n"), images))
}

/// A text part.
fn text_part(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

/// An image part: a `data:` URL of its bytes, or its URL.
fn image_part(source: &Source) -> Result<Value, Failure> {
    let url = match source {
        Source::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        Source::Url { url } => url.clone(),
        Source::Text { .. } | Source::Other => {
            return Err(Failure::invalid(
                "An image can go through the OpenAI bridge only inline (base64) or by its URL.",
            ));
        }
    };
    Ok(json!({ "type": "image_url", "image_url": { "url": url } }))
}

/// A document as text: its own when it is plain text, a note otherwise (Chat Completions has
/// no document part every endpoint takes).
fn document(source: &Source, title: Option<&str>) -> String {
    let title = title.map_or_else(String::new, |title| format!(" ({title})"));
    match source {
        Source::Text { data } => format!("Document{title}:\n{data}"),
        _ => format!(
            "[A document{title} was attached here, but it can't be passed to this model through \
             the OpenAI bridge.]"
        ),
    }
}

/// `tool` as a function, unless it is one of Anthropic's own tools.
fn function(tool: &Tool) -> Option<Value> {
    if tool.kind.as_deref().is_some_and(|kind| kind != "custom") {
        return None;
    }
    let parameters = tool
        .input_schema
        .clone()
        .unwrap_or_else(|| json!({ "type": "object" }));
    let mut function = Map::new();
    function.insert("name".to_owned(), json!(ToolNames::sent(&tool.name)));
    if let Some(description) = &tool.description {
        function.insert("description".to_owned(), json!(description));
    }
    function.insert("parameters".to_owned(), parameters);
    Some(json!({ "type": "function", "function": function }))
}

/// `choice` as `OpenAI`'s, and whether it allows one call at most.
fn tool_choice(choice: &ToolChoice) -> (Value, bool) {
    match choice {
        ToolChoice::Auto {
            disable_parallel_tool_use,
        } => (json!("auto"), *disable_parallel_tool_use),
        ToolChoice::Any {
            disable_parallel_tool_use,
        } => (json!("required"), *disable_parallel_tool_use),
        ToolChoice::Tool {
            name,
            disable_parallel_tool_use,
        } => (
            json!({ "type": "function", "function": { "name": ToolNames::sent(name) } }),
            *disable_parallel_tool_use,
        ),
        ToolChoice::None => (json!("none"), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> MessagesRequest {
        serde_json::from_value(value).unwrap()
    }

    fn sent(value: Value) -> Value {
        translate(&request(value), "gpt-test", Dialect::OpenAi).unwrap()
    }

    #[test]
    fn requests_translate_case_by_case() {
        // (what the CLI sends, the Chat Completions fields it becomes)
        let cases = [
            (
                json!({ "model": "m", "max_tokens": 100, "system": "Be brief.",
                    "messages": [{ "role": "user", "content": "Hi" }] }),
                json!({ "model": "gpt-test", "max_completion_tokens": 100, "messages": [
                    { "role": "system", "content": "Be brief." },
                    { "role": "user", "content": "Hi" },
                ] }),
            ),
            (
                json!({ "model": "m", "system": [
                        { "type": "text", "text": "One.", "cache_control": { "type": "ephemeral" } },
                        { "type": "text", "text": "Two." }],
                    "messages": [{ "role": "user", "content": [
                        { "type": "text", "text": "A" }, { "type": "text", "text": "B" }] }],
                    "temperature": 0.5, "top_p": 0.9, "top_k": 5,
                    "stop_sequences": ["1", "2", "3", "4", "5"],
                    "thinking": { "type": "enabled", "budget_tokens": 1000 },
                    "metadata": { "user_id": "u" } }),
                json!({ "model": "gpt-test", "messages": [
                    { "role": "system", "content": "One.\n\nTwo." },
                    { "role": "user", "content": "A\n\nB" },
                ], "temperature": 0.5, "top_p": 0.9, "stop": ["1", "2", "3", "4"] }),
            ),
            (
                json!({ "model": "m", "stream": true, "messages": [
                    { "role": "user", "content": "List it" },
                    { "role": "assistant", "content": [
                        { "type": "thinking", "thinking": "hm", "signature": "s" },
                        { "type": "text", "text": "Listing." },
                        { "type": "tool_use", "id": "toolu_1", "name": "Bash",
                            "input": { "command": "ls" } },
                        { "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} }] },
                    { "role": "user", "content": [
                        { "type": "tool_result", "tool_use_id": "toolu_1", "content": "a.txt" },
                        { "type": "tool_result", "tool_use_id": "toolu_2", "is_error": true,
                            "content": [{ "type": "text", "text": "no file" },
                                { "type": "text", "text": "at all" }] },
                        { "type": "text", "text": "Go on." }] },
                    { "role": "assistant", "content": [
                        { "type": "redacted_thinking", "data": "x" }] }] }),
                json!({ "model": "gpt-test", "stream": true,
                    "stream_options": { "include_usage": true }, "messages": [
                    { "role": "user", "content": "List it" },
                    { "role": "assistant", "content": "Listing.", "tool_calls": [
                        { "id": "toolu_1", "type": "function",
                            "function": { "name": "Bash", "arguments": "{\"command\":\"ls\"}" } },
                        { "id": "toolu_2", "type": "function",
                            "function": { "name": "Read", "arguments": "{}" } }] },
                    { "role": "tool", "tool_call_id": "toolu_1", "content": "a.txt" },
                    { "role": "tool", "tool_call_id": "toolu_2", "content": "Error: no file\nat all" },
                    { "role": "user", "content": "Go on." },
                ] }),
            ),
            (
                json!({ "model": "m", "messages": [{ "role": "user", "content": [
                    { "type": "text", "text": "What is it?" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png",
                        "data": "iVBORw0K" } },
                    { "type": "image", "source": { "type": "url",
                        "url": "https://images.example.com/cat.png" } }] }] }),
                json!({ "model": "gpt-test", "messages": [{ "role": "user", "content": [
                    { "type": "text", "text": "What is it?" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw0K" } },
                    { "type": "image_url",
                        "image_url": { "url": "https://images.example.com/cat.png" } },
                ] }] }),
            ),
            (
                json!({ "model": "m", "messages": [{ "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_9", "content": [
                        { "type": "text", "text": "Read the image." },
                        { "type": "image", "source": { "type": "base64",
                            "media_type": "image/jpeg", "data": "/9j/" } }] }] }] }),
                json!({ "model": "gpt-test", "messages": [
                    { "role": "tool", "tool_call_id": "toolu_9", "content": "Read the image." },
                    { "role": "user", "content": [
                        { "type": "text", "text": "The result of tool call toolu_9 came with an image:" },
                        { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,/9j/" } },
                    ] },
                ] }),
            ),
            (
                json!({ "model": "m", "messages": [{ "role": "user", "content": [
                    { "type": "document", "title": "notes",
                        "source": { "type": "text", "media_type": "text/plain", "data": "x = 1" } },
                    { "type": "document",
                        "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBE" } }] }] }),
                json!({ "model": "gpt-test", "messages": [{ "role": "user", "content":
                    "Document (notes):\nx = 1\n\n[A document was attached here, but it can't be \
                     passed to this model through the OpenAI bridge.]" }] }),
            ),
        ];
        for (index, (anthropic, openai)) in cases.into_iter().enumerate() {
            assert_eq!(sent(anthropic), openai, "case {index}");
        }
    }

    #[test]
    fn tools_and_the_choice_of_one_are_functions() {
        let schema = json!({ "type": "object", "properties": { "path": { "type": "string" } },
            "required": ["path"], "additionalProperties": false });
        let base = |choice: Value| {
            json!({ "model": "m", "messages": [{ "role": "user", "content": "x" }],
                "tool_choice": choice, "tools": [
                { "name": "Read", "description": "Reads a file.", "input_schema": schema,
                    "cache_control": { "type": "ephemeral" } },
                { "type": "web_search_20250305", "name": "web_search", "max_uses": 3 },
                { "type": "custom", "name": "Plain" }] })
        };
        let body = sent(base(json!({ "type": "auto" })));
        assert_eq!(
            body["tools"],
            json!([
                { "type": "function", "function": { "name": "Read",
                    "description": "Reads a file.", "parameters": schema } },
                { "type": "function", "function": { "name": "Plain",
                    "parameters": { "type": "object" } } },
            ]),
            "Anthropic's own tools are left out"
        );
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body.get("parallel_tool_calls"), None);
        // (Anthropic's choice, OpenAI's, whether parallel calls are off)
        for (choice, openai, single) in [
            (json!({ "type": "any" }), json!("required"), false),
            (
                json!({ "type": "auto", "disable_parallel_tool_use": true }),
                json!("auto"),
                true,
            ),
            (
                json!({ "type": "tool", "name": "Read", "disable_parallel_tool_use": true }),
                json!({ "type": "function", "function": { "name": "Read" } }),
                true,
            ),
            (json!({ "type": "none" }), json!("none"), false),
        ] {
            let body = sent(base(choice.clone()));
            assert_eq!(body["tool_choice"], openai, "{choice}");
            assert_eq!(
                body.get("parallel_tool_calls") == Some(&json!(false)),
                single,
                "{choice}"
            );
        }
        let none = sent(json!({ "model": "m", "tool_choice": { "type": "any" },
            "messages": [{ "role": "user", "content": "x" }] }));
        assert_eq!(none.get("tools"), None);
        assert_eq!(none.get("tool_choice"), None, "no choice without tools");
    }

    #[test]
    fn another_endpoint_gets_max_tokens() {
        let request = request(json!({ "model": "m", "max_tokens": 32000,
            "messages": [{ "role": "user", "content": "x" }] }));
        let body = translate(&request, "local", Dialect::Compatible).unwrap();
        assert_eq!(body["max_tokens"], 32000);
        assert_eq!(body.get("max_completion_tokens"), None);
    }

    #[test]
    fn a_tool_name_openai_refuses_is_sent_as_its_own_and_read_back() {
        let long = format!("mcp__{}__search", "a-very-long-server-name".repeat(3));
        let dotted = "mcp__docs.example__find";
        let request = request(json!({ "model": "m", "tools": [
                { "name": long, "input_schema": { "type": "object" } },
                { "name": "Bash", "input_schema": { "type": "object" } }],
            "messages": [
                { "role": "user", "content": "x" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "t1", "name": dotted, "input": {} }] }] }));
        let names = ToolNames::of(&request);
        let body = translate(&request, "m", Dialect::OpenAi).unwrap();
        let wire = body["tools"][0]["function"]["name"].as_str().unwrap();
        assert!(wire.len() <= NAME_LENGTH, "{wire}");
        assert!(wire.starts_with("mcp__a-very-long"));
        assert_eq!(names.original(wire), long);
        assert_eq!(body["tools"][1]["function"]["name"], "Bash");
        assert_eq!(names.original("Bash"), "Bash");
        let called = body["messages"][1]["tool_calls"][0]["function"]["name"]
            .as_str()
            .unwrap();
        assert!(called.starts_with("mcp__docs_example__find_"), "{called}");
        assert_eq!(called.len(), "mcp__docs_example__find_".len() + 8);
        assert_eq!(names.original(called), dotted);
        assert_ne!(
            ToolNames::sent(&format!("{long}2")),
            wire,
            "two long names stay apart"
        );
    }

    #[test]
    fn an_image_by_file_id_is_refused_plainly() {
        let request = request(
            json!({ "model": "m", "messages": [{ "role": "user", "content": [
            { "type": "image", "source": { "type": "file", "file_id": "file_1" } }] }] }),
        );
        let failure = translate(&request, "m", Dialect::OpenAi).unwrap_err();
        assert_eq!(failure.kind, "invalid_request_error");
        assert!(
            failure.message.contains("only inline"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn the_estimate_counts_a_token_per_four_characters() {
        let body = json!({ "messages": [{ "role": "user", "content": "abcd" }] });
        let characters = body["messages"].to_string().len();
        assert_eq!(
            estimate_tokens(&body),
            u64::try_from(characters.div_ceil(4)).unwrap()
        );
        assert_eq!(estimate_tokens(&json!({})), 0);
    }
}
