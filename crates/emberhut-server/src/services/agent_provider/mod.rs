//! The agent's model provider (`/api/agent/provider`): what the server's agent thinks with,
//! chosen from a signed-in device so an owner never needs a terminal for it.
//!
//! - **Checked before saved.** What is sent is checked with the provider ([`ProviderApi`]):
//!   one authenticated request that costs nothing and lists the models. `POST
//!   /api/agent/provider/test` stops there. `claude` (the agent CLI's own login) can only be
//!   checked by the CLI, so it is saved unchecked.
//! - **Kept apart.** One provider is saved at a time ([`ProviderStore`]). Its key is a file in
//!   the secrets folder (`agent-<provider>-key`, the server's user's only), and the keys of the
//!   others are removed when it is saved. The provider, its base URL and models, and when the
//!   check passed are a setting, with a fingerprint of the key so a key put in the folder by hand
//!   isn't reported as checked. No answer, log line or error carries a key.
//! - **Run where it works.** The agent CLI is Claude Code, which speaks Anthropic's API: every
//!   launch of it (a room's session, a configured agent's room, a worker, the guard's judge)
//!   reads the saved provider when it starts and gets its environment ([`ProviderEnv`]):
//!   Anthropic's key, or an Anthropic-compatible endpoint (`DeepSeek`'s, `OpenRouter`'s, a local
//!   Ollama's, any other) with its token and models. `OpenAI` and OpenAI-compatible endpoints are
//!   reached through the server's own [`bridge`], which speaks Anthropic's API to the CLI on
//!   loopback and `OpenAI`'s to the endpoint.
//! - **Accounts first.** While the server configures accounts (`EMBERHUT_AGENT_ACCOUNTS`), each
//!   a CLI login of its own with failover between them, the saved provider isn't used.

pub mod bridge;
mod check;
mod env;

use std::fmt;
use std::sync::Arc;

use base64::Engine as _;
use emberhut_api::agent_provider::{
    AgentProvider, ProviderKind, ProviderModel, ProviderRequest, ProviderTest,
};
use emberhut_api::time::Timestamp;
use emberhut_core::Clock;
use emberhut_db::{DbError, SettingKey, SettingStore};
use serde::{Deserialize, Serialize};

pub use check::{
    ANTHROPIC_VERSION, HttpProviderApi, NoProviderApi, OLLAMA_SINCE, ProviderApi, ProviderCheck,
    Target,
};
pub use env::{MODEL_VARS, OLLAMA_TOKEN, ProviderEnv};

use self::bridge::BridgeAccess;
use crate::convert::timestamp_to_api;
use crate::services::claude::AgentAccount;
use crate::services::messaging::secrets::Secrets;

/// What every Anthropic API key starts with.
pub const ANTHROPIC_KEY_PREFIX: &str = "sk-ant-";

/// The shortest and longest key taken.
const KEY_LENGTH: std::ops::RangeInclusive<usize> = 8..=512;

/// The longest model name taken.
const MODEL_LENGTH: usize = 200;

/// What is saved besides the key.
const SAVED: SettingKey<Saved> = SettingKey::new("agentProvider");

/// The provider saved, what it was given besides its key, and when it was checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fast_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked: Option<Checked>,
}

/// A check that passed: when, and for which key (a fingerprint, not the key; none without one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Checked {
    at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

/// An API key for a model provider. Its `Debug` leaves the key out, so a launch or a request
/// logged whole never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(…)")
    }
}

impl ApiKey {
    /// `key` as it is (its form is checked where an owner types it, not here).
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its last four characters, to tell keys apart without showing one.
    #[must_use]
    pub fn hint(&self) -> String {
        let count = self.0.chars().count();
        self.0.chars().skip(count.saturating_sub(4)).collect()
    }
}

/// Whether a provider takes something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Takes {
    /// It needs it.
    Required,
    /// It may have it.
    Optional,
    /// It takes none.
    No,
}

/// A provider this server knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Claude Code on the owner's own Claude login.
    Claude,
    /// Anthropic's API, with an API key.
    Anthropic,
    /// `DeepSeek`'s API.
    DeepSeek,
    /// `OpenRouter`.
    OpenRouter,
    /// A local Ollama.
    Ollama,
    /// An endpoint that speaks Anthropic's Messages API.
    AnthropicCompatible,
    /// `OpenAI`'s API.
    OpenAi,
    /// An endpoint that speaks `OpenAI`'s API.
    OpenAiCompatible,
}

