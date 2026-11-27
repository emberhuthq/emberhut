//! What the agent thinks with now, as far as the server can tell: read from the configuration,
//! the saved provider and what the agent CLI's turns showed of its login. The setup checklist
//! words it, and what runs unattended (routines, check-ins, the wake rule's turns, workers)
//! waits while the agent has nothing to think with.
//!
//! The CLI's own login is the one thing the server can't read: only a turn shows it. A turn the
//! CLI answers proves it signed in; one where it says it isn't logged in marks it signed out
//! until a provider is saved or removed, or a later turn answers.

use serde::{Deserialize, Serialize};

use emberhut_db::{DbError, SettingKey};

use super::{AgentLogin, Provider, ProviderStore};

/// What the CLI's turns last showed of its login.
const HEARD: SettingKey<Heard> = SettingKey::new("agentLogin");

/// What a turn of the agent CLI showed of its login.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Heard {
    /// It answered: it has something to think with.
    Answered,
    /// It said it isn't logged in.
    SignedOut,
}

/// What the agent thinks with now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThinksWith {
    /// No agent CLI is configured (`EMBERHUT_AGENT_COMMAND`).
    NoAgent,
    /// The CLI said it isn't logged in, and nothing changed since.
    SignedOut,
    /// The configured accounts (`EMBERHUT_AGENT_ACCOUNTS`), each its own login; a provider
    /// saved from the apps isn't used while they are.
    Accounts {
        /// How many.
        count: usize,
        /// The provider saved, if one is.
        saved: Option<Provider>,
    },
    /// Nothing is saved: the CLI runs on whatever login it has on the server.
    Unsaved {
        /// A turn showed that login works.
        answered: bool,
    },
    /// `claude` is saved: the CLI's own Claude login, which only a turn can check.
    Claude {
        /// A turn showed that login works.
        answered: bool,
    },
    /// The saved provider runs through the `OpenAI` bridge, which isn't running on this server.
    BridgeOff(Provider),
    /// The saved provider runs through the `OpenAI` bridge and needs a model, none was saved.
    ModelMissing(Provider),
    /// The saved provider's key file is gone.
    KeyMissing(Provider),
    /// A saved provider the CLI runs on, and its model when one was given.
    Saved {
        /// The provider.
        provider: Provider,
        /// Its model.
        model: Option<String>,
    },
}

impl ThinksWith {
    /// Whether the agent has something to think with, so a turn nobody asked for may start:
    /// accounts, a saved provider it runs on, `claude` (until a turn says it isn't logged in),
    /// or nothing saved once a turn showed the CLI's own login works.
    #[must_use]
    pub const fn thinks(&self) -> bool {
        matches!(
            self,
            Self::Accounts { .. }
                | Self::Unsaved { answered: true }
                | Self::Claude { .. }
                | Self::Saved { .. }
        )
    }
}

impl ProviderStore {
    /// What an agent CLI signing in as `login` thinks with now.
    ///
    /// # Errors
    ///
    /// [`DbError`] when a setting can't be read.
    pub async fn thinks_with(&self, login: AgentLogin) -> Result<ThinksWith, DbError> {
        let accounts = match login {
            AgentLogin::Off => return Ok(ThinksWith::NoAgent),
            AgentLogin::Accounts(count) => Some(count),
            AgentLogin::Saved => None,
        };
        let heard = self.heard().await?;
        if heard == Some(Heard::SignedOut) {
            return Ok(ThinksWith::SignedOut);
        }
        let answered = heard == Some(Heard::Answered);
        let current = self.current().await?;
        if let Some(count) = accounts {
            return Ok(ThinksWith::Accounts {
                count,
                saved: current.map(|current| current.provider),
            });
        }
        Ok(match current {
            None => ThinksWith::Unsaved { answered },
            Some(current) if current.provider == Provider::Claude => {
                ThinksWith::Claude { answered }
            }
            Some(current) if current.provider.bridged() && !self.bridges() => {
                ThinksWith::BridgeOff(current.provider)
            }
            Some(current) if current.provider.bridged() && current.model.is_none() => {
                ThinksWith::ModelMissing(current.provider)
            }
            Some(current) if self.env_of(&current).is_some() => ThinksWith::Saved {
                provider: current.provider,
                model: current.model,
            },
            Some(current) => ThinksWith::KeyMissing(current.provider),
        })
    }

    /// What the CLI's turns last showed of its login, if any showed anything since the provider
    /// last changed.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting can't be read.
    pub async fn heard(&self) -> Result<Option<Heard>, DbError> {
        self.settings.get(&HEARD).await
    }

    /// Keeps what a turn showed of the CLI's login; whether that changed what was kept (it is
    /// written only then).
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting can't be read or written.
    pub async fn hear(&self, heard: Heard) -> Result<bool, DbError> {
        if self.heard().await? == Some(heard) {
            return Ok(false);
        }
        self.settings.set(&HEARD, &heard).await?;
        Ok(true)
    }

    /// Forgets what the turns showed: the provider changed, so the next turn tells again.
    pub(super) async fn forget_heard(&self) -> Result<(), DbError> {
        self.settings.remove(&HEARD).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_the_cli_can_run_on_thinks() {
        let thinking = [
            ThinksWith::Accounts {
                count: 2,
                saved: None,
            },
            ThinksWith::Unsaved { answered: true },
            ThinksWith::Claude { answered: false },
            ThinksWith::Saved {
                provider: Provider::DeepSeek,
                model: None,
            },
            ThinksWith::Saved {
                provider: Provider::OpenAiCompatible,
                model: Some("m".to_owned()),
            },
        ];
        assert!(thinking.iter().all(ThinksWith::thinks));
        let waiting = [
            ThinksWith::NoAgent,
            ThinksWith::SignedOut,
            ThinksWith::Unsaved { answered: false },
            ThinksWith::BridgeOff(Provider::OpenAi),
            ThinksWith::ModelMissing(Provider::OpenAiCompatible),
            ThinksWith::KeyMissing(Provider::Anthropic),
        ];
        assert!(!waiting.iter().any(ThinksWith::thinks));
    }
}
