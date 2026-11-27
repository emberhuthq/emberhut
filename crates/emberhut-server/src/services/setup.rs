//! The setup checklist (`GET /api/setup`): what the server has set up of what the assistant
//! works with, item by item, for the apps' onboarding.
//!
//! Each item is read from the service that owns it, as it is now: nothing is started, no CLI
//! is run, and nothing is written. The Mac helper is asked for its health (within its own 5 s);
//! everything else is the server's configuration, its secrets folder and its tables. What the
//! server can't know isn't an item: whether the machine is linked to an account is the
//! account directory's to say (`GET /v1/machines`), and whether the agent CLI's own accounts
//! are logged in only the CLI can tell, so only the runners' reports of theirs are read. The
//! saved model provider is read as saved: it was checked when it was saved, and is not checked
//! again here.

use emberhut_api::setup::{SetupItem, SetupItemId as Id, SetupPage, SetupState as State};
use emberhut_core::messaging::mail::{CLIENT_ID_SECRET, CLIENT_SECRET_SECRET};
use emberhut_db::DbError;

use crate::services::accounts::AccountService;
use crate::services::agent_provider::{AgentLogin, AgentProviderService, Provider};
use crate::services::auth::{AuthError, AuthService};
use crate::services::chrome::ChromeService;
use crate::services::mac::{MacError, MacService};
use crate::services::messaging::Messaging;
use crate::services::push::PushService;
use crate::services::runners::RunnerService;

/// Why the checklist couldn't be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SetupError {
    /// The signed-in devices couldn't be read.
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// A table couldn't be read.
    #[error(transparent)]
    Database(#[from] DbError),
}

/// What the checklist is read from.
#[derive(Clone, Debug)]
pub struct SetupParts {
    /// An agent CLI is configured (`EMBERHUT_AGENT_COMMAND`).
    pub agent: bool,
    /// What the agent thinks with.
    pub provider: AgentProviderService,
    /// The public address the server's links point at, when one is configured.
    pub public_url: Option<String>,
    /// The sessions: the devices signed in.
    pub auth: AuthService,
    /// The runners' accounts.
    pub accounts: AccountService,
    /// The runners configured.
    pub runners: RunnerService,
    /// The push services.
    pub push: PushService,
    /// The Mac helper.
    pub mac: MacService,
    /// The paired Chrome profiles.
    pub chrome: ChromeService,
    /// Gmail, Slack and the assistant's WhatsApp number.
    pub messaging: Messaging,
}

/// The checklist.
#[derive(Clone, Debug)]
pub struct SetupService {
    parts: SetupParts,
}

impl SetupService {
    /// The checklist over `parts`.
    #[must_use]
    pub const fn new(parts: SetupParts) -> Self {
        Self { parts }
    }

    /// Every item, in the order the apps show it: the assistant first, then the devices and
    /// how they are reached, then what the assistant reads and sends through.
    ///
    /// # Errors
    ///
    /// [`SetupError`] when the sessions or a table can't be read.
    pub async fn page(&self) -> Result<SetupPage, SetupError> {
        // The Mac helper is the only item asked over the network: it is asked while the rest
        // is read.
        let (mac, accounts) = tokio::join!(self.mac(), self.accounts());
        let items = vec![
            self.agent(),
            self.provider().await?,
            accounts?,
            self.devices().await?,
            self.push().await,
            self.public_url(),
            mac,
            self.chrome().await?,
            self.gmail().await,
            self.slack().await,
            self.whatsapp().await,
        ];
        Ok(SetupPage { items })
    }

    fn agent(&self) -> SetupItem {
        if self.parts.agent {
            SetupItem::new(
                Id::Agent,
                State::Ready,
                "The assistant and its workers run on the agent CLI.",
            )
        } else {
            SetupItem::new(
                Id::Agent,
                State::Off,
                "EMBERHUT_AGENT_COMMAND isn't set: the assistant answers nothing.",
            )
        }
    }