impl Provider {
    /// Every provider, in the order the contract lists them.
    pub const ALL: [Self; 8] = [
        Self::Claude,
        Self::Anthropic,
        Self::DeepSeek,
        Self::OpenRouter,
        Self::Ollama,
        Self::AnthropicCompatible,
        Self::OpenAi,
        Self::OpenAiCompatible,
    ];

    /// The provider `kind` names, if this server knows it.
    #[must_use]
    pub fn of(kind: &ProviderKind) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.kind() == *kind)
    }

    /// Its word on the wire.
    #[must_use]
    pub fn kind(self) -> ProviderKind {
        match self {
            Self::Claude => ProviderKind::Claude,
            Self::Anthropic => ProviderKind::Anthropic,
            Self::DeepSeek => ProviderKind::DeepSeek,
            Self::OpenRouter => ProviderKind::OpenRouter,
            Self::Ollama => ProviderKind::Ollama,
            Self::AnthropicCompatible => ProviderKind::AnthropicCompatible,
            Self::OpenAi => ProviderKind::OpenAi,
            Self::OpenAiCompatible => ProviderKind::OpenAiCompatible,
        }
    }

    /// Its name, for a sentence.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Anthropic => "Anthropic",
            Self::DeepSeek => "DeepSeek",
            Self::OpenRouter => "OpenRouter",
            Self::Ollama => "Ollama",
            Self::AnthropicCompatible => "The Anthropic-compatible endpoint",
            Self::OpenAi => "OpenAI",
            Self::OpenAiCompatible => "The OpenAI-compatible endpoint",
        }
    }

    /// Whether the agent CLI (Claude Code, which speaks Anthropic's API only) reaches it
    /// through the `OpenAI` [`bridge`].
    #[must_use]
    pub const fn bridged(self) -> bool {
        matches!(self, Self::OpenAi | Self::OpenAiCompatible)
    }

    /// Its usual address, when it has one (Anthropic's is the configured one).
    #[must_use]
    pub const fn default_url(self) -> Option<&'static str> {
        match self {
            Self::DeepSeek => Some("https://api.deepseek.com"),
            Self::OpenRouter => Some("https://openrouter.ai/api"),
            Self::Ollama => Some("http://localhost:11434"),
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Claude | Self::Anthropic | Self::AnthropicCompatible | Self::OpenAiCompatible => {
                None
            }
        }
    }

    /// The models it runs on when none is given: the agent's, and the one for quick side calls.
    #[must_use]
    pub const fn default_models(self) -> (Option<&'static str>, Option<&'static str>) {
        match self {
            Self::DeepSeek => (Some("deepseek-v4-pro"), Some("deepseek-v4-flash")),
            _ => (None, None),
        }
    }

    const fn takes_key(self) -> Takes {
        match self {
            Self::Anthropic
            | Self::DeepSeek
            | Self::OpenRouter
            | Self::AnthropicCompatible
            | Self::OpenAi => Takes::Required,
            Self::OpenAiCompatible => Takes::Optional,
            Self::Claude | Self::Ollama => Takes::No,
        }
    }

    const fn takes_base_url(self) -> Takes {
        match self {
            Self::AnthropicCompatible | Self::OpenAiCompatible => Takes::Required,
            Self::DeepSeek | Self::OpenRouter | Self::Ollama | Self::OpenAi => Takes::Optional,
            Self::Claude | Self::Anthropic => Takes::No,
        }
    }

    const fn takes_model(self) -> Takes {
        match self {
            Self::OpenRouter
            | Self::Ollama
            | Self::AnthropicCompatible
            | Self::OpenAi
            | Self::OpenAiCompatible => Takes::Required,
            _ => Takes::Optional,
        }
    }

    /// The secrets folder's file with its key.
    #[must_use]
    pub fn secret(self) -> String {
        format!("agent-{}-key", self.kind().as_str())
    }
}

/// The provider saved now, with its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    /// The provider.
    pub provider: Provider,
    /// Its key, when one is saved.
    pub key: Option<ApiKey>,
    /// Its address, when not its usual one.
    pub base_url: Option<String>,
    /// The agent's model, when one was given.
    pub model: Option<String>,
    /// The model for quick side calls, when one was given.
    pub fast_model: Option<String>,
    checked: Option<Checked>,
}

/// Where the provider is kept: its key in the secrets folder, the rest in the settings. What
/// every launch of the agent CLI reads.
#[derive(Clone, Debug)]
pub struct ProviderStore {
    secrets: Secrets,
    settings: SettingStore,
    bridge: Option<BridgeAccess>,
}

