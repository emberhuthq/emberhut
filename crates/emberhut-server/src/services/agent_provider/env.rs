//! What a launch of the agent CLI gets for the saved provider: the variables Claude Code reads
//! to reach a model (its key or token, the endpoint, the models), set explicitly in the child's
//! environment and never on its command line.
//!
//! Whatever the server's own environment holds for those is kept from the child (every
//! `ANTHROPIC_*`, the switches to other clouds, an inherited token), so only the saved
//! provider's values reach it. The variables are the ones Claude Code 2.1 reads (its settings
//! reference, and the CLI's own list of them):
//!
//! | provider | `ANTHROPIC_BASE_URL` | credentials |
//! |---|---|---|
//! | `claude` | – | the CLI's own login |
//! | `anthropic` | – | `ANTHROPIC_API_KEY` |
//! | `deepseek` | `<base>/anthropic` | `ANTHROPIC_AUTH_TOKEN` |
//! | `openrouter` | `<base>` (no `/v1`) | `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY` empty |
//! | `ollama` | `<base>` | `ANTHROPIC_AUTH_TOKEN` = [`OLLAMA_TOKEN`] |
//! | `anthropic-compatible` | `<base>` | `ANTHROPIC_AUTH_TOKEN` |
//!
//! With a model (given, or the provider's default), every model the CLI resolves is mapped to
//! it ([`MODEL_VARS`]): the session's own (`ANTHROPIC_MODEL`), the `opus`, `sonnet` and `fable`
//! aliases a task or a sub-agent may name, and the quick model for side calls (`haiku`, and the
//! older `ANTHROPIC_SMALL_FAST_MODEL`). The launch's own `--model` is dropped then, so the
//! server's configured model (a Claude model's name) never reaches another provider.

use std::fmt;

use super::{ApiKey, Current, Provider};
use crate::services::claude::Launch;

/// The token Ollama is given: it takes any, but the CLI needs one to send.
pub const OLLAMA_TOKEN: &str = "ollama";

/// The prefix of every variable Claude Code reads for its endpoint, credentials and models.
const ANTHROPIC_PREFIX: &str = "ANTHROPIC_";

/// Variables outside that prefix that point the CLI elsewhere, or pick its sub-agents' model.
const ELSEWHERE: [&str; 4] = [
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

/// A Claude login's token from the environment: kept for `claude` (it is the owner's own
/// login), not for another provider.
const LOGIN_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// The variables a saved model is set as, and whether each takes the quick model.
pub const MODEL_VARS: [(&str, bool); 7] = [
    ("ANTHROPIC_MODEL", false),
    ("ANTHROPIC_DEFAULT_FABLE_MODEL", false),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", false),
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", false),
    ("CLAUDE_CODE_SUBAGENT_MODEL", false),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", true),
    ("ANTHROPIC_SMALL_FAST_MODEL", true),
];

/// The environment changes a launch makes for the saved provider. Its `Debug` names the
/// variables, never their values.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderEnv {
    set: Vec<(String, String)>,
    remove: Vec<String>,
    remove_prefixed: Vec<String>,
    models: bool,
    elsewhere: bool,
}

impl fmt::Debug for ProviderEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.set.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("ProviderEnv")
            .field("set", &names)
            .field("remove", &self.remove)
            .field("remove_prefixed", &self.remove_prefixed)
            .field("models", &self.models)
            .field("elsewhere", &self.elsewhere)
            .finish()
    }
}

