//! The server's configuration: command-line flags, each with an `EMBERHUT_*` environment variable,
//! parsed into typed values and checked before anything starts.
//!
//! Nothing secret is configured here: the server makes its own secrets and keeps only their
//! hashes in its database. The keys of outside services (the push services') are files in the
//! secrets directory, never values in the configuration or the environment.

use std::net::SocketAddr;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use emberhut_api::voice::{NEUTRAL_PERSON_LABEL, PERSON_LABEL};
use emberhut_db::DatabaseUrl;
use emberhut_identity::{MachineName, RelayDomain};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::ParseError;

/// The `emberhut-server` command line.
#[derive(Debug, Parser)]
#[command(
    name = "emberhut-server",
    version,
    about,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Which log lines to write, as a `tracing` filter (`info`, `emberhut_server=debug,info`).
    #[arg(long, env = "EMBERHUT_LOG", global = true, default_value = "info")]
    pub log: LogFilter,
    /// What to do; serving is the default.
    #[command(subcommand)]
    pub command: Option<Command>,
    /// How to serve, when no command is given.
    #[command(flatten)]
    pub serve: ServeConfig,
}

impl Cli {
    /// The command to run: the one given, or serving.
    #[must_use]
    pub fn into_command(self) -> Command {
        self.command.unwrap_or(Command::Serve(self.serve))
    }
}

/// What `emberhut-server` can do.
#[derive(Debug, Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "parsed once at start-up and never moved around"
)]
pub enum Command {
    /// Serve the API (the default).
    Serve(ServeConfig),
    /// Print a one-time code that pairs a new device, for the first device or one that cannot
    /// reach a signed-in one, with its pairing link and the link's QR code.
    Pair(PairConfig),
    /// Print this machine's id: the hash of its public key, as pairing links and the account's
    /// list of machines show it.
    Id(IdConfig),
    /// Link this machine to an emberhut.com account: sign in to the account in a browser, with the
    /// code this prints, and the machine signs its own link.
    Link(LinkConfig),
    /// Take this machine off its emberhut.com account, with no sign-in: the machine signs its own
    /// unlink.
    Unlink(UnlinkConfig),
}

/// How to serve.
#[derive(Clone, Debug, Args)]
pub struct ServeConfig {
    /// The database.
    #[command(flatten)]
    pub database: DatabaseConfig,
    /// The address and port to listen on.
    #[arg(long, env = "EMBERHUT_LISTEN", default_value = "127.0.0.1:18990")]
    pub listen: SocketAddr,
    /// Whether requests without a session are refused (`enforce`), or let through and logged
    /// (`report`, a migration aid). Read at start-up.
    #[arg(long, env = "EMBERHUT_AUTH_MODE", value_enum, default_value_t = AuthMode::Enforce)]
    pub auth_mode: AuthMode,
    /// How long a request may take before it is answered with 503, in seconds.
    #[arg(
        long = "request-timeout",
        env = "EMBERHUT_REQUEST_TIMEOUT",
        value_name = "SECONDS",
        default_value = "30",
        value_parser = seconds
    )]
    pub request_timeout: Duration,
    /// How long open requests and streams get to finish once a shutdown starts, in seconds.
    #[arg(
        long = "shutdown-grace",
        env = "EMBERHUT_SHUTDOWN_GRACE",
        value_name = "SECONDS",
        default_value = "10",
        value_parser = seconds
    )]
    pub shutdown_grace: Duration,
    /// Where the server keeps the secrets it is given: the push services' keys
    /// (`apns-key.p8`, `fcm-service-account.json`) and what is typed into one-time secret
    /// links. Made, for the server's user only, when a secret is first saved.
    #[arg(
        long = "secrets-dir",
        env = "EMBERHUT_SECRETS_DIR",
        default_value = "secrets"
    )]
    pub secrets_dir: PathBuf,
    /// The address the person's devices reach this server at (`https://emberhut.example.com`),
    /// for the links the server makes. Left out, `http://` and the listen address.
    #[arg(long = "public-url", env = "EMBERHUT_PUBLIC_URL")]
    pub public_url: Option<PublicUrl>,
    /// iPhone pushes.
    #[command(flatten)]
    pub apns: ApnsSettings,
    /// The helper on the owner's Mac.
    #[command(flatten)]
    pub mac: MacHelperConfig,
    /// The owner's messages from outside the app, and the assistant's own WhatsApp number.
    #[command(flatten)]
    pub messaging: MessagingConfig,
    /// The coding-thread app on each machine, `NAME=URL` (its MCP endpoint), repeated or
    /// comma-separated in the variable. None: the server follows no coding-thread app.
    #[arg(
        long = "coding-app",
        env = "EMBERHUT_CODING_APP",
        value_name = "NAME=URL",
        value_delimiter = ','
    )]
    pub coding_app: Vec<CodingAppMachine>,
    /// A file of `NAME=TOKEN` lines: the bearer token for each machine's coding-thread app,
    /// read at every call. None: no token is sent.
    #[arg(
        long = "coding-app-tokens",
        env = "EMBERHUT_CODING_APP_TOKENS",
        value_name = "FILE"
    )]
    pub coding_app_tokens: Option<PathBuf>,
    /// Tasks: the programs' key and the thumbnails of what tasks produce.
    #[command(flatten)]
    pub tasks: TaskConfig,
    /// The assistant: the agent CLI each room's session runs.
    #[command(flatten)]
    pub agent: AgentSettings,
    /// The assistant's workers, ultra runs and guard rails.
    #[command(flatten)]
    pub workers: WorkerSettings,
    /// The time zone routines and quiet hours are read in (an IANA name such as
    /// `Europe/Berlin`); the system's when left out.
    #[arg(long = "time-zone", env = "EMBERHUT_TIME_ZONE", value_name = "ZONE")]
    pub time_zone: Option<String>,
    /// Recall's search by meaning.
    #[command(flatten)]
    pub recall: RecallConfig,
    /// The runner on each of the person's machines, `NAME=URL` (its API), repeated or
    /// comma-separated in the variable. The name is the one the runner reports itself by.
    /// None: no runner sessions.
    #[arg(
        long = "runner",
        env = "EMBERHUT_RUNNER",
        value_name = "NAME=URL",
        value_delimiter = ','
    )]
    pub runner: Vec<CodingAppMachine>,
    /// A runner's machine in words, `NAME=WORDS` (`build=Build box`), repeated or
    /// comma-separated. Left out, a runner's machine is called by its name.
    #[arg(
        long = "runner-name",
        env = "EMBERHUT_RUNNER_NAMES",
        value_name = "NAME=WORDS",
        value_delimiter = ','
    )]
    pub runner_name: Vec<RunnerName>,
    /// A file of `NAME=TOKEN` lines: each runner's token, which the server sends it and it
    /// presents when it reports back. Read again whenever it changes. None: runners can't
    /// report.
    #[arg(
        long = "runner-tokens",
        env = "EMBERHUT_RUNNER_TOKENS",
        value_name = "FILE"
    )]
    pub runner_tokens: Option<PathBuf>,
    /// A JSON file `{ "mcpServers": { ... } }`: MCP servers the runners' sessions get too
    /// (`GET /api/runner/mcp`). Read at every request. None: none are shared.
    #[arg(long = "runner-mcp", env = "EMBERHUT_RUNNER_MCP", value_name = "FILE")]
    pub runner_mcp: Option<PathBuf>,
    /// Calls' transcription, and the voice provider.
    #[command(flatten)]
    pub voice: VoiceSettings,
    /// The browser the assistant drives.
    #[command(flatten)]
    pub browser: BrowserConfig,
    /// The household server that says who a family member's device key belongs to,
    /// `http://127.0.0.1:<port>` (its `GET /api/whoami`). None: no family member gets in.
    #[arg(
        long = "family-directory-url",
        env = "EMBERHUT_FAMILY_DIRECTORY_URL",
        value_name = "URL",
        value_parser = helper_url
    )]
    pub family_directory: Option<reqwest::Url>,
    /// The models the side apps talk through (parties, the off-the-record room).
    #[command(flatten)]
    pub side_models: SideModelsConfig,
    /// The machine's key.
    #[command(flatten)]
    pub machine_key: MachineKeyConfig,
    /// The machine's name, as its link statements sign it.
    #[command(flatten)]
    pub machine_name: MachineNameConfig,
    /// Serve as a new machine on purpose: take the key at the key file (made if there is none)
    /// even though the database was served with another. Every device pairs again, and the
    /// machine is linked to its account again. Without it the server refuses to start then,
    /// so a mistyped key path never makes a new machine. Use it for one start only, then
    /// remove it: the server logs a warning every time it is given.
    #[arg(long = "new-machine-key")]
    pub new_machine_key: bool,
}