    async fn provider(&self) -> Result<SetupItem, SetupError> {
        let service = &self.parts.provider;
        // No part of a key is shown here: the checklist is read in report mode too.
        let saved = service.status().await?;
        let provider = saved.provider.as_ref().and_then(Provider::of);
        let item = |state, detail: String| SetupItem::new(Id::Provider, state, detail);
        Ok(match (service.login(), provider) {
            (AgentLogin::Off, _) => item(
                State::Off,
                "EMBERHUT_AGENT_COMMAND isn't set: there is no agent to connect to a model."
                    .to_owned(),
            ),
            (AgentLogin::Accounts(accounts), provider) => {
                let mut detail = format!(
                    "The agent runs on {} (EMBERHUT_AGENT_ACCOUNTS).",
                    count(accounts, "configured account", "configured accounts")
                );
                if let Some(provider) = provider {
                    detail.push_str(" The provider saved from the apps (");
                    detail.push_str(provider.kind().as_str());
                    detail.push_str(") isn't used while they are.");
                }
                item(State::Ready, detail)
            }
            (AgentLogin::Saved, None) => item(
                State::Missing,
                "No model provider is saved: choose one, or sign the agent CLI in to Claude on \
                 the server."
                    .to_owned(),
            ),
            (AgentLogin::Saved, Some(provider)) if provider.bridged() && !service.bridges() => {
                item(
                    State::Off,
                    format!(
                        "{} is saved, but the OpenAI bridge it runs through isn't running on \
                         this server.",
                        provider.name()
                    ),
                )
            }
            (AgentLogin::Saved, Some(Provider::Claude)) => item(
                State::Missing,
                "The agent runs on the agent CLI's own Claude login, which the server can't \
                 check: sign it in on the server, or choose another provider."
                    .to_owned(),
            ),
            (AgentLogin::Saved, Some(provider)) if saved.in_use => {
                let model = saved
                    .model
                    .map_or_else(String::new, |model| format!(", on {model}"));
                let through = if provider.bridged() {
                    ", through the server's OpenAI bridge"
                } else {
                    ""
                };
                item(
                    State::Ready,
                    format!("The agent runs on {}{model}{through}.", saved_as(provider)),
                )
            }
            (AgentLogin::Saved, Some(provider)) if provider.bridged() && saved.model.is_none() => {
                item(
                    State::Missing,
                    format!(
                        "{} needs a model: choose one of its models.",
                        provider.name()
                    ),
                )
            }
            (AgentLogin::Saved, Some(provider)) => item(
                State::Missing,
                format!("{}'s key isn't saved: save it again.", provider.name()),
            ),
        })
    }

    async fn accounts(&self) -> Result<SetupItem, SetupError> {
        if self.parts.runners.machines().is_empty() {
            return Ok(SetupItem::new(
                Id::Accounts,
                State::Off,
                "No runner is configured (EMBERHUT_RUNNER).",
            ));
        }
        let accounts = self.parts.accounts.all().await?;
        let logged_in = accounts
            .iter()
            .filter(|account| account.logged_in == Some(true) && account.enabled != Some(false))
            .count();
        Ok(match (accounts.len(), logged_in) {
            (0, _) => SetupItem::new(
                Id::Accounts,
                State::Missing,
                "No runner has reported an account yet.",
            ),
            (1, 0) => SetupItem::new(
                Id::Accounts,
                State::Missing,
                "The account isn't logged in: log it in on its runner's machine.",
            ),
            (all, 0) => SetupItem::new(
                Id::Accounts,
                State::Missing,
                format!(
                    "None of the {all} accounts is logged in: log one in on its runner's machine."
                ),
            ),
            (all, logged_in) => SetupItem::new(
                Id::Accounts,
                State::Ready,
                format!(
                    "{logged_in} of {} logged in.",
                    count(all, "account", "accounts")
                ),
            ),
        })
    }

    async fn devices(&self) -> Result<SetupItem, SetupError> {
        let devices = self.parts.auth.devices().await?.len();
        Ok(if devices == 0 {
            SetupItem::new(
                Id::Devices,
                State::Missing,
                "No device is signed in: pair one.",
            )
        } else {
            SetupItem::new(
                Id::Devices,
                State::Ready,
                format!("{} signed in.", count(devices, "device", "devices")),
            )
        })
    }

    async fn push(&self) -> SetupItem {
        if self.parts.push.ready().await {
            SetupItem::new(Id::Push, State::Ready, "Phones get the assistant's alerts.")
        } else {
            SetupItem::new(
                Id::Push,
                State::Off,
                "No push service is set up (APNs or FCM): phones see replies only in the app.",
            )
        }
    }

