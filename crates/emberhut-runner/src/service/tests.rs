//! The service texts, and the service manager calls checked against a recording stand-in.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::config::Config;

/// A host that records each command line and answers from a script: the first answer whose
/// key starts the command line, else success with no output.
struct Fake {
    env: BTreeMap<String, String>,
    answers: Vec<(String, Ran)>,
    calls: RefCell<Vec<String>>,
    pauses: RefCell<usize>,
}

impl Fake {
    fn new(home: &Path) -> Self {
        let env = [
            ("HOME", home.to_str().unwrap()),
            ("PATH", "/home/user/.cargo/bin:relative/bin:/usr/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        Self {
            env,
            answers: Vec::new(),
            calls: RefCell::default(),
            pauses: RefCell::default(),
        }
    }

    fn answer(mut self, starts: &str, ok: bool, stdout: &str, stderr: &str) -> Self {
        self.answers.push((
            starts.to_owned(),
            Ran {
                ok,
                code: Some(i32::from(!ok)),
                stdout: stdout.to_owned(),
                stderr: stderr.to_owned(),
            },
        ));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Host for Fake {
    fn env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    fn uid(&self) -> u32 {
        501
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Ran, ServiceError> {
        let line = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.borrow_mut().push(line.clone());
        Ok(self
            .answers
            .iter()
            .find(|(k, _)| line.starts_with(k.as_str()))
            .map_or_else(
                || Ran {
                    ok: true,
                    code: Some(0),
                    ..Ran::default()
                },
                |(_, r)| r.clone(),
            ))
    }

    fn pause(&self, _: Duration) {
        *self.pauses.borrow_mut() += 1;
    }
}

/// A runner's settings with its home in `dir/runner`.
fn config(dir: &Path) -> Config {
    let home = dir.join("runner");
    let env: BTreeMap<&str, String> = [
        ("HOME", dir.to_str().unwrap().to_owned()),
        ("EMBERHUT_RUNNER_HOME", home.to_str().unwrap().to_owned()),
    ]
    .into_iter()
    .collect();
    Config::load(&|k| env.get(k).cloned(), "build-box").unwrap()
}

fn command() -> ServiceCommand {
    ServiceCommand {
        program: PathBuf::from("/opt/emberhut runner/current/emberhut-runner"),
        args: vec!["run".to_owned()],
    }
}

#[test]
fn the_version_is_the_builds() {
    let v = crate::version();
    assert_ne!(v, "");
    match option_env!("EMBERHUT_RUNNER_VERSION") {
        Some(set) if !set.is_empty() => assert_eq!(v, set),
        _ => assert_eq!(v, env!("CARGO_PKG_VERSION")),
    }
}

#[test]
fn a_command_runs_the_runner_in_the_foreground() {
    let c = ServiceCommand::new("/bin/emberhut-runner");
    assert_eq!(c.args[0], RUN);
    assert_eq!(c.args.contains(&SERVICE_FLAG.to_owned()), cfg!(windows));
    let c = ServiceCommand::current(&|k| (k == LAUNCHER_ENV).then(|| "/l/current/r".to_owned()))
        .unwrap();
    assert_eq!(c.program, PathBuf::from("/l/current/r"));
    let c = ServiceCommand::current(&|_| None).unwrap();
    assert_eq!(c.program, std::env::current_exe().unwrap());
}

#[test]
fn the_path_keeps_the_shells_absolute_folders_then_the_systems() {
    assert_eq!(
        service_path(Some("/a/bin:rel:/usr/bin::/a/bin"), &["/usr/bin", "/bin"]),
        "/a/bin:/usr/bin:/bin"
    );
    assert_eq!(service_path(None, &["/usr/bin"]), "/usr/bin");
}

#[test]
fn the_systemd_unit_reads_as_written() {
    let c = ServiceCommand {
        program: PathBuf::from("/usr/local/bin/emberhut-runner"),
        args: vec!["run".to_owned()],
    };
    let unit = systemd_unit(
        &c,
        Path::new("/home/user/.local/share/emberhut-runner"),
        "/usr/bin:/bin",
    );
    assert_eq!(
        unit,
        "# Written by emberhut-runner, which replaces this file when it installs the service again.
[Unit]
Description=Emberhut runner: runs coding-agent sessions for an Emberhut server

[Service]
ExecStart=/usr/local/bin/emberhut-runner run
Environment=\"EMBERHUT_RUNNER_HOME=/home/user/.local/share/emberhut-runner\"
# Sessions find the same tools as the shell the service was installed from.
Environment=\"PATH=/usr/bin:/bin\"
WorkingDirectory=/home/user/.local/share/emberhut-runner
Restart=on-failure
RestartSec=5
# A running turn resumes at the next start, so a stop needn't wait for it.
TimeoutStopSec=15
StandardOutput=append:/home/user/.local/share/emberhut-runner/runner.log
StandardError=append:/home/user/.local/share/emberhut-runner/runner.log

[Install]
WantedBy=default.target
"
    );
}

#[test]
fn the_systemd_unit_quotes_and_escapes_what_systemd_would_expand() {
    let c = ServiceCommand {
        program: PathBuf::from("/opt/my runner/emberhut-runner"),
        args: vec![
            "run".to_owned(),
            "100%".to_owned(),
            "$HOME".to_owned(),
            r#"a"b\c"#.to_owned(),
        ],
    };
    let unit = systemd_unit(&c, Path::new("/srv/50% \"home\""), "/x");
    assert!(
        unit.contains(r#"ExecStart="/opt/my runner/emberhut-runner" run 100%% $$HOME "a\"b\\c""#),
        "{unit}"
    );
    assert!(
        unit.contains(r#"Environment="EMBERHUT_RUNNER_HOME=/srv/50%% \"home\"""#),
        "{unit}"
    );
    assert!(
        unit.contains("WorkingDirectory=/srv/50%% \"home\"\n"),
        "{unit}"
    );
    assert!(
        unit.contains("StandardOutput=append:/srv/50%% \"home\"/runner.log\n"),
        "{unit}"
    );
}

#[test]
fn the_launchd_plist_escapes_its_strings() {
    let c = ServiceCommand {
        program: PathBuf::from("/Users/user/Library/R&D <x>/emberhut-runner"),
        args: vec!["run".to_owned()],
    };
    let plist = launchd_plist(
        &c,
        Path::new("/Users/user/Library/Application Support/Emberhut Runner"),
        "/a\"b:/usr/bin",
    );
    assert!(plist.starts_with(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Written by emberhut-runner"
    ));
    assert!(plist.contains(
        "  <array>\n    <string>/Users/user/Library/R&amp;D &lt;x&gt;/emberhut-runner</string>\n    <string>run</string>\n  </array>\n"
    ), "{plist}");
    assert!(plist.contains("<key>Label</key><string>com.emberhut.runner</string>"));
    assert!(plist.contains(
        "<key>EMBERHUT_RUNNER_HOME</key><string>/Users/user/Library/Application Support/Emberhut Runner</string>"
    ));
    assert!(plist.contains("<key>PATH</key><string>/a&quot;b:/usr/bin</string>"));
    assert!(plist.contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>"));
    assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    assert!(plist.contains("<key>ThrottleInterval</key><integer>5</integer>"));
    assert!(plist.contains(
        "<key>StandardErrorPath</key><string>/Users/user/Library/Application Support/Emberhut Runner/runner.log</string>"
    ));
    // Every element that opens closes.
    for tag in ["plist", "dict", "array", "key", "string"] {
        assert_eq!(
            plist.matches(&format!("<{tag}>")).count() + plist.matches(&format!("<{tag} ")).count(),
            plist.matches(&format!("</{tag}>")).count(),
            "{tag}"
        );
    }
}

#[test]
fn the_windows_service_runs_with_the_service_flag_and_the_runners_home() {
    let s = windows_service(
        &command(),
        Path::new(r"C:\Users\user\AppData\Local\Emberhut Runner"),
        Some(r"C:\Windows;C:\git\bin"),
    );
    assert_eq!(s.name, "emberhut-runner");
    assert_eq!(s.args, ["run", "--service"]);
    assert!(s.description.starts_with(MARK));
    assert_eq!(
        s.environment,
        [
            r"EMBERHUT_RUNNER_HOME=C:\Users\user\AppData\Local\Emberhut Runner",
            r"PATH=C:\Windows;C:\git\bin"
        ]
    );
    let flagged = ServiceCommand {
        args: vec!["run".to_owned(), "--service".to_owned()],
        ..command()
    };
    let s = windows_service(&flagged, Path::new("/h"), None);
    assert_eq!(s.args, ["run", "--service"]);
    assert_eq!(s.environment.len(), 1);
}

#[test]
fn a_file_the_runner_didnt_write_is_kept_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sub/unit.service");
    write_own(&file, "# Written by emberhut-runner.\nA\n", false).unwrap();
    write_own(&file, "# Written by emberhut-runner.\nB\n", false).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().ends_with("B\n"));

    std::fs::write(&file, "[Unit]\n# hand-written\n").unwrap();
    let e = write_own(&file, "# Written by emberhut-runner.\nC\n", false).unwrap_err();
    assert!(matches!(e, ServiceError::NotOurs(_)), "{e}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "[Unit]\n# hand-written\n"
    );
    assert_eq!(remove_own(&file).unwrap(), Removed::NotOurs);
    assert!(file.exists());

    write_own(&file, "# Written by emberhut-runner.\nC\n", true).unwrap();
    assert_eq!(remove_own(&file).unwrap(), Removed::Deleted);
    assert_eq!(remove_own(&file).unwrap(), Removed::Missing);
}

#[test]
fn the_mark_counts_only_near_the_top() {
    assert!(is_ours(b"<?xml?>\n<!-- Written by emberhut-runner. -->\n"));
    assert!(!is_ours(b"a\nb\nc\n# Written by emberhut-runner\n"));
    assert!(!is_ours(&[0xff, 0xfe]));
}

#[test]
fn systemd_install_writes_the_unit_and_starts_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer("loginctl", true, "Linger=no\n", "");
    let said = systemd::install(&host, &config, &command(), false).unwrap();

    let file = dir
        .path()
        .join(".config/systemd/user/emberhut-runner.service");
    let unit = std::fs::read_to_string(&file).unwrap();
    assert!(unit.starts_with("# Written by emberhut-runner"));
    assert!(unit.contains(r#"ExecStart="/opt/emberhut runner/current/emberhut-runner" run"#));
    assert!(
        unit.contains("Environment=\"PATH=/home/user/.cargo/bin:/usr/bin:/usr/local/bin:/bin\""),
        "{unit}"
    );
    assert!(config.home.is_dir());
    assert_eq!(
        host.calls(),
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable emberhut-runner.service",
            "systemctl --user restart emberhut-runner.service",
            "loginctl show-user 501 --property=Linger",
        ]
    );
    assert!(
        said.contains("enabled and started") && said.contains("loginctl enable-linger"),
        "{said}"
    );

    let lingering = Fake::new(dir.path()).answer("loginctl", true, "Linger=yes\n", "");
    let said = systemd::install(&lingering, &config, &command(), false).unwrap();
    assert!(!said.contains("linger"), "{said}");
}

#[test]
fn systemd_install_follows_xdg_config_home() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut host = Fake::new(dir.path());
    let xdg = dir.path().join("xdg");
    host.env.insert(
        "XDG_CONFIG_HOME".to_owned(),
        xdg.to_str().unwrap().to_owned(),
    );
    systemd::install(&host, &config, &command(), false).unwrap();
    assert!(xdg.join("systemd/user/emberhut-runner.service").is_file());
}

#[test]
fn systemd_install_leaves_a_hand_written_unit_and_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let file = dir
        .path()
        .join(".config/systemd/user/emberhut-runner.service");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "[Service]\nExecStart=/mine\n").unwrap();
    let host = Fake::new(dir.path());
    let e = systemd::install(&host, &config, &command(), false).unwrap_err();
    assert!(
        e.to_string().contains("wasn't written by emberhut-runner"),
        "{e}"
    );
    assert_eq!(host.calls(), Vec::<String>::new());
    systemd::install(&host, &config, &command(), true).unwrap();
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("# Written by")
    );
}

#[test]
fn a_failing_systemctl_says_what_it_said() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer(
        "systemctl --user daemon-reload",
        false,
        "",
        "Failed to connect to bus: No medium found\n",
    );
    let e = systemd::install(&host, &config, &command(), false).unwrap_err();
    assert_eq!(
        e.to_string(),
        "systemctl --user daemon-reload failed: Failed to connect to bus: No medium found"
    );
}

