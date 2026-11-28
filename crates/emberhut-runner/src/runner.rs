//! One runner: its store, accounts, reporter, sessions, machine samples and API, put together and
//! started. Used by the `run` command and, with a stand-in agent and server, by the tests.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::accounts::{Accounts, Logins};
use crate::config::Config;
use crate::engine::Engine;
use crate::engine::claude::ClaudeCode;
use crate::files::Files;
use crate::http::{self, Api};
use crate::machine::{MachineInfo, Sampler, SessionsSummary};
use crate::report::{Link, Reporter};
use crate::sessions::engine::Sessions;
use crate::store::{Store, now};

/// How often the accounts' use is looked at, and the shared MCP servers asked for.
const LOOK_EVERY: Duration = Duration::from_mins(10);

/// What a test may put in place of the real thing.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// The agent (Claude Code's command line by default).
    pub engine: Option<Arc<dyn Engine>>,
    /// Look only at credential files for logins, never the macOS Keychain.
    pub credential_files: bool,
    /// Don't look at the accounts' use (no agent process for it).
    pub no_probe: bool,
    /// Don't sample the machine.
    pub no_samples: bool,
    /// Where files may come from besides the sessions' folders (the system's temporary folder by default).
    pub tmp: Option<std::path::PathBuf>,
}

/// Why the runner didn't start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartError {
    /// Its home can't be made.
    #[error("cannot make {0}: {1}")]
    Home(String, std::io::Error),
    /// Its database can't be opened.
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    /// The accounts' database can't be opened.
    #[error("the accounts' database: {0}")]
    Accounts(#[from] sea_orm::DbErr),
    /// The HTTP client can't be made.
    #[error(transparent)]
    Client(#[from] crate::client::ClientError),
    /// An address can't be listened on.
    #[error("cannot listen on {0}: {1}")]
    Listen(SocketAddr, std::io::Error),
}

/// A started runner.
#[derive(Debug)]
pub struct Runner {
    /// The settings.
    pub config: Arc<Config>,
    /// The store.
    pub store: Store,
    /// The accounts.
    pub accounts: Arc<Accounts>,
    /// The sessions.
    pub sessions: Arc<Sessions>,
    /// The reporter.
    pub reporter: Arc<Reporter>,
    /// The addresses it listens on (the port the system picked for port 0).
    pub addresses: Vec<SocketAddr>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Runner {
    /// Starts a runner with these settings.
    ///
    /// # Errors
    /// Its home, database, client or listen addresses can't be had.
    #[expect(
        clippy::too_many_lines,
        reason = "every part of the runner made and started in the order they need each other"
    )]
    pub async fn start(config: Config, o: Options) -> Result<Self, StartError> {
        let config = Arc::new(config);
        create_private_dir(&config.home)
            .map_err(|e| StartError::Home(config.home.display().to_string(), e))?;
        let store = Store::open(&config.home).await?;
        let engine: Arc<dyn Engine> = o
            .engine
            .clone()
            .unwrap_or_else(|| Arc::new(ClaudeCode::new(config.agent_command.clone())));
        let user = config.user.clone();
        let logins = Logins::new(
            user,
            config.secrets_dir(),
            cfg!(target_os = "macos") && !o.credential_files,
        );
        let accounts = Arc::new(Accounts::open(&config, logins, Arc::clone(&engine)).await?);
        let token_config = Arc::clone(&config);
        let link = Link {
            server: config.server.clone(),
            token: Arc::new(move || token_config.token()),
            name: config.name.clone(),
            http: crate::client::client()?,
        };
        let reporter = Reporter::new(store.clone(), link);
        let tmp = o.tmp.clone().unwrap_or_else(|| {
            if cfg!(windows) {
                std::env::temp_dir()
            } else {
                std::path::PathBuf::from("/tmp")
            }
        });
        let files = Arc::new(Files::new(
            store.clone(),
            config.home.clone(),
            config.name.clone(),
            config.max_file_bytes,
            tmp,
        ));
        let sessions = Sessions::new(
            store.clone(),
            Arc::clone(&accounts),
            Arc::clone(&reporter),
            Arc::clone(&files),
            Arc::clone(&config),
            engine,
        );
        // The first MCP answer before boot resumes what a restart cut off, so those turns open
        // with them too (15 s at most).
        if config.share_mcp {
            reporter.load_mcp().await;
        }
        let machine = Arc::new(Mutex::new(None::<Value>));
        let api = Arc::new(Api {
            config: Arc::clone(&config),
            store: store.clone(),
            sessions: Arc::clone(&sessions),
            accounts: Arc::clone(&accounts),
            files,
            machine: Arc::clone(&machine),
            started: now(),
        });
        let mut addresses = Vec::new();
        let mut tasks = Vec::new();
        for ip in &config.listen {
            let at = SocketAddr::new(*ip, config.port);
            let listener = tokio::net::TcpListener::bind(at)
                .await
                .map_err(|e| StartError::Listen(at, e))?;
            let bound = listener
                .local_addr()
                .map_err(|e| StartError::Listen(at, e))?;
            tracing::info!(
                "emberhut-runner {} ({}) on http://{bound}",
                crate::version(),
                config.name
            );
            addresses.push(bound);
            let app = http::router(Arc::clone(&api));
            tasks.push(tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::error!("the API on {bound} stopped: {e}");
                }
            }));
        }
        sessions.boot().await;
        tasks.push(tokio::spawn(Arc::clone(&reporter).run()));
        let r = Arc::clone(&reporter);
        tasks.push(tokio::spawn(async move {
            let mut every = tokio::time::interval(LOOK_EVERY);
            every.tick().await;
            loop {
                every.tick().await;
                r.load_mcp().await;
            }
        }));
        if !o.no_probe {
            let a = Arc::clone(&accounts);
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                loop {
                    a.probe_all().await;
                    tokio::time::sleep(LOOK_EVERY).await;
                }
            }));
        }
        if !o.no_samples {
            tasks.push(tokio::spawn(sample(
                Arc::clone(&config),
                Arc::clone(&sessions),
                Arc::clone(&reporter),
                machine,
            )));
        }
        Ok(Self {
            config,
            store,
            accounts,
            sessions,
            reporter,
            addresses,
            tasks,
        })
    }

    /// The port it listens on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.addresses
            .first()
            .map_or(self.config.port, SocketAddr::port)
    }

    /// Stops everything; running turns resume at the next start.
    pub async fn close(self) {
        for t in &self.tasks {
            t.abort();
        }
        self.sessions.shutdown().await;
    }
}

