//! The hut against a fake spawner and a scripted loopback, with its folders in a temporary
//! home: what it runs, writes and answers. [`Setup`] is the fake computer the setup's tests use
//! too.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use emberhut_identity::{MachineId, SecretKey};
use gpui_kit::TestAppContext;

pub(crate) use super::paths::Os;
use super::paths::{HutPaths, Place, SERVICE_ENV, database_url};
use super::process::{Ran, Run, Running, Spawner};
use super::service::{KeepLit, Serve, launch_agent, systemd_unit};
use super::state::HutState;
use super::{Heard, HutError, HutNet, HutOptions, LocalHut, pair};
use crate::config::ServerUrl;

/// Who listens on a port of the fake loopback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listener {
    /// This computer's hut: the app's child `pid`, or a service's (`None`).
    Hut(Option<u32>),
    /// Another emberhut server, with a key of its own.
    OtherHut,
    /// Something that isn't an emberhut server.
    Stranger,
}

/// The fake computer: its ports, its processes, and every program run.
struct World {
    machine: MachineId,
    data: PathBuf,
    ports: BTreeMap<u16, Listener>,
    /// Every program run to its end, in order.
    ran: Vec<Run>,
    /// Every program left running, in order.
    spawned: Vec<Run>,
    /// The processes asked to stop.
    terminated: Vec<u32>,
    dead: BTreeSet<u32>,
    next_pid: u32,
    /// Whether a server started answers (else it never does).
    answers: bool,
    /// Whether a server started exits at once.
    exits: bool,
    /// What `pair` gives.
    pair: Ran,
    /// The launch agent's plist or the unit file, which a start of the service reads.
    managed_file: Option<String>,
}

impl World {
    /// Lights this computer's hut on the port `args` name, as `pid` (a child) or a service.
    fn light(&mut self, args: &[String], pid: Option<u32>) {
        if !self.answers {
            return;
        }
        let Some(port) = port_after_listen(args) else {
            return;
        };
        std::fs::write(self.data.join("machine.key"), "key").unwrap();
        self.ports.insert(port, Listener::Hut(pid));
    }

    /// Puts out the hut a service runs.
    fn put_out_service(&mut self) {
        self.ports
            .retain(|_, listener| *listener != Listener::Hut(None));
    }

    /// The programs run (to their end or left running), each as its program's file name and
    /// its arguments, in order.
    fn commands(&self) -> Vec<String> {
        self.ran
            .iter()
            .map(|run| {
                let name = run.program.file_name().unwrap().to_string_lossy();
                std::iter::once(name.into_owned())
                    .chain(run.words())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    /// [`World::commands`] of the service managers only.
    fn managed(&self) -> Vec<String> {
        self.commands()
            .into_iter()
            .filter(|command| command.starts_with("launchctl") || command.starts_with("systemctl"))
            .collect()
    }
}

/// The port of the `--listen 127.0.0.1:<port>` in `args`, or of a `127.0.0.1:<port>` in a
/// service file's text.
fn port_after_listen(args: &[String]) -> Option<u16> {
    args.iter()
        .find_map(|arg| arg.split("127.0.0.1:").nth(1))
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
}

type Shared = Arc<Mutex<World>>;

struct FakeSpawner(Shared);

struct FakeChild {
    pid: u32,
    world: Shared,
}

impl Running for FakeChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn exited(&mut self) -> Option<String> {
        let world = self.world.lock().unwrap();
        (world.exits || world.dead.contains(&self.pid)).then(|| "exit status: 1".to_owned())
    }

    fn kill(&mut self) {
        self.world.lock().unwrap().dead.insert(self.pid);
    }
}

impl Spawner for FakeSpawner {
    fn spawn(&self, run: &Run) -> io::Result<Box<dyn Running>> {
        let mut world = self.0.lock().unwrap();
        world.next_pid += 1;
        let pid = world.next_pid;
        world.spawned.push(run.clone());
        if !world.exits {
            world.light(&run.words(), Some(pid));
        }
        Ok(Box::new(FakeChild {
            pid,
            world: Arc::clone(&self.0),
        }))
    }

    fn run(&self, run: &Run) -> io::Result<Ran> {
        let mut world = self.0.lock().unwrap();
        world.ran.push(run.clone());
        let words = run.words();
        let ok = |stdout: &str| Ran {
            ok: true,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        };
        let file_text = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        let ran = match words.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["--version"] => ok("emberhut-server 0.1.0\n"),
            ["id", ..] => ok(&format!("{}\n", world.machine)),
            ["pair", ..] => world.pair.clone(),
            ["bootout", _] | ["--user", "stop", _] => {
                world.put_out_service();
                ok("")
            }
            ["kickstart", _] | ["--user", "restart", _] => {
                let text = world
                    .managed_file
                    .clone()
                    .map_or_else(String::new, |path| file_text(&path));
                world.put_out_service();
                world.light(&[text], None);
                ok("")
            }
            _ => ok(""),
        };
        Ok(ran)
    }

    fn terminate(&self, pid: u32) -> io::Result<()> {
        let mut world = self.0.lock().unwrap();
        world.terminated.push(pid);
        world.dead.insert(pid);
        world
            .ports
            .retain(|_, listener| *listener != Listener::Hut(Some(pid)));
        Ok(())
    }
}

struct FakeNet(Shared);

impl HutNet for FakeNet {
    fn health(&self, port: u16) -> Heard {
        match self.0.lock().unwrap().ports.get(&port) {
            None => Heard::Nothing,
            Some(Listener::Hut(_) | Listener::OtherHut) => Heard::Emberhut,
            Some(Listener::Stranger) => Heard::Other,
        }
    }

