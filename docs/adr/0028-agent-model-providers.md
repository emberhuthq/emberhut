# 0028. The agent's model provider: chosen from the apps, handed to the CLI in its environment

- Status: Proposed; `openai` and `openai-compatible` run through the bridge of
  [ADR 0029](0029-openai-bridge.md)
- Date: 2026-11-27
- Amends: [ADR 0014](0014-agent-runtime-and-wake-rule.md)'s accounts (`EMBERHUT_AGENT_ACCOUNTS`),
  which stay the first choice

## Context

The agent runtime (ADR 0014) and the workers (ADR 0018) run Claude Code. Until now the CLI signed
in only through the server's configuration: accounts that are CLI configuration directories, or
whatever login and `ANTHROPIC_*` variables the server's own environment happened to hold. An
owner setting up a new server had to log the CLI in on a terminal.

Owners also want the agent on other models: a Claude API key paid by use, `DeepSeek`,
`OpenRouter`, a local Ollama, a gateway of their own, `OpenAI`. Claude Code speaks Anthropic's
Messages API only; it reaches another provider through that provider's Anthropic-compatible
endpoint, set with `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN`, its models with
`ANTHROPIC_MODEL` and the alias variables (`ANTHROPIC_DEFAULT_{FABLE,OPUS,SONNET,HAIKU}_MODEL`,
`ANTHROPIC_SMALL_FAST_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL`), as Claude Code 2.1 reads them.
`OpenAI`'s Chat Completions API has no such endpoint.

## Decision

- **One provider, saved from the apps.** `PUT /api/agent/provider` (docs/api.md, "Agent
  provider") takes a provider (`claude`, `anthropic`, `deepseek`, `openrouter`, `ollama`,
  `anthropic-compatible`, `openai`, `openai-compatible`) and what it needs: a key, a base URL, a
  model and a quick model. It is checked first with one request that needs the key and costs
  nothing (a model list; `OpenRouter`'s key route; Ollama's version, 0.14 or later, and tags),
  through a `ProviderApi` given to the service, and only what passes is saved. `POST
  /api/agent/provider/test` checks and lists the models without saving.
- **Keys in the secrets folder, nothing secret in the database.** The key is a file,
  `agent-<provider>-key`, for the server's user only, like every other secret; the others' keys
  are removed when one is saved. The provider, base URL, models and when the check passed are a
  setting, with a fingerprint of the key so a key put in the folder by hand is used but not
  reported as checked. No answer, log line or error carries a key, and `Debug` of the request,
  the key and the launch's provider environment leaves it out.
- **Explicit, in the child's environment, everywhere the CLI starts.** A room's session, a
  configured agent's room (which runs with `--setting-sources=`, so the environment is its only
  route), a worker and the guard's judge each read the saved provider when they start and get
  its variables in the child's environment, never on the command line. The server's own
  `ANTHROPIC_*` (by prefix), the switches to other clouds and an inherited sub-agent model are
  kept from the child, so only the saved provider's values reach it; with a model mapped, the
  launch's `--model` (a Claude model's name from the configuration or a task) is dropped. With
  nothing saved, the environment is left as it was, so a server configured through it keeps
  working. A session started on another provider restarts on the new one at its next turn, on
  the same conversation.
- **Accounts first.** While accounts are configured, each is its own login with failover between
  them, and a key in the environment would win over that login: the saved provider isn't used,
  and the apps are told so (`inUse: false`, and the setup checklist).
- **Only what works is run.** `openai` and `openai-compatible` run through the server's OpenAI
  bridge ([ADR 0029](0029-openai-bridge.md)), which speaks Anthropic's API to the CLI on
  loopback and `OpenAI`'s to the endpoint; the CLI gets the bridge's address and a token of its
  launch's own, never the provider's key. Without the bridge they are saved and checked only,
  and the setup checklist reports them `off`.
- **Another endpoint's errors are not a Claude account's.** On a provider with its own base URL,
  a rate-limit notice or an error result is never read as an account out of usage; the turn
  fails with the usual note.
- **No default command.** A saved provider doesn't turn the agent on: without
  `EMBERHUT_AGENT_COMMAND` the server still runs the silent assistant, which is chosen at start-up,
  and the checklist's `provider` item is `off`. Running `claude` from the `PATH` of a server that
  never asked for an agent would surprise its owner.
- **Nothing unattended without something to think with.** With no account configured, a turn
  nobody asked for (a routine, a check-in, the wake rule's, a worker the tick starts) runs only
  on a saved provider the CLI runs on, on `claude`, or, with nothing saved, once a turn showed
  the CLI's own login answers. A CLI that says it isn't logged in is kept as signed out until a
  provider is saved or removed, or a turn answers; the chat gets a plain note instead of its
  words (ADR 0014, "Not logged in").
- **Runners keep their own credentials.** Nothing saved here is sent to a runner's machine.

## Consequences

- A new owner connects the agent from the apps' onboarding: pick a provider, paste a key (or
  choose a local Ollama and one of its models), and the next turn runs on it.
- The setup checklist gains a `provider` item: `ready` on configured accounts, a saved provider
  in use (through the bridge for a bridged one), or the CLI's own login once a turn answered on
  it; `missing` when the owner can fix it from the apps (a key or a bridged provider's model to
  save, or a CLI that said it isn't logged in); `off` without an agent CLI, or for a bridged
  provider on a server without the bridge.
- A fresh server starts no routine before the agent has something to think with, so its first
  chat line is never the CLI's terminal instruction.
- Tests check each provider against a stand-in on loopback and read each launch's environment
  from the scripted spawner; none reaches a provider.
- The checks follow the providers' public routes; a provider that moves them breaks its check,
  not the agent.
