//! Checking a provider: one request that costs nothing and needs the key, which lists the
//! models where the provider can.
//!
//! | provider | asked |
//! |---|---|
//! | `anthropic` | `GET <EMBERHUT_ANTHROPIC_URL>/v1/models` with `x-api-key` |
//! | `anthropic-compatible` | `GET <base>/v1/models` with the key as bearer (a 404: no list) |
//! | `deepseek` | `GET <base>/models` with the key as bearer |
//! | `openrouter` | `GET <base>/v1/key` with the key as bearer (its model list is open to anyone), then `GET <base>/v1/models` |
//! | `ollama` | `GET <base>/api/version` (0.14 or later speaks Anthropic's API), then `GET <base>/api/tags` |
//! | `openai`, `openai-compatible` | `GET <base>/models` with the key as bearer |
//! | `claude` | nothing: only the CLI can check its own login |
//!
//! Redirects are never followed, so a key is sent only where it was meant to go.

use std::fmt;
use std::time::Duration;

use emberhut_api::agent_provider::ProviderModel;
use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{ApiKey, Provider};
use crate::error::chain;
use crate::services::messaging::web::{WebAnswer, WebClient, WebRequest};

/// The version of Anthropic's API a check speaks.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The first Ollama that speaks Anthropic's Messages API.
pub const OLLAMA_SINCE: &str = "0.14";

/// How long a check may take.
const CHECK_WAIT: Duration = Duration::from_secs(15);

/// The longest answer a check reads (`OpenRouter`'s list of every model is the largest).
const CHECK_LIMIT: usize = 16 << 20;

/// A provider to check: which, where, and with what key.
#[derive(Clone, Debug)]
pub struct Target {
    /// The provider.
    pub provider: Provider,
    /// Its address, when not its usual one.
    pub base_url: Option<String>,
    /// Its key, when it takes one.
    pub key: Option<ApiKey>,
}

/// What a provider said of a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderCheck {
    /// It let the server in, and lists these models.
    Accepted(Vec<ProviderModel>),
    /// It refused the key (unknown, revoked, or not allowed), or needs one.
    Refused,
    /// It is a version too old to run the agent: this one.
    TooOld(String),
    /// No answer that says either: why, for the log.
    Unreachable(String),
}

/// The model providers' APIs, as far as checking one goes.
pub trait ProviderApi: fmt::Debug + Send + Sync {
    /// Asks `target`'s provider whether it lets the server in, and which models it lists.
    fn check<'a>(&'a self, target: &'a Target) -> BoxFuture<'a, ProviderCheck>;
}

/// No provider reachable: every check is unanswered.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProviderApi;

impl ProviderApi for NoProviderApi {
    fn check<'a>(&'a self, _: &'a Target) -> BoxFuture<'a, ProviderCheck> {
        Box::pin(async {
            ProviderCheck::Unreachable("no provider can be reached from this server".to_owned())
        })
    }
}

/// The providers over HTTPS (or plain HTTP, for one on the owner's network), Anthropic's at
/// `EMBERHUT_ANTHROPIC_URL`.
#[derive(Clone, Debug)]
pub struct HttpProviderApi {
    web: WebClient,
    anthropic: String,
}

/// How a request carries the key.
#[derive(Clone, Copy)]
enum Auth {
    /// `x-api-key`, with `anthropic-version`.
    Anthropic,
    /// `Authorization: Bearer`, with `anthropic-version` (as the CLI sends a token).
    AnthropicBearer,
    /// `Authorization: Bearer`.
    Bearer,
}

impl HttpProviderApi {
    /// The providers' APIs through `web`, Anthropic's at `anthropic` (without `/v1`).
    #[must_use]
    pub fn new(web: WebClient, anthropic: &str) -> Self {
        Self {
            web,
            anthropic: anthropic.trim_end_matches('/').to_owned(),
        }
    }

    /// `GET`s `url`, with `key` as `auth` says when there is one.
    async fn get(
        &self,
        url: String,
        key: Option<&ApiKey>,
        auth: Auth,
    ) -> Result<WebAnswer, String> {
        let mut request = WebRequest::get(url, CHECK_WAIT).limit(CHECK_LIMIT);
        if matches!(auth, Auth::Anthropic | Auth::AnthropicBearer) {
            request = request.header("anthropic-version", ANTHROPIC_VERSION);
        }
        if let Some(key) = key {
            request = match auth {
                Auth::Anthropic => request.header("x-api-key", key.as_str()),
                Auth::AnthropicBearer | Auth::Bearer => request.bearer(key.as_str()),
            };
        }
        self.web.send(request).await.map_err(|error| chain(&error))
    }

    /// Ollama at `base`: its version first, then its models.
    async fn ollama(&self, base: &str) -> ProviderCheck {
        let asked = self
            .get(format!("{base}/api/version"), None, Auth::Bearer)
            .await;
        let version = match judged(Provider::Ollama, asked) {
            Ok(body) => body["version"].as_str().unwrap_or_default().to_owned(),
            Err(check) => return check,
        };
        if !at_least(&version, OLLAMA_SINCE) {
            return ProviderCheck::TooOld(version);
        }
        let tags = self
            .get(format!("{base}/api/tags"), None, Auth::Bearer)
            .await;
        listed(Provider::Ollama, tags)
    }
}