    fn proves(&self, port: u16, machine: &MachineId) -> bool {
        let world = self.0.lock().unwrap();
        matches!(world.ports.get(&port), Some(Listener::Hut(_))) && *machine == world.machine
    }
}

/// A temporary computer: its home, the app with the server beside it, `claude` on the
/// `PATH`, and the fake world.
pub(crate) struct Setup {
    dir: tempfile::TempDir,
    os: Os,
    world: Shared,
}

/// Writes an executable file at `path`.
fn executable(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Setup {
    pub(crate) fn new(os: Os) -> Self {
        let dir = tempfile::tempdir().unwrap();
        executable(&dir.path().join("app/emberhut-server"), "server");
        executable(&dir.path().join("bin/claude"), "agent");
        let setup = Self {
            os,
            world: Arc::new(Mutex::new(World {
                machine: SecretKey::generate().unwrap().machine_id(),
                data: PathBuf::new(),
                ports: BTreeMap::new(),
                ran: Vec::new(),
                spawned: Vec::new(),
                terminated: Vec::new(),
                dead: BTreeSet::new(),
                next_pid: 100,
                answers: true,
                exits: false,
                pair: Ran {
                    ok: true,
                    stdout: "{\"code\":\"ABCD-EFGH\",\"expiresAt\":\"2026-11-27T12:10:00Z\"}\n"
                        .to_owned(),
                    stderr: String::new(),
                },
                managed_file: None,
            })),
            dir,
        };
        let paths = setup.paths();
        let managed = match paths.manager() {
            super::Manager::Launchd { plist, .. } => Some(plist.display().to_string()),
            super::Manager::Systemd { unit } => Some(unit.display().to_string()),
            super::Manager::None => None,
        };
        let mut world = setup.world.lock().unwrap();
        world.data = paths.data().to_path_buf();
        world.managed_file = managed;
        drop(world);
        setup
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn paths_with(&self, exe: PathBuf, vars: BTreeMap<&'static str, OsString>) -> HutPaths {
        HutPaths::new(Place {
            os: self.os,
            home: self.home(),
            exe: Some(exe),
            uid: Some(501),
            var: Box::new(move |name: &str| vars.get(name).cloned()),
        })
    }

    fn paths(&self) -> HutPaths {
        let vars = BTreeMap::from([("PATH", self.dir.path().join("bin").into_os_string())]);
        self.paths_with(self.dir.path().join("app/emberhut-desktop"), vars)
    }

    fn options() -> HutOptions {
        HutOptions {
            start_wait: Duration::from_millis(300),
            stop_wait: Duration::from_millis(300),
            poll: Duration::from_millis(10),
            ..HutOptions::default()
        }
    }

    /// A run of the app: a new hut over the same computer.
    pub(crate) fn hut(&self) -> LocalHut {
        LocalHut::new(
            self.paths(),
            Arc::new(FakeSpawner(Arc::clone(&self.world))),
            Arc::new(FakeNet(Arc::clone(&self.world))),
            Self::options(),
        )
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    /// From now on a server started exits at once.
    pub(crate) fn exits_at_start(&self) {
        self.world().exits = true;
    }

    fn state(&self) -> HutState {
        HutState::read(&self.paths().state())
    }

    fn data(&self) -> PathBuf {
        self.paths().data().to_path_buf()
    }
}

fn url(port: u16) -> ServerUrl {
    ServerUrl::loopback(port)
}

#[test]
fn the_server_is_found_in_the_bundle_then_beside_the_app_then_where_the_variable_says() {
    let setup = Setup::new(Os::Mac);
    let root = setup.dir.path();
    let none = BTreeMap::new;
    // In a bundle, the app's executable and the server are both in Contents/MacOS.
    let bundle = root.join("Emberhut.app/Contents/MacOS");
    executable(&bundle.join("emberhut-server"), "bundled");
    let paths = setup.paths_with(bundle.join("emberhut-desktop"), none());
    assert_eq!(paths.server(), Some(bundle.join("emberhut-server")));

    // A build from source: beside the app's binary.
    let paths = setup.paths_with(root.join("app/emberhut-desktop"), none());
    assert_eq!(paths.server(), Some(root.join("app/emberhut-server")));

    // Neither: the variable, and only when it names a program.
    let named = root.join("elsewhere/emberhut-server");
    let vars = || BTreeMap::from([("EMBERHUT_SERVER_BIN", named.clone().into_os_string())]);
    let paths = setup.paths_with(root.join("nowhere/emberhut-desktop"), vars());
    assert_eq!(paths.server(), None, "nothing there yet");
    std::fs::create_dir_all(named.parent().unwrap()).unwrap();
    std::fs::write(&named, "not executable").unwrap();
    assert_eq!(paths.server(), None, "a file it can't run");
    std::fs::set_permissions(&named, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(paths.server(), Some(named.clone()));

    // Beside the app wins over the variable.
    let paths = setup.paths_with(root.join("app/emberhut-desktop"), vars());
    assert_eq!(paths.server(), Some(root.join("app/emberhut-server")));
}

#[test]
fn without_a_server_the_hut_is_unavailable_and_says_so() {
    let setup = Setup::new(Os::Linux);
    std::fs::remove_file(setup.dir.path().join("app/emberhut-server")).unwrap();
    let hut = setup.hut();
    assert!(!hut.available());
    let error = hut.start_now().unwrap_err();
    assert!(matches!(error, HutError::NoServer), "{error:?}");
    assert!(error.to_string().contains("EMBERHUT_SERVER_BIN"), "{error}");
    assert_eq!(setup.world().spawned, Vec::<Run>::new());
}

#[test]
fn the_folders_are_the_systems_own() {
    let setup = Setup::new(Os::Mac);
    let paths = setup.paths();
    let home = setup.home();
    assert_eq!(
        paths.data(),
        home.join("Library/Application Support/Emberhut/Hut")
    );
    assert_eq!(paths.log(), home.join("Library/Logs/Emberhut/hut.log"));
    assert_eq!(
        *paths.manager(),
        super::Manager::Launchd {
            plist: home.join("Library/LaunchAgents/com.emberhut.hut.plist"),
            uid: 501,
        }
    );

    let setup = Setup::new(Os::Linux);
    let home = setup.home();
    let paths = setup.paths();
    assert_eq!(paths.data(), home.join(".local/share/emberhut/hut"));
    assert_eq!(paths.log(), home.join(".local/state/emberhut/hut.log"));
    assert_eq!(
        *paths.manager(),
        super::Manager::Systemd {
            unit: home.join(".config/systemd/user/emberhut-hut.service"),
        }
    );
    let xdg = BTreeMap::from([
        ("XDG_DATA_HOME", OsString::from("/xdg/data")),
        ("XDG_STATE_HOME", OsString::from("/xdg/state")),
        ("XDG_CONFIG_HOME", OsString::from("/xdg/config")),
    ]);
    let paths = setup.paths_with(setup.dir.path().join("app/emberhut-desktop"), xdg);
    assert_eq!(paths.data(), Path::new("/xdg/data/emberhut/hut"));
    assert_eq!(paths.log(), Path::new("/xdg/state/emberhut/hut.log"));
    assert_eq!(
        *paths.manager(),
        super::Manager::Systemd {
            unit: PathBuf::from("/xdg/config/systemd/user/emberhut-hut.service"),
        }
    );
}

#[test]
fn a_database_path_with_a_space_is_one_url() {
    assert_eq!(
        database_url(Path::new(
            "/Users/ada/Library/Application Support/Emberhut/Hut/emberhut.db"
        ))
        .as_deref(),
        Some("sqlite:///Users/ada/Library/Application%20Support/Emberhut/Hut/emberhut.db")
    );
    assert_eq!(
        database_url(Path::new("/a/100%?#/x.db")).as_deref(),
        Some("sqlite:///a/100%25%3F%23/x.db")
    );
}

#[test]
fn a_hut_not_running_is_started_on_the_first_port_with_its_files_the_users_own() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    assert!(hut.available());
    assert_eq!(hut.start_now().unwrap(), url(18990));

    let data = setup.data();
    let world = setup.world();
    assert_eq!(world.spawned.len(), 1);
    let serve = &world.spawned[0];
    assert_eq!(serve.program, setup.dir.path().join("app/emberhut-server"));
    assert_eq!(
        serve.words(),
        [
            "serve".to_owned(),
            "--listen".to_owned(),
            "127.0.0.1:18990".to_owned(),
            "--database-url".to_owned(),
            format!("sqlite://{}", data.join("emberhut.db").display()),
            "--secrets-dir".to_owned(),
            data.join("secrets").display().to_string(),
            "--agent-command".to_owned(),
            setup.dir.path().join("bin/claude").display().to_string(),
        ]
    );
    assert_eq!(serve.dir.as_deref(), Some(data.as_path()));
    assert_eq!(serve.log.as_deref(), Some(setup.paths().log()));
    assert_eq!(
        serve.env,
        [(
            "PATH".to_owned(),
            setup.dir.path().join("bin").into_os_string()
        )]
    );
    drop(world);

    for dir in [data.clone(), data.join("secrets")] {
        assert_eq!(crate::private_files::mode(&dir), 0o700, "{}", dir.display());
    }
    for file in ["hut.json", "emberhut.db"] {
        assert_eq!(
            crate::private_files::mode(&data.join(file)),
            0o600,
            "{file}"
        );
    }
    let state = setup.state();
    assert_eq!(state.port, Some(18990));
    assert_eq!(state.pid, Some(101));
    assert!(
        state
            .server
            .as_deref()
            .is_some_and(|build| build.starts_with("emberhut-server 0.1.0 (6 bytes")),
        "{state:?}"
    );
    assert_eq!(state.keep, KeepLit::default());
}

#[test]
fn a_hut_already_lit_is_used_as_it_is_by_this_run_and_the_next() {
    let setup = Setup::new(Os::Linux);
    let first = setup.hut();
    assert_eq!(first.start_now().unwrap(), url(18990));
    let hut = setup.hut();
    assert_eq!(hut.start_now().unwrap(), url(18990));
    assert_eq!(hut.start_now().unwrap(), url(18990));
    let world = setup.world();
    assert_eq!(world.spawned.len(), 1, "started once");
    assert_eq!(world.terminated, Vec::<u32>::new());
    drop(world);
    assert_eq!(
        hut.status_now().url,
        Some(url(18990)),
        "it answers as this computer's hut"
    );
}

#[test]
fn a_port_taken_by_something_else_moves_the_hut_to_the_next_and_it_is_remembered() {
    let setup = Setup::new(Os::Linux);
    {
        let mut world = setup.world();
        world.ports.insert(18990, Listener::Stranger);
        world.ports.insert(18991, Listener::OtherHut);
    }
    let first = setup.hut();
    assert_eq!(first.start_now().unwrap(), url(18992));
    assert_eq!(setup.state().port, Some(18992));
    assert_eq!(
        setup.world().spawned[0].words()[2],
        "127.0.0.1:18992",
        "another emberhut server, with another key, isn't this hut either"
    );

    // The next run goes to the remembered port, even once the first is free again.
    setup.world().ports.remove(&18990);
    assert_eq!(setup.hut().start_now().unwrap(), url(18992));
    assert_eq!(setup.world().spawned.len(), 1);
}

#[test]
fn every_port_taken_is_said() {
    let setup = Setup::new(Os::Linux);
    {
        let mut world = setup.world();
        for port in 18990..=19010 {
            world.ports.insert(port, Listener::Stranger);
        }
    }
    let error = setup.hut().start_now().unwrap_err();
    assert!(
        matches!(
            error,
            HutError::NoPort {
                first: 18990,
                last: 19010
            }
        ),
        "{error:?}"
    );
    assert_eq!(setup.world().spawned, Vec::<Run>::new());
}

#[test]
fn a_hut_that_never_answers_times_out_and_names_its_log() {
    let setup = Setup::new(Os::Linux);
    setup.world().answers = false;
    let error = setup.hut().start_now().unwrap_err();
    assert!(matches!(error, HutError::Timeout { .. }), "{error:?}");
    let words = error.to_string();
    assert!(words.contains("didn't answer"), "{words}");
    assert!(
        words.contains(&setup.paths().log().display().to_string()),
        "{words}"
    );
}

#[test]
fn a_hut_that_stops_while_starting_is_said() {
    let setup = Setup::new(Os::Linux);
    setup.world().exits = true;
    let error = setup.hut().start_now().unwrap_err();
    assert!(matches!(error, HutError::Exited { .. }), "{error:?}");
    assert!(error.to_string().contains("exit status: 1"), "{error}");
}

#[test]
fn a_new_build_of_the_server_restarts_the_hut_once() {
    let setup = Setup::new(Os::Linux);
    let first = setup.hut();
    assert_eq!(first.start_now().unwrap(), url(18990));
    let before = setup.state().server;

    // The app updated: a new binary in its place.
    executable(
        &setup.dir.path().join("app/emberhut-server"),
        "a newer server",
    );
    let hut = setup.hut();
    assert_eq!(hut.start_now().unwrap(), url(18990));
    {
        let world = setup.world();
        assert_eq!(world.terminated, [101], "the old hut was asked to stop");
        assert_eq!(world.spawned.len(), 2, "and the new one started");
    }
    let after = setup.state();
    assert_ne!(after.server, before);
    assert_eq!(after.pid, Some(102));

    // Once: the next start keeps it.
    assert_eq!(hut.start_now().unwrap(), url(18990));
    assert_eq!(setup.world().spawned.len(), 2);
}

#[test]
fn a_new_build_restarts_a_hut_kept_lit_by_systemd() {
    let setup = Setup::new(Os::Linux);
    let keep = KeepLit {
        when_closed: true,
        at_login: true,
    };
    setup.hut().keep_lit_now(keep).unwrap();
    executable(&setup.dir.path().join("app/emberhut-server"), "newer");
    setup.world().ran.clear();
    assert_eq!(setup.hut().start_now().unwrap(), url(18990));
    assert_eq!(
        setup.world().managed(),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable emberhut-hut.service",
            "systemctl --user restart emberhut-hut.service",
        ]
    );
}

#[test]
fn the_pairing_code_is_read_from_pair_json_and_from_its_words() {
    assert_eq!(
        pair::code_in(
            "{\"code\":\"ABCD-EFGH\",\"expiresAt\":\"2026-11-27T12:10:00Z\",\"machine\":\"m\"}\n"
        )
        .as_deref(),
        Some("ABCD-EFGH")
    );
    assert_eq!(
        pair::code_in("a log line\n  {\"code\": \"wxyz-2345\"}  \n").as_deref(),
        Some("WXYZ-2345"),
        "other lines passed over, the code in capitals"
    );
    assert_eq!(
        pair::code_in(
            "Pairing code: QRST-UVWX (works once, until 12:10:00 UTC; in /d/emberhut.db)\n\
             Pairing link: https://emberhut.com/pair#id=x&code=QRST-UVWX\n"
        )
        .as_deref(),
        Some("QRST-UVWX")
    );
    for nothing in [
        "",
        "{\"code\":\"\"}",
        "{\"code\":\"ABCDEFGH\"}",
        "{\"code\":12}",
        "Pairing code: soon",
        "{\"code\":\"AB-CDEFGH\"}",
    ] {
        assert_eq!(pair::code_in(nothing), None, "{nothing:?}");
    }
}

#[test]
fn a_pairing_code_is_made_by_the_servers_pair_in_the_huts_database() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    assert_eq!(hut.pair_code_now().unwrap(), "ABCD-EFGH");
    let data = setup.data();
    let world = setup.world();
    assert_eq!(world.spawned.len(), 1, "the hut was lit first");
    let pair = world
        .ran
        .iter()
        .find(|run| run.words().first().map(String::as_str) == Some("pair"))
        .unwrap();
    assert_eq!(
        pair.words(),
        [
            "pair".to_owned(),
            "--json".to_owned(),
            "--database-url".to_owned(),
            format!("sqlite://{}", data.join("emberhut.db").display()),
            "--listen".to_owned(),
            "127.0.0.1:18990".to_owned(),
        ]
    );
    drop(world);

    setup.world().pair = Ran {
        ok: false,
        stdout: String::new(),
        stderr: "Error: no pairing code was made\n\nCaused by:\n    the database is missing\n"
            .to_owned(),
    };
    let error = hut.pair_code_now().unwrap_err();
    assert_eq!(
        error.to_string(),
        "The hut didn't make a pairing code: the database is missing"
    );
    setup.world().pair = Ran {
        ok: true,
        stdout: "{}\n".to_owned(),
        stderr: String::new(),
    };
    assert!(matches!(
        hut.pair_code_now().unwrap_err(),
        HutError::NoCode { .. }
    ));
}

/// The server as the service tests run it.
fn serve() -> Serve {
    Serve {
        program: PathBuf::from("/Applications/Emberhut.app/Contents/MacOS/emberhut-server"),
        args: [
            "serve",
            "--listen",
            "127.0.0.1:18990",
            "--database-url",
            "sqlite:///Users/ada/Library/Application%20Support/Emberhut/Hut/emberhut.db",
            "--secrets-dir",
            "/Users/ada/Library/Application Support/Emberhut/Hut/secrets",
        ]
        .map(str::to_owned)
        .to_vec(),
        path: OsString::from("/Users/ada/.local/bin:/usr/bin:/bin"),
        dir: PathBuf::from("/Users/ada/Library/Application Support/Emberhut/Hut"),
        log: PathBuf::from("/Users/ada/Library/Logs/Emberhut/hut.log"),
    }
}

#[test]
fn the_launch_agent_runs_the_server_from_the_bundle_at_login_and_keeps_it_alive() {
    let plist = launch_agent(
        &serve(),
        KeepLit {
            when_closed: true,
            at_login: true,
        },
    );
    assert_eq!(
        plist,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.emberhut.hut</string>
  <key>ProgramArguments</key><array>
    <string>/Applications/Emberhut.app/Contents/MacOS/emberhut-server</string>
    <string>serve</string>
    <string>--listen</string>
    <string>127.0.0.1:18990</string>
    <string>--database-url</string>
    <string>sqlite:///Users/ada/Library/Application%20Support/Emberhut/Hut/emberhut.db</string>
    <string>--secrets-dir</string>
    <string>/Users/ada/Library/Application Support/Emberhut/Hut/secrets</string>
  </array>
  <key>WorkingDirectory</key><string>/Users/ada/Library/Application Support/Emberhut/Hut</string>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>/Users/ada/.local/bin:/usr/bin:/bin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>Umask</key><integer>63</integer>
  <key>StandardOutPath</key><string>/Users/ada/Library/Logs/Emberhut/hut.log</string>
  <key>StandardErrorPath</key><string>/Users/ada/Library/Logs/Emberhut/hut.log</string>
</dict>
</plist>
"#
    );
}

#[test]
fn the_launch_agent_kept_lit_only_while_the_app_is_closed_neither_runs_at_load_nor_comes_back() {
    let plist = launch_agent(
        &serve(),
        KeepLit {
            when_closed: true,
            at_login: false,
        },
    );
    assert!(plist.contains("<key>RunAtLoad</key><false/>"), "{plist}");
    assert!(plist.contains("<key>KeepAlive</key><false/>"), "{plist}");
    let mut odd = serve();
    odd.args.push("a&b<c>".to_owned());
    assert!(launch_agent(&odd, KeepLit::default()).contains("<string>a&amp;b&lt;c&gt;</string>"));
}

#[test]
fn the_systemd_unit_runs_the_server_and_restarts_it_when_it_fails() {
    let mut serve = serve();
    serve.program = PathBuf::from("/opt/emberhut/emberhut-server");
    serve.args = [
        "serve",
        "--listen",
        "127.0.0.1:18991",
        "--secrets-dir",
        "/h/100% \"$x\"",
    ]
    .map(str::to_owned)
    .to_vec();
    serve.path = OsString::from("/home/ada/.local/bin:/usr/bin");
    serve.dir = PathBuf::from("/home/ada/.local/share/emberhut/hut");
    serve.log = PathBuf::from("/home/ada/.local/state/emberhut/hut.log");
    assert_eq!(
        systemd_unit(&serve),
        r#"[Unit]
Description=Emberhut hut: this computer's emberhut server, run for the Emberhut app

[Service]
Type=simple
ExecStart="/opt/emberhut/emberhut-server" "serve" "--listen" "127.0.0.1:18991" "--secrets-dir" "/h/100%% \"$$x\""
WorkingDirectory=/home/ada/.local/share/emberhut/hut
Environment="PATH=/home/ada/.local/bin:/usr/bin"
UMask=0077
Restart=on-failure
RestartSec=2
StandardOutput=append:/home/ada/.local/state/emberhut/hut.log
StandardError=append:/home/ada/.local/state/emberhut/hut.log

[Install]
WantedBy=default.target
"#
    );
}

#[test]
fn keeping_the_hut_lit_on_linux_moves_it_to_its_unit_and_back() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    assert_eq!(hut.start_now().unwrap(), url(18990));
    let unit = setup
        .home()
        .join(".config/systemd/user/emberhut-hut.service");

    let keep = KeepLit {
        when_closed: true,
        at_login: true,
    };
    hut.keep_lit_now(keep).unwrap();
    {
        let world = setup.world();
        assert_eq!(world.terminated, [101], "the app's child made way");
        assert_eq!(
            world.managed(),
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable emberhut-hut.service",
                "systemctl --user restart emberhut-hut.service",
            ]
        );
        assert_eq!(world.ports.get(&18990), Some(&Listener::Hut(None)));
    }
    let text = std::fs::read_to_string(&unit).unwrap();
    assert!(text.contains("\"127.0.0.1:18990\""), "{text}");
    let state = setup.state();
    assert_eq!((state.keep, state.pid), (keep, None));
    assert_eq!(hut.status_now().url, Some(url(18990)));

