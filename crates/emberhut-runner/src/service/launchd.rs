//! macOS: a launchd agent in the user's `LaunchAgents`, so the runner runs as the user who
//! installed it, in their login session (their Keychain is open there), starts at login and comes
//! back after a crash.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{
    HOME_ENV, Host, LOG_FILE, MARK, Removed, ServiceCommand, ServiceError, must, prepare,
    remove_own, service_path, summary, user_home, write_own,
};
use crate::config::Config;

/// The agent's label.
pub const LABEL: &str = "com.emberhut.runner";

/// The folders after the installing shell's `PATH` in the agent's.
const SYSTEM_PATH: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

/// How many times `launchctl bootstrap` is tried: right after a `bootout` it can fail while the
/// old instance is still being torn down.
const BOOTSTRAP_TRIES: usize = 3;

/// Text for an XML element or attribute.
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The agent's property list for a runner with its home at `home`, its `PATH` set to `path`.
#[must_use]
pub fn launchd_plist(command: &ServiceCommand, home: &Path, path: &str) -> String {
    let home_text = xml(&home.to_string_lossy());
    let log = xml(&home.join(LOG_FILE).to_string_lossy());
    let mut args = String::new();
    for word in command.words() {
        args.push_str("    <string>");
        args.push_str(&xml(&word));
        args.push_str("</string>\n");
    }
    let path = xml(path);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- {MARK}, which replaces this file when it installs the service again. -->
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>{HOME_ENV}</key><string>{home_text}</string>
    <key>PATH</key><string>{path}</string>
  </dict>
  <key>WorkingDirectory</key><string>{home_text}</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
    )
}

/// `~/Library/LaunchAgents/com.emberhut.runner.plist`.
fn plist_file(host: &dyn Host) -> Result<PathBuf, ServiceError> {
    Ok(user_home(host)?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// The user's GUI domain, where agents of their login session live.
fn domain(host: &dyn Host) -> String {
    format!("gui/{}", host.uid())
}

pub(super) fn install(
    host: &dyn Host,
    config: &Config,
    command: &ServiceCommand,
    force: bool,
) -> Result<String, ServiceError> {
    prepare(config, command)?;
    let file = plist_file(host)?;
    let path = service_path(host.env("PATH").as_deref(), SYSTEM_PATH);
    write_own(&file, &launchd_plist(command, &config.home, &path), force)?;
    let domain = domain(host);
    let file_text = file.to_string_lossy();
    // Not loaded yet is fine: bootout only clears a previous install.
    host.run("launchctl", &["bootout", &format!("{domain}/{LABEL}")])?;
    for attempt in 1..=BOOTSTRAP_TRIES {
        match must(host, "launchctl", &["bootstrap", &domain, &file_text]) {
            Ok(_) => break,
            Err(e) if attempt == BOOTSTRAP_TRIES => return Err(e),
            Err(_) => host.pause(Duration::from_secs(1)),
        }
    }
    Ok(format!("launchd agent {} loaded", file.display()))
}

pub(super) fn uninstall(host: &dyn Host, config: &Config) -> Result<String, ServiceError> {
    let file = plist_file(host)?;
    // Not loaded is fine: what matters is that it's gone after.
    host.run(
        "launchctl",
        &["bootout", &format!("{}/{LABEL}", domain(host))],
    )?;
    let what = match remove_own(&file)? {
        Removed::Deleted => format!("launchd agent unloaded and {} removed", file.display()),
        Removed::Missing => "no launchd agent to remove".to_owned(),
        Removed::NotOurs => format!(
            "launchd agent unloaded; {} stays, as emberhut-runner didn't write it",
            file.display()
        ),
    };
    Ok(format!(
        "{what}. The runner's home ({}) stays.",
        config.home.display()
    ))
}

pub(super) fn restart(host: &dyn Host) -> Result<String, ServiceError> {
    let target = format!("{}/{LABEL}", domain(host));
    must(host, "launchctl", &["kickstart", "-k", &target])?;
    Ok("restarted".to_owned())
}

pub(super) fn status(host: &dyn Host) -> Result<String, ServiceError> {
    let target = format!("{}/{LABEL}", domain(host));
    Ok(summary(&host.run("launchctl", &["print", &target])?))
}
