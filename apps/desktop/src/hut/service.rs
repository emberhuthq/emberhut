//! Keeping the hut lit: what the server runs as (its command line, the same for the app's
//! child, the launch agent and the systemd unit), the launch agent's plist, the unit file, and
//! the `launchctl` and `systemctl` commands that load, start and stop them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::{LAUNCH_AGENT, SYSTEMD_UNIT};
use super::process::Run;

/// `launchctl`.
const LAUNCHCTL: &str = "/bin/launchctl";
/// `systemctl`, found on the `PATH` (its folder differs between distributions).
const SYSTEMCTL: &str = "systemctl";
/// The umask the hut runs with as a service: what it makes is the user's only.
const UMASK: u32 = 0o077;

/// Whether the hut stays lit without the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct KeepLit {
    /// It keeps running after the app quits.
    pub(crate) when_closed: bool,
    /// It starts when the user logs in, with or without the app (and so also runs while the
    /// app is closed).
    pub(crate) at_login: bool,
}

impl KeepLit {
    /// Whether a service manager runs the hut (either is asked); otherwise it is the app's
    /// child and stops with it.
    #[must_use]
    pub(crate) fn by_service(self) -> bool {
        self.when_closed || self.at_login
    }
}

/// The server as the hut runs it: its program, arguments, variables, folder and log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Serve {
    /// The server binary.
    pub(crate) program: PathBuf,
    /// `serve --listen … --database-url … --secrets-dir … [--agent-command …]`.
    pub(crate) args: Vec<String>,
    /// `PATH`, for the agent CLI and the tools it runs.
    pub(crate) path: OsString,
    /// The data folder, which it runs in.
    pub(crate) dir: PathBuf,
    /// Its log.
    pub(crate) log: PathBuf,
}

impl Serve {
    /// The server, run as the app's child.
    #[must_use]
    pub(crate) fn child(&self) -> Run {
        Run::new(&self.program)
            .args(&self.args)
            .env("PATH", self.path.clone())
            .dir(&self.dir)
            .log(&self.log)
    }
}

/// One plist value.
enum Value<'a> {
    /// `<string>`.
    String(&'a str),
    /// `<true/>` or `<false/>`.
    Bool(bool),
    /// `<integer>`.
    Integer(u32),
    /// `<array>`.
    Array(Vec<Value<'a>>),
    /// `<dict>`.
    Dict(Vec<(&'a str, Value<'a>)>),
}

impl Value<'_> {
    /// The value's XML, `depth` levels into the top dictionary.
    fn xml(&self, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        match self {
            Value::String(text) => format!("<string>{}</string>", escape(text)),
            Value::Bool(true) => "<true/>".to_owned(),
            Value::Bool(false) => "<false/>".to_owned(),
            Value::Integer(number) => format!("<integer>{number}</integer>"),
            Value::Array(items) => {
                let mut xml = String::from("<array>\n");
                for item in items {
                    xml.push_str(&indent);
                    xml.push_str("  ");
                    xml.push_str(&item.xml(depth + 1));
                    xml.push('\n');
                }
                xml.push_str(&indent);
                xml.push_str("</array>");
                xml
            }
            Value::Dict(entries) => {
                let inner: String = entries
                    .iter()
                    .map(|(key, value)| entry(key, value, depth + 1))
                    .collect();
                format!("<dict>\n{inner}{indent}</dict>")
            }
        }
    }
}

/// One `<key>` and its value, a line of a dictionary `depth` levels in.
fn entry(key: &str, value: &Value<'_>, depth: usize) -> String {
    format!(
        "{}<key>{}</key>{}\n",
        "  ".repeat(depth),
        escape(key),
        value.xml(depth)
    )
}

/// `text` with the characters XML gives a meaning to as entities.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// The launch agent `com.emberhut.hut` for `serve`, kept lit as `keep` says, as ADR 0024's
/// helper agent is written: the program and its arguments, its folder, its log for both
/// outputs, its `PATH`, and a umask that keeps what it makes the user's own.
///
/// From login (`at_login`), the agent starts when it is loaded (`RunAtLoad`, at every login)
/// and is kept running (`KeepAlive`). Kept lit only while the app is closed, it is loaded but
/// not run at load, and not kept alive: the app starts it (`launchctl kickstart`) and it then
/// runs until it stops or the user logs out.
#[must_use]
pub(crate) fn launch_agent(serve: &Serve, keep: KeepLit) -> String {
    let program = serve.program.to_string_lossy();
    let dir = serve.dir.to_string_lossy();
    let log = serve.log.to_string_lossy();
    let path = serve.path.to_string_lossy();
    let arguments = std::iter::once(Value::String(&program))
        .chain(serve.args.iter().map(|arg| Value::String(arg)))
        .collect();
    let entries = [
        ("Label", Value::String(LAUNCH_AGENT)),
        ("ProgramArguments", Value::Array(arguments)),
        ("WorkingDirectory", Value::String(&dir)),
        (
            "EnvironmentVariables",
            Value::Dict(vec![("PATH", Value::String(&path))]),
        ),
        ("RunAtLoad", Value::Bool(keep.at_login)),
        ("KeepAlive", Value::Bool(keep.at_login)),
        ("Umask", Value::Integer(UMASK)),
        ("StandardOutPath", Value::String(&log)),
        ("StandardErrorPath", Value::String(&log)),
    ];
    let mut xml = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    xml.extend(entries.iter().map(|(key, value)| entry(key, value, 1)));
    xml.push_str("</dict>\n</plist>\n");
    xml
}