impl ProviderStore {
    /// The provider kept in `secrets` and `settings`.
    #[must_use]
    pub fn new(secrets: Secrets, settings: SettingStore) -> Self {
        Self {
            secrets,
            settings,
            bridge: None,
        }
    }

    /// Runs the agent CLI on `openai` and `openai-compatible` through the `OpenAI` bridge
    /// `access` reaches; without it, they are saved but not run on.
    #[must_use]
    pub fn with_bridge(mut self, access: BridgeAccess) -> Self {
        self.bridge = Some(access);
        self
    }

    /// The provider saved, if there is one. A key in Anthropic's file with nothing saved (put
    /// there by hand) is Anthropic.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting can't be read.
    pub async fn current(&self) -> Result<Option<Current>, DbError> {
        let saved = self.settings.get(&SAVED).await?;
        let provider = match &saved {
            Some(saved) => Provider::of(&ProviderKind::from(saved.provider.as_str())),
            None if self.key(Provider::Anthropic).await.is_some() => Some(Provider::Anthropic),
            None => None,
        };
        let Some(provider) = provider else {
            return Ok(None);
        };
        let saved = saved.unwrap_or(Saved {
            provider: provider.kind().as_str().to_owned(),
            base_url: None,
            model: None,
            fast_model: None,
            checked: None,
        });
        Ok(Some(Current {
            provider,
            key: self.key(provider).await,
            base_url: saved.base_url,
            model: saved.model,
            fast_model: saved.fast_model,
            checked: saved.checked,
        }))
    }

    /// The environment a launch of the agent CLI on `accounts` gets for the saved provider:
    /// none while accounts are configured (each is its own login), when nothing is saved, or
    /// when the saved provider can't run the agent.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting can't be read.
    pub async fn env(&self, accounts: &[AgentAccount]) -> Result<Option<ProviderEnv>, DbError> {
        if !accounts.is_empty() {
            return Ok(None);
        }
        Ok(self
            .current()
            .await?
            .as_ref()
            .and_then(|current| self.env_of(current)))
    }

    /// What `current` gives a launch, through this store's bridge when it needs it.
    fn env_of(&self, current: &Current) -> Option<ProviderEnv> {
        ProviderEnv::of(current, self.bridge.as_ref())
    }

    /// Whether the `OpenAI` bridge runs for the agent CLI.
    #[must_use]
    pub fn bridges(&self) -> bool {
        self.bridge.is_some()
    }

    /// `provider`'s saved key, if there is one.
    async fn key(&self, provider: Provider) -> Option<ApiKey> {
        let key = self.secrets.read(&provider.secret()).await;
        (!key.is_empty()).then(|| ApiKey::new(key))
    }

    /// Saves `key` as `provider`'s, for the server's user only (a file put there by hand with
    /// looser permissions is narrowed too).
    async fn write_key(&self, provider: Provider, key: &ApiKey) -> std::io::Result<()> {
        let name = provider.secret();
        self.secrets.write(&name, key.as_str()).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::Permissions::from_mode(0o600);
            tokio::fs::set_permissions(self.secrets.dir().join(&name), mode).await?;
        }
        Ok(())
    }

    /// Removes `provider`'s key, if one is saved.
    async fn remove_key(&self, provider: Provider) -> std::io::Result<()> {
        self.secrets.remove(&provider.secret()).await.map(drop)
    }
}

/// How the server's agent CLI signs in, as its configuration has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLogin {
    /// No agent CLI is configured (`EMBERHUT_AGENT_COMMAND`).
    Off,
    /// Its sessions run on this many configured accounts (`EMBERHUT_AGENT_ACCOUNTS`), each
    /// its own login.
    Accounts(usize),
    /// No account is configured: the provider saved from the apps, or the CLI's own login.
    Saved,
}