/// The browser the assistant drives, its live view, and the browser extension the owner's
/// Chrome profiles pair with. Without a `DevTools` address the browser tools say no browser is
/// set up.
#[derive(Clone, Debug, Default, Args)]
pub struct BrowserConfig {
    /// The browser's `DevTools` address: `http://127.0.0.1:9223` (its WebSocket is read from
    /// `/json/version`), or the browser's `ws://` address itself.
    #[arg(long = "browser-cdp", env = "EMBERHUT_BROWSER_CDP", value_name = "URL")]
    pub cdp: Option<String>,
    /// The live view's address the owner's app loads (the browser's screen, streamed). Its
    /// passwords are the secrets `browser-view` and `browser-control`.
    #[arg(
        long = "browser-view",
        env = "EMBERHUT_BROWSER_VIEW",
        value_name = "URL"
    )]
    pub view: Option<String>,
    /// The owner's home folder: browser uploads come from it (or /tmp) and downloads go there.
    /// Left out: `$HOME`.
    #[arg(
        long = "browser-home",
        env = "EMBERHUT_BROWSER_HOME",
        value_name = "PATH"
    )]
    pub home: Option<PathBuf>,
    /// The browser extension's build, a zip with its `manifest.json` (its version is the one
    /// the paired profiles should run), served at `/chrome/emberhut-extension.zip`.
    #[arg(
        long = "chrome-extension-zip",
        env = "EMBERHUT_CHROME_EXTENSION_ZIP",
        value_name = "FILE"
    )]
    pub extension_zip: Option<PathBuf>,
    /// The address the owner's Chrome profiles reach the server at, for pairing links. Left
    /// out: the public URL.
    #[arg(long = "chrome-url", env = "EMBERHUT_CHROME_URL", value_name = "URL")]
    pub chrome_url: Option<String>,
}

/// What transcribes calls, and the speech provider for voice calls and dictation (its API key
/// is the file `openai-api-key` in the secrets directory, read at each call).
#[derive(Clone, Debug, Args)]
pub struct VoiceSettings {
    /// A program that transcribes a recorded call on this machine: run as `<program>
    /// <transcript.md> <title> <file>[=<who>]...` (by position, no options), and kept running
    /// as `<program> --live` for a call's live transcript. None: calls are kept untranscribed.
    #[arg(
        long = "call-transcriber",
        env = "EMBERHUT_CALL_TRANSCRIBER",
        value_name = "PROGRAM"
    )]
    pub call_transcriber: Option<PathBuf>,
    /// How many days a transcribed call's audio is kept (its transcript is kept for good).
    #[arg(
        long = "call-audio-days",
        env = "EMBERHUT_CALL_AUDIO_DAYS",
        value_name = "DAYS",
        default_value = "14"
    )]
    pub call_audio_days: u32,
    /// The longest one run of the call transcriber may take, in minutes: a run past it is
    /// stopped and its call fails, so the calls after it are transcribed.
    #[arg(
        long = "call-transcribe-minutes",
        env = "EMBERHUT_CALL_TRANSCRIBE_MINUTES",
        value_name = "MINUTES",
        default_value = "120",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub call_transcribe_minutes: u64,
    /// The speech provider's API (`OpenAI`'s, or one that speaks it).
    #[arg(
        long = "openai-url",
        env = "EMBERHUT_OPENAI_URL",
        value_name = "URL",
        default_value = "https://api.openai.com/v1"
    )]
    pub openai_url: String,
    /// The realtime model for voice calls.
    #[arg(
        long = "voice-model",
        env = "EMBERHUT_VOICE_MODEL",
        default_value = "gpt-realtime-2.1-mini"
    )]
    pub voice_model: String,
    /// The speech-to-text model for dictation.
    #[arg(
        long = "transcribe-model",
        env = "EMBERHUT_TRANSCRIBE_MODEL",
        default_value = "gpt-transcribe"
    )]
    pub transcribe_model: String,
    /// The voice the assistant speaks with on a call.
    #[arg(long = "voice", env = "EMBERHUT_VOICE", default_value = "marin")]
    pub voice: String,
    /// The labels a voice call's transcript gives the person's lines, repeated or
    /// comma-separated in the variable: a transcript with a line starting with one of them is
    /// followed up. The reference client's label and a neutral `Me:` by default.
    #[arg(
        long = "voice-person-label",
        env = "EMBERHUT_VOICE_PERSON_LABELS",
        value_name = "LABEL",
        value_delimiter = ',',
        default_values_t = [PERSON_LABEL.to_owned(), NEUTRAL_PERSON_LABEL.to_owned()]
    )]
    pub voice_person_labels: Vec<String>,
}

