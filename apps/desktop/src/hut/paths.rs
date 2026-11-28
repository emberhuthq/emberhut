//! Where the hut's files are, where its server binary and the agent CLI are found, and which
//! service manager keeps it lit: all worked out from a [`Place`] (the home folder, the
//! environment and the app's own executable), so the tests give a temporary one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The server binary's file name.
pub(crate) const SERVER: &str = "emberhut-server";
/// The variable that names a server binary when none is in the app's bundle or beside it.
pub(crate) const SERVER_ENV: &str = "EMBERHUT_SERVER_BIN";
/// The agent CLI the hut runs by default.
const AGENT: &str = "claude";
/// The launch agent's label and the name of its plist (macOS).
pub(crate) const LAUNCH_AGENT: &str = "com.emberhut.hut";
/// The systemd user unit (Linux).
pub(crate) const SYSTEMD_UNIT: &str = "emberhut-hut.service";
/// The variable that, set to `none` in a debug or automation build, leaves the service manager
/// out: the hut then runs only as the app's child, and keeping it lit without the app is
/// refused. A test run on a real computer sets it, so no launch agent or unit is ever written
/// or loaded for the user.
pub(crate) const SERVICE_ENV: &str = "EMBERHUT_HUT_SERVICE";

/// Which system the hut is on, for its folders and what keeps it lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Os {
    /// macOS: `~/Library`, and a launch agent.
    Mac,
    /// Linux: the XDG folders, and a systemd user unit.
    Linux,
    /// Windows: the user's local application data folder, and no service manager yet.
    Windows,
    /// Anything else: the XDG layout under the home folder, and no service manager.
    Other,
}

impl Os {
    /// The system this copy was built for.
    #[must_use]
    pub(crate) fn this() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Reads an environment variable.
pub(crate) type Vars = Box<dyn Fn(&str) -> Option<OsString> + Send + Sync>;

/// What the paths are worked out from.
pub(crate) struct Place {
    /// The system.
    pub(crate) os: Os,
    /// The user's home folder.
    pub(crate) home: PathBuf,
    /// The app's own executable, when known.
    pub(crate) exe: Option<PathBuf>,
    /// The user's id, for `launchctl`'s `gui/<uid>` domain (Unix).
    pub(crate) uid: Option<u32>,
    /// Reads an environment variable.
    pub(crate) var: Vars,
}

impl Place {
    /// This process's place: the home folder ([`HOME_VAR`]), its environment and its
    /// executable. `None` without a home folder.
    #[must_use]
    pub(crate) fn system() -> Option<Self> {
        let home = std::env::var_os(HOME_VAR)
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)?;
        Some(Self {
            os: Os::this(),
            home,
            exe: std::env::current_exe().ok(),
            uid: uid(),
            var: Box::new(|name: &str| std::env::var_os(name)),
        })
    }
}

/// The variable that names the user's home folder: `HOME`, or on Windows `USERPROFILE`
/// (`C:\Users\<name>`; Windows sets no `HOME` itself).
pub(crate) const HOME_VAR: &str = if cfg!(windows) { "USERPROFILE" } else { "HOME" };

/// This process's user id.
#[cfg(unix)]
#[expect(clippy::unnecessary_wraps, reason = "there is none off Unix")]
fn uid() -> Option<u32> {
    Some(rustix::process::getuid().as_raw())
}

/// This process's user id (none off Unix).
#[cfg(not(unix))]
fn uid() -> Option<u32> {
    None
}

/// What keeps the hut lit while the app is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Manager {
    /// launchd: the agent's plist, and the user's `gui/<uid>` domain.
    Launchd {
        /// `~/Library/LaunchAgents/com.emberhut.hut.plist`.
        plist: PathBuf,
        /// The user's id.
        uid: u32,
    },
    /// systemd's user instance: the unit file.
    Systemd {
        /// `~/.config/systemd/user/emberhut-hut.service`.
        unit: PathBuf,
    },
    /// None: the hut runs only as the app's child.
    None,
}