/// Why the provider wasn't read, checked or changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// What was sent isn't something the provider takes: the sentence says what.
    #[error("not taken: {0}")]
    Malformed(String),
    /// The provider refused the key, needs one, or can't run the agent; nothing was saved.
    /// The sentence says which.
    #[error("refused: {0}")]
    Refused(String),
    /// The provider didn't answer the check; nothing was saved.
    #[error("{provider} couldn't be reached: {why}")]
    Unreachable {
        /// The provider's name.
        provider: &'static str,
        /// Why, for the log.
        why: String,
    },
    /// The secrets folder couldn't be written.
    #[error("the key file couldn't be written")]
    Storage(#[source] std::io::Error),
    /// The setting couldn't be read or written.
    #[error(transparent)]
    Database(#[from] DbError),
}

/// What the provider is kept with.
#[derive(Clone, Debug)]
pub struct ProviderParts {
    /// Where it is kept.
    pub store: ProviderStore,
    /// When a check passed.
    pub clock: Arc<dyn Clock>,
    /// What checks a provider.
    pub api: Arc<dyn ProviderApi>,
    /// How the agent CLI signs in.
    pub login: AgentLogin,
}

/// The agent's model provider, as the owner chooses it.
#[derive(Clone, Debug)]
pub struct AgentProviderService {
    parts: Arc<ProviderParts>,
}

/// A request, checked for form: what a check and a save are made of.
struct Wanted {
    target: Target,
    model: Option<String>,
    fast_model: Option<String>,
}

impl AgentProviderService {
    /// The provider over `parts`.
    #[must_use]
    pub fn new(parts: ProviderParts) -> Self {
        Self {
            parts: Arc::new(parts),
        }
    }

    /// How the agent CLI signs in.
    #[must_use]
    pub fn login(&self) -> AgentLogin {
        self.parts.login
    }

    /// Whether the `OpenAI` bridge runs, for `openai` and `openai-compatible`.
    #[must_use]
    pub fn bridges(&self) -> bool {
        self.parts.store.bridges()
    }

    /// What is saved, without the key.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting can't be read.
    pub async fn status(&self) -> Result<AgentProvider, DbError> {
        let Some(current) = self.parts.store.current().await? else {
            return Ok(AgentProvider::NONE);
        };
        let checked_at = current
            .checked
            .clone()
            .filter(|checked| checked.key == current.key.as_ref().map(fingerprint))
            .map(|checked| checked.at);
        Ok(AgentProvider {
            provider: Some(current.provider.kind()),
            key_set: current.key.is_some(),
            key_hint: current.key.as_ref().map(ApiKey::hint),
            in_use: self.parts.login == AgentLogin::Saved
                && self.parts.store.env_of(&current).is_some(),
            usable: true,
            base_url: current.base_url,
            model: current.model,
            fast_model: current.fast_model,
            checked_at,
        })
    }

    /// Checks `request` with its provider, saving nothing.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`] for a request its provider doesn't take, and
    /// [`ProviderError::Refused`] and [`ProviderError::Unreachable`] when the check doesn't
    /// pass.
    pub async fn test(&self, request: &ProviderRequest) -> Result<ProviderTest, ProviderError> {
        let wanted = wanted(request)?;
        let models = self.check(&wanted.target).await?;
        Ok(ProviderTest {
            checked: wanted.target.provider != Provider::Claude,
            models,
        })
    }

    /// Checks `request` with its provider and, once it passes, saves it in place of whatever
    /// was saved before (another provider's key is removed). The agent's sessions run on it
    /// from their next turn.
    ///
    /// # Errors
    ///
    /// As [`Self::test`] (nothing is saved then), and the storage's own errors.
    pub async fn save(&self, request: &ProviderRequest) -> Result<AgentProvider, ProviderError> {
        let Wanted {
            target,
            model,
            fast_model,
        } = wanted(request)?;
        let provider = target.provider;
        // A test lists the models to choose from; what is saved needs one chosen.
        if provider.takes_model() == Takes::Required && model.is_none() {
            return Err(ProviderError::Malformed(format!(
                "{} needs a model: choose one of its models.",
                provider.name()
            )));
        }
        self.check(&target).await?;
        let checked = (provider != Provider::Claude).then(|| Checked {
            at: timestamp_to_api(self.parts.clock.now()),
            key: target.key.as_ref().map(fingerprint),
        });
        let saved = Saved {
            provider: provider.kind().as_str().to_owned(),
            base_url: target.base_url.clone(),
            model,
            fast_model,
            checked,
        };
        let store = &self.parts.store;
        // The setting first: one left behind by a failed write names a key that isn't saved,
        // and is reported unchecked.
        store.settings.set(&SAVED, &saved).await?;
        for other in Provider::ALL.into_iter().filter(|other| *other != provider) {
            store
                .remove_key(other)
                .await
                .map_err(ProviderError::Storage)?;
        }
        match &target.key {
            Some(key) => store.write_key(provider, key).await,
            None => store.remove_key(provider).await,
        }
        .map_err(ProviderError::Storage)?;
        tracing::info!(
            provider = provider.kind().as_str(),
            "the agent's model provider was saved"
        );
        Ok(self.status().await?)
    }

    /// Removes the saved provider and every key: the agent CLI runs on its own login again.
    ///
    /// # Errors
    ///
    /// The storage's own errors.
    pub async fn remove(&self) -> Result<AgentProvider, ProviderError> {
        let store = &self.parts.store;
        for provider in Provider::ALL {
            store
                .remove_key(provider)
                .await
                .map_err(ProviderError::Storage)?;
        }
        if store.settings.remove(&SAVED).await? {
            tracing::info!("the agent's model provider was removed");
        }
        Ok(AgentProvider::NONE)
    }

    /// `target`'s models, once its provider lets the server in.
    async fn check(&self, target: &Target) -> Result<Vec<ProviderModel>, ProviderError> {
        let provider = target.provider;
        if provider == Provider::Claude {
            // Only the CLI can check its own login.
            return Ok(Vec::new());
        }
        let name = provider.name();
        match self.parts.api.check(target).await {
            ProviderCheck::Accepted(models) => Ok(models),
            ProviderCheck::Refused => Err(ProviderError::Refused(match target.key {
                Some(_) => format!(
                    "{name} didn't accept that key. Check that it was copied whole and is still \
                     active, or make a new one."
                ),
                None => format!("{name} didn't let the server in: it needs an API key."),
            })),
            ProviderCheck::TooOld(version) => Err(ProviderError::Refused(format!(
                "{name} {version} can't run the agent: it needs {OLLAMA_SINCE} or later. \
                 Update it, then try again."
            ))),
            ProviderCheck::Unreachable(why) => Err(ProviderError::Unreachable {
                provider: name,
                why,
            }),
        }
    }
}

/// `request` checked for form: a provider this server knows, given what it takes.
fn wanted(request: &ProviderRequest) -> Result<Wanted, ProviderError> {
    let malformed = |why: String| Err(ProviderError::Malformed(why));
    let Some(provider) = Provider::of(&request.provider) else {
        return malformed(format!(
            "{} isn't a provider this server knows.",
            request.provider.as_str()
        ));
    };
    let name = provider.name();
    let given = |field: &Option<String>| {
        field
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let key = match (provider.takes_key(), given(&request.api_key)) {
        (Takes::No, Some(_)) => return malformed(format!("{name} takes no API key.")),
        (Takes::Required, None) => return malformed(format!("{name} needs an API key.")),
        (_, key) => key.map(|key| api_key(provider, key)).transpose()?,
    };
    let base_url = match (provider.takes_base_url(), given(&request.base_url)) {
        (Takes::No, Some(_)) => return malformed(format!("{name} takes no base URL.")),
        (Takes::Required, None) => return malformed(format!("{name} needs a base URL.")),
        (_, url) => url.as_deref().map(base_url).transpose()?,
    };
    let model = given(&request.model);
    let fast_model = given(&request.fast_model);
    let named = |model: &String| {
        model.chars().count() <= MODEL_LENGTH && !model.chars().any(char::is_control)
    };
    if !model.iter().chain(&fast_model).all(named) {
        return malformed("That isn't a model's name.".to_owned());
    }
    Ok(Wanted {
        target: Target {
            provider,
            base_url,
            key,
        },
        model,
        fast_model,
    })
}

/// `key` as `provider`'s key: one line of visible characters, of a key's length; an
/// Anthropic key starts with [`ANTHROPIC_KEY_PREFIX`].
fn api_key(provider: Provider, key: String) -> Result<ApiKey, ProviderError> {
    let formed = KEY_LENGTH.contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_graphic());
    let anthropic = |key: &str| {
        key.starts_with(ANTHROPIC_KEY_PREFIX)
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    };
    if provider == Provider::Anthropic && !(formed && anthropic(&key)) {
        return Err(ProviderError::Malformed(
            "That isn't an Anthropic API key. A key starts with sk-ant- and has no spaces: copy \
             it again from the Claude Console."
                .to_owned(),
        ));
    }
    if formed {
        Ok(ApiKey::new(key))
    } else {
        Err(ProviderError::Malformed(
            "That isn't an API key: copy it again, whole and on one line.".to_owned(),
        ))
    }
}

/// `text` as a base URL: an `http://` or `https://` address with a host, and no query or
/// sign-in in it, without a final `/`.
fn base_url(text: &str) -> Result<String, ProviderError> {
    match url::Url::parse(text) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.has_host()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Ok(text.trim_end_matches('/').to_owned())
        }
        _ => Err(ProviderError::Malformed(
            "That isn't a base URL: it is an http:// or https:// address, such as \
             https://api.example.com/v1."
                .to_owned(),
        )),
    }
}