/// What the tasks area takes from the configuration.
#[derive(Clone, Debug, Default, Args)]
pub struct TaskConfig {
    /// A file holding the key a program on this machine sends as its bearer to hand the
    /// assistant a job (`POST /api/tasks`); read at start-up, surrounding whitespace ignored.
    /// None: no program can.
    #[arg(
        long = "program-key-file",
        env = "EMBERHUT_PROGRAM_KEY_FILE",
        value_name = "PATH"
    )]
    pub program_key_file: Option<PathBuf>,
    /// The ffmpeg that makes the thumbnails of a task's pictures and videos (a name on the
    /// `PATH`, or a path).
    #[arg(
        long = "ffmpeg",
        env = "EMBERHUT_FFMPEG",
        value_name = "PATH",
        default_value = "ffmpeg"
    )]
    pub ffmpeg: PathBuf,
}

/// The agent CLI the assistant runs, a session per room. Without a command the server answers
/// nothing (the silent assistant). It runs on the accounts configured here, or, with none, on
/// the model provider saved from the apps (`PUT /api/agent/provider`), or else on its own login.
#[derive(Clone, Debug, Default, Args)]
pub struct AgentSettings {
    /// The agent CLI's program (`claude`, or a path to it).
    #[arg(
        long = "agent-command",
        env = "EMBERHUT_AGENT_COMMAND",
        value_name = "PROGRAM"
    )]
    pub command: Option<PathBuf>,
    /// More arguments for it, separated by spaces, after the server's own.
    #[arg(
        long = "agent-args",
        env = "EMBERHUT_AGENT_ARGS",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    pub args: Option<String>,
    /// The model its sessions use; the CLI's default when left out.
    #[arg(long = "agent-model", env = "EMBERHUT_AGENT_MODEL")]
    pub model: Option<String>,
    /// The directory its sessions run in; the server's when left out.
    #[arg(long = "agent-workdir", env = "EMBERHUT_AGENT_WORKDIR")]
    pub workdir: Option<PathBuf>,
    /// The CLI's permission mode. Its sessions run unattended (nobody answers a permission
    /// prompt), and the server's own MCP tools are allowed in every mode. The default,
    /// `dontAsk`, refuses every other tool that would ask (the shell, file edits), so the
    /// assistant acts only through the server's tools; `bypassPermissions` lets every tool
    /// run, the shell and file edits in the working directory included, as the user the server
    /// runs as.
    #[arg(
        long = "agent-permission-mode",
        env = "EMBERHUT_AGENT_PERMISSION_MODE",
        default_value = "dontAsk"
    )]
    pub permission_mode: String,
    /// The accounts its sessions run on, in the order they are tried when one runs out:
    /// `ID` (the CLI's default login) or `ID=CONFIG_DIR`, comma-separated. None: the CLI's own
    /// login, with no failover.
    #[arg(
        long = "agent-accounts",
        env = "EMBERHUT_AGENT_ACCOUNTS",
        value_name = "ID[=DIR]",
        value_delimiter = ','
    )]
    pub accounts: Vec<String>,
    /// A file whose text is added to the sessions' system prompt.
    #[arg(
        long = "agent-prompt-file",
        env = "EMBERHUT_AGENT_PROMPT_FILE",
        value_name = "FILE"
    )]
    pub prompt_file: Option<PathBuf>,
    /// Anthropic's API, where an Anthropic API key saved from the apps is checked
    /// (`PUT /api/agent/provider`); a stand-in's address in tests.
    #[arg(
        long = "anthropic-url",
        env = "EMBERHUT_ANTHROPIC_URL",
        value_name = "URL",
        default_value = "https://api.anthropic.com"
    )]
    pub anthropic_url: String,
    /// Sessions open at once, Main's included.
    #[arg(
        long = "agent-max-sessions",
        env = "EMBERHUT_AGENT_MAX_SESSIONS",
        default_value_t = 4
    )]
    pub max_sessions: usize,
}

/// The assistant's workers (a CLI session per task, run when the agent CLI is configured),
/// its ultra runs' caps, and the guard rails around them.
#[derive(Clone, Debug, Default, Args)]
pub struct WorkerSettings {
    /// The model workers run on, when their task names none (the agent's model otherwise).
    #[arg(long = "worker-model", env = "EMBERHUT_WORKER_MODEL")]
    pub worker_model: Option<String>,
    /// The most workers at once; none: no ceiling.
    #[arg(long = "worker-max", env = "EMBERHUT_WORKER_MAX")]
    pub worker_max: Option<usize>,
    /// An ultra run's sub-agents at a time.
    #[arg(
        long = "ultra-parallel",
        env = "EMBERHUT_ULTRA_PARALLEL",
        default_value_t = 4
    )]
    pub ultra_parallel: u32,
    /// An ultra run's spending cap, in its sub-agents' tokens.
    #[arg(
        long = "ultra-budget",
        env = "EMBERHUT_ULTRA_BUDGET",
        default_value_t = 3_000_000.0
    )]
    pub ultra_budget: f64,
    /// An ultra plan over this many tokens waits for the owner's OK.
    #[arg(
        long = "ultra-ask-tokens",
        env = "EMBERHUT_ULTRA_ASK_TOKENS",
        default_value_t = 1_500_000.0
    )]
    pub ultra_ask_tokens: f64,
    /// An ultra plan of more sub-agents than this waits for the owner's OK.
    #[arg(
        long = "ultra-ask-agents",
        env = "EMBERHUT_ULTRA_ASK_AGENTS",
        default_value_t = 12
    )]
    pub ultra_ask_agents: u32,
    /// The runner an ultra run's lanes start on; the first runner configured when left out.
    #[arg(long = "ultra-lane-runner", env = "EMBERHUT_ULTRA_LANE_RUNNER")]
    pub lane_runner: Option<String>,
    /// The repositories whose push and merge always go through (`owner/name`, or `*/name`),
    /// comma-separated.
    #[arg(
        long = "always-merge",
        env = "EMBERHUT_ALWAYS_MERGE",
        value_delimiter = ','
    )]
    pub always_merge: Vec<String>,
    /// A JSON file of the people the assistant may message without an OK per text
    /// (`{ "Name": ["slack:team:id"] }`), read at every send.
    #[arg(
        long = "send-without-ok",
        env = "EMBERHUT_SEND_WITHOUT_OK",
        value_name = "FILE"
    )]
    pub send_without_ok: Option<PathBuf>,
    /// The ask guard's judge model (with the agent CLI configured); `off` turns the guard
    /// off.
    #[arg(long = "ask-guard-model", env = "EMBERHUT_ASK_GUARD_MODEL")]
    pub ask_guard_model: Option<String>,
}

