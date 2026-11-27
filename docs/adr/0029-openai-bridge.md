# 0029. The OpenAI bridge: Anthropic's Messages API on loopback, answered by a Chat Completions endpoint

- Status: Proposed
- Date: 2026-11-27
- Amends: [ADR 0028](0028-agent-model-providers.md), whose `openai` and `openai-compatible`
  providers were saved and checked but not run on

## Context

The agent CLI is Claude Code, which speaks Anthropic's Messages API only. `OpenAI` and the
endpoints that copy its API (vLLM, LM Studio, Groq, Together, a gateway of the owner's) speak
Chat Completions, and have no Anthropic-compatible endpoint to point `ANTHROPIC_BASE_URL` at.
ADR 0028 saves and checks such a provider, then runs nothing on it.

Something has to speak Anthropic's API to the CLI and Chat Completions to the endpoint. The
server already starts every launch of the CLI and holds the provider's key; a bridge inside it
needs no other process to install, supervise or update.

## Decision

- **A listener of its own, on loopback.** The server binds the bridge on `127.0.0.1` at a port
  the system picks, when an agent CLI is configured (`EMBERHUT_AGENT_COMMAND`), and serves it
  beside the API. Not a route on the API's listener: that one may face the network
  (`EMBERHUT_LISTEN=0.0.0.0:…`), a relay or a sealed transport, and carries the API's session
  auth, request time limit and OpenAPI document, none of which fit a stream that runs for
  minutes and is no device's business. A loopback listener can't be reached from another
  device whatever the configuration says; the bridge also refuses a request whose peer isn't
  loopback, or that has no peer address. Its routes are not the documented API: they are
  listed here, not in `docs/api.md` or `openapi.json`.
- **A token per launch, the key never in the child.** Each launch on a bridged provider gets
  `ANTHROPIC_BASE_URL` set to the bridge, `ANTHROPIC_AUTH_TOKEN` set to 256 random bits of its
  own, and `ANTHROPIC_API_KEY` empty. The launch holds its token's grant; the spawner keeps it
  until the CLI has exited, and dropping it revokes the token. The bridge keeps only the
  tokens' SHA-256 hashes and compares a presented one with every live hash in constant time,
  as a bearer or as `x-api-key`. The provider's key stays in the secrets folder: the bridge
  reads it, and the base URL and models, at each request, so a key saved again is used at once,
  and a provider changed to another is refused (403 `permission_error`) until the session
  restarts on the new one at its next turn (ADR 0028). Another local user would need the token,
  which is only in the CLI's environment (readable by the server's user alone); a web page in
  the owner's browser can't send the `Authorization` header across origins without a CORS
  answer the bridge never gives.
- **A model is needed.** `openai` and `openai-compatible` need a model to be saved (a test
  lists them without one); the quick model defaults to it. Every model variable the CLI reads
  is mapped to them (ADR 0028), and a name the CLI has built in reaches the bridge as the
  saved quick model when it is a `haiku` model and the saved model otherwise.
- **Routes.** `POST /v1/messages` (with or without `?beta=true`), streamed or not, sent on as
  `POST <base>/chat/completions` with the key as bearer; `POST /v1/messages/count_tokens`,
  answered with an estimate, a token for four characters of the translated messages and tools,
  as Chat Completions has no such route (the CLI uses the count for its own bookkeeping, and an
  estimate keeps it working where a 404 would leave it guessing); anything else 404
  `not_found_error`. Requests up to 32 MiB, the Messages API's own limit.
