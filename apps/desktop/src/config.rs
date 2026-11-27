//! Where the app keeps its files, and which server it talks to.
//!
//! The files live in the platform's config directory: on macOS
//! `~/Library/Application Support/com.emberhut.desktop`, named after the bundle id as the system
//! expects; elsewhere `emberhut` in the config directory (`$XDG_CONFIG_HOME/emberhut`, usually
//! `~/.config/emberhut`, on Linux; `%APPDATA%\emberhut` on Windows). The server's address comes from
//! `EMBERHUT_URL`, else `config.json`'s `server`, else [`DEFAULT_SERVER`].

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::brand::APP_ID;
use crate::wire::DEFAULT_SERVER;

/// The environment variable that names the server.
pub(crate) const SERVER_ENV: &str = "EMBERHUT_URL";

/// A server's base address: `http` or `https`, no trailing slash.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ServerUrl(String);

/// Why a server address was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a server address: it must start with http:// or https://")]
pub(crate) struct BadServerUrl(String);

impl ServerUrl {
    /// Checks an address and drops its trailing slashes.
    ///
    /// # Errors
    ///
    /// [`BadServerUrl`] when it is neither `http://…` nor `https://…`.
    pub(crate) fn parse(address: &str) -> Result<Self, BadServerUrl> {
        let address = address.trim().trim_end_matches('/');
        let rest = address
            .strip_prefix("http://")
            .or_else(|| address.strip_prefix("https://"));
        match rest {
            Some(host) if !host.is_empty() => Ok(Self(address.to_owned())),
            _ => Err(BadServerUrl(address.to_owned())),
        }
    }

    /// The address of a path on this server (`path` starts with `/`).
    #[must_use]
    pub(crate) fn join(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }

    /// The address as text.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `url` is a path on the server: the only kind of link the app fetches itself,
    /// with this device's token. The rule is emberhut-client's ([`emberhut_client::is_server_path`]),
    /// so what the app takes for a server path is exactly what the client fetches.
    #[must_use]
    pub(crate) fn is_path(url: &str) -> bool {
        emberhut_client::is_server_path(url)
    }

    /// A link from a message as the browser may open it: a server path made absolute (with
    /// its fragment, which is the browser's), an `http`/`https` URL as it is; `None` for
    /// anything else (a `file:`, `smb:` or app scheme in a message is never handed to the
    /// system).
    #[must_use]
    pub(crate) fn link(&self, url: &str) -> Option<String> {
        let path = url.split_once('#').map_or(url, |(path, _)| path);
        if Self::is_path(path) {
            return Some(self.join(url));
        }
        let scheme = url
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        if matches!(scheme.as_deref(), Some("http" | "https")) {
            Some(url.to_owned())
        } else {
            log::warn!("not opening a link that isn't a web address: {url}");
            None
        }
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `config.json`, the settings a user writes by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct ConfigFile {
    /// The server's address.
    pub(crate) server: Option<String>,
}

/// The app's own files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppFiles {
    dir: PathBuf,
}

impl AppFiles {
    /// The platform's config directory for this app, or `None` when the system has no home.
    #[must_use]
    pub(crate) fn locate() -> Option<Self> {
        let base = directories::BaseDirs::new()?;
        Some(Self {
            dir: base.config_dir().join(dir_name()),
        })
    }

    /// The files in another directory (the tests, `--self-check`).
    pub(crate) fn in_dir(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The directory (`window.json`, the windows' frame and the sidebar's folds, is kept in it:
    /// see `prefs`).
    #[must_use]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// `view.json`: how the window was left (the open room, the sidebar's folds and width), as
    /// the store keeps it. The report panel's width is the window's, in [`Self::layout`]: each
    /// file has one writer, so neither write can lose the other's keys.
    #[must_use]
    pub(crate) fn view(&self) -> PathBuf {
        self.dir.join("view.json")
    }

    /// `config.json`: the user's settings.
    #[must_use]
    pub(crate) fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    /// `session.json`: this device's session (kept readable by the user only).
    #[must_use]
    pub(crate) fn session(&self) -> PathBuf {
        self.dir.join("session.json")
    }

    /// `theme.json`: the last theme the server sent, to start in it offline.
    #[must_use]
    pub(crate) fn theme(&self) -> PathBuf {
        self.dir.join("theme.json")
    }

    /// `layout.json`: how the window was laid out (the report panel's width), to open the same
    /// way next time.
    #[must_use]
    pub(crate) fn layout(&self) -> PathBuf {
        self.dir.join("layout.json")
    }

    /// `filters.json`: the lists' filters picked on this device (the Tasks list's).
    #[must_use]
    pub(crate) fn filters(&self) -> PathBuf {
        self.dir.join("filters.json")
    }

    /// `chat-put-away.json`: what the user put away over the composer (ask cards, Main's line).
    #[must_use]
    pub(crate) fn put_away(&self) -> PathBuf {
        self.dir.join("chat-put-away.json")
    }

    /// `last-launch-error.txt`: why the last launch failed, for an app started without a
    /// terminal.
    #[must_use]
    pub(crate) fn launch_error(&self) -> PathBuf {
        self.dir.join("last-launch-error.txt")
    }

    /// Writes why the launch failed to [`AppFiles::launch_error`] (a failure to write is
    /// logged; the app is exiting anyway).
    pub(crate) fn note_launch_failure(&self, error: &anyhow::Error) {
        let path = self.launch_error();
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&path, format!("{error:#}\n")));
        if let Err(write_error) = written {
            log::warn!("can't write {}: {write_error}", path.display());
        }
    }