/// What recall indexes and searches with besides the server's own records.
#[derive(Clone, Debug, Default, Args)]
pub struct RecallConfig {
    /// An embedding model's OpenAI-style endpoint (`http://127.0.0.1:8795`, serving
    /// `POST /v1/embeddings`), for recall's search by meaning. None: recall searches by words.
    #[arg(long = "embed-url", env = "EMBERHUT_EMBED_URL", value_name = "URL")]
    pub embed_url: Option<String>,
    /// The model to ask the endpoint for, when it serves more than one.
    #[arg(
        long = "embed-model",
        env = "EMBERHUT_EMBED_MODEL",
        value_name = "NAME"
    )]
    pub embed_model: Option<String>,
    /// A folder of agent session transcripts (`.jsonl` files, in a folder per project) for
    /// recall to index. None: none are indexed.
    #[arg(long = "transcripts", env = "EMBERHUT_TRANSCRIPTS", value_name = "DIR")]
    pub transcripts: Option<PathBuf>,
}

/// A runner's machine in words: `NAME=WORDS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerName {
    /// The runner's name.
    pub name: String,
    /// Its machine, in words.
    pub words: String,
}

impl FromStr for RunnerName {
    type Err = InvalidRunnerName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (name, words) = text.split_once('=').ok_or(InvalidRunnerName)?;
        let (name, words) = (name.trim(), words.trim());
        if name.is_empty() || words.is_empty() {
            return Err(InvalidRunnerName);
        }
        Ok(Self {
            name: name.to_owned(),
            words: words.to_owned(),
        })
    }
}

/// Not a `NAME=WORDS` pair.
#[derive(Debug, thiserror::Error)]
#[error("not NAME=WORDS")]
pub struct InvalidRunnerName;

/// The language models the side apps talk through, each an OpenAI-style chat completions
/// endpoint (`https://gateway.example/v1`). A key, when the endpoint needs one, is a file in the
/// secrets directory ([`PARTY_MODEL_KEY`], [`DARK_MODEL_KEY`], [`DARK_LOCAL_KEY`]), read at every
/// call. Without an endpoint, that app answers that no model is set up.
#[derive(Clone, Debug, Default, Args)]
pub struct SideModelsConfig {
    /// Where parties' personas talk (the persona's model id is sent as it is).
    #[arg(
        long = "party-model-url",
        env = "EMBERHUT_PARTY_MODEL_URL",
        value_name = "URL"
    )]
    pub party_url: Option<String>,
    /// The off-the-record room's model endpoint.
    #[arg(
        long = "dark-model-url",
        env = "EMBERHUT_DARK_MODEL_URL",
        value_name = "URL"
    )]
    pub dark_url: Option<String>,
    /// The model id asked for there.
    #[arg(long = "dark-model", env = "EMBERHUT_DARK_MODEL", value_name = "ID")]
    pub dark_model: Option<String>,
    /// Its name, as the room shows it.
    #[arg(
        long = "dark-model-label",
        env = "EMBERHUT_DARK_MODEL_LABEL",
        default_value = "the cloud model"
    )]
    pub dark_label: String,
    /// A local model the off-the-record room tries first (a llama-server on the private
    /// network, health-checked before each reply); the endpoint above answers when it is off.
    #[arg(
        long = "dark-local-url",
        env = "EMBERHUT_DARK_LOCAL_URL",
        value_name = "URL"
    )]
    pub dark_local_url: Option<String>,
    /// The local model's name, as the room shows it.
    #[arg(
        long = "dark-local-label",
        env = "EMBERHUT_DARK_LOCAL_LABEL",
        default_value = "the local model"
    )]
    pub dark_local_label: String,
}

/// The parties' model key, in the secrets directory.
pub const PARTY_MODEL_KEY: &str = "party-model-key";
/// The off-the-record room's model key, in the secrets directory.
pub const DARK_MODEL_KEY: &str = "dark-model-key";
/// The off-the-record room's local model key, in the secrets directory.
pub const DARK_LOCAL_KEY: &str = "dark-local-key";

/// One machine's coding-thread app: the name the server gives the machine, and the app's MCP
/// endpoint there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodingAppMachine {
    /// The machine's name: letters, digits, `-` and `_`.
    pub name: String,
    /// The app's MCP endpoint, `http://` or `https://`.
    pub url: String,
}

impl FromStr for CodingAppMachine {
    type Err = InvalidCodingApp;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (name, url) = text.split_once('=').ok_or(InvalidCodingApp)?;
        let (name, url) = (name.trim(), url.trim());
        let named = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let web = url.starts_with("http://") || url.starts_with("https://");
        if named && web {
            Ok(Self {
                name: name.to_owned(),
                url: url.to_owned(),
            })
        } else {
            Err(InvalidCodingApp)
        }
    }
}

/// Not a `NAME=URL` pair.
#[derive(Debug, thiserror::Error)]
#[error("not NAME=URL (a name of letters, digits, - and _; an http:// or https:// URL)")]
pub struct InvalidCodingApp;

/// Where the helper on the owner's Mac is, if there is one: without it, the Mac shows as
/// offline and "Lock now" says no helper is set up.
#[derive(Clone, Debug, Default, Args)]
pub struct MacHelperConfig {
    /// The Mac helper's address, `http://<the Mac's private address>:<port>`.
    #[arg(
        id = "mac_helper_url",
        long = "mac-helper-url",
        env = "EMBERHUT_MAC_HELPER_URL",
        value_name = "URL",
        requires = "mac_helper_token_file",
        value_parser = helper_url
    )]
    pub url: Option<reqwest::Url>,
    /// A file holding the token the server sends the Mac helper as its bearer (read at
    /// start-up; surrounding whitespace is ignored).
    #[arg(
        id = "mac_helper_token_file",
        long = "mac-helper-token-file",
        env = "EMBERHUT_MAC_HELPER_TOKEN_FILE",
        value_name = "PATH",
        requires = "mac_helper_url"
    )]
    pub token_file: Option<PathBuf>,
}