- **Translation** (`services/agent_provider/bridge/{request,reply,stream}.rs` have the tables):
  - the system prompt (a string, or text blocks joined) as a first `system` message;
  - user text and images (base64 as a `data:` URL, or by URL) as a `user` message; an image by
    file id is refused (400) with a sentence saying so;
  - `tool_result` blocks as `tool` messages by `tool_call_id`, ahead of the rest of their user
    message; `is_error` puts `Error: ` before the text; a result's images follow in the user
    message, as a `tool` message takes text only;
  - assistant text and `tool_use` blocks as one `assistant` message with `tool_calls`;
  - tools as `function` tools, the input schema passed through as `parameters`; a tool name
    `OpenAI` doesn't take (over 64 characters, other characters than `[A-Za-z0-9_-]`) as a
    shortened one with a hash, read back in the answer;
  - `tool_choice` `auto`/`any`/`tool`/`none` as `auto`/`required`/the function/`none`,
    `disable_parallel_tool_use` as `parallel_tool_calls: false`;
  - `max_tokens` as `max_completion_tokens` for `OpenAI` (its reasoning models refuse
    `max_tokens`) and `max_tokens` for any other endpoint; `temperature`, `top_p`, and the
    first four `stop_sequences` as `stop`;
  - the stream: `message_start` and a `ping` once the endpoint answers; a `text` block per run
    of content (`text_delta`); a `tool_use` block per call once its name is known, its
    arguments as `input_json_delta` pieces as they come; `content_block_stop` as the next block
    begins; `message_delta` with the stop reason (`stop` → `end_turn`, `length` →
    `max_tokens`, `tool_calls` → `tool_use`, `content_filter` → `refusal`, and `tool_use`
    whenever a call was made) and the usage (`prompt_tokens` less the cached ones as
    `input_tokens`, `cached_tokens` as `cache_read_input_tokens`, `completion_tokens` as
    `output_tokens`), then `message_stop`. A `ping` goes out every 15 s of silence; ten minutes
    of it end the stream;
  - an answer that isn't streamed is the same message whole.
- **Left out, safely.** `thinking` and `redacted_thinking` blocks and the `thinking` setting
  (Anthropic's thinking needs a signature no other endpoint gives; reasoning text an endpoint
  streams is dropped too), `cache_control`, `top_k`, `metadata`, Anthropic's server tools and
  their result blocks, and every field the bridge doesn't know. A PDF (or any document that
  isn't plain text) becomes a sentence saying it couldn't be passed on; a plain-text document
  is passed as text.
- **Errors are the Messages API's.** `{"type":"error","error":{"type":…,"message":…}}`, the
  message plain and the endpoint's own words after it (its key cut out if it quotes it). The
  endpoint's 400/422 are `invalid_request_error` (a context too long reads `prompt is too long:
  …`, which the CLI compacts on), 401 `authentication_error`, 403 `permission_error`, 404
  `not_found_error`, 413 `request_too_large`, 429 `rate_limit_error` with its `retry-after`,
  503 and 529 `overloaded_error` (529), any other 502 `api_error`; an endpoint that can't be
  reached is 502, one that doesn't answer in ten minutes 504. Mid-stream, an error the endpoint
  streams, a broken connection, a stream that ends before a finish reason, and arguments of a
  call that come after the next block began (an endpoint interleaving two calls, which the
  Messages API's sequential blocks can't hold) end the stream with an `error` event, so the
  CLI never takes half an answer for a whole one.
- **Its own HTTP client**, made like the shared one (no redirects, no proxy, rustls with
  ring), given to the bridge at construction: the agent's long streams don't hold the pool the
  other services share, and tests give it a client for a stand-in on loopback.

## Consequences

- An owner can run the agent on `OpenAI`, or on a local or hosted model behind an OpenAI-style
  endpoint, from the apps' onboarding; the setup checklist reads `ready` ("through the server's
  OpenAI bridge").
- Verified only against stand-ins: tests translate requests case by case, translate recorded
  chunk sequences (text only; a call's arguments over many chunks; two calls; a name after its
  first arguments; `length`; a whole call without an index or `[DONE]`; errors), and run a
  Claude-Code-like client through the bridge to a stand-in `OpenAI` endpoint on loopback, with
  and without the launch's token. No test reaches a real provider, and no real Claude Code was
  run through the bridge here.
- Claude Code's prompts and tools are written for Claude. Other models, and smaller ones most,
  may call its tools less well: malformed arguments reach the CLI as they are (text that isn't
  a JSON object is passed as `{"arguments": "<text>"}`, so the tool says what is wrong), and
  some endpoints cap `max_tokens` below the 32 000 the CLI asks for and refuse the request. The
  bridge passes those refusals on; it doesn't second-guess the model.
- Images through an endpoint without vision are its refusal to make. Prompt caching,
  extended thinking and Anthropic's server tools (web search, code execution) are not
  available on a bridged provider.
- The bridge's port changes at every start; launches read it from the server, so nothing is
  configured for it.