    /// Reads `config.json`; a missing file is an empty one, an unreadable one is logged.
    #[must_use]
    pub(crate) fn read_config(&self) -> ConfigFile {
        let path = self.config();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("{} isn't valid JSON ({error}); ignoring it", path.display());
                ConfigFile::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigFile::default(),
            Err(error) => {
                log::warn!("can't read {} ({error}); ignoring it", path.display());
                ConfigFile::default()
            }
        }
    }
}

/// The platform's cache directory for this app (`~/Library/Caches/com.emberhut.desktop` on macOS,
/// `$XDG_CACHE_HOME/emberhut` on Linux, `%LOCALAPPDATA%\emberhut` on Windows): what the app keeps only
/// to start faster, which the system may clear. `None` when the system has no home.
#[must_use]
pub(crate) fn cache_dir() -> Option<PathBuf> {
    Some(directories::BaseDirs::new()?.cache_dir().join(dir_name()))
}

/// The app's directory's name in the config directory: the bundle id on macOS, `emberhut` elsewhere.
fn dir_name() -> &'static str {
    if cfg!(target_os = "macos") {
        APP_ID
    } else {
        "emberhut"
    }
}

/// The server to talk to: `from_env` (the value of [`SERVER_ENV`]), else the config file's,
/// else [`DEFAULT_SERVER`]. An address that isn't one is skipped, with a warning.
#[must_use]
pub(crate) fn server_url(from_env: Option<&str>, file: &ConfigFile) -> ServerUrl {
    [from_env, file.server.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|address| {
            ServerUrl::parse(address)
                .inspect_err(|error| log::warn!("{error}; trying the next setting"))
                .ok()
        })
        .unwrap_or_else(|| ServerUrl(DEFAULT_SERVER.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_loses_its_trailing_slashes() {
        let url = ServerUrl::parse(" https://example.com:8000// ").unwrap();
        assert_eq!(url.to_string(), "https://example.com:8000");
        assert_eq!(url.join("/api/state"), "https://example.com:8000/api/state");
    }

    #[test]
    fn only_http_addresses_are_servers() {
        assert!(ServerUrl::parse("example.com").is_err());
        assert!(ServerUrl::parse("ftp://example.com").is_err());
        assert!(ServerUrl::parse("http://").is_err());
    }

    #[test]
    fn links_are_made_absolute_only_when_they_are_paths() {
        let url = ServerUrl::parse("http://localhost:1").unwrap();
        assert_eq!(
            url.link("/r/1.html").as_deref(),
            Some("http://localhost:1/r/1.html")
        );
        assert_eq!(
            url.link("https://example.com/x").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(
            url.link("HTTP://example.com").as_deref(),
            Some("HTTP://example.com")
        );
        assert_eq!(
            url.link("/r/1.html#week").as_deref(),
            Some("http://localhost:1/r/1.html#week"),
            "the fragment is the browser's"
        );
        assert_eq!(url.link("/a b.png"), None, "not a server path");
    }

    #[test]
    fn only_plain_paths_are_server_paths() {
        for url in ["/uploads/a.png", "/uploads/a@2x.png", "/x@evil"] {
            assert!(ServerUrl::is_path(url), "{url}");
        }
        for url in [
            "@evil.example/x",
            ".evil.net/x",
            "//evil/x",
            "//user@evil/x",
            "https://evil/x",
            "/\\evil",
            "/a\nb",
            "/a b.png",
        ] {
            assert!(!ServerUrl::is_path(url), "{url}");
        }
    }

    #[test]
    fn other_schemes_are_never_opened() {
        let url = ServerUrl::parse("http://localhost:1").unwrap();
        for link in [
            "file:///etc/passwd",
            "smb://host/share",
            "tel:123",
            "x-apple-reminder://",
            "//host/x",
            "javascript:alert(1)",
            "notes",
        ] {
            assert_eq!(url.link(link), None, "{link}");
        }
    }

    #[test]
    fn the_environment_wins_then_the_file_then_the_default() {
        let file = ConfigFile {
            server: Some("http://file.example".into()),
        };
        assert_eq!(
            server_url(Some("http://env.example"), &file).to_string(),
            "http://env.example"
        );
        assert_eq!(server_url(None, &file).to_string(), "http://file.example");
        assert_eq!(
            server_url(None, &ConfigFile::default()).to_string(),
            DEFAULT_SERVER
        );
    }

    #[test]
    fn a_bad_address_falls_through_to_the_next() {
        let file = ConfigFile {
            server: Some("http://file.example".into()),
        };
        assert_eq!(
            server_url(Some("nonsense"), &file).to_string(),
            "http://file.example"
        );
    }

    #[test]
    fn the_directory_is_named_for_the_platform() {
        let expected = if cfg!(target_os = "macos") {
            "com.emberhut.desktop"
        } else {
            "emberhut"
        };
        assert_eq!(dir_name(), expected);
        if let Some(files) = AppFiles::locate() {
            assert!(files.dir().ends_with(expected));
        }
    }

    #[test]
    fn a_missing_config_file_is_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        assert_eq!(files.read_config(), ConfigFile::default());
        assert_eq!(files.session(), dir.path().join("session.json"));
    }

    #[test]
    fn the_config_file_names_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(files.config(), r#"{"server": "http://example.com"}"#).unwrap();
        assert_eq!(
            files.read_config().server.as_deref(),
            Some("http://example.com")
        );
        std::fs::write(files.config(), "not json").unwrap();
        assert_eq!(files.read_config(), ConfigFile::default());
    }
}
