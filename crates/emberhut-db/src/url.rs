//! The database's address: a SQLite file URL, checked before anything opens it.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use sea_orm::sqlx::sqlite::SqliteConnectOptions;

/// A SQLite database file URL that has been checked, such as `sqlite://emberhut.db`.
///
/// Only a file the server can write is accepted: an in-memory database would lose every
/// session as soon as its connections idle out, and a read-only one could not sign anyone in.
///
/// ```
/// use emberhut_db::DatabaseUrl;
///
/// assert!("sqlite://emberhut.db".parse::<DatabaseUrl>().is_ok());
/// assert!("sqlite::memory:".parse::<DatabaseUrl>().is_err());
/// assert!("postgres://localhost/emberhut".parse::<DatabaseUrl>().is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseUrl {
    url: String,
    path: PathBuf,
}

impl DatabaseUrl {
    /// The URL of the SQLite file at `path`: `sqlite://` and the path percent-encoded, all but
    /// letters, digits and `/-._~`, so a space, `?`, `#` or `%` in it, and on Windows the drive's
    /// `:` and the `\` separators, are read as part of the path (the driver decodes it; a
    /// Windows path written as is isn't a URL its connector takes).
    ///
    /// ```
    /// use std::path::Path;
    /// use emberhut_db::DatabaseUrl;
    ///
    /// let url = DatabaseUrl::from_path(Path::new("/srv/my data/emberhut.db")).unwrap();
    /// assert_eq!(url.as_str(), "sqlite:///srv/my%20data/emberhut.db");
    /// assert_eq!(url.path(), Path::new("/srv/my data/emberhut.db"));
    /// ```
    ///
    /// # Errors
    ///
    /// As [`DatabaseUrl::from_str`]: a path that isn't UTF-8 is [`InvalidDatabaseUrl::Malformed`],
    /// an empty one [`InvalidDatabaseUrl::NoFile`].
    pub fn from_path(path: &Path) -> Result<Self, InvalidDatabaseUrl> {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut url = String::from("sqlite://");
        for &byte in path.as_os_str().as_encoded_bytes() {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                url.push(char::from(byte));
            } else {
                url.push('%');
                url.push(char::from(HEX[usize::from(byte >> 4)]));
                url.push(char::from(HEX[usize::from(byte & 0x0F)]));
            }
        }
        url.parse()
    }

    /// The URL as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.url
    }

    /// The database file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.url)
    }
}

/// Why a URL is not one the server can keep its database at.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidDatabaseUrl {
    /// Not a `sqlite:` URL; only SQLite is supported.
    #[error("not a SQLite URL such as sqlite://emberhut.db (only SQLite is supported)")]
    NotSqlite,
    /// An in-memory database (or a SQLite `file:` URI, which can name one), which would lose
    /// its data.
    #[error(
        "an in-memory database would lose every session; give a file, such as sqlite://emberhut.db"
    )]
    InMemory,
    /// A read-only database, which could not sign anyone in.
    #[error("a read-only database could not sign anyone in")]
    ReadOnly,
    /// A query option other than `mode=rw` or `mode=rwc`. Every other option (`immutable`,
    /// `vfs`, `cache`, and any the driver learns later) changes how the file is opened, so
    /// none is taken.
    #[error("the SQLite URL option `{0}` is not supported (only mode=rw or mode=rwc)")]
    UnsupportedOption(String),
    /// A `sqlite:` URL that names no file.
    #[error("the SQLite URL names no database file")]
    NoFile,
    /// A `sqlite:` URL that SQLite's driver cannot read.
    #[error("not a valid SQLite URL")]
    Malformed(#[source] Box<dyn Error + Send + Sync>),
}

impl FromStr for DatabaseUrl {
    type Err = InvalidDatabaseUrl;