#[test]
fn systemd_uninstall_stops_and_removes_only_its_own_unit() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer("systemctl --user disable", false, "", "not loaded");
    systemd::install(&host, &config, &command(), false).unwrap();
    host.calls.borrow_mut().clear();
    let said = systemd::uninstall(&host, &config).unwrap();
    assert!(said.contains("removed") && said.contains("home"), "{said}");
    assert!(
        !dir.path()
            .join(".config/systemd/user/emberhut-runner.service")
            .exists()
    );
    assert_eq!(
        host.calls(),
        [
            "systemctl --user disable --now emberhut-runner.service",
            "systemctl --user daemon-reload",
        ]
    );
    assert!(
        systemd::uninstall(&host, &config)
            .unwrap()
            .starts_with("no systemd user unit")
    );
}

#[test]
fn systemd_restart_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path()).answer(
        "systemctl --user status",
        false,
        "● emberhut-runner.service - Emberhut runner\n     Active: inactive (dead)\n",
        "",
    );
    assert_eq!(systemd::restart(&host).unwrap(), "restarted");
    let status = systemd::status(&host).unwrap();
    assert!(status.contains("inactive (dead)"), "{status}");
    assert_eq!(
        host.calls(),
        [
            "systemctl --user restart emberhut-runner.service",
            "systemctl --user status --no-pager --lines=0 emberhut-runner.service",
        ]
    );
    let none = Fake::new(dir.path()).answer("systemctl", false, "", "");
    assert_eq!(systemd::status(&none).unwrap(), "not installed");
}