impl ProviderEnv {
    /// What `current` gives a launch; none when its provider can't run the agent CLI, or a key
    /// it needs isn't saved (a file removed by hand).
    #[must_use]
    pub fn of(current: &Current) -> Option<Self> {
        let provider = current.provider;
        if !provider.usable() {
            return None;
        }
        let key = || current.key.as_ref().map(ApiKey::as_str);
        let base = current
            .base_url
            .as_deref()
            .or_else(|| provider.default_url());
        let mut set: Vec<(&str, String)> = Vec::new();
        match provider {
            Provider::Claude => {}
            Provider::Anthropic => set.push(("ANTHROPIC_API_KEY", key()?.to_owned())),
            Provider::DeepSeek => {
                set.push(("ANTHROPIC_BASE_URL", format!("{}/anthropic", base?)));
                set.push(("ANTHROPIC_AUTH_TOKEN", key()?.to_owned()));
            }
            Provider::OpenRouter => {
                set.push(("ANTHROPIC_BASE_URL", base?.to_owned()));
                set.push(("ANTHROPIC_AUTH_TOKEN", key()?.to_owned()));
                // So no Anthropic key of its own is tried alongside.
                set.push(("ANTHROPIC_API_KEY", String::new()));
            }
            Provider::Ollama => {
                set.push(("ANTHROPIC_BASE_URL", base?.to_owned()));
                set.push(("ANTHROPIC_AUTH_TOKEN", OLLAMA_TOKEN.to_owned()));
            }
            Provider::AnthropicCompatible => {
                set.push(("ANTHROPIC_BASE_URL", base?.to_owned()));
                set.push(("ANTHROPIC_AUTH_TOKEN", key()?.to_owned()));
            }
            Provider::OpenAi | Provider::OpenAiCompatible => return None,
        }
        let (main, quick) = provider.default_models();
        let model = current.model.as_deref().or(main);
        let fast = current.fast_model.as_deref().or(quick).or(model);
        if let (Some(model), Some(fast)) = (model, fast) {
            for (name, quick) in MODEL_VARS {
                set.push((name, if quick { fast } else { model }.to_owned()));
            }
        }
        let mut remove: Vec<String> = ELSEWHERE.map(str::to_owned).to_vec();
        if provider != Provider::Claude {
            remove.push(LOGIN_TOKEN.to_owned());
        }
        Some(Self {
            set: set
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
            remove,
            remove_prefixed: vec![ANTHROPIC_PREFIX.to_owned()],
            models: model.is_some(),
            elsewhere: base.is_some() && provider != Provider::Anthropic,
        })
    }

    /// Whether it points the CLI at another endpoint than Anthropic's: its errors are not a
    /// Claude account's, so no usage limit is read into them.
    #[must_use]
    pub fn elsewhere(&self) -> bool {
        self.elsewhere
    }

