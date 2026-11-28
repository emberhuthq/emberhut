//! The runner's settings: where its files live, its name on the server, the server's address,
//! its token, the port it listens on, the worktrees, the account groups and the instructions
//! its sessions get.
//!
//! Everything comes from one JSON file in the runner's home (`config.json`), with a few
//! environment variables on top for service managers and tests. Nothing about one
//! installation is written in the code.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

/// The settings file's name in the runner's home.
pub const CONFIG_FILE: &str = "config.json";
/// The runner's API port when none is set.
pub const DEFAULT_PORT: u16 = 8806;
/// The largest file taken in or sent out when none is set: 200 MiB.
pub const DEFAULT_MAX_FILE: u64 = 200 * 1024 * 1024;
/// How often a machine sample goes to the server when none is set, in milliseconds.
pub const DEFAULT_MACHINE_EVERY_MS: u64 = 20_000;

/// A runner name the server takes: letters, digits, `-` and `_`, 1 to 40 of them.
static NAME: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,40}$").ok());

/// Whether `name` is one the server takes as a runner's name.
#[must_use]
pub fn is_runner_name(name: &str) -> bool {
    NAME.as_ref().is_some_and(|re| re.is_match(name))
}

/// Work that belongs to a group (a client, a team): a session is in it when its project or the
/// folder it was asked to start in matches.
#[derive(Debug, Clone)]
pub struct Group {
    /// Matched case-insensitively against the session's project name.
    pub projects: Option<Regex>,
    /// Matched case-insensitively against the folder the session was asked to start in.
    pub folders: Option<Regex>,
}

impl Group {
    /// Whether work with this project and folder is in the group.
    #[must_use]
    pub fn holds(&self, project: Option<&str>, folder: Option<&str>) -> bool {
        let hit = |re: &Option<Regex>, s: Option<&str>| matches!((re, s), (Some(re), Some(s)) if re.is_match(s));
        hit(&self.projects, project) || hit(&self.folders, folder)
    }
}

/// Text added to every session's system prompt, after the agent's own.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Instructions {
    /// For sessions the assistant opens for work (replaces the default).
    pub work: Option<String>,
    /// For the owner's own sessions, opened and driven from an app (replaces the default).
    pub direct: Option<String>,
    /// About this machine (what it has, what to leave alone), for both.
    pub machine: Option<String>,
}

/// The runner's settings, resolved: every path absolute, every default filled in.
#[derive(Debug, Clone)]
pub struct Config {
    /// The runner's home: its database, accounts file, inboxes and secrets.
    pub home: PathBuf,
    /// The user's home folder (where `~` points, and the agent's default folder lives).
    pub user: PathBuf,
    /// The runner's name on the server; the server knows its runners by it.
    pub name: String,
    /// The server's address, or `None` before pairing.
    pub server: Option<String>,
    /// The file holding the runner's token, which both sides present to each other.
    pub token_file: PathBuf,
    /// The port the runner's API listens on (0: one the system picks).
    pub port: u16,
    /// The addresses it listens on.
    pub listen: Vec<IpAddr>,
    /// The address the server calls it at, when pairing (`None`: the first listen address).
    pub advertise: Option<String>,
    /// The accounts file.
    pub accounts_file: PathBuf,
    /// Where session worktrees are made: `<worktrees_dir>/<repo>/<branch>`.
    pub worktrees_dir: PathBuf,
    /// The prefix of the branch a session's worktree gets when it names none.
    pub branch_prefix: String,
    /// The model sessions run on when they name none (`None`: the agent's own default).
    pub model: Option<String>,
    /// The account that goes first for any work it may take.
    pub default_account: Option<String>,
    /// Work groups by name.
    pub groups: BTreeMap<String, Group>,
    /// Text added to the sessions' system prompts.
    pub instructions: Instructions,
    /// A folder of skills linked into every account's skills folder.
    pub skills_dir: Option<PathBuf>,
    /// How often a machine sample goes to the server.
    pub machine_every: std::time::Duration,
    /// The largest file taken in or sent out, in bytes.
    pub max_file_bytes: u64,
    /// Whether sessions get the MCP servers the server hands out.
    pub share_mcp: bool,
    /// macOS: launchd labels whose jobs the Background jobs page lists.
    pub job_labels: Option<Regex>,
    /// The agent's command (`claude` on the `PATH` when not set).
    pub agent_command: PathBuf,
}