/// Where the owner's messages come from and go to. Every token is a file in the secrets
/// directory (`slack-<workspace>-user`, `gmail-client-id`, `gmail-client-secret`,
/// `gmail-<account>-refresh`, `openai-api-key`, `whatsapp-token`), never a value here.
#[derive(Clone, Debug, Args)]
pub struct MessagingConfig {
    /// Slack's Web API.
    #[arg(
        long = "slack-api",
        env = "EMBERHUT_SLACK_API",
        default_value = "https://slack.com/api"
    )]
    pub slack_api: String,
    /// One stand-in address for Google's token, consent and Gmail endpoints (`<it>/token`,
    /// `<it>/auth`, `<it>/gmail/v1/...`); left out, Google's own.
    #[arg(long = "gmail-api", env = "EMBERHUT_GMAIL_API")]
    pub gmail_api: Option<String>,
    /// Where Google sends the owner back after signing in, as registered with the OAuth
    /// client; left out, `<public url>/gmail/callback`.
    #[arg(long = "gmail-callback", env = "EMBERHUT_GMAIL_CALLBACK")]
    pub gmail_callback: Option<String>,
    /// The longest voice note transcribed, in seconds.
    #[arg(
        long = "voice-max-seconds",
        env = "EMBERHUT_VOICE_MAX_SECONDS",
        default_value = "600"
    )]
    pub voice_max_seconds: f64,
    /// The daemon that holds the assistant's own WhatsApp number (`http://127.0.0.1:8806`);
    /// left out, there is none.
    #[arg(long = "whatsapp-url", env = "EMBERHUT_WHATSAPP_URL")]
    pub whatsapp_url: Option<String>,
    /// The name a WhatsApp group message starts with to address the assistant.
    #[arg(
        long = "whatsapp-name",
        env = "EMBERHUT_WHATSAPP_NAME",
        default_value = "emberhut"
    )]
    pub whatsapp_name: String,
    /// The owner's own phone number, so the owner is named in the WhatsApp chats.
    #[arg(long = "whatsapp-owner", env = "EMBERHUT_WHATSAPP_OWNER")]
    pub whatsapp_owner: Option<String>,
    /// The assistant's WhatsApp number, to link.
    #[arg(long = "whatsapp-phone", env = "EMBERHUT_WHATSAPP_PHONE")]
    pub whatsapp_phone: Option<String>,
    /// The owner's name: a group chat's message that mentions it wakes the assistant at once.
    #[arg(long = "owner-name", env = "EMBERHUT_OWNER_NAME")]
    pub owner_name: Option<String>,
}

/// Why a helper URL was not taken.
#[derive(Debug, thiserror::Error)]
enum InvalidHelperUrl {
    /// Not a URL.
    #[error("not a URL: {0}")]
    Malformed(String),
    /// Not `http` (the helper is reached on a private network, as the reference server does).
    #[error("only an http:// URL is supported")]
    NotHttp,
}

/// An `http://` URL.
fn helper_url(text: &str) -> Result<reqwest::Url, InvalidHelperUrl> {
    let url = reqwest::Url::parse(text)
        .map_err(|error| InvalidHelperUrl::Malformed(error.to_string()))?;
    if url.scheme() == "http" && url.has_host() {
        Ok(url)
    } else {
        Err(InvalidHelperUrl::NotHttp)
    }
}

impl ServeConfig {
    /// Refuses a configuration the server must not serve: report mode with a public URL,
    /// which would let anyone on the internet in (ADR 0007: report mode stays private).
    ///
    /// # Errors
    ///
    /// [`InvalidConfig::ReportInPublic`].
    pub fn check(&self) -> Result<(), InvalidConfig> {
        if self.auth_mode == AuthMode::Report && self.public_url.is_some() {
            return Err(InvalidConfig::ReportInPublic);
        }
        Ok(())
    }

    /// The token and key files this configuration names (the runners', the coding-thread
    /// app's, the Mac helper's, and the runners' MCP servers file, which may carry keys), and
    /// the agent accounts' configuration directories (`ID=DIR`, where the CLI keeps its
    /// login): never sent to a runner's machine nor shown in the chat.
    #[must_use]
    pub fn private_files(&self) -> Vec<PathBuf> {
        [
            &self.runner_tokens,
            &self.coding_app_tokens,
            &self.mac.token_file,
            &self.runner_mcp,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .chain(self.agent.accounts.iter().filter_map(|account| {
            let (_, dir) = account.split_once('=')?;
            Some(PathBuf::from(dir.trim())).filter(|dir| !dir.as_os_str().is_empty())
        }))
        .collect()
    }

    /// The address links point at: [`Self::public_url`], or `http://` and the listen address.
    #[must_use]
    pub fn link_base(&self) -> String {
        self.public_url.as_ref().map_or_else(
            || format!("http://{}", self.listen),
            |url| url.as_str().to_owned(),
        )
    }
}

/// What iPhone pushes (APNs) need besides the key file (`apns-key.p8` in the secrets
/// directory). All three, or iPhones get no pushes.
#[derive(Clone, Debug, Default, Args)]
pub struct ApnsSettings {
    /// The APNs key's id, from Apple's developer site.
    #[arg(long = "apns-key-id", env = "EMBERHUT_APNS_KEY_ID")]
    pub key_id: Option<String>,
    /// The Apple developer team's id the key belongs to.
    #[arg(long = "apns-team-id", env = "EMBERHUT_APNS_TEAM_ID")]
    pub team_id: Option<String>,
    /// The iPhone app's bundle id, which every alert names.
    #[arg(long = "apns-topic", env = "EMBERHUT_APNS_TOPIC")]
    pub topic: Option<String>,
}

/// An `http://` or `https://` address with no query or fragment, kept without a final `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicUrl(String);

impl PublicUrl {
    /// The address, without a final `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PublicUrl {
    type Err = InvalidPublicUrl;