    /// Makes `launch` run on the provider: the server's own variables for it kept out, its own
    /// set, and the launch's `--model` dropped when the provider's models take its place.
    pub fn apply(&self, launch: &mut Launch) {
        if self.models {
            launch.args.retain(|arg| !arg.starts_with("--model="));
        }
        launch.env_remove.extend(self.remove.iter().cloned());
        launch
            .env_remove_prefixed
            .extend(self.remove_prefixed.iter().cloned());
        launch
            .env
            .retain(|(name, _)| !self.set.iter().any(|(set, _)| set == name));
        launch.env.extend(self.set.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn current(provider: Provider, key: Option<&str>, model: Option<&str>) -> Current {
        Current {
            provider,
            key: key.map(ApiKey::new),
            base_url: None,
            model: model.map(str::to_owned),
            fast_model: None,
            checked: None,
        }
    }

    fn launched(env: &ProviderEnv) -> Launch {
        let mut launch = Launch {
            program: PathBuf::from("claude"),
            args: vec!["--model=claude-opus-5-5".to_owned(), "--verbose".to_owned()],
            env: vec![("MCP_TOOL_TIMEOUT".to_owned(), "1".to_owned())],
            env_remove: vec!["CLAUDECODE".to_owned()],
            env_remove_prefixed: Vec::new(),
            cwd: None,
        };
        env.apply(&mut launch);
        launch
    }

    #[test]
    fn deepseek_runs_on_its_anthropic_endpoint_with_its_two_models() {
        let env = ProviderEnv::of(&current(Provider::DeepSeek, Some("sk-ds-1"), None)).unwrap();
        let launch = launched(&env);
        assert_eq!(
            launch.var("ANTHROPIC_BASE_URL"),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some("sk-ds-1"));
        assert_eq!(launch.var("ANTHROPIC_MODEL"), Some("deepseek-v4-pro"));
        assert_eq!(
            launch.var("ANTHROPIC_DEFAULT_OPUS_MODEL"),
            Some("deepseek-v4-pro")
        );
        assert_eq!(
            launch.var("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            launch.var("ANTHROPIC_SMALL_FAST_MODEL"),
            Some("deepseek-v4-flash")
        );
        assert_eq!(launch.flag("--model"), None, "the provider's models decide");
        assert!(launch.args.contains(&"--verbose".to_owned()));
        assert!(launch.strips("ANTHROPIC_CUSTOM_HEADERS"));
        assert!(launch.strips("CLAUDE_CODE_USE_BEDROCK"));
        assert!(launch.strips("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(
            launch.strips("CLAUDECODE"),
            "the launch's own removals stay"
        );
        assert!(env.elsewhere());
        assert!(!format!("{env:?}").contains("sk-ds-1"));
    }

    #[test]
    fn anthropic_and_claude_keep_the_configured_model_unless_one_is_saved() {
        let env = ProviderEnv::of(&current(Provider::Anthropic, Some("sk-ant-1"), None)).unwrap();
        let launch = launched(&env);
        assert_eq!(launch.var("ANTHROPIC_API_KEY"), Some("sk-ant-1"));
        assert_eq!(launch.var("ANTHROPIC_BASE_URL"), None);
        assert_eq!(launch.flag("--model"), Some("claude-opus-5-5"));
        assert!(!env.elsewhere());

        let own =
            ProviderEnv::of(&current(Provider::Claude, None, Some("claude-sonnet-5-5"))).unwrap();
        let launch = launched(&own);
        assert_eq!(launch.var("ANTHROPIC_MODEL"), Some("claude-sonnet-5-5"));
        assert_eq!(launch.flag("--model"), None);
        assert!(
            launch.strips("ANTHROPIC_API_KEY"),
            "an inherited key would win over the login"
        );
        assert!(
            !launch.strips("CLAUDE_CODE_OAUTH_TOKEN"),
            "the owner's own login"
        );
    }

    #[test]
    fn openrouter_ollama_and_any_endpoint_get_their_base_and_token() {
        let mut router = current(Provider::OpenRouter, Some("sk-or-1"), Some("z-ai/glm-5"));
        router.fast_model = Some("z-ai/glm-5-air".to_owned());
        let launch = launched(&ProviderEnv::of(&router).unwrap());
        assert_eq!(
            launch.var("ANTHROPIC_BASE_URL"),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some("sk-or-1"));
        assert_eq!(
            launch.var("ANTHROPIC_API_KEY"),
            Some(""),
            "explicitly empty"
        );
        assert_eq!(
            launch.var("ANTHROPIC_DEFAULT_SONNET_MODEL"),
            Some("z-ai/glm-5")
        );
        assert_eq!(
            launch.var("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            Some("z-ai/glm-5-air")
        );

        let mut ollama = current(Provider::Ollama, None, Some("qwen3-coder"));
        ollama.base_url = Some("http://10.0.0.5:11434".to_owned());
        let launch = launched(&ProviderEnv::of(&ollama).unwrap());
        assert_eq!(
            launch.var("ANTHROPIC_BASE_URL"),
            Some("http://10.0.0.5:11434")
        );
        assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some(OLLAMA_TOKEN));
        assert_eq!(
            launch.var("ANTHROPIC_SMALL_FAST_MODEL"),
            Some("qwen3-coder")
        );

        let mut any = current(Provider::AnthropicCompatible, Some("tok-1"), Some("m-1"));
        any.base_url = Some("https://gateway.example".to_owned());
        let launch = launched(&ProviderEnv::of(&any).unwrap());
        assert_eq!(
            launch.var("ANTHROPIC_BASE_URL"),
            Some("https://gateway.example")
        );
        assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some("tok-1"));
    }

    #[test]
    fn a_provider_that_cannot_run_the_agent_or_lost_its_key_changes_nothing() {
        assert_eq!(
            ProviderEnv::of(&current(Provider::OpenAi, Some("sk-1"), None)),
            None
        );
        assert_eq!(
            ProviderEnv::of(&current(Provider::OpenAiCompatible, None, Some("m"))),
            None
        );
        assert_eq!(
            ProviderEnv::of(&current(Provider::Anthropic, None, None)),
            None
        );
        assert_eq!(
            ProviderEnv::of(&current(Provider::DeepSeek, None, None)),
            None
        );
    }
}
