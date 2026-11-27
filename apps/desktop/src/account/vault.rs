//! Where the account's tokens are kept: on macOS one generic-password item of the login
//! Keychain, service `com.emberhut.desktop.account`, never synced and never in `session.json`. It
//! is apart from the device sessions' items, so signing out of the account touches no device
//! session, and signing a device out touches no account.
//!
//! The item is behind `SecretItem`, so the rules here are tested with an item in memory.
//! Other systems have no item yet, and a Mac keeps none in a test run or for a user with no
//! default keychain: the account works there for one launch.

#[cfg(target_os = "macos")]
mod system;

use std::fmt;

use super::tokens::AccountTokens;

/// Why the tokens couldn't be read or kept.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VaultError {
    /// This system has no place for them yet.
    #[error("keeping the account's sign-in isn't supported on this system yet")]
    Unsupported,
    /// A newer app's tokens are in the item: neither read nor overwritten.
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only a keychain item holds tokens")
    )]
    #[error("a newer version of the app keeps the account's sign-in")]
    Newer,
    /// The keychain failed (locked, or the access was refused).
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only a keychain item holds tokens")
    )]
    #[error("the Keychain couldn't be used")]
    Keychain(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Keeps the account's tokens. Blocks: call it on a background thread.
pub(crate) trait Vault: Send + Sync + fmt::Debug {
    /// The tokens kept, if any.
    ///
    /// # Errors
    ///
    /// [`VaultError`] when they can't be read.
    fn load(&self) -> Result<Option<AccountTokens>, VaultError>;

    /// Keeps `tokens` in place of any kept.
    ///
    /// # Errors
    ///
    /// [`VaultError`] when they can't be kept.
    fn save(&self, tokens: &AccountTokens) -> Result<(), VaultError>;

    /// Forgets the tokens; none kept is fine.
    ///
    /// # Errors
    ///
    /// [`VaultError`] when the item can't be removed.
    fn clear(&self) -> Result<(), VaultError>;
}

#[cfg(any(target_os = "macos", test))]
/// One secret's bytes in a keychain.
pub(crate) trait SecretItem: Send + Sync {
    /// The bytes; `None` without an item.
    ///
    /// # Errors
    ///
    /// [`VaultError::Keychain`] when it can't be read.
    fn read(&self) -> Result<Option<Vec<u8>>, VaultError>;

    /// Makes `bytes` the item's, creating it if there is none.
    ///
    /// # Errors
    ///
    /// [`VaultError::Keychain`] when it can't be written.
    fn write(&self, bytes: &[u8]) -> Result<(), VaultError>;

    /// Removes the item; none is fine.
    ///
    /// # Errors
    ///
    /// [`VaultError::Keychain`] when it can't be removed.
    fn delete(&self) -> Result<(), VaultError>;
}

#[cfg(any(target_os = "macos", test))]
/// The tokens as JSON in one item.
pub(crate) struct ItemVault<I> {
    item: I,
}

#[cfg(any(target_os = "macos", test))]
impl<I> fmt::Debug for ItemVault<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ItemVault")
    }
}

#[cfg(any(target_os = "macos", test))]
impl<I: SecretItem> ItemVault<I> {
    /// The tokens in `item`.
    pub(crate) fn new(item: I) -> Self {
        Self { item }
    }
}

#[cfg(any(target_os = "macos", test))]
/// The format version `bytes` were written in, when they read as tokens at all.
fn version_of(bytes: &[u8]) -> Option<u32> {
    #[derive(serde::Deserialize)]
    struct Versioned {
        version: u32,
    }
    serde_json::from_slice::<Versioned>(bytes)
        .ok()
        .map(|versioned| versioned.version)
}

#[cfg(any(target_os = "macos", test))]
impl<I: SecretItem> Vault for ItemVault<I> {
    fn load(&self) -> Result<Option<AccountTokens>, VaultError> {
        let Some(bytes) = self.item.read()? else {
            return Ok(None);
        };
        if version_of(&bytes).is_some_and(|version| version > AccountTokens::FORMAT_VERSION) {
            return Err(VaultError::Newer);
        }
        if let Ok(tokens) = serde_json::from_slice(&bytes) {
            Ok(Some(tokens))
        } else {
            // Not serde's error itself: its words can quote what the item holds.
            log::warn!("the account's Keychain item isn't its tokens; removing it");
            self.item.delete()?;
            Ok(None)
        }
    }