/// `text` as one quoted word of a unit file: in double quotes, with `\` and `"` escaped, and
/// `%` and `$` doubled so systemd takes them as they are.
fn unit_word(text: &str) -> String {
    let mut word = String::with_capacity(text.len() + 2);
    word.push('"');
    for c in text.chars() {
        match c {
            '\\' => word.push_str("\\\\"),
            '"' => word.push_str("\\\""),
            '%' => word.push_str("%%"),
            '$' => word.push_str("$$"),
            c => word.push(c),
        }
    }
    word.push('"');
    word
}

/// A path in a unit file's setting that takes one path: `%` doubled.
fn unit_path(path: &Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}

/// The systemd user unit `emberhut-hut.service` for `serve`: the server restarted when it
/// fails, its output appended to its log, a umask that keeps what it makes the user's own,
/// and wanted by the user's `default.target`, which `systemctl --user enable` uses for a
/// start at login.
#[must_use]
pub(crate) fn systemd_unit(serve: &Serve) -> String {
    let command: Vec<String> = std::iter::once(serve.program.to_string_lossy().into_owned())
        .chain(serve.args.iter().cloned())
        .map(|word| unit_word(&word))
        .collect();
    let path = unit_word(&format!("PATH={}", serve.path.to_string_lossy()));
    let log = unit_path(&serve.log);
    format!(
        "[Unit]\n\
         Description=Emberhut hut: this computer's emberhut server, run for the Emberhut app\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={command}\n\
         WorkingDirectory={dir}\n\
         Environment={path}\n\
         UMask={UMASK:04o}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        command = command.join(" "),
        dir = unit_path(&serve.dir),
    )
}

/// The launch agent's commands, for the user `uid`.
pub(crate) mod launchctl {
    use std::path::Path;

    use super::{LAUNCH_AGENT, LAUNCHCTL, Run};

    /// The agent's name in the user's GUI domain, `gui/<uid>/com.emberhut.hut`.
    fn target(uid: u32) -> String {
        format!("gui/{uid}/{LAUNCH_AGENT}")
    }

    /// Unloads the agent (stopping it); fails harmlessly when it isn't loaded.
    #[must_use]
    pub(crate) fn bootout(uid: u32) -> Run {
        Run::new(LAUNCHCTL).arg("bootout").arg(target(uid))
    }

    /// Loads the agent from `plist` (which starts it when it runs at load).
    #[must_use]
    pub(crate) fn bootstrap(uid: u32, plist: &Path) -> Run {
        Run::new(LAUNCHCTL)
            .arg("bootstrap")
            .arg(format!("gui/{uid}"))
            .arg(plist)
    }

    /// Starts the loaded agent now, unless it runs.
    #[must_use]
    pub(crate) fn kickstart(uid: u32) -> Run {
        Run::new(LAUNCHCTL).arg("kickstart").arg(target(uid))
    }
}

/// The systemd user unit's commands.
pub(crate) mod systemctl {
    use super::{Run, SYSTEMCTL, SYSTEMD_UNIT};

    /// `systemctl --user <verb> [emberhut-hut.service]`.
    fn user(verb: &str, unit: bool) -> Run {
        let run = Run::new(SYSTEMCTL).args(["--user", verb]);
        if unit { run.arg(SYSTEMD_UNIT) } else { run }
    }

    /// Reads the unit files again, after one was written or removed.
    #[must_use]
    pub(crate) fn daemon_reload() -> Run {
        user("daemon-reload", false)
    }

    /// Starts the unit at login.
    #[must_use]
    pub(crate) fn enable() -> Run {
        user("enable", true)
    }

    /// Stops starting the unit at login.
    #[must_use]
    pub(crate) fn disable() -> Run {
        user("disable", true)
    }

    /// Starts the unit, or restarts it when it runs (a new build, a new port).
    #[must_use]
    pub(crate) fn restart() -> Run {
        user("restart", true)
    }

    /// Stops the unit.
    #[must_use]
    pub(crate) fn stop() -> Run {
        user("stop", true)
    }
}
