//! The server on a real socket: it answers, and it stops when asked.

use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::common::{REQUEST_TIMEOUT, TestApp};
use crate::fake_cli::{FakeCli, agent, config, init, is_user, result};

#[tokio::test]
async fn serves_on_a_socket_and_stops_when_asked() {
    let app = TestApp::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));

    let response = crate::common::http_client()
        .get(format!("http://{address}/nowhere"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(response.text().await.unwrap(), r#"{"error":"not found"}"#);

    stop.send(()).unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(5), server).await;
    finished.unwrap().unwrap().unwrap();
    assert!(
        app.state.stopping().is_cancelled(),
        "streams were told to end"
    );
    assert!(
        crate::common::http_client()
            .get(format!("http://{address}/nowhere"))
            .send()
            .await
            .is_err(),
        "nothing listens any more"
    );
}

#[tokio::test]
async fn the_pair_command_refuses_to_make_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("elsewhere.db").display());
    let config = pair_config(
        url.parse().unwrap(),
        "127.0.0.1:18990".parse().unwrap(),
        None,
    );
    let refused = emberhut_server::make_pairing_code(&config).await;
    assert!(
        matches!(
            refused,
            Err(emberhut_server::RunError::Database(
                emberhut_db::DbError::Missing(_)
            ))
        ),
        "{refused:?}"
    );
    assert!(!dir.path().join("elsewhere.db").exists());
    assert!(!dir.path().join("machine.key").exists(), "nor a key");
}

/// `emberhut-server pair`'s settings for the database at `url`.
fn pair_config(
    url: emberhut_db::DatabaseUrl,
    listen: std::net::SocketAddr,
    public_url: Option<emberhut_server::config::PublicUrl>,
) -> emberhut_server::config::PairConfig {
    emberhut_server::config::PairConfig {
        database: emberhut_server::config::DatabaseConfig { url },
        machine_key: emberhut_server::config::MachineKeyConfig::default(),
        listen,
        public_url,
        json: false,
    }
}

#[tokio::test]
async fn the_pair_command_makes_a_working_code() {
    let app = TestApp::new().await;
    let config = pair_config(
        app.url.clone(),
        "192.0.2.10:18990".parse().unwrap(),
        Some("https://emberhut.example.com".parse().unwrap()),
    );
    // A command never makes the key: the server does, when it first serves the database.
    let refused = emberhut_server::make_pairing_code(&config)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, emberhut_server::RunError::MachineKey(_)),
        "{refused:?}"
    );
    let key = app.serve_once().await;
    let made = emberhut_server::make_pairing_code(&config).await.unwrap();
    let shown = made.issued.code.to_string();
    assert_eq!(shown.len(), 9);
    assert_eq!(&shown[4..5], "-");
    assert_eq!(made.machine, key.machine_id());

    // `pair --json`: the same code, its expiry, the machine and the link, for a program.
    let json = made.to_json(&config.database.url);
    let expires = made.issued.expires_at.to_string();
    assert_eq!(
        json,
        serde_json::json!({
            "code": shown,
            "expiresAt": expires,
            "machine": key.machine_id().to_string(),
            "database": config.database.url.path().display().to_string(),
            "link": made.link.as_ref().unwrap().to_url(),
        })
    );
    assert!(
        expires.parse::<jiff::Timestamp>().is_ok(),
        "RFC 3339: {expires}"
    );

    // The link carries the same code, the machine's id (its key beside the database) and
    // the configured routes; the QR code is drawn from it.
    let link = made.link.as_ref().unwrap();
    assert_eq!(link.code(), shown);
    assert_eq!(
        link.routes(),
        ["http://192.0.2.10:18990", "https://emberhut.example.com"]
    );
    let id_config = emberhut_server::config::IdConfig {
        database: config.database.clone(),
        machine_key: emberhut_server::config::MachineKeyConfig::default(),
    };
    assert_eq!(emberhut_server::machine_id(&id_config).unwrap(), *link.id());
    assert_eq!(key.machine_id(), *link.id(), "the key the server made");
    let parsed = emberhut_identity::PairingLink::parse(&link.to_url()).unwrap();
    assert_eq!(&parsed, link);
    // Dark on light: the quiet zone (the first line) is light, so it is blank; in colour, each
    // line sets black on white itself.
    let plain = emberhut_server::terminal_qr(link, false).unwrap();
    assert!(
        plain.lines().count() > 10 && plain.contains('\u{2588}'),
        "{plain}"
    );
    assert!(plain.lines().next().unwrap().trim().is_empty(), "{plain}");
    let coloured = emberhut_server::terminal_qr(link, true).unwrap();
    assert!(
        coloured
            .lines()
            .all(|line| line.starts_with("\x1b[30;47m") && line.ends_with("\x1b[0m")),
        "{coloured}"
    );

    // A code made from the command line works in the running server.
    let session = app
        .state
        .pairing()
        .redeem(&shown, &crate::common::device("first-device"))
        .await
        .unwrap();
    assert!(
        app.state
            .auth()
            .authenticate(session.tokens.access.as_str())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_waits_for_a_running_turn_within_the_grace() {
    let app = TestApp::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    app.assistant
        .answers(&["Thinking"], Some("Answered before the end."));
    app.assistant.hold();
    app.state
        .talk()
        .talk(emberhut_core::RoomId::MAIN, "hello".to_owned(), None)
        .await
        .unwrap();
    while app.assistant.turns().is_empty() {
        tokio::task::yield_now().await;
    }
    stop.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!server.is_finished(), "a turn is still running");
    app.assistant.release();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let chat = app
        .state
        .state()
        .state(emberhut_core::RoomId::MAIN)
        .await
        .unwrap()
        .chat;
    assert_eq!(chat.last().unwrap().text, "Answered before the end.");
}

#[tokio::test]
async fn serve_refuses_report_mode_with_a_public_url_before_opening_anything() {
    use clap::Parser as _;
    use emberhut_server::config::{Cli, Command, InvalidConfig};

    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("emberhut.db");
    let url = format!("sqlite://{}", database.display());
    let cli = Cli::try_parse_from([
        "emberhut-server",
        "--auth-mode",
        "report",
        "--public-url",
        "https://emberhut.example.com",
        "--listen",
        "127.0.0.1:0",
        "--database-url",
        &url,
    ])
    .unwrap();
    let Command::Serve(config) = cli.into_command() else {
        panic!("serving is the default");
    };
    // Were the refusal skipped, the server would serve until the timeout.
    let refused = tokio::time::timeout(
        Duration::from_secs(10),
        emberhut_server::serve(&config, std::future::pending()),
    )
    .await
    .expect("serve returns at once");
    assert!(
        matches!(
            refused,
            Err(emberhut_server::RunError::Config(
                InvalidConfig::ReportInPublic
            ))
        ),
        "{refused:?}"
    );
    assert!(!database.exists(), "no database was made");
}

#[tokio::test]
async fn stopping_closes_the_agent_sessions_kept_between_turns() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-1"), result("Hi")]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    app.state
        .talk()
        .talk(emberhut_core::RoomId::MAIN, "hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 1);
    assert!(cli.stopped().is_empty(), "kept open between turns");
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(cli.stopped(), [0], "closed before the server returned");
}
