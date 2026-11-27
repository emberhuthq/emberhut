//! The emberhut server: the HTTP/SSE API the Emberhut apps talk to.
//!
//! The binary is a thin `main` over this library: [`serve`] runs the server,
//! [`make_pairing_code`] backs `emberhut-server pair`, [`machine_id`] `emberhut-server id`, and
//! [`link`] `emberhut-server link` and `unlink`. Tests build the same app in-process with
//! [`app::AppState`] and [`app::router`], over a real database.
//!
//! The design is in `docs/adr/0002-rust-server.md`.

use std::future::{Future, IntoFuture as _};
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use emberhut_core::{OsRandom, SystemClock};
use emberhut_db::{Database, DbError};
pub use http::HttpSetupError;
use tokio::net::TcpListener;

pub mod app;
pub mod config;
mod convert;
pub mod error;
mod extract;
mod http;
pub mod identity;
pub mod link;
pub mod mcp;
pub mod oauth;
pub mod peer;
mod routes;
pub mod services;
mod settings;
pub mod shutdown;
pub mod telemetry;

pub use routes::{DocumentError, openapi, openapi_json};

use app::{AppState, Dependencies, router};
use config::{AuthMode, IdConfig, MacHelperConfig, PairConfig, ServeConfig, VoiceSettings};
use identity::{KeyFileError, Machine};
use services::calendar::{CalendarProvider, HttpMacCalendar, NoCalendar};
use services::calls::{CallTranscriber, CallsConfig, CommandTranscriber, NoTranscriber};
use services::chat_model::{ChatModel, NoChatModel, OpenAiChat};
use services::coding_app::McpCodingApp;
use services::dark::DarkModels;
use services::family::{HttpFamilyDirectory, NoFamily};
use services::live_call::{LiveTranscriber, NoLiveTranscriber, ResidentTranscriber};
use services::mac::{HttpMacHelper, MacHelper, NoMacHelper};
use services::pairing::{CodeMaker, IssuedCode, PairingError, PairingService};
use services::personal::PersonalDependencies;
use services::push::PushSenders;
use services::push::apns::{self, ApnsConfig, HttpApns};
use services::push::fcm::{self, FcmEndpoints, HttpFcm};
use services::recall::{HttpEmbedder, NoEmbedder, RECALL_EVERY};
use services::runner_api::{HttpRunners, RunnerAt};
use services::runners::TICK_EVERY;
use services::threads::POLL_EVERY;
use services::voice::{self, OpenAiConfig, OpenAiVoice, VoiceConfig};

/// How often the tasks' tick runs: paused projects come back, stale asks are checked again.
const TASKS_TICK: Duration = Duration::from_mins(5);

/// How often approvals that are due expire.
const APPROVALS_TICK: Duration = Duration::from_mins(1);

/// How long the workers get to end as the server stops: the CLI's grace after `SIGTERM`, then
/// its process group's `SIGKILL` and the wrap-up.
const WORKERS_GRACE: Duration = services::claude::STOP_GRACE.saturating_add(Duration::from_secs(3));

