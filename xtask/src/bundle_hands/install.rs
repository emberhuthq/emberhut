//! Installing the helper for the owner: the running agent is stopped, `~/Applications/Emberhut
//! Hands.app` replaced (only when it is this helper; the new copy put in place by renames on
//! the same volume), its launch agent `~/Library/LaunchAgents/com.emberhut.hands.plist` written,
//! and the agent started again with `launchctl`.
//! Nothing of any other helper is touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

use super::app::{BUNDLE_DIR, BUNDLE_ID, EXECUTABLE, LABEL};
use super::files::launch_agent;
use crate::bundle_macos::ownership::read_identifier;
use crate::bundle_macos::tool;
use crate::files::remove_dir_if_present;
use crate::{cmd, say};

/// Starts and stops launch agents.
const LAUNCHCTL: &str = "/bin/launchctl";
/// Says the user's id.
const ID: &str = "/usr/bin/id";
/// How many times the agent's bootstrap is tried.
const BOOTSTRAP_TRIES: u32 = 5;
/// How long between two tries.
const BOOTSTRAP_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// What the launch agent sets for the helper.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Agent {
    /// `EMBERHUT_HANDS_LISTEN`.
    pub(crate) listen: Option<String>,
    /// `EMBERHUT_HANDS_SERVER_URL`.
    pub(crate) server_url: Option<String>,
    /// `EMBERHUT_HANDS_SERVER_TOKEN_FILE`.
    pub(crate) server_token_file: Option<PathBuf>,
}

impl Agent {
    /// The environment variables, set ones only.
    #[must_use]
    pub(crate) fn environment(&self) -> Vec<(&'static str, String)> {
        let file = self
            .server_token_file
            .as_ref()
            .map(|file| file.to_string_lossy().into_owned());
        [
            ("EMBERHUT_HANDS_LISTEN", self.listen.clone()),
            ("EMBERHUT_HANDS_SERVER_URL", self.server_url.clone()),
            ("EMBERHUT_HANDS_SERVER_TOKEN_FILE", file),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect()
    }
}

/// Where an install writes, under the owner's home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Places {
    /// `~/Applications/Emberhut Hands.app`.
    pub(crate) app: PathBuf,
    /// The new copy, written first: `~/Applications/.Emberhut Hands.app.new`.
    pub(crate) new: PathBuf,
    /// The old copy while the new one goes in: `~/Applications/.Emberhut Hands.app.old`.
    pub(crate) old: PathBuf,
    /// `~/Library/LaunchAgents/com.emberhut.hands.plist`.
    pub(crate) agent: PathBuf,
    /// `~/Library/Logs/Emberhut Hands.log`.
    pub(crate) log: PathBuf,
}

impl Places {
    /// The places under `home`.
    #[must_use]
    pub(crate) fn under(home: &Path) -> Self {
        let apps = home.join("Applications");
        Self {
            app: apps.join(BUNDLE_DIR),
            new: apps.join(format!(".{BUNDLE_DIR}.new")),
            old: apps.join(format!(".{BUNDLE_DIR}.old")),
            agent: home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist")),
            log: home.join("Library").join("Logs").join("Emberhut Hands.log"),
        }
    }

    /// The helper's binary once installed.
    #[must_use]
    pub(crate) fn program(&self) -> PathBuf {
        self.app.join("Contents").join("MacOS").join(EXECUTABLE)
    }
}

/// The `launchctl` commands that stop the agent (if loaded) and start it from `plist`, for the
/// user `uid`.
#[must_use]
pub(crate) fn launchctl(uid: &str, plist: &Path) -> (Command, Command) {
    let mut stop = Command::new(LAUNCHCTL);
    stop.arg("bootout").arg(format!("gui/{uid}/{LABEL}"));
    let mut start = Command::new(LAUNCHCTL);
    start.arg("bootstrap").arg(format!("gui/{uid}")).arg(plist);
    (stop, start)
}

