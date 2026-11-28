//! Linux: a systemd user unit, so the runner runs as the user who installed it, starts with their
//! session (or at boot, with lingering on) and comes back after a crash.

use std::path::{Path, PathBuf};

use super::{
    HOME_ENV, Host, LOG_FILE, MARK, Removed, ServiceCommand, ServiceError, must, prepare,
    remove_own, service_path, summary, user_home, write_own,
};
use crate::config::Config;

/// The unit's name.
pub const UNIT: &str = "emberhut-runner.service";

/// The folders after the installing shell's `PATH` in the unit's.
const SYSTEM_PATH: &[&str] = &["/usr/local/bin", "/usr/bin", "/bin"];

/// The unit file's text for a runner with its home at `home`, its `PATH` set to `path`.
#[must_use]
pub fn systemd_unit(command: &ServiceCommand, home: &Path, path: &str) -> String {
    let home_text = home.to_string_lossy();
    let log = specifiers(&home.join(LOG_FILE).to_string_lossy());
    let exec = command
        .words()
        .iter()
        .map(|w| exec_word(w))
        .collect::<Vec<_>>()
        .join(" ");
    [
        format!("# {MARK}, which replaces this file when it installs the service again."),
        "[Unit]".to_owned(),
        "Description=Emberhut runner: runs coding-agent sessions for an Emberhut server".to_owned(),
        String::new(),
        "[Service]".to_owned(),
        format!("ExecStart={exec}"),
        environment(HOME_ENV, &home_text),
        "# Sessions find the same tools as the shell the service was installed from.".to_owned(),
        environment("PATH", path),
        format!("WorkingDirectory={}", specifiers(&home_text)),
        "Restart=on-failure".to_owned(),
        "RestartSec=5".to_owned(),
        "# A running turn resumes at the next start, so a stop needn't wait for it.".to_owned(),
        "TimeoutStopSec=15".to_owned(),
        format!("StandardOutput=append:{log}"),
        format!("StandardError=append:{log}"),
        String::new(),
        "[Install]".to_owned(),
        "WantedBy=default.target".to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// `%` doubled: systemd expands `%` specifiers in most settings.
fn specifiers(s: &str) -> String {
    s.replace('%', "%%")
}

/// Text inside a quoted systemd word: backslashes, quotes and line breaks escaped.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One word of `ExecStart=`: quoted when it holds a space, a quote or a backslash, with `%`
/// and `$` doubled (systemd expands specifiers and variables there).
fn exec_word(word: &str) -> String {
    let word = specifiers(word).replace('$', "$$");
    if word.is_empty() || word.contains(|c: char| c.is_whitespace() || "\"'\\".contains(c)) {
        quoted(&word)
    } else {
        word
    }
}

/// An `Environment=` line, the whole assignment quoted.
fn environment(key: &str, value: &str) -> String {
    format!(
        "Environment={}",
        quoted(&specifiers(&format!("{key}={value}")))
    )
}

/// `$XDG_CONFIG_HOME/systemd/user/emberhut-runner.service` (`~/.config` without it).
fn unit_file(host: &dyn Host) -> Result<PathBuf, ServiceError> {
    let config = match host.env("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => user_home(host)?.join(".config"),
    };
    Ok(config.join("systemd").join("user").join(UNIT))
}

pub(super) fn install(
    host: &dyn Host,
    config: &Config,
    command: &ServiceCommand,
    force: bool,
) -> Result<String, ServiceError> {
    prepare(config, command)?;
    let file = unit_file(host)?;
    let path = service_path(host.env("PATH").as_deref(), SYSTEM_PATH);
    write_own(&file, &systemd_unit(command, &config.home, &path), force)?;
    must(host, "systemctl", &["--user", "daemon-reload"])?;
    must(host, "systemctl", &["--user", "enable", UNIT])?;
    must(host, "systemctl", &["--user", "restart", UNIT])?;
    // Without lingering, the user's services stop when their last session ends.
    let uid = host.uid().to_string();
    let linger = host.run("loginctl", &["show-user", &uid, "--property=Linger"]);
    let note = match linger {
        Ok(ran) if ran.ok && ran.stdout.trim() == "Linger=no" => {
            ". It stops when you log out; to keep it running then: loginctl enable-linger"
        }
        _ => "",
    };
    Ok(format!(
        "systemd user unit {} enabled and started{note}",
        file.display()
    ))
}

pub(super) fn uninstall(host: &dyn Host, config: &Config) -> Result<String, ServiceError> {
    let file = unit_file(host)?;
    // It may be stopped or unknown already; what matters is that it's gone after.
    host.run("systemctl", &["--user", "disable", "--now", UNIT])?;
    let removed = remove_own(&file)?;
    host.run("systemctl", &["--user", "daemon-reload"])?;
    let what = match removed {
        Removed::Deleted => format!("systemd user unit stopped and {} removed", file.display()),
        Removed::Missing => "no systemd user unit to remove".to_owned(),
        Removed::NotOurs => format!(
            "systemd user unit stopped; {} stays, as emberhut-runner didn't write it",
            file.display()
        ),
    };
    Ok(format!(
        "{what}. The runner's home ({}) stays.",
        config.home.display()
    ))
}

pub(super) fn restart(host: &dyn Host) -> Result<String, ServiceError> {
    must(host, "systemctl", &["--user", "restart", UNIT])?;
    Ok("restarted".to_owned())
}

pub(super) fn status(host: &dyn Host) -> Result<String, ServiceError> {
    let ran = host.run(
        "systemctl",
        &["--user", "status", "--no-pager", "--lines=0", UNIT],
    )?;
    Ok(summary(&ran))
}