/// A machine sample every so often (20 s by default), kept for `GET /machine` and posted to the server.
async fn sample(
    config: Arc<Config>,
    sessions: Arc<Sessions>,
    reporter: Arc<Reporter>,
    latest: Arc<Mutex<Option<Value>>>,
) {
    let mut sampler = Sampler::new(MachineInfo {
        name: config.name.clone(),
    });
    let mut every = tokio::time::interval(config.machine_every);
    let mut failing = false;
    loop {
        every.tick().await;
        let s = sessions.summary().await;
        let to_u32 = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        let body = sampler
            .sample(SessionsSummary {
                running: to_u32(s.running),
                open: to_u32(s.open),
                queued: to_u32(s.queued),
                list: s.list,
            })
            .await;
        let body = serde_json::to_value(&body).unwrap_or(Value::Null);
        if let Ok(mut m) = latest.lock() {
            *m = Some(body.clone());
        }
        match reporter.sample(&body).await {
            Ok(()) if failing => {
                failing = false;
                tracing::info!("machine samples reach the server again");
            }
            Err(why) if !failing && why != "not paired" => {
                failing = true;
                tracing::warn!(
                    "machine sample not taken by the server ({why}); trying again every {} s",
                    config.machine_every.as_secs()
                );
            }
            _ => {}
        }
    }
}

fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}
