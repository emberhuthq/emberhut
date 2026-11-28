//! Where each account's agent keeps its login, and what a session on it runs with: its
//! configuration folder (shared parts linked to the default folder, so a session moves between
//! accounts with its transcript), its environment, and the one place a project's memory lives
//! whichever account runs it. No token is read here: only whether one is there.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::list::{Account, Provider};

/// What a fresh account folder shares with the default one, so sessions, settings and skills follow it.
const SHARED: [&str; 7] = [
    "projects",
    "settings.json",
    "CLAUDE.md",
    "agents",
    "commands",
    "skills",
    "plugins",
];

/// Variables of the runner's own agent, login or provider that must never reach a session on
/// another account.
pub const SCRUBBED: [&str; 11] = [
    "CLAUDECODE",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

/// The environment a session on an account runs with, as changes to the runner's own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// Variables to set.
    pub set: Vec<(String, String)>,
    /// Variables to remove.
    pub remove: Vec<String>,
}

/// A secret of the runner's (a provider's key), or `None`.
#[must_use]
pub fn secret(secrets: &Path, file: Option<&str>) -> Option<String> {
    let file = file.filter(|f| !f.is_empty() && !f.contains(['/', '\\']) && !f.starts_with('.'))?;
    std::fs::read_to_string(secrets.join(file))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Logins: the default agent folder, and whether each account can run now.
#[derive(Debug)]
pub struct Logins {
    home: PathBuf,
    secrets: PathBuf,
    keychain: bool,
    seen: Mutex<HashMap<String, (bool, Instant)>>,
}

impl Logins {
    /// Logins under the user's `home`, keys in `secrets`. `keychain`: on macOS, ask the login
    /// Keychain (else only credential files count).
    #[must_use]
    pub fn new(home: PathBuf, secrets: PathBuf, keychain: bool) -> Self {
        Self {
            home,
            secrets,
            keychain,
            seen: Mutex::default(),
        }
    }

    /// The agent's default configuration folder.
    #[must_use]
    pub fn default_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }

    /// An account's configuration folder.
    #[must_use]
    pub fn dir_of(&self, a: &Account) -> PathBuf {
        a.config_dir.as_deref().map_or_else(
            || self.default_dir(),
            |d| crate::config::untilde(d, &self.home),
        )
    }

    /// The runner's secrets folder.
    #[must_use]
    pub fn secrets(&self) -> &Path {
        &self.secrets
    }

    /// Whether the account has its login or key. On macOS the Keychain's answer is kept a
    /// minute ([`Logins::refresh`] asks again).
    #[must_use]
    pub fn logged_in(&self, a: &Account) -> bool {
        match a.provider() {
            Provider::Claude => {}
            Provider::Api => return secret(&self.secrets, a.key_file()).is_some(),
            Provider::Bedrock => {
                return secret(&self.secrets, a.key_file()).is_some() && a.region.is_some();
            }
        }
        if self.dir_of(a).join(".credentials.json").exists() {
            return true;
        }
        if !self.keychain {
            return false;
        }
        self.seen
            .lock()
            .is_ok_and(|m| m.get(&a.id).is_some_and(|(ok, _)| *ok))
    }

    /// Asks the Keychain again about the accounts it was last asked about over a minute ago.
    pub async fn refresh(&self, accounts: &[Account]) {
        if !self.keychain {
            return;
        }
        for a in accounts.iter().filter(|a| a.provider() == Provider::Claude) {
            let fresh = self.seen.lock().is_ok_and(|m| {
                m.get(&a.id)
                    .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(60))
            });
            if fresh {
                continue;
            }
            let service =
                keychain_service(a.config_dir.as_ref().map(|_| self.dir_of(a)).as_deref());
            let ok = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::process::Command::new("/usr/bin/security")
                    .args(["find-generic-password", "-s", &service])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .status(),
            )
            .await
            .is_ok_and(|s| s.is_ok_and(|s| s.success()));
            if let Ok(mut m) = self.seen.lock() {
                m.insert(a.id.clone(), (ok, Instant::now()));
            }
        }
    }

    /// Forgets what the Keychain said about an account.
    pub fn forget(&self, id: &str) {
        if let Ok(mut m) = self.seen.lock() {
            m.remove(id);
        }
    }

    /// The environment a session on this account runs with: its folder, and for a provider its
    /// key and endpoint. Makes the account's folder ready first.
    #[must_use]
    pub fn env_for(&self, a: &Account, skills_dir: Option<&Path>) -> Env {
        self.prepare_dir(a, skills_dir);
        let mut env = Env {
            remove: SCRUBBED.iter().map(|s| (*s).to_owned()).collect(),
            set: Vec::new(),
        };
        // The default folder must run without CLAUDE_CONFIG_DIR, or the agent reads another file
        // for its settings.
        if a.config_dir.is_some() {
            env.set.push((
                "CLAUDE_CONFIG_DIR".to_owned(),
                self.dir_of(a).display().to_string(),
            ));
        } else {
            env.remove.push("CLAUDE_CONFIG_DIR".to_owned());
        }
        let key = secret(&self.secrets, a.key_file()).unwrap_or_default();
        match a.provider() {
            Provider::Claude => {}
            Provider::Api => match &a.base_url {
                Some(url) => {
                    env.set.push(("ANTHROPIC_BASE_URL".to_owned(), url.clone()));
                    env.set.push(("ANTHROPIC_AUTH_TOKEN".to_owned(), key));
                }
                None => env.set.push(("ANTHROPIC_API_KEY".to_owned(), key)),
            },
            Provider::Bedrock => {
                env.set
                    .push(("CLAUDE_CODE_USE_BEDROCK".to_owned(), "1".to_owned()));
                env.set.push(("AWS_BEARER_TOKEN_BEDROCK".to_owned(), key));
                env.set.push((
                    "AWS_REGION".to_owned(),
                    a.region.clone().unwrap_or_default(),
                ));
                // Its sub-agents' aliases (opus, sonnet, haiku) on the provider's ids.
                let models = a.models.clone().unwrap_or_default();
                for (family, var) in [
                    ("claude-opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                    ("claude-sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                    ("claude-haiku", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
                ] {
                    if let Some((_, id)) = models.iter().find(|(k, _)| k.starts_with(family)) {
                        env.set.push((var.to_owned(), id.clone()));
                    }
                }
            }
        }
        env.remove.retain(|k| !env.set.iter().any(|(s, _)| s == k));
        env
    }

    /// Makes an account's folder: a new one links the default folder's shared parts, so any
    /// account resumes any session. With a skills folder set, its skills folder is its own (each
    /// default skill linked one by one), so linking those skills never writes into the default
    /// folder's.
    pub fn prepare_dir(&self, a: &Account, skills_dir: Option<&Path>) -> PathBuf {
        let default = self.default_dir();
        if a.config_dir.is_none() {
            share_skills(&default, skills_dir);
            return default;
        }
        let dir = self.dir_of(a);
        let fresh = !dir.exists();
        if create_private_dir(&dir).is_err() {
            return dir;
        }
        if fresh || !dir.join("projects").exists() {
            for f in SHARED {
                let (from, to) = (default.join(f), dir.join(f));
                if !from.exists() || to.exists() {
                    continue;
                }
                if f == "skills" && skills_dir.is_some() {
                    if std::fs::create_dir_all(&to).is_ok() {
                        for k in read_names(&from) {
                            if !to.join(&k).exists() {
                                link(&from.join(&k), &to.join(&k));
                            }
                        }
                    }
                } else {
                    link(&from, &to);
                }
            }
        }
        share_skills(&dir, skills_dir);
        dir
    }

    /// Puts a session's transcript where this account's agent looks for it: found under any
    /// listed account's folder (the real file, not a link) and linked into this one's. False
    /// when no account has it yet (a session never started).
    #[must_use]
    pub fn link_session(&self, session: &str, a: &Account, all: &[Account]) -> bool {
        let dirs: BTreeSet<PathBuf> = all
            .iter()
            .map(|x| self.dir_of(x))
            .chain([self.default_dir()])
            .collect();
        let name = format!("{session}.jsonl");
        let mut found = None;
        'outer: for d in dirs.iter().filter(|d| d.join("projects").is_dir()) {
            for slug in read_names(&d.join("projects")) {
                let f = d.join("projects").join(&slug).join(&name);
                if f.symlink_metadata().is_ok_and(|m| m.is_file()) {
                    found = Some((slug, std::fs::canonicalize(&f).unwrap_or(f)));
                    break 'outer;
                }
            }
        }
        let Some((slug, file)) = found else {
            return false;
        };
        let mine = self.dir_of(a).join("projects").join(&slug).join(&name);
        if mine.exists() {
            return true;
        }
        if let Some(parent) = mine.parent()
            && std::fs::create_dir_all(parent).is_ok()
        {
            link(&file, &mine);
            // Its sub-agents' transcripts, in the folder beside it.
            let (sub, mine_sub) = (file.with_extension(""), mine.with_extension(""));
            if sub.exists() && !mine_sub.exists() {
                link(&sub, &mine_sub);
            }
        }
        mine.exists()
    }

    /// One memory per project on this machine, whichever account runs a session there: the
    /// default folder's, under the project's path (the git repository the folder is in, a
    /// worktree's main checkout, else the folder itself).
    pub async fn memory_dir(&self, cwd: &Path) -> PathBuf {
        let mut root = cwd.to_path_buf();
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await;
        if let Ok(Ok(o)) = out
            && o.status.success()
        {
            let common = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
            if common.file_name().is_some_and(|n| n == ".git")
                && let Some(p) = common.parent()
            {
                root = p.to_path_buf();
            }
        }
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        let slug: String = root
            .display()
            .to_string()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self
            .default_dir()
            .join("projects")
            .join(slug)
            .join("memory");
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }
}