/// The hut's places on this computer.
pub(crate) struct HutPaths {
    home: PathBuf,
    exe: Option<PathBuf>,
    data: PathBuf,
    log: PathBuf,
    manager: Manager,
    var: Vars,
}

impl HutPaths {
    /// The places for `place`:
    ///
    /// - the data folder: macOS `~/Library/Application Support/Emberhut/Hut`, Windows
    ///   `%LOCALAPPDATA%\Emberhut\Hut`, elsewhere `$XDG_DATA_HOME/emberhut/hut`
    ///   (`~/.local/share/emberhut/hut`);
    /// - the log: macOS `~/Library/Logs/Emberhut/hut.log`, Windows
    ///   `%LOCALAPPDATA%\Emberhut\Logs\hut.log`, elsewhere `$XDG_STATE_HOME/emberhut/hut.log`
    ///   (`~/.local/state/emberhut/hut.log`);
    /// - the service: macOS the launch agent `~/Library/LaunchAgents/com.emberhut.hut.plist`,
    ///   Linux the unit `$XDG_CONFIG_HOME/systemd/user/emberhut-hut.service` (none when
    ///   [`SERVICE_ENV`] says so), Windows none yet: the hut runs as the app's child there.
    #[must_use]
    pub(crate) fn new(place: Place) -> Self {
        let Place {
            os,
            home,
            exe,
            uid,
            var,
        } = place;
        let xdg = |name: &str, fallback: &[&str]| {
            var(name)
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| fallback.iter().fold(home.clone(), |dir, at| dir.join(at)))
        };
        let no_service = super::READS_TEST_VARS
            && var(SERVICE_ENV).is_some_and(|value| value.eq_ignore_ascii_case("none"));
        let (data, log, manager) = match os {
            Os::Mac => {
                let library = home.join("Library");
                let manager = uid.map_or(Manager::None, |uid| Manager::Launchd {
                    plist: library
                        .join("LaunchAgents")
                        .join(format!("{LAUNCH_AGENT}.plist")),
                    uid,
                });
                (
                    library
                        .join("Application Support")
                        .join("Emberhut")
                        .join("Hut"),
                    library.join("Logs").join("Emberhut").join("hut.log"),
                    manager,
                )
            }
            Os::Windows => {
                // The folder the system keeps per user and per machine (not roamed): a
                // database and its log don't belong in the roaming profile.
                let local = var("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .filter(|dir| dir.is_absolute())
                    .unwrap_or_else(|| home.join("AppData").join("Local"))
                    .join("Emberhut");
                (
                    local.join("Hut"),
                    local.join("Logs").join("hut.log"),
                    Manager::None,
                )
            }
            Os::Linux | Os::Other => {
                let manager = if os == Os::Linux {
                    Manager::Systemd {
                        unit: xdg("XDG_CONFIG_HOME", &[".config"])
                            .join("systemd")
                            .join("user")
                            .join(SYSTEMD_UNIT),
                    }
                } else {
                    Manager::None
                };
                (
                    xdg("XDG_DATA_HOME", &[".local", "share"])
                        .join("emberhut")
                        .join("hut"),
                    xdg("XDG_STATE_HOME", &[".local", "state"])
                        .join("emberhut")
                        .join("hut.log"),
                    manager,
                )
            }
        };
        let manager = if no_service { Manager::None } else { manager };
        Self {
            home,
            exe,
            data,
            log,
            manager,
            var,
        }
    }

    /// The variable `name` as this place reads it.
    #[must_use]
    pub(crate) fn var(&self, name: &str) -> Option<std::ffi::OsString> {
        (self.var)(name)
    }

    /// The data folder: the database, the machine's key, the secrets and the hut's own state.
    #[must_use]
    pub(crate) fn data(&self) -> &Path {
        &self.data
    }

    /// The database, `emberhut.db`.
    #[must_use]
    pub(crate) fn database(&self) -> PathBuf {
        self.data.join("emberhut.db")
    }