    // Kept lit while closed, no longer from login.
    setup.world().ran.clear();
    let closed = KeepLit {
        when_closed: true,
        at_login: false,
    };
    hut.keep_lit_now(closed).unwrap();
    assert_eq!(
        setup.world().managed(),
        [
            "systemctl --user daemon-reload",
            "systemctl --user disable emberhut-hut.service",
            "systemctl --user restart emberhut-hut.service",
        ]
    );

    // Off: the unit goes, and the hut is the app's child again.
    setup.world().ran.clear();
    hut.keep_lit_now(KeepLit::default()).unwrap();
    {
        let world = setup.world();
        assert_eq!(
            world.managed(),
            [
                "systemctl --user disable emberhut-hut.service",
                "systemctl --user stop emberhut-hut.service",
                "systemctl --user daemon-reload",
            ]
        );
        assert_eq!(world.spawned.len(), 2);
        assert_eq!(world.ports.get(&18990), Some(&Listener::Hut(Some(102))));
    }
    assert!(!unit.exists());
    assert_eq!(setup.state().pid, Some(102));
}

#[test]
fn keeping_the_hut_lit_on_macos_loads_its_launch_agent_and_unloads_it() {
    let setup = Setup::new(Os::Mac);
    let hut = setup.hut();
    let plist = setup
        .home()
        .join("Library/LaunchAgents/com.emberhut.hut.plist");
    let keep = KeepLit {
        when_closed: true,
        at_login: true,
    };
    hut.keep_lit_now(keep).unwrap();
    assert_eq!(
        setup.world().managed(),
        [
            "launchctl bootout gui/501/com.emberhut.hut".to_owned(),
            format!("launchctl bootstrap gui/501 {}", plist.display()),
            "launchctl kickstart gui/501/com.emberhut.hut".to_owned(),
        ]
    );
    assert_eq!(
        setup.world().spawned,
        Vec::<Run>::new(),
        "never the app's child"
    );
    let text = std::fs::read_to_string(&plist).unwrap();
    assert!(text.contains("<key>RunAtLoad</key><true/>"), "{text}");
    assert!(
        text.contains(&format!(
            "<string>{}</string>",
            setup.dir.path().join("app/emberhut-server").display()
        )),
        "{text}"
    );

    // Stopping unloads it (kept alive, a killed hut would come back) and keeps its plist.
    setup.world().ran.clear();
    hut.stop_now().unwrap();
    assert_eq!(
        setup.world().managed(),
        ["launchctl bootout gui/501/com.emberhut.hut"]
    );
    assert!(plist.exists());
    assert_eq!(hut.status_now().url, None);

    setup.world().ran.clear();
    hut.keep_lit_now(KeepLit::default()).unwrap();
    assert_eq!(
        setup.world().managed(),
        ["launchctl bootout gui/501/com.emberhut.hut"]
    );
    assert!(!plist.exists());
    assert_eq!(setup.world().spawned.len(), 1, "the app's child again");
}