/// A setting that is wrong, with the key it came from.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The settings file can't be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The settings file isn't the JSON object it should be.
    #[error("{path} is not a settings object: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// One setting is wrong.
    #[error("{key}: {why}")]
    Invalid {
        /// The setting's key (or environment variable).
        key: String,
        /// What is wrong with it.
        why: String,
    },
    /// No home folder to put the runner's files in.
    #[error("no home folder: set EMBERHUT_RUNNER_HOME")]
    NoHome,
}

fn invalid(key: &str, why: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
        why: why.into(),
    }
}

/// The settings file as written. Unknown keys are ignored, so a file another version wrote (or
/// one that also holds an older runner's keys) still loads.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    name: Option<String>,
    server: Option<String>,
    token_file: Option<String>,
    port: Option<u16>,
    listen: Option<Vec<String>>,
    advertise: Option<String>,
    accounts_file: Option<String>,
    worktrees_dir: Option<String>,
    branch_prefix: Option<String>,
    model: Option<String>,
    default_account: Option<String>,
    groups: Option<BTreeMap<String, GroupFile>>,
    instructions: Option<Instructions>,
    skills_dir: Option<String>,
    machine_every: Option<u64>,
    max_file_bytes: Option<u64>,
    share_mcp: Option<bool>,
    job_labels: Option<String>,
    agent_command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GroupFile {
    projects: Option<String>,
    folders: Option<String>,
}

/// The user's home folder, from the environment.
#[must_use]
pub fn user_home(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("HOME")
        .or_else(|| env("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `~` or `~/…` as the home folder.
#[must_use]
pub fn untilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => home.join(&rest[1..]),
        _ => PathBuf::from(p),
    }
}

/// The runner's home when `EMBERHUT_RUNNER_HOME` doesn't say: the platform's usual place for an
/// app's own data.
#[must_use]
pub fn default_home(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Emberhut Runner")
    } else if cfg!(windows) {
        env("LOCALAPPDATA")
            .map_or_else(|| home.join("AppData").join("Local"), PathBuf::from)
            .join("Emberhut Runner")
    } else {
        env("XDG_DATA_HOME")
            .map_or_else(|| home.join(".local/share"), PathBuf::from)
            .join("emberhut-runner")
    }
}

/// This machine's name as a runner name.
#[must_use]
pub fn machine_name(hostname: &str) -> String {
    let short = hostname
        .split('.')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let mut name: String = short
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    name = name.trim_matches('-').chars().take(40).collect();
    if name.is_empty() {
        "runner".to_owned()
    } else {
        name
    }
}

fn regex(key: &str, s: &str) -> Result<Regex, ConfigError> {
    regex::RegexBuilder::new(s)
        .case_insensitive(true)
        .build()
        .map_err(|e| invalid(key, format!("not a regular expression: {e}")))
}