    fn from_str(url: &str) -> Result<Self, Self::Err> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .ok_or(InvalidPublicUrl)?;
        let host = rest.split('/').next().unwrap_or_default();
        let plain = !host.is_empty()
            && !url.contains(['?', '#'])
            && !url.chars().any(|c| c.is_whitespace() || c.is_control());
        if plain {
            Ok(Self(url.trim_end_matches('/').to_owned()))
        } else {
            Err(InvalidPublicUrl)
        }
    }
}

/// A configuration the server refuses to serve.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidConfig {
    /// Report mode lets requests without a session through, so it stays on a private network.
    #[error("report mode lets anyone in; it can't be used with a relay or a public URL")]
    ReportInPublic,
}

/// The text is not a public address.
#[derive(Debug, thiserror::Error)]
#[error("not an http:// or https:// address with a host and no query")]
pub struct InvalidPublicUrl;

/// Where the database is.
#[derive(Clone, Debug, Args)]
pub struct DatabaseConfig {
    /// The SQLite database, made if it is not there.
    #[arg(
        long = "database-url",
        env = "EMBERHUT_DATABASE_URL",
        default_value = "sqlite://emberhut.db"
    )]
    pub url: DatabaseUrl,
}

/// Where the machine's key is (ADR 0007): the one secret the server keeps that isn't a hash.
#[derive(Clone, Debug, Default, Args)]
pub struct MachineKeyConfig {
    /// The machine's Ed25519 key file, made on first start (for the server's user only) and
    /// never replaced. Left out: `machine.key` beside the database. A machine whose key is
    /// lost is a new machine: its devices pair again.
    #[arg(
        long = "machine-key",
        env = "EMBERHUT_MACHINE_KEY",
        value_name = "FILE"
    )]
    pub file: Option<PathBuf>,
}

impl MachineKeyConfig {
    /// The key file: the one configured, or `machine.key` beside `database`.
    #[must_use]
    pub fn path(&self, database: &DatabaseConfig) -> PathBuf {
        self.beside(database.url.path())
    }

    /// The key file: the one configured, or `machine.key` beside the database file `database`.
    #[must_use]
    pub fn beside(&self, database: &std::path::Path) -> PathBuf {
        self.file.clone().unwrap_or_else(|| {
            database
                .parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .join(crate::services::transfer::MACHINE_KEY)
        })
    }
}

/// What the machine is called where it is listed: the account's list of machines.
#[derive(Clone, Debug, Default, Args)]
pub struct MachineNameConfig {
    /// The machine's name: 1 to 60 characters, signed into its link. Left out: the host name.
    #[arg(
        long = "machine-name",
        env = "EMBERHUT_MACHINE_NAME",
        value_name = "NAME"
    )]
    pub name: Option<MachineName>,
}

/// `emberhut-server pair`: the database the code goes in, the machine's key for the link's id,
/// and the routes the link names, from the same settings as serving.
#[derive(Clone, Debug, Args)]
pub struct PairConfig {
    /// The database.
    #[command(flatten)]
    pub database: DatabaseConfig,
    /// The machine's key.
    #[command(flatten)]
    pub machine_key: MachineKeyConfig,
    /// The address the server listens on: a route in the link when other devices can reach
    /// it (not loopback, not every interface).
    #[arg(long, env = "EMBERHUT_LISTEN", default_value = "127.0.0.1:18990")]
    pub listen: SocketAddr,
    /// The address the person's devices reach this server at: a route in the link.
    #[arg(long = "public-url", env = "EMBERHUT_PUBLIC_URL")]
    pub public_url: Option<PublicUrl>,
    /// Print one line of JSON for a program to read (`code`, `expiresAt`, `machine`,
    /// `database`, `link`) instead of the code, its link and the QR code in words.
    #[arg(long)]
    pub json: bool,
}

/// `emberhut-server id`: the database the key is kept beside, and the key.
#[derive(Clone, Debug, Args)]
pub struct IdConfig {
    /// The database.
    #[command(flatten)]
    pub database: DatabaseConfig,
    /// The machine's key.
    #[command(flatten)]
    pub machine_key: MachineKeyConfig,
}

/// The emberhut.com account and its directory. Official builds set them when they are built (the
/// same variables, at build time); builds from source have none, and the account is off.
#[derive(Clone, Debug, Default, Args)]
pub struct AccountConfig {
    /// The account's sign-in issuer: its OIDC discovery document names the device
    /// authorization and token endpoints.
    #[arg(
        long = "account-issuer",
        env = "EMBERHUT_ACCOUNT_ISSUER",
        value_name = "URL",
        default_value = option_env!("EMBERHUT_ACCOUNT_ISSUER").filter(|value| !value.is_empty())
    )]
    pub issuer: Option<url::Url>,
    /// The account's OAuth client id for native apps and the command line (a public client,
    /// with no secret).
    #[arg(
        long = "account-client-id",
        env = "EMBERHUT_ACCOUNT_CLIENT_ID",
        value_name = "ID",
        default_value = option_env!("EMBERHUT_ACCOUNT_CLIENT_ID").filter(|value| !value.is_empty())
    )]
    pub client_id: Option<String>,
    /// The relay's domain, which the link and unlink statements name.
    #[arg(
        long = "relay-domain",
        env = "EMBERHUT_RELAY_DOMAIN",
        value_name = "DOMAIN",
        value_parser = RelayDomain::normalize,
        default_value = option_env!("EMBERHUT_RELAY_DOMAIN").filter(|value| !value.is_empty())
    )]
    pub relay_domain: Option<RelayDomain>,
    /// The account directory's address, before its `/v1`. Left out: `https://` and the relay's
    /// domain.
    #[arg(
        long = "directory-url",
        env = "EMBERHUT_DIRECTORY_URL",
        value_name = "URL",
        default_value = option_env!("EMBERHUT_DIRECTORY_URL").filter(|value| !value.is_empty())
    )]
    pub directory_url: Option<url::Url>,
}

/// `emberhut-server link`: the machine (its key and name) and the account it is linked to.
#[derive(Clone, Debug, Args)]
pub struct LinkConfig {
    /// The database.
    #[command(flatten)]
    pub database: DatabaseConfig,
    /// The machine's key.
    #[command(flatten)]
    pub machine_key: MachineKeyConfig,
    /// The machine's name.
    #[command(flatten)]
    pub machine_name: MachineNameConfig,
    /// The account.
    #[command(flatten)]
    pub account: AccountConfig,
}