#[test]
fn keeping_the_hut_lit_without_a_service_manager_is_refused() {
    let setup = Setup::new(Os::Other);
    let error = setup
        .hut()
        .keep_lit_now(KeepLit {
            when_closed: true,
            at_login: false,
        })
        .unwrap_err();
    assert!(matches!(error, HutError::NoService), "{error:?}");
    setup.hut().keep_lit_now(KeepLit::default()).unwrap();
}

#[test]
fn stopping_puts_out_the_apps_child_and_one_left_by_an_earlier_run() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    hut.start_now().unwrap();
    hut.stop_now().unwrap();
    assert_eq!(setup.world().terminated, [101]);
    assert_eq!(hut.status_now().url, None);
    assert_eq!(setup.state().pid, None);

    // An earlier run's hut, still lit: its process is remembered.
    let earlier = setup.hut();
    earlier.start_now().unwrap();
    let later = setup.hut();
    later.stop_now().unwrap();
    assert_eq!(setup.world().terminated, [101, 102]);
    assert!(setup.world().ports.is_empty());
}

#[test]
fn a_dropped_hut_takes_its_child_with_it() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    hut.start_now().unwrap();
    drop(hut);
    assert_eq!(setup.world().terminated, [101]);
}

#[gpui_kit::test]
fn the_calls_are_tasks_on_the_background_executor(cx: &mut TestAppContext) {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    let started = cx.update(|cx| hut.start(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.foreground_executor().block_test(started).unwrap(),
        url(18990)
    );
    let code = cx.update(|cx| hut.pair_code(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.foreground_executor().block_test(code).unwrap(),
        "ABCD-EFGH"
    );
    let status = cx.update(|cx| hut.status(cx));
    cx.run_until_parked();
    let status = cx.foreground_executor().block_test(status);
    assert_eq!(status.url, Some(url(18990)));
    assert_eq!(
        status.server,
        Some(setup.dir.path().join("app/emberhut-server"))
    );
    let stopped = cx.update(|cx| hut.stop(cx));
    cx.run_until_parked();
    cx.foreground_executor().block_test(stopped).unwrap();
    let kept = cx.update(|cx| hut.keep_lit(KeepLit::default(), cx));
    cx.run_until_parked();
    cx.foreground_executor().block_test(kept).unwrap();
    assert_eq!(setup.world().spawned.len(), 2);
}

#[test]
fn a_test_run_names_the_first_port_and_leaves_the_service_out() {
    let port = |value: Option<&str>| {
        let value = value.map(OsString::from);
        HutOptions::from_vars(&move |name: &str| {
            (name == "EMBERHUT_HUT_PORT")
                .then(|| value.clone())
                .flatten()
        })
    };
    assert_eq!(port(None), HutOptions::default());
    assert_eq!(port(Some("18995")).port, 18995);
    for bad in ["", "0", "port", "70000"] {
        assert_eq!(port(Some(bad)).port, 18990, "{bad:?}");
    }

    let setup = Setup::new(Os::Linux);
    let no_service = BTreeMap::from([(SERVICE_ENV, OsString::from("none"))]);
    let paths = setup.paths_with(setup.dir.path().join("app/emberhut-desktop"), no_service);
    assert_eq!(*paths.manager(), super::Manager::None);
    assert!(matches!(
        setup.paths().manager(),
        super::Manager::Systemd { .. }
    ));
}

#[test]
fn a_quitting_app_asks_its_child_to_stop_without_waiting() {
    let setup = Setup::new(Os::Linux);
    let hut = setup.hut();
    hut.start_now().unwrap();
    hut.let_go();
    assert_eq!(setup.world().terminated, [101]);
    drop(hut);
    assert_eq!(setup.world().terminated, [101], "let go once");

    // A hut a service runs isn't the app's to stop.
    let kept = setup.hut();
    kept.keep_lit_now(KeepLit {
        when_closed: true,
        at_login: false,
    })
    .unwrap();
    kept.let_go();
    assert_eq!(setup.world().terminated, [101]);
}
