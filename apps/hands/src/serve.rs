//! Starting the helper on the Mac: its options, its token, the HTTP server on its own thread
//! with the screen session's watchdog and the recordings' retries, and the app loop on the main
//! thread.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use emberhut_hands::config::{Config, Places};
use emberhut_hands::extension::ExtensionFolder;
use emberhut_hands::extension::id::EXTENSION_ID;
use emberhut_hands::hands::{Clock, Hands, SystemClock};
use emberhut_hands::http::{AppState, router};
use emberhut_hands::mac::{self, MacDesktop, ScreenAudio, SessionUploader};
use emberhut_hands::network::Callers;
use emberhut_hands::record::{AudioSource, Recorder, Server, Uploader};
use emberhut_hands::token::Token;
use tracing_subscriber::EnvFilter;

/// How often the screen session is checked for its time and the owner's touch.
const WATCH: Duration = Duration::from_millis(250);
/// How long after start calls left from before are sent again.
const FIRST_RESEND: Duration = Duration::from_secs(10);
/// How often calls not sent yet are tried again.
const RESEND: Duration = Duration::from_secs(300);

/// Runs the helper until it is stopped.
pub(crate) fn run() -> anyhow::Result<()> {
    let config = Config::parse();
    config.check()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let places = Places::under(&home);
    let token_file = config.token_file.clone().unwrap_or_else(|| places.token());
    let (token, made) = Token::load_or_create(&token_file)
        .with_context(|| format!("reading the token from {}", token_file.display()))?;
    if made {
        tracing::info!(
            file = %token_file.display(),
            "made a new token: the server's Mac helper token file must hold the same"
        );
    }
    let server = match (&config.server_url, &config.server_token_file) {
        (Some(url), Some(file)) => {
            let device = std::fs::read_to_string(file)
                .with_context(|| format!("reading the server token from {}", file.display()))?;
            Some(Server {
                url: url.clone(),
                token: device.trim().to_owned(),
            })
        }
        _ => None,
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let recorder = Recorder::new(
        Arc::new(ScreenAudio) as Arc<dyn AudioSource>,
        Arc::new(SessionUploader) as Arc<dyn Uploader>,
        server,
        places.calls(),
        Arc::clone(&clock),
    );
    let hands = Arc::new(Hands::new(Arc::new(MacDesktop::new(home)), clock, recorder));
    let stopping = Arc::downgrade(&hands);
    mac::on_stop(move || {
        if let Some(hands) = stopping.upgrade() {
            hands.owner_stopped();
        }
    });
    let losing = Arc::downgrade(&hands);
    hands.recorder().when_lost(move |number| {
        if let Some(hands) = losing.upgrade() {
            hands.capture_lost(number);
        }
    });
    let grants = mac::ask_for_grants();
    tracing::info!(
        accessibility = grants.accessibility,
        screen_recording = grants.screen_recording,
        "the Mac's grants (the owner turns each on in System Settings → Privacy & Security)"
    );
    let extension = ExtensionFolder::new(
        config
            .extension_dir
            .clone()
            .unwrap_or_else(|| places.extension()),
        EXTENSION_ID,
    );
    let state = AppState::new(
        Arc::clone(&hands) as _,
        token,
        Callers::for_listen(config.listen),
        extension,
    );
    let addresses = config.addresses();
    std::thread::Builder::new()
        .name("emberhut-hands-server".to_owned())
        .spawn(move || {
            if let Err(error) = serve(state, &hands, &addresses) {
                tracing::error!(error = format!("{error:#}"), "the helper stopped serving");
                // launchd starts it again.
                std::process::exit(1);
            }
        })
        .context("starting the server's thread")?;
    mac::run_app()?;
    Ok(())
}

/// Serves `state` on `addresses`, and runs the watchdog and the retries, until it fails.
fn serve(
    state: AppState,
    hands: &Arc<Hands<MacDesktop>>,
    addresses: &[SocketAddr],
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    runtime.block_on(async {
        let app = router(state);
        let mut servers = tokio::task::JoinSet::new();
        for address in addresses {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .with_context(|| format!("listening on {address}"))?;
            tracing::info!(%address, "serving");
            let app = app.clone();
            let service = app.into_make_service_with_connect_info::<SocketAddr>();
            servers.spawn(async move { axum::serve(listener, service).await });
        }
        let watcher = Arc::clone(hands);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(WATCH);
            loop {
                every.tick().await;
                let watcher = Arc::clone(&watcher);
                let _watched = tokio::task::spawn_blocking(move || watcher.watch()).await;
            }
        });
        let sender = Arc::clone(hands);
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_RESEND).await;
            let mut every = tokio::time::interval(RESEND);
            loop {
                every.tick().await;
                let sender = Arc::clone(&sender);
                let _sent = tokio::task::spawn_blocking(move || sender.recorder().resend()).await;
            }
        });
        match servers.join_next().await {
            Some(Ok(Ok(()))) | None => Ok(()),
            Some(Ok(Err(error))) => Err(anyhow::Error::new(error).context("serving")),
            Some(Err(error)) => Err(anyhow::Error::new(error).context("a server's task")),
        }
    })
}