/// Installs `bundle` and (re)starts its agent with `agent`'s settings: the installed app.
///
/// # Errors
///
/// Fails when another app sits at the place, a copy or rename fails (the old app is put back),
/// or `launchctl` can't start it.
pub(crate) fn install(bundle: &Path, agent: &Agent) -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let places = Places::under(&home);
    if places.app.exists() {
        let info = places.app.join("Contents").join("Info.plist");
        let id = read_identifier(&info)?;
        if id.as_deref().map(str::trim) != Some(BUNDLE_ID) {
            bail!(
                "{} is not Emberhut Hands ({BUNDLE_ID}): it is left alone",
                places.app.display()
            );
        }
    }
    if let Some(parent) = places.app.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    remove_dir_if_present(&places.new)?;
    remove_dir_if_present(&places.old)?;
    cmd::run(Command::new(tool::DITTO).arg(bundle).arg(&places.new))?;

    // The running helper stops first, so the bundle is never swapped under it.
    let uid = cmd::stdout(Command::new(ID).arg("-u"))?.trim().to_owned();
    let (mut stop, mut start) = launchctl(&uid, &places.agent);
    // Not loaded yet is fine: this is its first install.
    let _stopped = cmd::capture(&mut stop)?;

    // A new file rather than one overwritten in place: the Mac checks a running app's
    // signature against the file it started from.
    let had_one = places.app.exists();
    if had_one && let Err(error) = fs::rename(&places.app, &places.old) {
        // It was stopped above and is still in place: start it again, as it was.
        restart_old(&mut start);
        return Err(error).context("moving the old helper aside");
    }
    if let Err(error) = fs::rename(&places.new, &places.app) {
        if had_one {
            fs::rename(&places.old, &places.app).context("putting the old helper back")?;
            restart_old(&mut start);
        }
        return Err(error).context("putting the new helper in place");
    }
    remove_dir_if_present(&places.old)?;

    let log = places.log.to_string_lossy().into_owned();
    let program = places.program().to_string_lossy().into_owned();
    let environment = agent.environment();
    let pairs: Vec<(&str, &str)> = environment
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    if let Some(parent) = places.agent.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&places.agent, launch_agent(&program, &log, &pairs))
        .with_context(|| format!("writing {}", places.agent.display()))?;

    bootstrap(&mut start)?;
    say(&format!("Logging to {log}"));
    Ok(places.app)
}

/// Starts the agent with `start`. Right after a bootout, launchd may still be tearing the old
/// job down and refuse the bootstrap for a moment: it is tried a few times.
///
/// # Errors
///
/// When every try failed, with `launchctl`'s words.
fn bootstrap(start: &mut Command) -> anyhow::Result<()> {
    let mut tries = 0;
    loop {
        tries += 1;
        let started = cmd::capture(start)?;
        if started.status.success() {
            return Ok(());
        }
        if tries >= BOOTSTRAP_TRIES {
            bail!(
                "`{}` failed ({}): {}",
                cmd::describe(start),
                started.status,
                started.combined().trim()
            );
        }
        std::thread::sleep(BOOTSTRAP_WAIT);
    }
}

/// Starts the old helper again after a failed swap (it was stopped for it); a failure is said,
/// as the swap's own error is the one returned.
fn restart_old(start: &mut Command) {
    if let Err(error) = bootstrap(start) {
        say(&format!("The old helper did not start again: {error:#}"));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Agent, Places, launchctl};
    use crate::cmd::describe;

    #[test]
    fn everything_sits_under_the_owner_s_home_and_nowhere_else() {
        let places = Places::under(Path::new("/Users/example"));
        assert_eq!(
            places.app,
            Path::new("/Users/example/Applications/Emberhut Hands.app")
        );
        assert_eq!(
            places.new,
            Path::new("/Users/example/Applications/.Emberhut Hands.app.new")
        );
        assert_eq!(
            places.agent,
            Path::new("/Users/example/Library/LaunchAgents/com.emberhut.hands.plist")
        );
        assert_eq!(
            places.program(),
            Path::new(
                "/Users/example/Applications/Emberhut Hands.app/Contents/MacOS/emberhut-hands"
            )
        );
    }

    #[test]
    fn the_agent_is_restarted_by_its_own_label() {
        let (stop, start) = launchctl("501", Path::new("/a/com.emberhut.hands.plist"));
        assert_eq!(
            describe(&stop),
            "/bin/launchctl bootout gui/501/com.emberhut.hands"
        );
        assert_eq!(
            describe(&start),
            "/bin/launchctl bootstrap gui/501 /a/com.emberhut.hands.plist"
        );
    }

    #[test]
    fn only_the_settings_given_are_set() {
        assert_eq!(Agent::default().environment(), []);
        let agent = Agent {
            listen: Some("10.0.0.5".into()),
            ..Agent::default()
        };
        assert_eq!(
            agent.environment(),
            [("EMBERHUT_HANDS_LISTEN", "10.0.0.5".to_owned())]
        );
    }
}