    fn public_url(&self) -> SetupItem {
        match &self.parts.public_url {
            Some(url) => SetupItem::new(Id::PublicUrl, State::Ready, url.clone()),
            None => SetupItem::new(
                Id::PublicUrl,
                State::Off,
                "EMBERHUT_PUBLIC_URL isn't set: links point at the listen address.",
            ),
        }
    }

    async fn mac(&self) -> SetupItem {
        match self.parts.mac.helper().health().await {
            Ok(health) => SetupItem::new(
                Id::Mac,
                State::Ready,
                if health.locked {
                    "Online; the screen is locked."
                } else {
                    "Online."
                },
            ),
            Err(MacError::NotSetUp) => SetupItem::new(
                Id::Mac,
                State::Off,
                "No Mac helper is set up (EMBERHUT_MAC_HELPER_URL).",
            ),
            Err(error) => SetupItem::new(Id::Mac, State::Error, error.to_string()),
        }
    }

    async fn chrome(&self) -> Result<SetupItem, SetupError> {
        let profiles = self.parts.chrome.summary().await?;
        let online = profiles.iter().filter(|profile| profile.connected).count();
        Ok(if profiles.is_empty() {
            SetupItem::new(Id::Chrome, State::Missing, "No Chrome profile is paired.")
        } else {
            SetupItem::new(
                Id::Chrome,
                State::Ready,
                format!(
                    "{} paired, {online} online.",
                    count(profiles.len(), "profile", "profiles")
                ),
            )
        })
    }

    async fn gmail(&self) -> SetupItem {
        let gmail = self.parts.messaging.gmail();
        if !gmail.client_saved().await {
            return SetupItem::new(
                Id::Gmail,
                State::Missing,
                format!(
                    "The server's Google OAuth client isn't saved (secrets {CLIENT_ID_SECRET} \
                     and {CLIENT_SECRET_SECRET})."
                ),
            );
        }
        let accounts = gmail.accounts().await;
        if accounts.is_empty() {
            return SetupItem::new(Id::Gmail, State::Missing, "No account is signed in.");
        }
        for account in &accounts {
            if let Some(error) = gmail.state(account).await.error {
                return SetupItem::new(Id::Gmail, State::Error, format!("{account}: {error}"));
            }
        }
        SetupItem::new(
            Id::Gmail,
            State::Ready,
            format!(
                "{} signed in: {}.",
                count(accounts.len(), "account", "accounts"),
                accounts.join(", ")
            ),
        )
    }

    async fn slack(&self) -> SetupItem {
        let workspaces = self.parts.messaging.slack().saved_workspaces().await;
        if workspaces.is_empty() {
            SetupItem::new(Id::Slack, State::Missing, "No Slack token is saved.")
        } else {
            SetupItem::new(
                Id::Slack,
                State::Ready,
                format!(
                    "A token for {}: {}.",
                    count(workspaces.len(), "workspace", "workspaces"),
                    workspaces.join(", ")
                ),
            )
        }
    }

    async fn whatsapp(&self) -> SetupItem {
        let connection = self.parts.messaging.whatsapp().connection().await;
        if !connection.set_up {
            return SetupItem::new(
                Id::WhatsApp,
                State::Off,
                "No WhatsApp daemon is set up (EMBERHUT_WHATSAPP_URL).",
            );
        }
        if connection.state == "open" {
            let detail = connection.number.map_or_else(
                || "Linked.".to_owned(),
                |number| format!("Linked as {number}."),
            );
            return SetupItem::new(Id::WhatsApp, State::Ready, detail);
        }
        // What went wrong last counts only while the number isn't connected.
        if let Some(error) = connection.error.filter(|error| !error.is_empty()) {
            return SetupItem::new(Id::WhatsApp, State::Error, error);
        }
        SetupItem::new(
            Id::WhatsApp,
            State::Missing,
            format!(
                "The assistant's number isn't linked ({}).",
                connection.state
            ),
        )
    }
}

/// How the checklist names a saved provider: `the saved Anthropic API key`, `DeepSeek`.
fn saved_as(provider: Provider) -> String {
    match provider {
        Provider::Anthropic => "the saved Anthropic API key".to_owned(),
        Provider::Claude => "the agent CLI's own Claude login".to_owned(),
        Provider::AnthropicCompatible => "the Anthropic-compatible endpoint".to_owned(),
        Provider::OpenAiCompatible => "the OpenAI-compatible endpoint".to_owned(),
        other => other.name().to_owned(),
    }
}

/// `n` and the word for it: `1 device`, `2 devices`.
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