    fn save(&self, tokens: &AccountTokens) -> Result<(), VaultError> {
        if let Some(bytes) = self.item.read()?
            && version_of(&bytes).is_some_and(|version| version > AccountTokens::FORMAT_VERSION)
        {
            return Err(VaultError::Newer);
        }
        let bytes =
            serde_json::to_vec(tokens).map_err(|error| VaultError::Keychain(Box::new(error)))?;
        self.item.write(&bytes)
    }

    fn clear(&self) -> Result<(), VaultError> {
        self.item.delete()
    }
}

/// The vault where this system keeps the account's tokens, for the account at `issuer`: the
/// login Keychain on macOS; none in a test run or for a user with no default keychain
/// ([`crate::backend::has_keychain`]), where the account works for one launch.
#[cfg(target_os = "macos")]
#[must_use]
pub(crate) fn for_system(issuer: &str) -> Box<dyn Vault> {
    if crate::backend::has_keychain() {
        Box::new(ItemVault::new(system::SystemItem::new(issuer)))
    } else {
        Box::new(Unsupported)
    }
}

/// The vault where this system keeps the account's tokens: none yet off macOS.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub(crate) fn for_system(_issuer: &str) -> Box<dyn Vault> {
    Box::new(Unsupported)
}

/// The vault of a system with no keychain support yet: it keeps nothing, and says so
/// ([`VaultError::Unsupported`]) for every save.
#[derive(Debug)]
struct Unsupported;

impl Vault for Unsupported {
    fn load(&self) -> Result<Option<AccountTokens>, VaultError> {
        Ok(None)
    }

    fn save(&self, _tokens: &AccountTokens) -> Result<(), VaultError> {
        Err(VaultError::Unsupported)
    }

    fn clear(&self) -> Result<(), VaultError> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use emberhut_identity::AccountId;

    use super::*;

    /// An item in memory; its clones share it.
    #[derive(Clone, Debug, Default)]
    pub(crate) struct MemoryItem(Arc<Mutex<Option<Vec<u8>>>>);

    impl MemoryItem {
        /// What it holds.
        pub(crate) fn held(&self) -> Option<Vec<u8>> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// Makes it hold `bytes`.
        pub(crate) fn put(&self, bytes: &[u8]) {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(bytes.to_vec());
        }
    }

    impl SecretItem for MemoryItem {
        fn read(&self) -> Result<Option<Vec<u8>>, VaultError> {
            Ok(self.held())
        }

        fn write(&self, bytes: &[u8]) -> Result<(), VaultError> {
            self.put(bytes);
            Ok(())
        }

        fn delete(&self) -> Result<(), VaultError> {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Ok(())
        }
    }

    fn tokens() -> AccountTokens {
        AccountTokens {
            version: AccountTokens::FORMAT_VERSION,
            account: AccountId::new("user_example").unwrap(),
            access: "a".to_owned(),
            refresh: Some("r".to_owned()),
            expires_at: 1_000,
        }
    }

    #[test]
    fn tokens_are_kept_read_back_and_forgotten() {
        let item = MemoryItem::default();
        let vault = ItemVault::new(item.clone());
        assert!(vault.load().unwrap().is_none());
        vault.save(&tokens()).unwrap();
        assert_eq!(vault.load().unwrap(), Some(tokens()));
        vault.clear().unwrap();
        assert_eq!(item.held(), None);
    }

    #[test]
    fn a_newer_apps_tokens_are_neither_read_nor_overwritten() {
        let item = MemoryItem::default();
        let newer = br#"{"version": 9, "whatever": true}"#;
        item.put(newer);
        let vault = ItemVault::new(item.clone());
        assert!(matches!(vault.load(), Err(VaultError::Newer)));
        assert!(matches!(vault.save(&tokens()), Err(VaultError::Newer)));
        assert_eq!(item.held().as_deref(), Some(&newer[..]));
    }

    #[test]
    fn an_item_that_isnt_tokens_is_removed() {
        let item = MemoryItem::default();
        item.put(b"not json");
        let vault = ItemVault::new(item.clone());
        assert!(vault.load().unwrap().is_none());
        assert_eq!(item.held(), None);
    }

    #[test]
    fn a_system_without_a_keychain_says_so() {
        let vault = Unsupported;
        assert!(vault.load().unwrap().is_none());
        let refused = vault.save(&tokens()).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "keeping the account's sign-in isn't supported on this system yet"
        );
    }
}
