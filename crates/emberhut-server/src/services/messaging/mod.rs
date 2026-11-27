//! The owner's messages from outside the app, and the assistant's own WhatsApp number: Slack
//! ([`slack`]), Gmail read only ([`gmail`]), voice notes as text ([`voice_notes`]), the Mac's
//! iMessage, SMS and WhatsApp through the Mac helper ([`messages`]), the assistant's own
//! WhatsApp number ([`whatsapp`]), and sending as the owner on Slack and through WhatsApp Web,
//! each only after the owner's OK on the exact text ([`guard`]).
//!
//! Every outside service is a trait with a real implementation, configured from the server's
//! configuration, and a stand-in for when it isn't set up ([`MessagingDeps`]); the tokens are
//! files in the secrets directory, read at every use. What messaging needs from the tasks and
//! the guard rails is behind [`hooks`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use emberhut_core::{Clock, RandomSource};
use emberhut_db::Database;
use tokio_util::sync::CancellationToken;

pub mod context;
pub mod gmail;
pub mod guard;
pub mod hooks;
pub mod mac;
pub mod messages;
pub mod secrets;
pub mod slack;
pub mod voice_notes;
pub mod web;
pub mod whatsapp;
pub mod whatsapp_web;

use self::context::{Context, ContextParts};
use self::gmail::{GmailParts, GmailService, GoogleApi, NoGoogleApi};
use self::hooks::{CardOnly, SendOks, Tasks};
use self::mac::{MacMessages, NoMacMessages};
use self::messages::Messages;
use self::secrets::Secrets;
use self::slack::{NoSlackApi, SlackApi, SlackParts, SlackService};
use self::voice_notes::{NoTranscriber, Transcriber, VoiceNotes, VoiceParts};
use self::whatsapp::{NoWaDaemon, WaDaemon, WhatsAppOptions, WhatsAppParts, WhatsAppService};
use self::whatsapp_web::{NoWhatsAppWeb, WhatsAppSend, WhatsAppWeb};
use crate::services::push::PushService;

/// How often new messages are collected.
const COLLECT_EVERY: Duration = Duration::from_secs(120);
/// How often voice notes due a try are started.
const VOICE_EVERY: Duration = Duration::from_secs(60);

/// The outside services messaging reaches, and how it is set up: what a test swaps.
#[derive(Clone, Debug)]
pub struct MessagingDeps {
    /// Slack's Web API.
    pub slack: Arc<dyn SlackApi>,
    /// Google's OAuth and Gmail APIs.
    pub google: Arc<dyn GoogleApi>,
    /// Speech to text.
    pub transcriber: Arc<dyn Transcriber>,
    /// The Mac helper's message routes.
    pub mac: Arc<dyn MacMessages>,
    /// The WhatsApp daemon of the assistant's own number.
    pub wa_daemon: Arc<dyn WaDaemon>,
    /// WhatsApp Web in the server's browser.
    pub wa_web: Arc<dyn WhatsAppWeb>,
    /// The tasks; none: the server's own ([`MessagingParts::tasks`]).
    pub tasks: Option<Arc<dyn Tasks>>,
    /// The other ways the owner OKs a message.
    pub oks: Arc<dyn SendOks>,
    /// Where Google sends the owner back after signing in; none: `<public url>/gmail/callback`.
    pub gmail_callback: Option<String>,
    /// The name a WhatsApp group message starts with to address the assistant.
    pub wake_name: String,
    /// The owner's own phone number, to name the owner in WhatsApp chats.
    pub owner_phone: Option<String>,
    /// The assistant's WhatsApp number, to link.
    pub assistant_phone: Option<String>,
    /// The longest voice note transcribed, in seconds.
    pub voice_max_seconds: f64,
    /// The pause before a voice note is tried again at once.
    pub voice_retry_pause: Duration,
}

impl Default for MessagingDeps {
    /// Nothing set up: every outside service stands in, the server's own tasks, only the
    /// card's OK.
    fn default() -> Self {
        Self {
            slack: Arc::new(NoSlackApi),
            google: Arc::new(NoGoogleApi),
            transcriber: Arc::new(NoTranscriber),
            mac: Arc::new(NoMacMessages),
            wa_daemon: Arc::new(NoWaDaemon),
            wa_web: Arc::new(NoWhatsAppWeb),
            tasks: None,
            oks: Arc::new(CardOnly),
            gmail_callback: None,
            wake_name: "emberhut".to_owned(),
            owner_phone: None,
            assistant_phone: None,
            voice_max_seconds: emberhut_core::messaging::voice::MAX_SECONDS,
            voice_retry_pause: Duration::from_secs(3),
        }
    }
}

/// What the messaging services are built from besides their dependencies.
#[derive(Clone, Debug)]
pub struct MessagingParts {
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where one-time links get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The phones.
    pub push: PushService,
    /// The secrets directory.
    pub secrets_dir: PathBuf,
    /// Where the owner's devices reach this server.
    pub public_url: String,
    /// The server's tasks ([`hooks::ServerTasks`]; [`hooks::NoTasks`] where there are none), unless
    /// [`MessagingDeps::tasks`] names others.
    pub tasks: Arc<dyn Tasks>,
    /// Sign-in is only reported: a message sent as the owner goes out on its own card's answer
    /// or the standing list only ([`ContextParts::card_oks_only`]).
    pub card_oks_only: bool,
}

