//! The hut for real: a built `emberhut-server` started through [`LocalHut::system`], a code
//! from its `pair --json`, the app's `POST /api/auth/pair` with it, and the hut stopped.
//!
//! Ignored by default: it runs a real server. It refuses a `HOME` outside the temporary
//! folder, so it never writes the user's own files; run it with an empty environment:
//!
//! ```text
//! cargo build -p emberhut-server
//! cargo nextest run -p emberhut-desktop --no-run
//! HOME=$(mktemp -d) && env -i HOME="$HOME" PATH=/usr/bin:/bin \
//!   XDG_DATA_HOME="$HOME/data" XDG_STATE_HOME="$HOME/state" XDG_CONFIG_HOME="$HOME/config" \
//!   EMBERHUT_SERVER_BIN="$CARGO_TARGET_DIR/debug/emberhut-server" \
//!   <the emberhut-desktop test binary> --ignored --exact \
//!   hut::real_tests::a_real_hut_starts_pairs_this_app_and_stops
//! ```

use std::path::PathBuf;
use std::time::Duration;

use super::{HutOptions, LocalHut, net};

/// The ports this test must never use: the default hut's and another local service's.
const NEVER: [u16; 2] = [super::DEFAULT_PORT, 8791];

#[test]
#[ignore = "runs a real emberhut-server: see the module for how"]
fn a_real_hut_starts_pairs_this_app_and_stops() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let temporary = std::env::temp_dir();
    assert!(
        home.starts_with(&temporary) && home != temporary,
        "HOME must be a folder in {}, not {}",
        temporary.display(),
        home.display()
    );
    let port = (0..20)
        .map(|_| net::any_free_port().unwrap())
        .find(|port| !NEVER.contains(port))
        .unwrap();
    let options = HutOptions {
        port,
        more_ports: 5,
        ..HutOptions::default()
    };
    let hut = LocalHut::system(options).unwrap();
    assert!(
        hut.available(),
        "EMBERHUT_SERVER_BIN names the built server"
    );

    let url = hut.start_now().unwrap();
    assert_eq!(url.as_str(), format!("http://127.0.0.1:{port}"));
    let data = home.join("data/emberhut/hut");
    assert_eq!(crate::private_files::mode(&data), 0o700);
    for file in ["emberhut.db", "machine.key", "hut.json"] {
        assert!(data.join(file).is_file(), "{file} in {}", data.display());
        assert_eq!(
            crate::private_files::mode(&data.join(file)),
            0o600,
            "{file}"
        );
    }

    // Started once: a second start finds it lit.
    assert_eq!(hut.start_now().unwrap(), url);
    let status = hut.status_now();
    assert_eq!(status.url.as_ref(), Some(&url));

    // The code `pair` made pairs this app through the normal route, once.
    let code = hut.pair_code_now().unwrap();
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let pair = |code: &str| {
        let body = serde_json::json!({ "code": code, "name": "This computer" });
        let mut answer = agent
            .post(url.join("/api/auth/pair"))
            .header("Content-Type", "application/json")
            .send(serde_json::to_vec(&body).unwrap().as_slice())
            .unwrap();
        let status = answer.status().as_u16();
        let body: serde_json::Value =
            serde_json::from_slice(&answer.body_mut().read_to_vec().unwrap()).unwrap();
        (status, body)
    };
    let (status, paired) = pair(&code);
    assert_eq!(status, 200, "{paired}");
    assert!(paired["access"].is_string(), "{paired}");
    assert!(paired["refresh"].is_string(), "{paired}");
    let (again, refused) = pair(&code);
    assert_ne!(again, 200, "a code works once: {refused}");

    hut.stop_now().unwrap();
    assert_eq!(hut.status_now().url, None);
    assert!(
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "the port is free again"
    );
}
