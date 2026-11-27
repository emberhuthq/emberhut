//! The account a machine is linked to, as the directory names it.

use std::fmt;
use std::str::FromStr;

/// An account's user id at the account provider (Clerk's `sub`), as a link statement names it:
/// 1 to 255 bytes of visible ASCII (`!` to `~`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(String);

/// Why text isn't an account's user id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum AccountIdError {
    /// Empty, or over 255 bytes.
    #[error("an account id is 1 to 255 characters")]
    Length,
    /// A character that isn't visible ASCII.
    #[error("an account id is visible ASCII")]
    Character,
}

impl AccountId {
    /// The longest id, in bytes.
    pub const MAX_LEN: usize = 255;

    /// Wraps a user id.
    ///
    /// # Errors
    ///
    /// [`AccountIdError`] for an id that is empty, too long, or not visible ASCII.
    pub fn new(id: impl Into<String>) -> Result<Self, AccountIdError> {
        let id = id.into();
        if id.is_empty() || id.len() > Self::MAX_LEN {
            return Err(AccountIdError::Length);
        }
        if !id.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(AccountIdError::Character);
        }
        Ok(Self(id))
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AccountId {
    type Err = AccountIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::new(text)
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_id_is_visible_ascii() {
        assert_eq!(
            AccountId::new("user_2example").unwrap().as_str(),
            "user_2example"
        );
        assert_eq!(AccountId::new(""), Err(AccountIdError::Length));
        assert!(AccountId::new("a".repeat(255)).is_ok());
        assert_eq!(AccountId::new("a".repeat(256)), Err(AccountIdError::Length));
        assert_eq!(AccountId::new("user x"), Err(AccountIdError::Character));
        assert_eq!(AccountId::new("user_é"), Err(AccountIdError::Character));
        assert_eq!("user_x".parse::<AccountId>().unwrap().to_string(), "user_x");
    }
}