    fn from_str(url: &str) -> Result<Self, Self::Err> {
        if !url.starts_with("sqlite:") {
            return Err(InvalidDatabaseUrl::NotSqlite);
        }
        // The driver's own parser decides what the URL means; what follows only looks at the
        // same parts, decoded the same way, to refuse what the server must not run on.
        let options = SqliteConnectOptions::from_str(url)
            .map_err(|error| InvalidDatabaseUrl::Malformed(Box::new(error)))?;
        // The driver's split: the scheme taken off, then the database and the query at the
        // first `?`. The database is ":memory:" before any decoding, as the driver checks it.
        let rest = url
            .trim_start_matches("sqlite://")
            .trim_start_matches("sqlite:");
        let (database, query) = rest.split_once('?').unwrap_or((rest, ""));
        if database == ":memory:" {
            return Err(InvalidDatabaseUrl::InMemory);
        }
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match (&*key, &*value) {
                ("mode", "rw" | "rwc") => {}
                ("mode", "memory") => return Err(InvalidDatabaseUrl::InMemory),
                ("mode", "ro") | ("immutable", "true" | "1") => {
                    return Err(InvalidDatabaseUrl::ReadOnly);
                }
                _ => return Err(InvalidDatabaseUrl::UnsupportedOption(key.into_owned())),
            }
        }
        // The filename as the driver hands it to SQLite, decoded: SQLite itself takes
        // ":memory:" and any "file:" URI (`file::memory:`) as something other than a plain file.
        let path = options.get_filename();
        if path.as_os_str().is_empty() {
            return Err(InvalidDatabaseUrl::NoFile);
        }
        let decoded = path.to_string_lossy();
        if decoded == ":memory:" || decoded.starts_with("file:") {
            return Err(InvalidDatabaseUrl::InMemory);
        }
        Ok(Self {
            url: url.to_owned(),
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sqlite_file_is_accepted_with_its_path() {
        let url: DatabaseUrl = "sqlite:///var/lib/emberhut/emberhut.db".parse().unwrap();
        assert_eq!(url.path(), Path::new("/var/lib/emberhut/emberhut.db"));
        let relative: DatabaseUrl = "sqlite://emberhut.db?mode=rwc".parse().unwrap();
        assert_eq!(relative.path(), Path::new("emberhut.db"));
    }

    #[test]
    fn a_path_is_one_url_whatever_its_characters() {
        let path = Path::new("/a/100% sure?#/x.db");
        let url = DatabaseUrl::from_path(path).unwrap();
        assert_eq!(url.as_str(), "sqlite:///a/100%25%20sure%3F%23/x.db");
        assert_eq!(url.path(), path);
        // A Windows path, with its drive and backslashes, is taken whole.
        let windows = DatabaseUrl::from_path(Path::new(r"C:\Users\Ada\emberhut.db")).unwrap();
        assert_eq!(
            windows.as_str(),
            "sqlite://C%3A%5CUsers%5CAda%5Cemberhut.db"
        );
        assert_eq!(windows.path(), Path::new(r"C:\Users\Ada\emberhut.db"));
        assert!(matches!(
            DatabaseUrl::from_path(Path::new("")),
            Err(InvalidDatabaseUrl::NoFile)
        ));
    }

    #[test]
    fn other_databases_are_refused() {
        assert!(matches!(
            "mysql://localhost/emberhut".parse::<DatabaseUrl>(),
            Err(InvalidDatabaseUrl::NotSqlite)
        ));
    }

    #[test]
    fn in_memory_and_read_only_databases_are_refused() {
        for url in [
            "sqlite::memory:",
            "sqlite://:memory:",
            "sqlite://x.db?mode=memory",
            "sqlite://x.db?mode=memor%79",
            "sqlite:%3Amemory%3A",
            "sqlite://%3Amemory%3A",
            "sqlite://file%3A%3Amemory%3A",
            "sqlite://file%3Aemberhut.db",
        ] {
            assert!(
                matches!(
                    url.parse::<DatabaseUrl>(),
                    Err(InvalidDatabaseUrl::InMemory)
                ),
                "{url}"
            );
        }
        for url in [
            "sqlite://x.db?mode=ro",
            "sqlite://x.db?mode=r%6F",
            "sqlite://x.db?immutable=true",
            "sqlite://x.db?immutable=1",
        ] {
            assert!(
                matches!(
                    url.parse::<DatabaseUrl>(),
                    Err(InvalidDatabaseUrl::ReadOnly)
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn only_the_mode_option_is_taken() {
        for (url, option) in [
            ("sqlite://x.db?vfs=memdb", "vfs"),
            ("sqlite://x.db?cache=shared", "cache"),
            ("sqlite://x.db?immutable=0", "immutable"),
            ("sqlite://x.db?mode=rw&v%66s=memdb", "vfs"),
        ] {
            match url.parse::<DatabaseUrl>() {
                Err(InvalidDatabaseUrl::UnsupportedOption(name)) => assert_eq!(name, option),
                other => panic!("{url}: {other:?}"),
            }
        }
        assert!("sqlite://x.db?mode=rw".parse::<DatabaseUrl>().is_ok());
    }

    #[test]
    fn a_url_without_a_file_is_refused() {
        assert!(matches!(
            "sqlite://?mode=rwc".parse::<DatabaseUrl>(),
            Err(InvalidDatabaseUrl::NoFile)
        ));
    }

    #[test]
    fn a_malformed_url_keeps_the_driver_s_reason() {
        let error = "sqlite://x.db?mode=sideways"
            .parse::<DatabaseUrl>()
            .unwrap_err();
        assert!(matches!(error, InvalidDatabaseUrl::Malformed(_)));
        assert!(error.source().is_some());
    }
}
