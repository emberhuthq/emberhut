//! The macOS Keychain's item for one server: a generic password, its service this app's bundle
//! id and its account the server's address, not synced to other devices.

use std::path::PathBuf;

use emberhut_client::{FileSessionStore, StoreError};
use security_framework::base::Error as KeychainError;
use security_framework::passwords::{
    PasswordOptions, delete_generic_password_options, generic_password,
    set_generic_password_options,
};

use super::{Item, KeychainSessionStore};
use crate::brand::APP_ID;

/// The Keychain's "no such item" (`errSecItemNotFound`).
const NOT_FOUND: i32 = -25_300;

/// This app's item for one server.
#[derive(Debug)]
pub(crate) struct SystemItem {
    account: String,
}

impl SystemItem {
    /// The item's attributes: this app's generic password for the server, not synced.
    fn options(&self) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(APP_ID, &self.account);
        options.set_access_synchronized(Some(false));
        options
    }
}

fn failed(error: KeychainError) -> StoreError {
    StoreError::Backend(Box::new(error))
}

impl Item for SystemItem {
    fn read(&self) -> Result<Option<Vec<u8>>, StoreError> {
        match generic_password(self.options()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.code() == NOT_FOUND => Ok(None),
            Err(error) => Err(failed(error)),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), StoreError> {
        set_generic_password_options(bytes, self.options()).map_err(failed)
    }

    fn delete(&self) -> Result<(), StoreError> {
        match delete_generic_password_options(self.options()) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == NOT_FOUND => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }
}

impl KeychainSessionStore<SystemItem> {
    /// The session for the server at `server` in this Mac's Keychain, moving in the one kept at
    /// `earlier_file`.
    pub(crate) fn system(server: &str, earlier_file: PathBuf) -> Self {
        Self::new(
            SystemItem {
                account: server.to_owned(),
            },
            FileSessionStore::new(earlier_file),
        )
    }
}
