//! The bearer token the server sends: kept in a file (never a flag, so it isn't in the process
//! list), made on first start when the file isn't there, and compared in constant time.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use base64::Engine as _;
use subtle::ConstantTimeEq as _;

/// Random bytes in a new token.
const TOKEN_BYTES: usize = 32;

/// Why the token could not be had.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The file could not be read or written.
    #[error("the token file {path} could not be used")]
    File {
        /// The file.
        path: String,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The file is empty.
    #[error("the token file {0} is empty: write the token in it, or remove it for a new one")]
    Empty(String),
    /// Others than the owner may read or write the file.
    #[error("the token file {0} is open to others than its owner: chmod 600 it")]
    Open(String),
    /// No random bytes for a new token.
    #[error("no random bytes for a new token")]
    Random(#[source] getrandom::Error),
}

/// The bearer token.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(…)")
    }
}

impl Token {
    /// A token of `text` (trimmed).
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self(text.trim().to_owned())
    }

    /// The token in `path`; a new random one is written there (owner-only) when there is no
    /// file. The second value says whether it was made now. A new file is written whole under
    /// another name first and then linked into place, so a crash never leaves half a token, and
    /// two starts at once agree on one.
    ///
    /// # Errors
    ///
    /// [`TokenError`] when the file can't be read or written, is empty, or is open to others
    /// than its owner.
    pub fn load_or_create(path: &Path) -> Result<(Self, bool), TokenError> {
        match Self::load(path) {
            Err(TokenError::File { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
            loaded => return loaded.map(|token| (token, false)),
        }
        let file = |source| TokenError::File {
            path: path.display().to_string(),
            source,
        };
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::getrandom(&mut bytes).map_err(TokenError::Random)?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(file)?;
        // A name of its own, from other random bytes than the token's.
        let mut name = [0_u8; 8];
        getrandom::getrandom(&mut name).map_err(TokenError::Random)?;
        let name = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(name);
        let temporary = dir.join(format!(".token-{name}"));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut out| {
                out.write_all(format!("{token}\n").as_bytes())?;
                out.sync_all()
            })
            .map_err(file)?;
        let linked = fs::hard_link(&temporary, path);
        let _removed = fs::remove_file(&temporary);
        match linked {
            Ok(()) => Ok((Self(token), true)),
            // Another start made it first: use that one.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Self::load(path).map(|token| (token, false))
            }
            Err(error) => Err(file(error)),
        }
    }

    /// The token in the existing file `path`, owner-only and not empty.
    fn load(path: &Path) -> Result<Self, TokenError> {
        use std::os::unix::fs::PermissionsExt as _;
        let file = |source| TokenError::File {
            path: path.display().to_string(),
            source,
        };
        let metadata = fs::metadata(path).map_err(file)?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(TokenError::Open(path.display().to_string()));
        }
        let text = fs::read_to_string(path).map_err(file)?;
        if text.trim().is_empty() {
            return Err(TokenError::Empty(path.display().to_string()));
        }
        Ok(Self::new(&text))
    }

    /// Whether an `Authorization` header's value is `Bearer <this token>`.
    #[must_use]
    pub fn accepts(&self, header: Option<&str>) -> bool {
        let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
            return false;
        };
        bool::from(given.trim().as_bytes().ct_eq(self.0.as_bytes()))
    }

    /// The token's text, to send it.
    #[must_use]
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn only_the_exact_bearer_is_accepted() {
        let token = Token::new(" s3cret \n");
        assert!(token.accepts(Some("Bearer s3cret")));
        assert!(!token.accepts(Some("Bearer s3cre")));
        assert!(!token.accepts(Some("Bearer s3crets")));
        assert!(!token.accepts(Some("bearer s3cret")));
        assert!(!token.accepts(Some("s3cret")));
        assert!(!token.accepts(None));
        assert_eq!(format!("{token:?}"), "Token(…)");
    }

    #[test]
    fn a_missing_file_gets_a_new_private_token_kept_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Hands").join("token");
        let (made, new) = Token::load_or_create(&path).unwrap();
        assert!(new);
        assert_eq!(made.reveal().len(), 43);
        let permissions = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(permissions, 0o600);
        let (read, new) = Token::load_or_create(&path).unwrap();
        assert!(!new);
        assert_eq!(read, made);
    }

    #[test]
    fn an_empty_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        fs::write(&path, "\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            Token::load_or_create(&path),
            Err(TokenError::Empty(_))
        ));
    }

    #[test]
    fn a_file_others_may_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        fs::write(&path, "s3cret\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            Token::load_or_create(&path),
            Err(TokenError::Open(_))
        ));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(Token::load_or_create(&path).unwrap().0.reveal(), "s3cret");
    }

    #[test]
    fn no_temporary_file_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        Token::load_or_create(&path).unwrap();
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["token"]);
    }
}
