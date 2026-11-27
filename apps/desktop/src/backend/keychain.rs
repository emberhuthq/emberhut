//! The session kept in the macOS Keychain: a generic password of this app's, one per server,
//! in this Mac's own keychain (never synced to other devices, readable while the Mac is
//! unlocked). It holds the session as its file form (the install id, the refresh token, the
//! device row), so nothing of it sits in a file. A session file kept by an earlier launch moves
//! into the Keychain the first time it is read. Elsewhere the session stays in the private
//! session file.
//!
//! It keeps [`SessionStore`]'s rules: a newer client's session (a higher format version) is
//! neither read nor overwritten ([`StoreError::NewerFormatInStore`]), so a downgrade or a second,
//! older install doesn't sign the newer one out. An item that isn't a session at all is removed
//! once, with a warning, and this device starts a new install and signs in again (a new launch
//! then finds the new item, not the broken one).
//!
//! The Keychain itself is behind [`Item`], so this logic is tested with a fake; the system's item
//! is `system::SystemItem` (macOS only).

#[cfg(target_os = "macos")]
pub(super) mod system;

use std::fmt;

use emberhut_client::{FileSessionStore, Session, SessionStore, StoreError, stored_version};

/// One secret in a keychain: the bytes of this app's item for one server.
pub(crate) trait Item: Send + Sync {
    /// The item's bytes; `None` when there is no item.
    ///
    /// # Errors
    ///
    /// The keychain couldn't be read (locked, say): [`StoreError::Backend`].
    fn read(&self) -> Result<Option<Vec<u8>>, StoreError>;

    /// Makes `bytes` the item's, creating it if there is none.
    ///
    /// # Errors
    ///
    /// The keychain couldn't be written: [`StoreError::Backend`].
    fn write(&self, bytes: &[u8]) -> Result<(), StoreError>;

    /// Removes the item; none is fine.
    ///
    /// # Errors
    ///
    /// The keychain couldn't be written: [`StoreError::Backend`].
    fn delete(&self) -> Result<(), StoreError>;
}

/// The session in a keychain item, moving in the one an earlier launch kept in a file.
pub(crate) struct KeychainSessionStore<I> {
    item: I,
    /// The session file an earlier launch kept: read once, moved in, then cleared.
    earlier: FileSessionStore,
}

impl<I> fmt::Debug for KeychainSessionStore<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeychainSessionStore")
            .field("earlier", &self.earlier)
            .finish_non_exhaustive()
    }
}

impl<I: Item> KeychainSessionStore<I> {
    /// The session in `item`, moving in the one kept in `earlier`.
    pub(crate) fn new(item: I, earlier: FileSessionStore) -> Self {
        Self { item, earlier }
    }

    /// Moves the session an earlier launch kept in its file into the item (none: `None`).
    fn move_in(&self) -> Result<Option<Session>, StoreError> {
        let Some(session) = self.earlier.load()? else {
            return Ok(None);
        };
        self.save(&session)?;
        if let Err(error) = self.earlier.clear() {
            log::warn!("the session is in the Keychain now, but its old file stays: {error}");
        }
        Ok(Some(session))
    }
}

/// [`StoreError::NewerFormatInStore`] when `bytes` are a newer client's session.
fn refuse_newer(bytes: &[u8]) -> Result<(), StoreError> {
    match stored_version(bytes) {
        Some(version) if version > Session::FORMAT_VERSION => {
            Err(StoreError::NewerFormatInStore { version })
        }
        _ => Ok(()),
    }
}

impl<I: Item> SessionStore for KeychainSessionStore<I> {
    fn load(&self) -> Result<Option<Session>, StoreError> {
        let Some(bytes) = self.item.read()? else {
            return self.move_in();
        };
        refuse_newer(&bytes)?;
        match serde_json::from_slice(&bytes) {
            Ok(session) => Ok(Some(session)),
            Err(error) => {
                log::warn!(
                    "the session in the Keychain isn't one ({error}); removing it: this device \
                     signs in again"
                );
                self.item.delete()?;
                Ok(None)
            }
        }
    }

    fn save(&self, session: &Session) -> Result<(), StoreError> {
        if let Some(bytes) = self.item.read()? {
            refuse_newer(&bytes)?;
        }
        let bytes = serde_json::to_vec(session).map_err(StoreError::Encode)?;
        self.item.write(&bytes)
    }

    fn clear(&self) -> Result<(), StoreError> {
        self.item.delete()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::*;

    /// A keychain item in memory; its clones share it.
    #[derive(Clone, Debug, Default)]
    pub(crate) struct FakeItem(Arc<Mutex<Option<Vec<u8>>>>);

    impl FakeItem {
        /// An item holding `bytes`.
        pub(crate) fn holding(bytes: &[u8]) -> Self {
            Self(Arc::new(Mutex::new(Some(bytes.to_vec()))))
        }

        /// What the item holds.
        pub(crate) fn bytes(&self) -> Option<Vec<u8>> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Item for FakeItem {
        fn read(&self) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.bytes())
        }

        fn write(&self, bytes: &[u8]) -> Result<(), StoreError> {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(bytes.to_vec());
            Ok(())
        }

        fn delete(&self) -> Result<(), StoreError> {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Ok(())
        }
    }

    const NEWER: &[u8] = br#"{"version": 9, "install": "zr-newer"}"#;

    fn store(item: &FakeItem, dir: &tempfile::TempDir) -> KeychainSessionStore<FakeItem> {
        KeychainSessionStore::new(
            item.clone(),
            FileSessionStore::new(dir.path().join("session.json")),
        )
    }
    #[test]
    fn a_session_saved_loads_back_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let item = FakeItem::default();
        let store = store(&item, &dir);
        assert_eq!(store.load().unwrap(), None, "nothing kept");
        let session = Session::new_install().unwrap();
        store.save(&session).unwrap();
        assert_eq!(store.load().unwrap(), Some(session));
        store.clear().unwrap();
        assert_eq!(item.bytes(), None);
    }

    #[test]
    fn the_session_file_an_earlier_launch_kept_moves_in() {
        let dir = tempfile::tempdir().unwrap();
        let file = FileSessionStore::new(dir.path().join("session.json"));
        let session = Session::new_install().unwrap();
        file.save(&session).unwrap();
        let item = FakeItem::default();
        let store = store(&item, &dir);
        assert_eq!(store.load().unwrap(), Some(session.clone()));
        assert!(item.bytes().is_some(), "in the Keychain now");
        assert!(!dir.path().join("session.json").exists(), "the file went");
        assert_eq!(store.load().unwrap(), Some(session), "read from the item");
    }

    #[test]
    fn a_newer_clients_session_is_neither_read_nor_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let item = FakeItem::holding(NEWER);
        let store = store(&item, &dir);
        assert!(matches!(
            store.load(),
            Err(StoreError::NewerFormatInStore { version: 9 })
        ));
        assert!(matches!(
            store.save(&Session::new_install().unwrap()),
            Err(StoreError::NewerFormatInStore { version: 9 })
        ));
        assert_eq!(item.bytes().as_deref(), Some(NEWER), "as it was");
    }

    #[test]
    fn an_item_that_isnt_a_session_is_removed_once() {
        let dir = tempfile::tempdir().unwrap();
        let item = FakeItem::holding(b"{ broken");
        let store = store(&item, &dir);
        assert_eq!(store.load().unwrap(), None, "a new install follows");
        assert_eq!(item.bytes(), None, "removed");
        let session = Session::new_install().unwrap();
        store.save(&session).unwrap();
        assert_eq!(
            store.load().unwrap(),
            Some(session),
            "the next launch reads it"
        );
    }
}