#[test]
fn launchd_install_writes_the_agent_and_bootstraps_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer(
        "launchctl bootout",
        false,
        "",
        "Boot-out failed: 3: No such process",
    );
    let said = launchd::install(&host, &config, &command(), false).unwrap();
    let file = dir
        .path()
        .join("Library/LaunchAgents/com.emberhut.runner.plist");
    let plist = std::fs::read_to_string(&file).unwrap();
    assert!(plist.contains("<string>/opt/emberhut runner/current/emberhut-runner</string>"));
    assert!(
        plist.contains(
            "/home/user/.cargo/bin:/usr/bin:/opt/homebrew/bin:/usr/local/bin:/bin:/usr/sbin:/sbin"
        ),
        "{plist}"
    );
    assert_eq!(
        host.calls(),
        [
            "launchctl bootout gui/501/com.emberhut.runner".to_owned(),
            format!("launchctl bootstrap gui/501 {}", file.display()),
        ]
    );
    assert!(said.contains("loaded"), "{said}");
}

#[test]
fn launchd_bootstrap_is_tried_again_then_gives_up() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer(
        "launchctl bootstrap",
        false,
        "",
        "Bootstrap failed: 5: Input/output error",
    );
    let e = launchd::install(&host, &config, &command(), false).unwrap_err();
    assert!(e.to_string().contains("Bootstrap failed: 5"), "{e}");
    let bootstraps = host
        .calls()
        .iter()
        .filter(|c| c.starts_with("launchctl bootstrap"))
        .count();
    assert_eq!(bootstraps, 3);
    assert_eq!(*host.pauses.borrow(), 2);
}