impl Config {
    /// The settings: `<home>/config.json`, then the environment (`EMBERHUT_RUNNER_HOME`,
    /// `EMBERHUT_RUNNER_NAME`, `EMBERHUT_SERVER`, `EMBERHUT_RUNNER_PORT`,
    /// `EMBERHUT_RUNNER_TOKEN_FILE`, `EMBERHUT_RUNNER_ACCOUNTS`, `EMBERHUT_RUNNER_AGENT`), then the
    /// defaults.
    ///
    /// # Errors
    /// A file that can't be read or parsed, or a setting that is wrong.
    pub fn load(env: &dyn Fn(&str) -> Option<String>, hostname: &str) -> Result<Self, ConfigError> {
        let user = user_home(env).ok_or(ConfigError::NoHome)?;
        let home = env("EMBERHUT_RUNNER_HOME")
            .map_or_else(|| default_home(&user, env), |h| untilde(&h, &user));
        let f = read_file(&home)?;
        let path = |v: Option<String>, fallback: &str| -> PathBuf {
            let p = untilde(v.as_deref().unwrap_or(fallback), &user);
            if p.is_absolute() { p } else { home.join(p) }
        };

        let name = env("EMBERHUT_RUNNER_NAME")
            .or(f.name)
            .unwrap_or_else(|| machine_name(hostname));
        if !is_runner_name(&name) {
            return Err(invalid(
                "name",
                format!("{name:?}: letters, digits, - and _ only (40 at most)"),
            ));
        }
        let server = env("EMBERHUT_SERVER")
            .or(f.server)
            .map(|s| s.trim_end_matches('/').to_owned());
        if let Some(s) = &server {
            check_url("server", s)?;
        }
        let port = match env("EMBERHUT_RUNNER_PORT") {
            Some(p) => p
                .parse()
                .map_err(|_| invalid("EMBERHUT_RUNNER_PORT", "a port, 0-65535"))?,
            None => f.port.unwrap_or(DEFAULT_PORT),
        };
        let listen = f
            .listen
            .unwrap_or_else(|| vec!["127.0.0.1".to_owned()])
            .iter()
            .map(|a| {
                a.parse::<IpAddr>()
                    .map_err(|_| invalid("listen", format!("{a:?} is not an IP address")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if listen.is_empty() {
            return Err(invalid("listen", "at least one address"));
        }
        let advertise = f.advertise.map(|a| a.trim_end_matches('/').to_owned());
        if let Some(a) = &advertise {
            check_url("advertise", a)?;
        }
        let mut groups = BTreeMap::new();
        for (k, g) in f.groups.unwrap_or_default() {
            let projects = g
                .projects
                .as_deref()
                .map(|r| regex(&format!("groups.{k}.projects"), r))
                .transpose()?;
            let folders = g
                .folders
                .as_deref()
                .map(|r| regex(&format!("groups.{k}.folders"), r))
                .transpose()?;
            groups.insert(k, Group { projects, folders });
        }
        let machine_every = f.machine_every.unwrap_or(DEFAULT_MACHINE_EVERY_MS);
        if machine_every < 1000 {
            return Err(invalid("machineEvery", "1000 ms at least"));
        }
        Ok(Self {
            name,
            server,
            token_file: path(env("EMBERHUT_RUNNER_TOKEN_FILE").or(f.token_file), "token"),
            port,
            listen,
            advertise,
            accounts_file: path(
                env("EMBERHUT_RUNNER_ACCOUNTS").or(f.accounts_file),
                "accounts.json",
            ),
            worktrees_dir: untilde(
                f.worktrees_dir
                    .as_deref()
                    .unwrap_or("~/Documents/worktrees"),
                &user,
            ),
            branch_prefix: f.branch_prefix.unwrap_or_else(|| "emberhut/".to_owned()),
            model: f.model.filter(|m| !m.is_empty()),
            default_account: f.default_account.filter(|a| !a.is_empty() && a != "auto"),
            groups,
            instructions: f.instructions.unwrap_or_default(),
            skills_dir: f.skills_dir.map(|s| path(Some(s), "")),
            machine_every: std::time::Duration::from_millis(machine_every),
            max_file_bytes: f.max_file_bytes.unwrap_or(DEFAULT_MAX_FILE),
            share_mcp: f.share_mcp.unwrap_or(true),
            job_labels: f
                .job_labels
                .as_deref()
                .map(|r| regex("jobLabels", r))
                .transpose()?,
            agent_command: env("EMBERHUT_RUNNER_AGENT")
                .or(f.agent_command)
                .map_or_else(|| PathBuf::from("claude"), |c| untilde(&c, &user)),
            home,
            user,
        })
    }

    /// The runner's token, or `None` when its file is missing or empty.
    #[must_use]
    pub fn token(&self) -> Option<String> {
        std::fs::read_to_string(&self.token_file)
            .ok()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    }

    /// The address the server calls this runner at.
    #[must_use]
    pub fn advertised(&self) -> String {
        self.advertise.clone().unwrap_or_else(|| {
            let first = self
                .listen
                .first()
                .copied()
                .unwrap_or(IpAddr::from([127, 0, 0, 1]));
            match first {
                IpAddr::V4(a) => format!("http://{a}:{}", self.port),
                IpAddr::V6(a) => format!("http://[{a}]:{}", self.port),
            }
        })
    }

    /// The runner's secrets folder (a provider's key, for instance), inside its home.
    #[must_use]
    pub fn secrets_dir(&self) -> PathBuf {
        self.home.join("secrets")
    }

    /// A session's inbox, where files sent to it land.
    #[must_use]
    pub fn inbox(&self, session: &str) -> PathBuf {
        self.home.join("inbox").join(session)
    }
}

/// Writes a file only its owner may read (mode 600 on Unix), replacing it whole.
///
/// # Errors
/// It can't be written.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(&tmp)?;
        std::io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn check_url(key: &str, s: &str) -> Result<(), ConfigError> {
    let ok = url::Url::parse(s)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some());
    if ok {
        Ok(())
    } else {
        Err(invalid(
            key,
            format!("{s:?} must be an http:// or https:// address"),
        ))
    }
}

fn read_file(home: &Path) -> Result<Stored, ConfigError> {
    let path = home.join(CONFIG_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|source| ConfigError::Parse { path, source })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(source) => Err(ConfigError::Read { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn defaults_fill_in_without_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        let c = Config::load(
            &env_of(&[("HOME", home), ("EMBERHUT_RUNNER_HOME", home)]),
            "Build-Box.local",
        )
        .unwrap();
        assert_eq!(c.name, "build-box");
        assert_eq!(c.port, DEFAULT_PORT);
        assert_eq!(c.token_file, dir.path().join("token"));
        assert_eq!(c.advertised(), "http://127.0.0.1:8806");
        assert!(c.server.is_none() && c.share_mcp);
    }

    #[test]
    fn a_file_with_another_runners_keys_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{"runnerPort": 1, "name": "build", "server": "https://hut.example.com/", "listen": ["127.0.0.1", "::1"],
               "groups": {"client": {"folders": "client-work"}}, "tokenFile": "secrets/key"}"#,
        )
        .unwrap();
        let home = dir.path().to_str().unwrap();
        let c = Config::load(
            &env_of(&[("HOME", home), ("EMBERHUT_RUNNER_HOME", home)]),
            "x",
        )
        .unwrap();
        assert_eq!(c.server.as_deref(), Some("https://hut.example.com"));
        assert_eq!(c.listen.len(), 2);
        assert_eq!(c.token_file, dir.path().join("secrets/key"));
        assert!(c.groups["client"].holds(None, Some("/w/client-work/app")));
        assert!(!c.groups["client"].holds(Some("other"), Some("/w/app")));
    }

    #[test]
    fn wrong_settings_say_which() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), r#"{"name": "no spaces"}"#).unwrap();
        let e = Config::load(
            &env_of(&[("HOME", home), ("EMBERHUT_RUNNER_HOME", home)]),
            "x",
        )
        .unwrap_err();
        assert!(e.to_string().starts_with("name:"), "{e}");
        std::fs::write(dir.path().join(CONFIG_FILE), r#"{"server": "ftp://x"}"#).unwrap();
        let e = Config::load(
            &env_of(&[("HOME", home), ("EMBERHUT_RUNNER_HOME", home)]),
            "x",
        )
        .unwrap_err();
        assert!(e.to_string().starts_with("server:"), "{e}");
    }
}