/// Every messaging service, wired once. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Messaging {
    slack: SlackService,
    gmail: GmailService,
    voice: VoiceNotes,
    whatsapp: WhatsAppService,
    whatsapp_send: WhatsAppSend,
    messages: Messages,
}

impl Messaging {
    /// The services over `db`, wired from `deps` and `parts`.
    #[must_use]
    pub fn new(db: &Database, deps: MessagingDeps, parts: MessagingParts) -> Self {
        let secrets = Secrets::new(parts.secrets_dir);
        let ctx = Context::new(ContextParts {
            clock: Arc::clone(&parts.clock),
            random: parts.random,
            settings: db.settings(),
            log: db.log(),
            events: db.events(),
            chat: db.messages(),
            agents: db.agents(),
            push: parts.push,
            tasks: deps.tasks.unwrap_or(parts.tasks),
            oks: deps.oks,
            secrets: secrets.clone(),
            card_oks_only: parts.card_oks_only,
        });
        let voice = VoiceNotes::new(
            ctx.clone(),
            VoiceParts {
                store: db.voice_notes(),
                slack_store: db.slack(),
                slack: Arc::clone(&deps.slack),
                mac: Arc::clone(&deps.mac),
                transcriber: deps.transcriber,
                max_seconds: deps.voice_max_seconds,
                retry_pause: deps.voice_retry_pause,
            },
        );
        let files = crate::services::uploads::UploadService::folder_beside(db.path())
            .with_file_name("files")
            .join("slack");
        let slack = SlackService::new(
            ctx.clone(),
            SlackParts {
                store: db.slack(),
                api: deps.slack,
                mac: Arc::clone(&deps.mac),
                voice: voice.clone(),
                files,
            },
        );
        let public = parts.public_url.trim_end_matches('/').to_owned();
        let gmail = GmailService::new(
            ctx.clone(),
            GmailParts {
                store: db.mail(),
                api: deps.google,
                callback: deps
                    .gmail_callback
                    .unwrap_or_else(|| format!("{public}/gmail/callback")),
            },
        );
        let whatsapp = WhatsAppService::new(
            ctx.clone(),
            WhatsAppParts {
                store: db.whatsapp(),
                rooms: db.rooms(),
                daemon: deps.wa_daemon,
                options: WhatsAppOptions {
                    name: deps.wake_name,
                    owner_phone: deps.owner_phone,
                    phone: deps.assistant_phone,
                    public_url: public,
                },
            },
        );
        let whatsapp_send = WhatsAppSend::new(ctx, Arc::clone(&deps.mac), deps.wa_web);
        let messages = Messages::new(
            deps.mac,
            slack.clone(),
            gmail.clone(),
            voice.clone(),
            parts.clock,
        );
        Self {
            slack,
            gmail,
            voice,
            whatsapp,
            whatsapp_send,
            messages,
        }
    }

    /// The owner's Slack.
    #[must_use]
    pub fn slack(&self) -> &SlackService {
        &self.slack
    }

    /// The owner's Gmail accounts.
    #[must_use]
    pub fn gmail(&self) -> &GmailService {
        &self.gmail
    }

    /// Voice notes.
    #[must_use]
    pub fn voice(&self) -> &VoiceNotes {
        &self.voice
    }

    /// The assistant's own WhatsApp number.
    #[must_use]
    pub fn whatsapp(&self) -> &WhatsAppService {
        &self.whatsapp
    }

    /// Sending as the owner through WhatsApp Web.
    #[must_use]
    pub fn whatsapp_send(&self) -> &WhatsAppSend {
        &self.whatsapp_send
    }

    /// The owner's messages, read together.
    #[must_use]
    pub fn messages(&self) -> &Messages {
        &self.messages
    }

    /// One round of collecting: Slack's workspaces connected and the owner's own messages
    /// read, Slack's notifications and the Slack app's saved messages from the Mac, and new
    /// mail. Each source's failure is logged, never thrown.
    pub async fn collect(&self) {
        self.slack.connect_all().await;
        self.slack.collect_mine().await;
        if let Err(error) = self.slack.collect_notifications().await {
            tracing::debug!(error, "Slack's notifications weren't read from the Mac");
        }
        if let Err(error) = self.slack.collect_own().await {
            tracing::debug!(
                error,
                "the owner's Slack messages weren't read from the Mac"
            );
        }
        self.gmail.collect().await;
    }

    /// Collects every 2 minutes and starts the voice notes due a try every minute, until
    /// `stop`.
    pub async fn keep_collecting(self, stop: CancellationToken) {
        let mut collect = tokio::time::interval(COLLECT_EVERY);
        let mut voice = tokio::time::interval(VOICE_EVERY);
        collect.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        voice.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = collect.tick() => self.collect().await,
                _ = voice.tick() => {
                    if let Err(error) = self.voice.tick().await {
                        tracing::warn!(%error, "voice notes due weren't started");
                    }
                }
            }
        }
    }
}