impl ProviderApi for HttpProviderApi {
    fn check<'a>(&'a self, target: &'a Target) -> BoxFuture<'a, ProviderCheck> {
        Box::pin(async move {
            let provider = target.provider;
            let key = target.key.as_ref();
            let base = target
                .base_url
                .as_deref()
                .or_else(|| provider.default_url())
                .unwrap_or_default();
            match provider {
                // Only the CLI can check its own login: nothing to ask.
                Provider::Claude => ProviderCheck::Accepted(Vec::new()),
                Provider::Anthropic => {
                    let url = format!("{}/v1/models?limit=1000", self.anthropic);
                    listed(provider, self.get(url, key, Auth::Anthropic).await)
                }
                Provider::AnthropicCompatible => {
                    let url = format!("{base}/v1/models?limit=1000");
                    match self.get(url, key, Auth::AnthropicBearer).await {
                        // It let the server in and has no list to give.
                        Ok(answer) if answer.status.as_u16() == 404 => {
                            ProviderCheck::Accepted(Vec::new())
                        }
                        other => listed(provider, other),
                    }
                }
                Provider::OpenRouter => {
                    let asked = self.get(format!("{base}/v1/key"), key, Auth::Bearer).await;
                    match judged(provider, asked) {
                        Ok(_) => {
                            let models = self.get(format!("{base}/v1/models"), key, Auth::Bearer);
                            listed(provider, models.await)
                        }
                        Err(check) => check,
                    }
                }
                Provider::Ollama => self.ollama(base).await,
                Provider::DeepSeek | Provider::OpenAi | Provider::OpenAiCompatible => listed(
                    provider,
                    self.get(format!("{base}/models"), key, Auth::Bearer).await,
                ),
            }
        })
    }
}

/// What `answer` says: its body when it let the server in, the check's end otherwise.
fn judged(provider: Provider, answer: Result<WebAnswer, String>) -> Result<Value, ProviderCheck> {
    match answer {
        Ok(answer) if answer.ok() => Ok(answer.json()),
        Ok(answer) if matches!(answer.status.as_u16(), 401 | 403) => Err(ProviderCheck::Refused),
        Ok(answer) => Err(ProviderCheck::Unreachable(format!(
            "{} answered {}",
            provider.name(),
            answer.status
        ))),
        Err(why) => Err(ProviderCheck::Unreachable(why)),
    }
}

/// The models `answer` lists, as `provider` lists them.
fn listed(provider: Provider, answer: Result<WebAnswer, String>) -> ProviderCheck {
    match judged(provider, answer) {
        Ok(body) => models(provider, &body).map_or_else(
            || {
                ProviderCheck::Unreachable(format!(
                    "{} answered, but not with a list of models",
                    provider.name()
                ))
            },
            ProviderCheck::Accepted,
        ),
        Err(check) => check,
    }
}

/// The models in `body`, as `provider` lists them; none when it isn't such a list.
fn models(provider: Provider, body: &Value) -> Option<Vec<ProviderModel>> {
    let text = |model: &Value, field: &str| model[field].as_str().map(str::to_owned);
    let (list, id, name) = match provider {
        Provider::Ollama => ("models", "name", None),
        Provider::Anthropic | Provider::AnthropicCompatible => ("data", "id", Some("display_name")),
        Provider::OpenRouter => ("data", "id", Some("name")),
        Provider::Claude | Provider::DeepSeek | Provider::OpenAi | Provider::OpenAiCompatible => {
            ("data", "id", None)
        }
    };
    let models = body[list].as_array()?;
    Some(
        models
            .iter()
            .filter_map(|model| {
                Some(ProviderModel {
                    id: text(model, id)?,
                    name: name.and_then(|field| text(model, field)),
                })
            })
            .collect(),
    )
}

/// Whether `version` (`0.14.2`, `0.15.0-rc1`) is `since` (`0.14`) or later; an unreadable one
/// isn't.
fn at_least(version: &str, since: &str) -> bool {
    let parts = |text: &str| -> Option<Vec<u64>> {
        text.split(['-', '+'])
            .next()?
            .split('.')
            .map(|part| part.parse().ok())
            .collect()
    };
    match (parts(version), parts(since)) {
        (Some(version), Some(since)) => version >= since,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn models_are_read_as_each_provider_lists_them() {
        let anthropic =
            json!({ "data": [{ "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5" }] });
        assert_eq!(
            models(Provider::Anthropic, &anthropic).unwrap(),
            [ProviderModel {
                id: "claude-opus-5-5".to_owned(),
                name: Some("Claude Opus 5.5".to_owned())
            }]
        );
        let ollama =
            json!({ "models": [{ "name": "qwen3-coder:latest", "size": 1 }, { "size": 2 }] });
        assert_eq!(
            models(Provider::Ollama, &ollama).unwrap(),
            [ProviderModel {
                id: "qwen3-coder:latest".to_owned(),
                name: None
            }]
        );
        assert_eq!(
            models(Provider::DeepSeek, &json!({ "data": [] })).unwrap(),
            []
        );
        assert_eq!(models(Provider::OpenAi, &json!("<html>")), None);
    }

    #[test]
    fn ollama_speaks_anthropic_s_api_from_0_14() {
        for (version, ok) in [
            ("0.14.0", true),
            ("0.14", true),
            ("0.15.1-rc2", true),
            ("1.0.0", true),
            ("0.13.9", false),
            ("0.9", false),
            ("", false),
            ("dev", false),
        ] {
            assert_eq!(at_least(version, OLLAMA_SINCE), ok, "{version}");
        }
    }
}