/// Why the server could not run, or a command could not finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunError {
    /// The database failed: it could not be opened or migrated, or a query failed.
    #[error("the database failed")]
    Database(#[from] DbError),
    /// The address could not be listened on.
    #[error("cannot listen on {address}")]
    Listen {
        /// The address.
        address: SocketAddr,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
    /// Serving failed.
    #[error("serving failed")]
    Serve(#[source] std::io::Error),
    /// The `OpenAPI` document could not be written: the server would serve a broken one.
    #[error("cannot write the OpenAPI document")]
    Document(#[from] DocumentError),
    /// A pairing code could not be made.
    #[error("cannot make a pairing code")]
    Pairing(#[from] PairingError),
    /// The machine's key couldn't be read.
    #[error(transparent)]
    MachineKey(#[from] KeyFileError),
    /// The server won't serve with the key it found (or none), or couldn't make one.
    #[error(transparent)]
    ServingKey(#[from] identity::ServingKeyError),
    /// The configuration is one the server refuses to serve.
    #[error("cannot serve this configuration")]
    Config(#[from] config::InvalidConfig),
    /// The coding-thread app's client could not be made.
    #[error("cannot set up the coding-thread app's client")]
    CodingApp(#[from] services::coding_app::CodingAppSetupError),
    /// The Mac helper's client could not be made.
    #[error("cannot set up the Mac helper")]
    MacHelper(#[from] services::mac::MacSetupError),
    /// The HTTP client the services share could not be made.
    #[error("cannot set up the HTTP client")]
    Http(#[from] HttpSetupError),
    /// The programs' key file could not be read.
    #[error("cannot read the programs' key from {}", .path.display())]
    ProgramKey {
        /// The file.
        path: std::path::PathBuf,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
    /// The agent's prompt file could not be read.
    #[error("cannot read the agent's prompt file {}", .path.display())]
    AgentPrompt {
        /// The file.
        path: std::path::PathBuf,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
    /// The Mac helper's token file could not be read.
    #[error("cannot read the Mac helper's token from {}", .path.display())]
    MacHelperToken {
        /// The file.
        path: std::path::PathBuf,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
}

/// Serves the API as `config` says until `stop` resolves, then lets open requests and streams
/// finish for up to `config.shutdown_grace`.
///
/// # Errors
///
/// [`RunError`] when the configuration is refused (report mode with a public URL), the
/// database cannot be opened, the address cannot be listened on, or serving fails.
#[expect(
    clippy::too_many_lines,
    reason = "every background loop the server runs, started and stopped in one place"
)]
pub async fn serve(config: &ServeConfig, stop: impl Future<Output = ()>) -> Result<(), RunError> {
    config.check()?;
    openapi_json()?;
    let db = Database::connect(&config.database.url).await?;
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(|source| RunError::Listen {
            address: config.listen,
            source,
        })?;
    let state = AppState::new(&db, dependencies(config, &db).await?, config.auth_mode);
    let paused = state.parties().pause_left_running().await?;
    if paused > 0 {
        tracing::info!(paused, "parties left running were paused");
    }
    tracing::info!(push = state.push().ready().await, "push services");
    // The coding threads are polled by the wake rule, so a followed thread's end wakes Main.
    let collecting = tokio::spawn(
        state
            .messaging()
            .clone()
            .keep_collecting(state.stopping().clone()),
    );
    let polling = tokio::spawn(
        state
            .wake()
            .clone()
            .keep_following(POLL_EVERY, state.stopping().clone()),
    );
    let waking = tokio::spawn(
        state
            .wake()
            .clone()
            .keep_ticking(services::wake::TICK_EVERY, state.stopping().clone()),
    );
    let prepping = tokio::spawn(
        state
            .meeting_prep()
            .clone()
            .keep_ticking(services::meeting_prep::TICK, state.stopping().clone()),
    );
    let ticking = tokio::spawn(
        state
            .tasks()
            .clone()
            .keep_ticking(TASKS_TICK, state.stopping().clone()),
    );
    let indexing = tokio::spawn(
        state
            .recall()
            .clone()
            .keep_indexing(RECALL_EVERY, state.stopping().clone()),
    );
    let looking = tokio::spawn(
        state
            .runners()
            .clone()
            .keep_looking(TICK_EVERY, state.stopping().clone()),
    );
    let calls_work = tokio::spawn(state.calls().clone().keep_working(state.stopping().clone()));
    let working = tokio::spawn(
        state
            .workers()
            .clone()
            .keep_ticking(services::workers::TICK, state.stopping().clone()),
    );
    let expiring = tokio::spawn(
        state
            .approvals()
            .clone()
            .keep_expiring(APPROVALS_TICK, state.stopping().clone()),
    );
    let live_ticks = tokio::spawn(
        state
            .live_calls()
            .clone()
            .keep_ticking(state.stopping().clone()),
    );
    // Its first round runs beside start-up, which neither waits for it nor fails with it.
    let keeping_folder = tokio::spawn(
        state
            .chrome()
            .clone()
            .keep_folder(services::chrome::FOLDER_EVERY, state.stopping().clone()),
    );
    match listener.local_addr() {
        Ok(address) => tracing::info!(%address, auth_mode = ?config.auth_mode, "listening"),
        Err(error) => tracing::warn!(%error, "listening, on an address the system won't name"),
    }
    if config.auth_mode == AuthMode::Report {
        tracing::warn!(
            listen = %config.listen,
            "auth mode is report: requests without a session are let through and only logged; \
             use it only behind a network that already limits who can connect"
        );
    }
    let workers = state.workers().clone();
    let served = run(
        listener,
        state,
        config.request_timeout,
        config.shutdown_grace,
        stop,
    )
    .await;
    // Before the tasks are dropped: every worker's process group ends with the server.
    workers.close_all(WORKERS_GRACE).await;
    served.map_err(RunError::Serve)?;
    polling.abort();
    ticking.abort();
    waking.abort();
    prepping.abort();
    indexing.abort();
    looking.abort();
    calls_work.abort();
    live_ticks.abort();
    keeping_folder.abort();
    expiring.abort();
    working.abort();
    collecting.abort();
    db.close().await?;
    Ok(())
}

/// What `serve` builds the app from: the machine's identity (its key made on first start), the
/// system's clock and randomness, and the push services, secrets directory, Mac helper and
/// coding-thread apps `config` names.
#[expect(
    clippy::too_many_lines,
    reason = "every dependency the configuration names, a line or a few each"
)]
async fn dependencies(config: &ServeConfig, db: &Database) -> Result<Dependencies, RunError> {
    let key = identity::serving_key(
        &config.machine_key.path(&config.database),
        &db.settings(),
        config.new_machine_key,
    )
    .await?;
    let machine = Machine::new(
        key,
        identity::machine_name(config.machine_name.name.as_ref()),
    );
    tracing::info!(id = %machine.id(), name = machine.name().as_str(), "the machine");
    let clock: Arc<dyn emberhut_core::Clock> = Arc::new(SystemClock);
    let http = http::shared_client()?;
    let mut push = PushSenders::default();
    let apns = &config.apns;
    match (&apns.key_id, &apns.team_id, &apns.topic) {
        (Some(key_id), Some(team_id), Some(topic)) => {
            let settings = ApnsConfig {
                key_file: config.secrets_dir.join(apns::KEY_FILE),
                key_id: key_id.clone(),
                team_id: team_id.clone(),
                topic: topic.clone(),
            };
            match HttpApns::new(settings, Arc::clone(&clock)) {
                Ok(sender) => push.apns = Some(Arc::new(sender)),
                Err(error) => tracing::error!(%error, "iPhones get no pushes"),
            }
        }
        (None, None, None) => tracing::info!("APNs is not configured: iPhones get no pushes"),
        _ => tracing::warn!(
            "APNs needs the key id, the team id and the topic: iPhones get no pushes"
        ),
    }
    let account = config.secrets_dir.join(fcm::ACCOUNT_FILE);
    match HttpFcm::new(account, FcmEndpoints::default(), Arc::clone(&clock)) {
        Ok(sender) => push.fcm = Some(Arc::new(sender)),
        Err(error) => tracing::error!(%error, "Android phones get no pushes"),
    }
    let program_key = match &config.tasks.program_key_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|source| RunError::ProgramKey {
                    path: path.clone(),
                    source,
                })?
                .trim()
                .to_owned(),
        )
        .filter(|key| !key.is_empty()),
        None => None,
    };
    let (assistant, assistant_tasks) = assistant(config, db, &clock)?;
    let zone = time_zone(config.time_zone.as_deref());
    let voice = Arc::new(openai_voice(config, http.clone()));
    Ok(Dependencies {
        clock,
        push,
        program_key,
        thumbnailer: Arc::new(services::tasks::Ffmpeg::new(config.tasks.ffmpeg.clone())),
        schedule: vec![Arc::new(services::scheduled::SystemdTimers)],
        secrets_dir: config.secrets_dir.clone(),
        public_url: config.link_base(),
        mac: mac_helper(&config.mac).await?,
        coding_app: Arc::new(McpCodingApp::new(
            config.coding_app.clone(),
            config.coding_app_tokens.clone(),
        )?),
        assistant,
        assistant_tasks,
        zone: zone.clone(),
        embedder: match &config.recall.embed_url {
            Some(url) => Arc::new(HttpEmbedder::new(
                http.clone(),
                url,
                config.recall.embed_model.clone(),
            )),
            None => Arc::new(NoEmbedder),
        },
        transcripts: config.recall.transcripts.clone(),
        runners: Arc::new(HttpRunners::new(
            http.clone(),
            config
                .runner
                .iter()
                .map(|runner| RunnerAt {
                    name: runner.name.clone(),
                    url: runner.url.clone(),
                })
                .collect(),
            config
                .runner_name
                .iter()
                .map(|name| (name.name.clone(), name.words.clone()))
                .collect(),
            config.runner_tokens.clone(),
        )),
        home: std::env::var_os("HOME").map(std::path::PathBuf::from),
        runner_mcp: config.runner_mcp.clone(),
        // The machine's key, where `EMBERHUT_MACHINE_KEY` puts it instead of beside the database.
        private_files: config
            .private_files()
            .into_iter()
            .chain(config.machine_key.file.clone())
            .collect(),
        personal: PersonalDependencies {
            calendar: mac_calendar(&config.mac, http.clone()).await?,
            call_transcriber: call_transcriber(&config.voice),
            live_transcriber: live_transcriber(&config.voice),
            speech: voice.clone(),
            realtime: voice,
            calls: CallsConfig {
                audio_days: config.voice.call_audio_days,
            },
            voice: VoiceConfig {
                person_labels: config.voice.voice_person_labels.clone(),
            },
            zone,
            ..PersonalDependencies::default()
        },
        messaging: messaging(config, http.clone())?,
        guard: guard_deps(config, db),
        worker_setup: worker_setup(config, db),
        browser: browser_engine(&config.browser, http.clone()),
        browser_view: config.browser.view.clone(),
        browser_home: config.browser.home.clone(),
        chrome_extension_zip: config.browser.extension_zip.clone(),
        chrome_url: config.browser.chrome_url.clone(),
        party_model: party_model(config, http.clone()),
        dark: dark_models(config, &http),
        family_directory: match &config.family_directory {
            Some(url) => Arc::new(HttpFamilyDirectory::new(http.clone(), url.clone())),
            None => Arc::new(NoFamily),
        },
        machine: Some(Arc::new(machine)),
        pairing_routes: services::advertise::configured_routes(
            config.listen,
            config.public_url.as_ref(),
        ),
        approvals: approval_deps(config, &http),
        agent_slack: Arc::new(services::agents::slack::HttpAgentSlack::new(
            services::messaging::web::WebClient::new(http.clone()),
            &config.messaging.slack_api,
        )),
        provider_api: Arc::new(services::agent_provider::HttpProviderApi::new(
            services::messaging::web::WebClient::new(http.clone()),
            &config.agent.anthropic_url,
        )),
        ..Dependencies::default()
    })
}

/// How approvals reach external approvers: Slack's Web API and their apps' webhooks over the
/// shared client, from the public address `config` names (none: the apps only).
fn approval_deps(
    config: &ServeConfig,
    http: &reqwest::Client,
) -> services::approvals::channels::ApprovalDeps {
    use services::approvals::channels::{ApprovalDeps, HttpSlackDm, HttpWebhook};
    let web = services::messaging::web::WebClient::new(http.clone());
    ApprovalDeps {
        slack: Arc::new(HttpSlackDm::new(web.clone(), &config.messaging.slack_api)),
        webhook: Arc::new(HttpWebhook::new(web)),
        public_url: config
            .public_url
            .as_ref()
            .map(|url| url.as_str().to_owned()),
    }
}

/// Where the model provider the server's own agent sessions run on is saved (`PUT
/// /api/agent/provider`): its key in the secrets folder, the rest in the settings.
fn agent_provider(config: &ServeConfig, db: &Database) -> services::agent_provider::ProviderStore {
    services::agent_provider::ProviderStore::new(
        services::messaging::secrets::Secrets::new(config.secrets_dir.clone()),
        db.settings(),
    )
}

/// The agent CLI's accounts as the configuration gives them (`ID` or `ID=CONFIG_DIR`).
fn agent_accounts(config: &ServeConfig) -> Vec<services::claude::AgentAccount> {
    config
        .agent
        .accounts
        .iter()
        .map(|account| account.trim())
        .filter(|account| !account.is_empty())
        .map(|account| match account.split_once('=') {
            Some((id, dir)) => services::claude::AgentAccount {
                id: id.trim().to_owned(),
                config_dir: Some(std::path::PathBuf::from(dir.trim())),
            },
            None => services::claude::AgentAccount {
                id: account.to_owned(),
                config_dir: None,
            },
        })
        .collect()
}

/// The server's own address, as a session on this machine reaches it, at `path`.
fn local_url(config: &ServeConfig, path: &str) -> String {
    let host = if config.listen.ip().is_unspecified() {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        config.listen.ip()
    };
    format!(
        "http://{}{path}",
        SocketAddr::new(host, config.listen.port())
    )
}

/// How the workers run: with the agent CLI when it is configured (none otherwise), and the
/// ultra runs' caps.
fn worker_setup(config: &ServeConfig, db: &Database) -> app::WorkerSetup {
    let workers = &config.workers;
    let runtime = config
        .agent
        .command
        .as_ref()
        .map(|command| services::workers::WorkerRuntime {
            spawner: Arc::new(services::claude::ProcessSpawner::default()),
            config: services::claude::worker::WorkerConfig {
                command: command.clone(),
                args: config
                    .agent
                    .args
                    .as_deref()
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                model: workers
                    .worker_model
                    .clone()
                    .or_else(|| config.agent.model.clone()),
                permission_mode: Some(config.agent.permission_mode.clone())
                    .filter(|m| !m.is_empty()),
                accounts: agent_accounts(config),
                mcp_url: local_url(config, emberhut_api::paths::MCP_WORKER),
                gate_url: local_url(config, emberhut_api::paths::ULTRA_GATE),
                workdir: config.agent.workdir.clone(),
                provider: Some(agent_provider(config, db)),
            },
        });
    app::WorkerSetup {
        runtime,
        limits: emberhut_core::ultra::Limits {
            parallel: workers.ultra_parallel,
            budget: workers.ultra_budget,
            ask_tokens: workers.ultra_ask_tokens,
            ask_agents: workers.ultra_ask_agents,
        },
        lane_machine: workers.lane_runner.clone(),
        max_working: workers.worker_max,
    }
}

/// The guard rails' setup: the repositories always let through, the people messaged without
/// an OK per text, and the ask guard's judge (the agent CLI, unless turned off).
fn guard_deps(config: &ServeConfig, db: &Database) -> app::GuardDeps {
    let workers = &config.workers;
    let judge: Arc<dyn services::guard::Judge> = match &config.agent.command {
        Some(_) if workers.ask_guard_model.as_deref() == Some("off") => {
            Arc::new(services::guard::NoJudge)
        }
        Some(command) => {
            let accounts = agent_accounts(config);
            let judge = services::claude::judge::CliJudge::new(
                Arc::new(services::claude::ProcessSpawner::default()),
                command.clone(),
                workers
                    .ask_guard_model
                    .clone()
                    .or_else(|| config.agent.model.clone()),
                accounts.iter().find_map(|a| a.config_dir.clone()),
            );
            // As the sessions: the saved provider only while no account is configured.
            Arc::new(if accounts.is_empty() {
                judge.with_provider(agent_provider(config, db))
            } else {
                judge
            })
        }
        None => Arc::new(services::guard::NoJudge),
    };
    app::GuardDeps {
        config: services::guard::GuardConfig {
            always_merge: workers
                .always_merge
                .iter()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty())
                .collect(),
            send_without_ok: workers.send_without_ok.clone(),
            home: std::env::var_os("HOME").map(std::path::PathBuf::from),
        },
        judge,
        remotes: Arc::new(services::guard::GitRemotes),
    }
}

/// The assistant, and its background tasks.
type Runtime = (
    Arc<dyn services::talk::Assistant>,
    Arc<dyn services::workers_view::AssistantTasks>,
);

/// The assistant `config` names, with its background tasks: the agent CLI's sessions, or the
/// silent one (no tasks) without a command.
fn assistant(
    config: &ServeConfig,
    db: &Database,
    clock: &Arc<dyn emberhut_core::Clock>,
) -> Result<Runtime, RunError> {
    let agent = &config.agent;
    let Some(command) = &agent.command else {
        tracing::info!("no agent command is configured: the assistant answers nothing");
        return Ok((
            Arc::new(services::talk::Silent),
            Arc::new(services::workers_view::NoWorkers),
        ));
    };
    let prompt = match &agent.prompt_file {
        Some(path) => {
            Some(
                std::fs::read_to_string(path).map_err(|source| RunError::AgentPrompt {
                    path: path.clone(),
                    source,
                })?,
            )
        }
        None => None,
    };
    let accounts = agent
        .accounts
        .iter()
        .map(|account| account.trim())
        .filter(|account| !account.is_empty())
        .map(|account| match account.split_once('=') {
            Some((id, dir)) => services::claude::AgentAccount {
                id: id.trim().to_owned(),
                config_dir: Some(std::path::PathBuf::from(dir.trim())),
            },
            None => services::claude::AgentAccount {
                id: account.to_owned(),
                config_dir: None,
            },
        })
        .collect();
    let host = if config.listen.ip().is_unspecified() {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        config.listen.ip()
    };
    let mcp_url = format!(
        "http://{}{}",
        SocketAddr::new(host, config.listen.port()),
        emberhut_api::paths::MCP_ROOM
    );
    let settings = services::claude::ClaudeConfig {
        command: command.clone(),
        args: agent
            .args
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        model: agent.model.clone(),
        workdir: agent.workdir.clone(),
        permission_mode: Some(agent.permission_mode.clone()).filter(|mode| !mode.is_empty()),
        accounts,
        mcp_url,
        prompt,
        max_sessions: agent.max_sessions,
    };
    tracing::info!(command = %command.display(), "the assistant runs the agent CLI");
    let runtime = Arc::new(
        services::claude::ClaudeAssistant::new(
            settings,
            Arc::new(services::claude::ProcessSpawner::default()),
            db.settings(),
            db.rooms(),
            Arc::clone(clock),
        )
        .with_jobs(services::plan::PlanService::new(
            db.settings(),
            db.log(),
            Arc::clone(clock),
        ))
        .with_rules(services::learning::LearningService::new(
            db.corrections(),
            db.settings(),
            db.conversation(),
            Arc::clone(clock),
        ))
        .with_accounts(db.prefs())
        .with_provider(agent_provider(config, db))
        .with_agents(services::claude::AgentRooms::new(
            db.agents(),
            config.secrets_dir.clone(),
            db.path().parent().map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            ),
        )),
    );
    Ok((Arc::clone(&runtime) as _, runtime as _))
}

/// The time zone `name` names, or the system's (also when `name` is not one).
fn time_zone(name: Option<&str>) -> jiff::tz::TimeZone {
    match name {
        Some(name) => jiff::tz::TimeZone::get(name).unwrap_or_else(|error| {
            tracing::warn!(name, %error, "not a time zone: using the system's");
            jiff::tz::TimeZone::system()
        }),
        None => jiff::tz::TimeZone::system(),
    }
}

/// The messaging services `config` sets up: Slack's and Google's APIs, speech to text, the
/// Mac helper's message routes and the WhatsApp daemon, each over HTTPS. The tasks are the
/// server's own, and the guard rails' OKs take the place of the `SendOks` stand-in (see
/// `services::messaging::hooks`).
fn messaging(
    config: &ServeConfig,
    http: reqwest::Client,
) -> Result<services::messaging::MessagingDeps, RunError> {
    use services::messaging::{
        MessagingDeps, gmail::HttpGoogle, mac::HttpMacMessages, secrets::Secrets, slack::HttpSlack,
        voice_notes::OpenAiTranscriber, whatsapp::HttpWaDaemon,
    };
    let settings = &config.messaging;
    let web = services::messaging::web::WebClient::new(http);
    let secrets = Secrets::new(config.secrets_dir.clone());
    let mut deps = MessagingDeps {
        slack: Arc::new(HttpSlack::new(web.clone(), &settings.slack_api)),
        google: Arc::new(HttpGoogle::new(web.clone(), settings.gmail_api.as_deref())),
        transcriber: Arc::new(OpenAiTranscriber::new(
            web.clone(),
            // Voice notes go to the same speech provider and model as dictation.
            &config.voice.openai_url,
            &config.voice.transcribe_model,
            secrets.clone(),
        )),
        gmail_callback: settings.gmail_callback.clone(),
        wake_name: settings.whatsapp_name.clone(),
        owner_phone: settings.whatsapp_owner.clone(),
        owner_name: settings.owner_name.clone(),
        assistant_phone: settings.whatsapp_phone.clone(),
        voice_max_seconds: settings.voice_max_seconds,
        ..MessagingDeps::default()
    };
    if let (Some(url), Some(path)) = (&config.mac.url, &config.mac.token_file) {
        let token = std::fs::read_to_string(path).map_err(|source| RunError::MacHelperToken {
            path: path.clone(),
            source,
        })?;
        deps.mac = Arc::new(HttpMacMessages::new(
            web.clone(),
            url.as_str(),
            token.trim().to_owned(),
        ));
    }
    if let Some(url) = &settings.whatsapp_url {
        deps.wa_daemon = Arc::new(HttpWaDaemon::new(web, url, secrets));
    }
    Ok(deps)
}

/// The browser `config` names (over the `DevTools` protocol), or none, reached with `http`.
fn browser_engine(
    config: &config::BrowserConfig,
    http: reqwest::Client,
) -> Arc<dyn services::browser::engine::BrowserEngine> {
    match &config.cdp {
        Some(address) => {
            tracing::info!(%address, "the assistant's browser");
            services::browser::cdp::CdpEngine::new(
                services::browser::cdp::DirectLink::new(http, address),
                true,
            )
        }
        None => Arc::new(services::browser::engine::NoBrowser),
    }
}

/// The parties' model `config` names, or none.
fn party_model(config: &ServeConfig, http: reqwest::Client) -> Arc<dyn ChatModel> {
    let Some(url) = &config.side_models.party_url else {
        return Arc::new(NoChatModel);
    };
    let key = config.secrets_dir.join(config::PARTY_MODEL_KEY);
    Arc::new(OpenAiChat::new(http, url, Some(key)))
}

/// The off-the-record room's models `config` names: the cloud one (or none) and a local one
/// tried first.
fn dark_models(config: &ServeConfig, http: &reqwest::Client) -> DarkModels {
    let side = &config.side_models;
    let secrets = &config.secrets_dir;
    let made = |url: &str, key: &str| OpenAiChat::new(http.clone(), url, Some(secrets.join(key)));
    let cloud: Arc<dyn ChatModel> = side
        .dark_url
        .as_deref()
        .map(|url| made(url, config::DARK_MODEL_KEY))
        .map_or_else(
            || Arc::new(NoChatModel) as Arc<dyn ChatModel>,
            |model| match &side.dark_model {
                Some(id) => Arc::new(model.with_model(id.clone())),
                None => Arc::new(model),
            },
        );
    let local = side
        .dark_local_url
        .as_deref()
        .map(|url| made(url, config::DARK_LOCAL_KEY))
        .map(|model| {
            (
                Arc::new(model.with_health_check()) as Arc<dyn ChatModel>,
                side.dark_local_label.clone(),
            )
        });
    DarkModels {
        cloud,
        cloud_label: side.dark_label.clone(),
        local,
    }
}

/// The Mac helper `config` names, or none.
async fn mac_helper(config: &MacHelperConfig) -> Result<Arc<dyn MacHelper>, RunError> {
    let (Some(url), Some(path)) = (&config.url, &config.token_file) else {
        return Ok(Arc::new(NoMacHelper));
    };
    let token =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|source| RunError::MacHelperToken {
                path: path.clone(),
                source,
            })?;
    let helper = HttpMacHelper::new(url.clone(), token.trim().to_owned())?;
    tracing::info!(url = url.as_str(), "the Mac helper is set up");
    Ok(Arc::new(helper))
}

/// The calendar through the Mac helper `config` names, or none, reached with `http`.
async fn mac_calendar(
    config: &MacHelperConfig,
    http: reqwest::Client,
) -> Result<Arc<dyn CalendarProvider>, RunError> {
    let (Some(url), Some(path)) = (&config.url, &config.token_file) else {
        return Ok(Arc::new(NoCalendar));
    };
    let token =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|source| RunError::MacHelperToken {
                path: path.clone(),
                source,
            })?;
    Ok(Arc::new(HttpMacCalendar::new(
        http,
        url.clone(),
        token.trim().to_owned(),
    )))
}

/// The call transcriber `config` names, or none.
fn call_transcriber(config: &VoiceSettings) -> Arc<dyn CallTranscriber> {
    if let Some(program) = &config.call_transcriber {
        Arc::new(CommandTranscriber::new(
            program.clone(),
            Duration::from_secs(config.call_transcribe_minutes.saturating_mul(60)),
        ))
    } else {
        tracing::info!("no call transcriber is set up: recorded calls are kept untranscribed");
        Arc::new(NoTranscriber)
    }
}

/// The live transcriber over the call transcriber `config` names, or none.
fn live_transcriber(config: &VoiceSettings) -> Arc<dyn LiveTranscriber> {
    match &config.call_transcriber {
        Some(program) => Arc::new(ResidentTranscriber::new(program.clone())),
        None => Arc::new(NoLiveTranscriber),
    }
}

/// The speech provider, with its key in the secrets directory.
fn openai_voice(config: &ServeConfig, http: reqwest::Client) -> OpenAiVoice {
    OpenAiVoice::new(
        OpenAiConfig {
            url: config.voice.openai_url.trim_end_matches('/').to_owned(),
            key_file: config.secrets_dir.join(voice::KEY_FILE),
            voice_model: config.voice.voice_model.clone(),
            transcribe_model: config.voice.transcribe_model.clone(),
            voice: config.voice.voice.clone(),
        },
        http,
    )
}

/// Serves `state`'s app on `listener` until `stop` resolves; then tells long-lived responses
/// to end and takes no new turns ([`AppState::stopping`]), gives open connections and the
/// assistant's running turns, together, up to `grace` to finish, and waits for the assistant
/// to close what it keeps running ([`services::talk::Assistant::close_all`]).
///
/// # Errors
///
/// The I/O error that stopped the server, if one did.
pub async fn run(
    listener: TcpListener,
    state: AppState,
    request_timeout: Duration,
    grace: Duration,
    stop: impl Future<Output = ()>,
) -> std::io::Result<()> {
    let stopping = state.stopping().clone();
    let talk = state.talk().clone();
    drop(tokio::spawn(state.lag().clone().probe(stopping.clone())));
    // The requests of this TCP listener, and only they, are marked as direct TCP, with the
    // local address that accepted each connection.
    let app =
        router(state, request_timeout).layer(axum::middleware::map_request(peer::mark_direct));
    let mut server = pin!(
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<peer::Accepted>()
        )
        .with_graceful_shutdown(stopping.clone().cancelled_owned())
        .into_future()
    );
    tokio::select! {
        result = &mut server => return result,
        () = stop => stopping.cancel(),
    }
    let deadline = tokio::time::Instant::now() + grace;
    let drained = if let Ok(result) = tokio::time::timeout_at(deadline, server).await {
        result
    } else {
        tracing::warn!(
            ?grace,
            "connections still open after the grace period; stopping anyway"
        );
        Ok(())
    };
    talk.finish(deadline.saturating_duration_since(tokio::time::Instant::now()))
        .await;
    // The sessions kept between turns get their own grace to exit (the agent CLI's).
    talk.assistant().close_all().await;
    drained
}

/// A pairing code made on the command line, and its pairing link.
#[derive(Debug)]
pub struct CommandLineCode {
    /// The code, and when it stops working.
    pub issued: IssuedCode,
    /// The link that carries it, the machine's id and the configured routes; or why there is
    /// none (routes too long for a QR code), when the code still works typed.
    pub link: Result<emberhut_identity::PairingLink, emberhut_identity::PairingLinkError>,
}

/// `link` as a QR code drawn in a terminal, two rows of modules to a line of half blocks, with
/// its quiet zone. It is dark on light, as scanners expect: with `colors`, each line sets a
/// white background and black foreground itself (ANSI), so it scans on dark and light themes
/// alike; without (the output isn't a terminal), dark modules are full blocks on the
/// background.
///
/// # Errors
///
/// [`qrcode::types::QrError`] when the link doesn't fit a QR code (it always does: a link is
/// at most 2,000 characters).
pub fn terminal_qr(
    link: &emberhut_identity::PairingLink,
    colors: bool,
) -> Result<String, qrcode::types::QrError> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::with_error_correction_level(link.to_url(), qrcode::EcLevel::L)?;
    let drawn = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build();
    if !colors {
        return Ok(drawn);
    }
    Ok(drawn
        .lines()
        .map(|line| format!("\x1b[30;47m{line}\x1b[0m"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Makes a one-time pairing code in the server's database, which `config` names, for
/// `emberhut-server pair`, with its pairing link: the machine's id, and the routes the
/// configuration gives (the listen address when other devices can reach it, and the public
/// URL; ADR 0007, section 2).
///
/// The database has to exist already, with its schema up to date: a code made in a new, empty
/// database (the server's configuration missed, or another directory) would be one the server
/// does not know, and migrating from here would change the schema under a running server.
///
/// # Errors
///
/// [`RunError::Database`] (also when there is no database there, or it needs migrating),
/// [`RunError::MachineKey`] or [`RunError::Pairing`]. A link that can't be made doesn't stop
/// the code: [`CommandLineCode::link`] says why.
pub async fn make_pairing_code(config: &PairConfig) -> Result<CommandLineCode, RunError> {
    let db = Database::open_existing(&config.database.url).await?;
    let key = identity::existing_key(&config.machine_key.path(&config.database))?;
    let machine = Machine::new(key, identity::machine_name(None));
    let pairing = PairingService::new(db.pairing(), Arc::new(SystemClock), Arc::new(OsRandom));
    let issued = pairing.new_code(CodeMaker::CommandLine).await?;
    db.close().await?;
    let routes = services::advertise::configured_routes(config.listen, config.public_url.as_ref());
    let link = machine.pairing_link(&issued.code.to_string(), &routes);
    Ok(CommandLineCode { issued, link })
}

/// This machine's id, for `emberhut-server id`: from its key beside the database `config` names
/// (the server makes it when it first serves the database).
///
/// # Errors
///
/// [`RunError::MachineKey`] when there is no key, or it can't be read.
pub fn machine_id(config: &IdConfig) -> Result<emberhut_identity::MachineId, RunError> {
    let key = identity::existing_key(&config.machine_key.path(&config.database))?;
    Ok(key.machine_id())
}
