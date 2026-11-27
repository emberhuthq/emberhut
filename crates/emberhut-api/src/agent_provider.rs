//! `/api/agent/provider`: what the server's agent thinks with, chosen from a signed-in device
//! (`docs/api.md`, "Agent provider"): Claude Code on the owner's own Claude login, an Anthropic
//! API key, or another model provider (`DeepSeek`, `OpenRouter`, a local Ollama, any
//! Anthropic-compatible endpoint, and `OpenAI` and OpenAI-compatible endpoints through the
//! server's `OpenAI` bridge).
//!
//! A key goes one way: a device sends it, the server checks it with the provider and keeps it
//! in its secrets folder, and no answer ever carries it back. What the apps read is which
//! provider is saved, the key's last four characters, the base URL and model, when the provider
//! last accepted them, and whether the agent runs on them ([`AgentProvider::in_use`]).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// A model provider the agent can be pointed at.
    pub enum ProviderKind {
        /// Claude Code on the owner's own Claude login (the agent CLI's, or the server's
        /// configured accounts): no key.
        Claude = "claude",
        /// Anthropic's API, with an API key (`sk-ant-…`).
        Anthropic = "anthropic",
        /// `DeepSeek`'s API, with an API key, through its Anthropic-compatible endpoint.
        DeepSeek = "deepseek",
        /// `OpenRouter`, with an API key, through its Anthropic-compatible endpoint.
        OpenRouter = "openrouter",
        /// A local Ollama, 0.14 or later (`http://localhost:11434` unless a base URL says
        /// otherwise): no key.
        Ollama = "ollama",
        /// Any endpoint that speaks Anthropic's Messages API: a base URL, a key and a model.
        AnthropicCompatible = "anthropic-compatible",
        /// `OpenAI`'s API, with an API key and a model, through the server's `OpenAI` bridge.
        OpenAi = "openai",
        /// Any endpoint that speaks `OpenAI`'s Chat Completions API (vLLM, LM Studio, Groq,
        /// Together...): a base URL, a model, and a key when it needs one; through the server's
        /// `OpenAI` bridge.
        OpenAiCompatible = "openai-compatible",
    }
}

/// `PUT /api/agent/provider` and `POST /api/agent/provider/test`: a provider and what it needs.
/// Its `Debug` leaves the key out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ProviderRequest {
    /// The provider.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub provider: ProviderKind,
    /// Its API key: needed by `anthropic`, `deepseek`, `openrouter`, `anthropic-compatible`
    /// and `openai`, optional for `openai-compatible`, not taken by `claude` and `ollama`.
    /// Surrounding whitespace is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub api_key: Option<String>,
    /// Where its API is: needed by `anthropic-compatible` (what the agent CLI is pointed at)
    /// and `openai-compatible` (`https://…/v1`); for `deepseek` (`https://api.deepseek.com`),
    /// `openrouter` (`https://openrouter.ai/api`), `ollama` (`http://localhost:11434`) and
    /// `openai` (`https://api.openai.com/v1`) in place of their usual address; not taken by
    /// `claude` and `anthropic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub base_url: Option<String>,
    /// The model the agent runs on, as the provider names it: needed by `openrouter`,
    /// `ollama`, `anthropic-compatible`, `openai` and `openai-compatible` (to save; a test
    /// takes none); none: the provider's default (`deepseek-v4-pro` for `deepseek`) or, for
    /// `claude` and `anthropic`, the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub model: Option<String>,
    /// The model for quick side calls (titles, summaries, the guard's judge); none: the
    /// provider's default (`deepseek-v4-flash` for `deepseek`) or `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub fast_model: Option<String>,
}

impl fmt::Debug for ProviderRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRequest")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("fast_model", &self.fast_model)
            .finish_non_exhaustive()
    }
}

/// What the server holds of the agent's provider, without the key: the answer of `GET`, `PUT`
/// and `DELETE /api/agent/provider`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentProvider {
    /// The provider saved; left out when none is (the agent CLI runs on its own login).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// A key is saved for it.
    pub key_set: bool,
    /// The saved key's last four characters, to tell keys apart; only when one is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hint: Option<String>,
    /// The base URL saved, when one was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The model saved, when one was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model for quick side calls saved, when one was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_model: Option<String>,
    /// When the provider last accepted what is saved; left out when the server didn't check it
    /// (a key put in the secrets folder by hand, or `claude`, which only the CLI can check).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<Timestamp>,
    /// The agent can run on this provider: true for every provider this server knows
    /// (`openai` and `openai-compatible` run through its `OpenAI` bridge), false when nothing is
    /// saved. Kept for apps that read it.
    pub usable: bool,
    /// The server's agent sessions run on it now: no when no agent CLI is configured, when the
    /// server's configured accounts (`EMBERHUT_AGENT_ACCOUNTS`) are its credentials instead,
    /// or when what it needs isn't saved (its key's file removed, no model for a bridged one).
    pub in_use: bool,
}

impl AgentProvider {
    /// Nothing saved.
    pub const NONE: Self = Self {
        provider: None,
        key_set: false,
        key_hint: None,
        base_url: None,
        model: None,
        fast_model: None,
        checked_at: None,
        usable: false,
        in_use: false,
    };
}

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderModel {
    /// Its id, as a request names it.
    pub id: String,
    /// Its name for people, when the provider gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `POST /api/agent/provider/test`'s answer: the provider accepted what was sent (nothing is
/// saved), and the models it lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderTest {
    /// The provider was asked: no for `claude`, which only the agent CLI itself can check.
    pub checked: bool,
    /// The models it lists, as it lists them; empty when it lists none or wasn't asked.
    pub models: Vec<ProviderModel>,
}
