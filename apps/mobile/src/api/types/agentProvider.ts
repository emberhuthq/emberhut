import type { Instant } from "./common";

/**
 * A model provider the server's agent can think with (`/api/agent/provider`). `claude` is the agent CLI on its own login,
 * no key; `ollama` is a local Ollama, no key; `openai` and `openai-compatible` are saved and checked but not run on yet. A
 * newer server may send a provider this app doesn't know, so any other word is let through as it is.
 */
export type ProviderKind =
  "claude" | "anthropic" | "deepseek" | "openrouter" | "ollama" | "anthropic-compatible" | "openai" | "openai-compatible" | (string & {});

/**
 * PUT /api/agent/provider and POST /api/agent/provider/test: a provider and what it needs. A field left blank is left out,
 * never sent as "". The server keeps no key to fall back on: every check of a provider that takes one sends the key.
 */
export interface ProviderRequest {
  provider: ProviderKind;
  /** Its API key; not taken by `claude` and `ollama`. */
  apiKey?: string;
  /** Where its API is: needed by `anthropic-compatible` and `openai-compatible`, else in place of the usual address. */
  baseUrl?: string;
  /** The model the agent runs on, as the provider names it; not needed for a test. */
  model?: string;
  /** The model for quick side calls (titles, summaries); none: the provider's default or `model`. */
  fastModel?: string;
}

/** GET, PUT and DELETE /api/agent/provider: what the server holds of the agent's provider, never the key. */
export interface AgentProvider {
  /** Left out when nothing is saved: the agent CLI runs on its own login. */
  provider?: ProviderKind;
  /** A key is saved for it. */
  keySet: boolean;
  /** The saved key's last four characters, to tell keys apart; only when one is saved. */
  keyHint?: string;
  baseUrl?: string;
  model?: string;
  fastModel?: string;
  /** When the provider last accepted what is saved; left out when the server didn't check it. */
  checkedAt?: Instant;
  /** The agent can run on the saved provider today. */
  usable: boolean;
  /** The server's agent sessions run on it now: not when the server's own configured accounts come first. */
  inUse: boolean;
}

/** One model a provider offers. */
export interface ProviderModel {
  /** Its id, as a request names it. */
  id: string;
  /** Its name for people, when the provider gives one. */
  name?: string;
}

/** POST /api/agent/provider/test: the provider accepted what was sent (nothing is saved), and the models it lists. */
export interface ProviderTest {
  /** The provider was asked: no for `claude`, which only the agent CLI itself can check. */
  checked: boolean;
  /** As the provider lists them; empty when it lists none or wasn't asked. */
  models: ProviderModel[];
}