#[test]
fn launchd_uninstall_restart_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path()).answer(
        "launchctl print",
        true,
        "gui/501/com.emberhut.runner = {\n\tstate = running\n}\n",
        "",
    );
    launchd::install(&host, &config, &command(), false).unwrap();
    host.calls.borrow_mut().clear();
    assert_eq!(launchd::restart(&host).unwrap(), "restarted");
    assert!(launchd::status(&host).unwrap().contains("state = running"));
    let said = launchd::uninstall(&host, &config).unwrap();
    assert!(said.contains("removed"), "{said}");
    assert!(
        !dir.path()
            .join("Library/LaunchAgents/com.emberhut.runner.plist")
            .exists()
    );
    assert_eq!(
        host.calls(),
        [
            "launchctl kickstart -k gui/501/com.emberhut.runner",
            "launchctl print gui/501/com.emberhut.runner",
            "launchctl bootout gui/501/com.emberhut.runner",
        ]
    );
}

#[test]
fn a_path_with_a_line_break_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let host = Fake::new(dir.path());
    let bad = ServiceCommand::new("/opt/a\nb");
    let e = systemd::install(&host, &config, &bad, false).unwrap_err();
    assert!(matches!(e, ServiceError::Path(_)), "{e}");
    assert_eq!(host.calls(), Vec::<String>::new());
}

#[test]
fn no_home_no_unit() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut host = Fake::new(dir.path());
    host.env.remove("HOME");
    assert!(matches!(
        systemd::install(&host, &config, &command(), false),
        Err(ServiceError::NoHome)
    ));
}

#[tokio::test]
async fn a_stop_is_seen_once_asked() {
    let (stopper, stop) = stop_pair();
    assert!(!stop.requested());
    let waiting = tokio::spawn(stop.clone().wait());
    stopper.stop();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(stop.requested());

    let (stopper, stop) = stop_pair();
    drop(stopper);
    tokio::time::timeout(Duration::from_secs(5), stop.wait())
        .await
        .unwrap();
}

#[cfg(not(windows))]
#[test]
fn only_windows_runs_a_windows_service() {
    let e = run_as_service(|_stop| Ok::<(), String>(())).unwrap_err();
    assert!(matches!(e, ServiceError::Unsupported(_)), "{e}");
}