/// `emberhut-server unlink`: the machine's key and the account's directory.
#[derive(Clone, Debug, Args)]
pub struct UnlinkConfig {
    /// The database.
    #[command(flatten)]
    pub database: DatabaseConfig,
    /// The machine's key.
    #[command(flatten)]
    pub machine_key: MachineKeyConfig,
    /// The account.
    #[command(flatten)]
    pub account: AccountConfig,
}

/// What happens to a request that needs a session and has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum AuthMode {
    /// It is refused with 401 (the default).
    #[default]
    Enforce,
    /// It is let through and logged (once an hour per address), to see what enforcing would
    /// refuse: a migration aid, only behind a network that already limits who can connect.
    /// The mode is read at start-up; changing it takes a restart.
    Report,
}

impl AuthMode {
    /// The mode as the contract names it, for the state's `auth.mode`.
    #[must_use]
    pub fn to_api(self) -> emberhut_api::auth::AuthMode {
        match self {
            Self::Enforce => emberhut_api::auth::AuthMode::Enforce,
            Self::Report => emberhut_api::auth::AuthMode::Report,
        }
    }
}

/// A checked log filter, in `tracing_subscriber`'s `EnvFilter` syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFilter(String);

impl LogFilter {
    /// The filter, ready for the subscriber.
    #[must_use]
    pub fn to_env_filter(&self) -> EnvFilter {
        // The text was parsed strictly when this value was made, so nothing is lost here.
        EnvFilter::builder().parse_lossy(&self.0)
    }
}

impl FromStr for LogFilter {
    type Err = InvalidLogFilter;

    fn from_str(filter: &str) -> Result<Self, Self::Err> {
        EnvFilter::try_new(filter).map_err(InvalidLogFilter)?;
        Ok(Self(filter.to_owned()))
    }
}

/// The text is not a log filter.
#[derive(Debug, thiserror::Error)]
#[error("not a log filter: {0}")]
pub struct InvalidLogFilter(ParseError);

/// The most seconds a duration setting takes: an hour.
const MAX_SECONDS: u64 = 3_600;