    /// The machine's key, `machine.key` beside the database (the server's default).
    #[must_use]
    pub(crate) fn machine_key(&self) -> PathBuf {
        self.data.join("machine.key")
    }

    /// The secrets folder.
    #[must_use]
    pub(crate) fn secrets(&self) -> PathBuf {
        self.data.join("secrets")
    }

    /// What the hut remembers: its port, the server it was started with, its process.
    #[must_use]
    pub(crate) fn state(&self) -> PathBuf {
        self.data.join("hut.json")
    }

    /// The hut's log.
    #[must_use]
    pub(crate) fn log(&self) -> &Path {
        &self.log
    }

    /// What keeps the hut lit while the app is closed.
    #[must_use]
    pub(crate) fn manager(&self) -> &Manager {
        &self.manager
    }

    /// The server binary: in the app's bundle (macOS: `Emberhut.app/Contents/MacOS/`, beside
    /// the app's own binary, where `cargo xtask bundle-macos` puts it), else beside the
    /// app's executable (a build from source: `target/<profile>/`), else the file
    /// `EMBERHUT_SERVER_BIN` names. Only an executable file counts.
    #[must_use]
    pub(crate) fn server(&self) -> Option<PathBuf> {
        let name = format!("{SERVER}{}", std::env::consts::EXE_SUFFIX);
        let bundled = self
            .exe
            .as_deref()
            .and_then(bundle_of)
            .map(|bundle| bundle.join("Contents").join("MacOS").join(&name));
        let beside = self
            .exe
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join(&name));
        let named = (self.var)(SERVER_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        [bundled, beside, named]
            .into_iter()
            .flatten()
            .find(|path| is_executable(path))
    }

    /// The agent CLI, `claude`: the first on the app's `PATH`, else in the places its
    /// installers use (`~/.local/bin`, `~/.claude/local`, Homebrew's, `/usr/local/bin`): an
    /// app opened from the Finder or a desktop launcher gets a short `PATH`.
    #[must_use]
    pub(crate) fn agent(&self) -> Option<PathBuf> {
        let name = format!("{AGENT}{}", std::env::consts::EXE_SUFFIX);
        let known = [
            self.home.join(".local").join("bin"),
            self.home.join(".claude").join("local"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        self.path_dirs()
            .into_iter()
            .chain(known)
            .map(|dir| dir.join(&name))
            .find(|path| is_executable(path))
    }

    /// The `PATH` the hut runs with: the agent's folder first (so the CLI finds its own
    /// tools), then the app's `PATH`, or the system's folders when the app has none.
    #[must_use]
    pub(crate) fn hut_path(&self, agent: Option<&Path>) -> OsString {
        let mut dirs: Vec<PathBuf> = agent
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
        let path = self.path_dirs();
        if path.is_empty() {
            dirs.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
        } else {
            for dir in path {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
        std::env::join_paths(dirs).unwrap_or_default()
    }

    /// The folders of the app's `PATH`.
    fn path_dirs(&self) -> Vec<PathBuf> {
        (self.var)("PATH").map_or_else(Vec::new, |path| std::env::split_paths(&path).collect())
    }
}

/// The `.app` bundle `exe` runs from, when it is a bundle's `Contents/MacOS/<binary>`.
fn bundle_of(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then_some(bundle)
}

/// Whether `path` is a file this user may run.
fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    // Windows runs a file by its extension: only a program (`.exe`) counts.
    #[cfg(not(unix))]
    {
        metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    }
}

/// The `sqlite://` URL of the database at `path`, percent-encoded so a space (`Application
/// Support`), `?` or `#` in it is read as part of the path. `None` for a path that isn't UTF-8.
#[must_use]
pub(crate) fn database_url(path: &Path) -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let text = path.to_str()?;
    let mut url = String::from("sqlite://");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push('%');
            url.push(char::from(HEX[usize::from(byte >> 4)]));
            url.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    Some(url)
}
