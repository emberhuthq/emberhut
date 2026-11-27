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