/// Why a duration setting was not taken.
#[derive(Debug, thiserror::Error)]
enum InvalidSeconds {
    /// Not a whole number.
    #[error("not a whole number of seconds")]
    NotANumber(#[source] ParseIntError),
    /// Zero, or more than an hour.
    #[error("{0} is not between 1 and {MAX_SECONDS} seconds")]
    OutOfRange(u64),
}

/// A whole number of seconds, from 1 to an hour.
fn seconds(text: &str) -> Result<Duration, InvalidSeconds> {
    let seconds: u64 = text.parse().map_err(InvalidSeconds::NotANumber)?;
    if (1..=MAX_SECONDS).contains(&seconds) {
        Ok(Duration::from_secs(seconds))
    } else {
        Err(InvalidSeconds::OutOfRange(seconds))
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("emberhut-server").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_command_serves_with_the_defaults() {
        let Command::Serve(config) = parse(&[]).unwrap().into_command() else {
            panic!("serving is the default");
        };
        assert_eq!(config.listen, "127.0.0.1:18990".parse().unwrap());
        assert_eq!(config.auth_mode, AuthMode::Enforce);
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.shutdown_grace, Duration::from_secs(10));
        assert_eq!(config.database.url.as_str(), "sqlite://emberhut.db");
        assert!(config.mac.url.is_none() && config.mac.token_file.is_none());
        assert!(config.coding_app.is_empty() && config.coding_app_tokens.is_none());
        assert_eq!(config.agent.permission_mode, "dontAsk");
        assert_eq!(
            config.voice.voice_person_labels,
            [PERSON_LABEL, NEUTRAL_PERSON_LABEL]
        );
        assert_eq!(config.voice.call_transcribe_minutes, 120);
        assert!(parse(&["--call-transcribe-minutes", "0"]).is_err());
    }

    #[test]
    fn the_person_labels_are_a_list() {
        let Command::Serve(config) = parse(&["--voice-person-label", "You:,Moi:"])
            .unwrap()
            .into_command()
        else {
            panic!("serving is the default");
        };
        assert_eq!(config.voice.voice_person_labels, ["You:", "Moi:"]);
    }

    #[test]
    fn coding_apps_are_name_url_pairs() {
        let Command::Serve(config) = parse(&[
            "--coding-app",
            "laptop=http://127.0.0.1:7000/mcp,desk=https://desk.example/mcp",
            "--coding-app-tokens",
            "tokens.txt",
        ])
        .unwrap()
        .into_command() else {
            panic!("serving is the default");
        };
        let names: Vec<&str> = config.coding_app.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["laptop", "desk"]);
        assert_eq!(config.coding_app[1].url, "https://desk.example/mcp");
        assert_eq!(
            config.coding_app_tokens.as_deref(),
            Some(std::path::Path::new("tokens.txt"))
        );
        for wrong in ["laptop", "=http://x", "lap top=http://x", "laptop=ftp://x"] {
            assert!(wrong.parse::<CodingAppMachine>().is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_configured_token_files_are_private() {
        let Command::Serve(config) = parse(&[
            "--runner-tokens",
            "/etc/emberhut/runners",
            "--coding-app-tokens",
            "/etc/emberhut/coding",
            "--runner-mcp",
            "/etc/emberhut/mcp.json",
            "--mac-helper-url",
            "http://mac.example:8793",
            "--mac-helper-token-file",
            "/etc/emberhut/mac",
            "--agent-accounts",
            "main,spare=/srv/agent/spare, other = /srv/agent/other ",
        ])
        .unwrap()
        .into_command() else {
            panic!("serving is the default");
        };
        assert_eq!(
            config.private_files(),
            [
                "/etc/emberhut/runners",
                "/etc/emberhut/coding",
                "/etc/emberhut/mac",
                "/etc/emberhut/mcp.json",
                "/srv/agent/spare",
                "/srv/agent/other"
            ]
            .map(PathBuf::from)
        );
        let Command::Serve(config) = parse(&[]).unwrap().into_command() else {
            panic!("serving is the default");
        };
        assert_eq!(config.private_files(), Vec::<PathBuf>::new());
    }

    #[test]
    fn the_mac_helper_takes_its_url_and_token_file_together() {
        let Command::Serve(config) = parse(&[
            "--mac-helper-url",
            "http://mac.example:8793",
            "--mac-helper-token-file",
            "/run/secrets/mac-helper",
        ])
        .unwrap()
        .into_command() else {
            panic!("serving is the default");
        };
        assert_eq!(config.mac.url.unwrap().as_str(), "http://mac.example:8793/");
        assert_eq!(
            config.mac.token_file.unwrap(),
            PathBuf::from("/run/secrets/mac-helper")
        );
        assert!(parse(&["--mac-helper-url", "http://mac.example:8793"]).is_err());
        assert!(parse(&["--mac-helper-token-file", "/run/secrets/mac-helper"]).is_err());
        assert!(
            parse(&[
                "--mac-helper-url",
                "ftp://mac.example",
                "--mac-helper-token-file",
                "/t",
            ])
            .is_err()
        );
    }

    #[test]
    fn flags_set_each_value() {
        let cli = parse(&[
            "--listen",
            "0.0.0.0:8080",
            "--auth-mode",
            "report",
            "--request-timeout",
            "5",
            "--database-url",
            "sqlite:///var/lib/emberhut/emberhut.db",
            "--log",
            "emberhut_server=debug,info",
        ])
        .unwrap();
        assert_eq!(cli.log, LogFilter("emberhut_server=debug,info".to_owned()));
        let Command::Serve(config) = cli.into_command() else {
            panic!("serving is the default");
        };
        assert_eq!(config.listen.port(), 8080);
        assert_eq!(config.auth_mode, AuthMode::Report);
        config.check().unwrap();
        assert_eq!(config.request_timeout, Duration::from_secs(5));
        assert_eq!(
            config.database.url.as_str(),
            "sqlite:///var/lib/emberhut/emberhut.db"
        );
    }

    #[test]
    fn pair_takes_the_database_the_key_and_the_routes() {
        let Command::Pair(pair) = parse(&[
            "pair",
            "--database-url",
            "sqlite:///srv/emberhut/other.db",
            "--public-url",
            "https://emberhut.example.com/",
        ])
        .unwrap()
        .into_command() else {
            panic!("pair was asked for");
        };
        assert_eq!(
            pair.database.url.as_str(),
            "sqlite:///srv/emberhut/other.db"
        );
        assert_eq!(pair.listen, "127.0.0.1:18990".parse().unwrap());
        assert_eq!(
            pair.public_url.as_ref().map(PublicUrl::as_str),
            Some("https://emberhut.example.com")
        );
        assert_eq!(
            pair.machine_key.path(&pair.database),
            PathBuf::from("/srv/emberhut/machine.key"),
            "beside the database"
        );
        assert!(parse(&["--listen", "0.0.0.0:1", "pair"]).is_err());
        assert!(!pair.json, "words unless asked");
        let Command::Pair(json) = parse(&["pair", "--json"]).unwrap().into_command() else {
            panic!("pair was asked for");
        };
        assert!(json.json);
    }

    #[test]
    fn the_machine_key_and_name_are_settings() {
        let Command::Id(id) = parse(&["id", "--machine-key", "/keys/this.key"])
            .unwrap()
            .into_command()
        else {
            panic!("id was asked for");
        };
        assert_eq!(
            id.machine_key.path(&id.database),
            PathBuf::from("/keys/this.key")
        );
        let Command::Serve(config) = parse(&["--machine-name", "Desk"]).unwrap().into_command()
        else {
            panic!("serving is the default");
        };
        assert_eq!(
            config.machine_name.name.as_ref().map(MachineName::as_str),
            Some("Desk")
        );
        assert!(parse(&["--machine-name", " padded "]).is_err());
    }

    #[test]
    fn link_takes_the_account_s_settings() {
        let Command::Link(link) = parse(&[
            "link",
            "--account-issuer",
            "https://accounts.example.com",
            "--account-client-id",
            "client-1",
            "--relay-domain",
            "Relay.Example",
        ])
        .unwrap()
        .into_command() else {
            panic!("link was asked for");
        };
        let account = link.account;
        assert_eq!(
            account.issuer.map(String::from).as_deref(),
            Some("https://accounts.example.com/")
        );
        assert_eq!(account.client_id.as_deref(), Some("client-1"));
        assert_eq!(
            account.relay_domain.as_ref().map(RelayDomain::as_str),
            Some("relay.example")
        );
        assert!(parse(&["unlink", "--relay-domain", "not a domain"]).is_err());
    }

    #[test]
    fn links_point_at_the_public_url_or_the_listen_address() {
        let Command::Serve(config) = parse(&["--listen", "127.0.0.1:9"]).unwrap().into_command()
        else {
            panic!("serving is the default");
        };
        assert_eq!(config.link_base(), "http://127.0.0.1:9");
        assert_eq!(config.secrets_dir, PathBuf::from("secrets"));
        assert_eq!(config.agent.anthropic_url, "https://api.anthropic.com");
        assert_eq!(config.apns.key_id, None);
        let Command::Serve(config) = parse(&["--public-url", "https://emberhut.example.com/"])
            .unwrap()
            .into_command()
        else {
            panic!("serving is the default");
        };
        assert_eq!(config.link_base(), "https://emberhut.example.com");
        for bad in [
            "emberhut.example.com",
            "ftp://x",
            "https://",
            "https://x/?a=1",
            "http://a b",
        ] {
            assert!(bad.parse::<PublicUrl>().is_err(), "{bad}");
        }
    }

    #[test]
    fn bad_values_are_refused_before_anything_starts() {
        assert!(parse(&["--listen", "nowhere"]).is_err());
        assert!(parse(&["--auth-mode", "maybe"]).is_err());
        assert!(parse(&["--request-timeout", "0"]).is_err());
        assert!(parse(&["--request-timeout", "3601"]).is_err());
        assert!(parse(&["--shutdown-grace", "soon"]).is_err());
        assert!(parse(&["--database-url", "postgres://localhost/emberhut"]).is_err());
        assert!(parse(&["--log", "emberhut_server=loud"]).is_err());
    }

    #[test]
    fn report_mode_with_a_public_url_is_refused() {
        let serve = |args: &[&str]| {
            let Command::Serve(config) = parse(args).unwrap().into_command() else {
                panic!("serving is the default");
            };
            config
        };
        let public = serve(&[
            "--auth-mode",
            "report",
            "--public-url",
            "https://emberhut.example.com",
        ]);
        assert!(matches!(public.check(), Err(InvalidConfig::ReportInPublic)));
        assert_eq!(
            public.check().unwrap_err().to_string(),
            "report mode lets anyone in; it can't be used with a relay or a public URL"
        );
        serve(&["--public-url", "https://emberhut.example.com"])
            .check()
            .unwrap();
        serve(&["--auth-mode", "report"]).check().unwrap();
    }
}