/// The agent's Keychain item: `Claude Code-credentials` for the default folder,
/// `-<first 8 hex of sha256(folder)>` after it for others.
#[must_use]
pub fn keychain_service(dir: Option<&Path>) -> String {
    use sha2::Digest as _;
    match dir {
        None => "Claude Code-credentials".to_owned(),
        Some(d) => {
            let hash = sha2::Sha256::digest(d.display().to_string().as_bytes());
            let mut name = "Claude Code-credentials-".to_owned();
            for b in hash.iter().take(4) {
                use std::fmt::Write as _;
                write!(name, "{b:02x}").unwrap_or_default();
            }
            name
        }
    }
}

/// Links each skill of `skills_dir` into the folder's skills, once (a dangling link is made again).
pub fn share_skills(folder: &Path, skills_dir: Option<&Path>) {
    let Some(skills) = skills_dir.filter(|d| d.is_dir()) else {
        return;
    };
    let dir = folder.join("skills");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for s in read_names(skills) {
        let to = dir.join(&s);
        if to.exists() {
            continue;
        }
        if to.symlink_metadata().is_ok() {
            // A dangling link: made again below.
            std::fs::remove_file(&to).unwrap_or_default();
        }
        link(&skills.join(&s), &to);
    }
}

fn read_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        },
    )
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// A symbolic link at `to` pointing at `from`; nothing when the system refuses one (Windows
/// without the right to make links).
fn link(from: &Path, to: &Path) {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(from, to);
    #[cfg(windows)]
    let made = if from.is_dir() {
        std::os::windows::fs::symlink_dir(from, to)
    } else {
        std::os::windows::fs::symlink_file(from, to)
    };
    #[cfg(not(any(unix, windows)))]
    let made: std::io::Result<()> = Err(std::io::Error::other("no links here"));
    if let Err(e) = made {
        tracing::debug!(from = %from.display(), to = %to.display(), "no link: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, dir: Option<&str>) -> Account {
        Account {
            id: id.to_owned(),
            name: id.to_owned(),
            config_dir: dir.map(str::to_owned),
            ..Account::default()
        }
    }

    #[test]
    fn a_session_on_another_account_finds_its_transcript() {
        let home = tempfile::tempdir().unwrap();
        let logins = Logins::new(
            home.path().to_path_buf(),
            home.path().join("secrets"),
            false,
        );
        let default = home.path().join(".claude/projects/-w-app");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("s1.jsonl"), "{}\n").unwrap();
        let b = account("b", Some("~/.agent-b"));
        logins.prepare_dir(&b, None);
        assert!(
            home.path().join(".agent-b/projects").exists(),
            "a new folder shares the transcripts"
        );
        let c = account("c", Some("~/.agent-c"));
        std::fs::create_dir_all(home.path().join(".agent-c/projects")).unwrap();
        assert!(logins.link_session("s1", &c, &[account("default", None), b, c.clone()]));
        assert!(
            home.path()
                .join(".agent-c/projects/-w-app/s1.jsonl")
                .exists()
        );
        assert!(!logins.link_session("nope", &c, &[]));
    }

    #[test]
    fn an_account_runs_with_its_own_folder_and_key_and_nothing_else() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("secrets")).unwrap();
        std::fs::write(home.path().join("secrets/aws-bedrock-api-key"), "k1\n").unwrap();
        let logins = Logins::new(
            home.path().to_path_buf(),
            home.path().join("secrets"),
            false,
        );
        let aws = Account {
            provider: Some(Provider::Bedrock),
            region: Some("us-east-1".to_owned()),
            ..account("aws", Some("~/.agent-aws"))
        };
        assert!(logins.logged_in(&aws));
        let env = logins.env_for(&aws, None);
        assert!(
            env.set
                .contains(&("AWS_BEARER_TOKEN_BEDROCK".to_owned(), "k1".to_owned()))
        );
        assert!(
            env.remove.contains(&"ANTHROPIC_API_KEY".to_owned())
                && !env.remove.contains(&"CLAUDE_CODE_USE_BEDROCK".to_owned())
        );
        let def = logins.env_for(&account("d", None), None);
        assert!(def.remove.contains(&"CLAUDE_CONFIG_DIR".to_owned()));
        assert!(!logins.logged_in(&account("d", None)));
    }

    #[test]
    fn keychain_items_are_named_as_the_agent_names_them() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        assert_eq!(
            keychain_service(Some(Path::new("/x"))).len(),
            "Claude Code-credentials-".len() + 8
        );
    }
}