/// Which key a check was for, without the key: the start of its SHA-256.
fn fingerprint(key: &ApiKey) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, key.as_str().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest.as_ref()[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(provider: &str, key: Option<&str>, url: Option<&str>) -> ProviderRequest {
        ProviderRequest {
            provider: provider.into(),
            api_key: key.map(str::to_owned),
            base_url: url.map(str::to_owned),
            model: Some("a-model".to_owned()),
            fast_model: None,
        }
    }

    fn refusal(request: &ProviderRequest) -> String {
        match wanted(request) {
            Err(ProviderError::Malformed(why)) => why,
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("taken"),
        }
    }

    #[test]
    fn a_key_shows_only_its_last_four_characters() {
        let key = ApiKey::new("sk-ant-api03-abcdefgh");
        assert_eq!(key.hint(), "efgh");
        assert_eq!(format!("{key:?}"), "ApiKey(…)");
        assert_eq!(ApiKey::new("abc").hint(), "abc");
    }

    #[test]
    fn every_provider_has_its_word_and_its_own_key_file() {
        assert_eq!(Provider::Anthropic.secret(), "agent-anthropic-key");
        assert_eq!(
            Provider::AnthropicCompatible.secret(),
            "agent-anthropic-compatible-key"
        );
        for provider in Provider::ALL {
            assert_eq!(Provider::of(&provider.kind()), Some(provider));
        }
        assert_eq!(Provider::of(&"gemini".into()), None);
        assert_eq!(ProviderKind::KNOWN.len(), Provider::ALL.len());
    }

    #[test]
    fn each_provider_takes_what_it_needs_and_nothing_else() {
        let key = "sk-ant-api03-AbCd_ef-0123456789abcdefXYZ";
        let taken = wanted(&request("anthropic", Some(&format!(" {key}\n")), None)).unwrap();
        assert_eq!(taken.target.key.unwrap().as_str(), key);
        for bad in ["sk-proj-0123456789", "sk-ant-api03 0123456789"] {
            assert!(
                refusal(&request("anthropic", Some(bad), None))
                    .starts_with("That isn't an Anthropic API key.")
            );
        }
        assert_eq!(
            refusal(&request("anthropic", None, None)),
            "Anthropic needs an API key."
        );
        assert_eq!(
            refusal(&request(
                "anthropic",
                Some(key),
                Some("https://proxy.example")
            )),
            "Anthropic takes no base URL."
        );
        assert_eq!(
            refusal(&request("claude", Some(key), None)),
            "Claude takes no API key."
        );
        assert_eq!(
            refusal(&request("anthropic-compatible", Some("token-1234"), None)),
            "The Anthropic-compatible endpoint needs a base URL."
        );
        let local = wanted(&request(
            "openai-compatible",
            None,
            Some("http://10.0.0.5:8000/v1/"),
        ))
        .unwrap();
        assert_eq!(
            local.target.base_url.as_deref(),
            Some("http://10.0.0.5:8000/v1")
        );
        assert!(local.target.key.is_none());
        for url in [
            "ftp://x.example",
            "not a url",
            "https://x.example/v1?key=1",
            "https://name:pw@[::1]/v1",
        ] {
            assert!(
                refusal(&request("ollama", None, Some(url))).starts_with("That isn't a base URL")
            );
        }
        assert_eq!(
            refusal(&request("gemini", None, None)),
            "gemini isn't a provider this server knows."
        );
        assert!(
            refusal(&request("deepseek", Some("sk-one two"), None))
                .starts_with("That isn't an API key")
        );
        let mut odd_model = request("deepseek", Some("sk-0123456789"), None);
        odd_model.fast_model = Some("line\nbreak".to_owned());
        assert_eq!(refusal(&odd_model), "That isn't a model's name.");
    }

    #[test]
    fn a_fingerprint_tells_keys_apart_without_holding_one() {
        let one = ApiKey::new("sk-ant-api03-one-0123456789abcdef");
        let two = ApiKey::new("sk-ant-api03-two-0123456789abcdef");
        assert_eq!(fingerprint(&one), fingerprint(&one.clone()));
        assert_ne!(fingerprint(&one), fingerprint(&two));
        assert!(!fingerprint(&one).contains("0123"));
    }
}
