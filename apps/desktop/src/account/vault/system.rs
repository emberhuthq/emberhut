//! The login Keychain's item for the account: a generic password, service
//! `com.emberhut.desktop.account` and account the issuer, not synced to other devices.

use security_framework::base::Error as KeychainError;
use security_framework::passwords::{
    PasswordOptions, delete_generic_password_options, generic_password,
    set_generic_password_options,
};

use super::{SecretItem, VaultError};

/// The item's service: one item for the account, apart from the device sessions' items.
const SERVICE: &str = "com.emberhut.desktop.account";

/// The Keychain's "no such item" (`errSecItemNotFound`).
const NOT_FOUND: i32 = -25_300;

/// The account's item.
#[derive(Debug)]
pub(super) struct SystemItem {
    issuer: String,
}

impl SystemItem {
    /// The item for the account at `issuer`.
    pub(super) fn new(issuer: &str) -> Self {
        Self {
            issuer: issuer.to_owned(),
        }
    }

    /// The item's attributes: this service's generic password for the issuer, not synced.
    fn options(&self) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(SERVICE, &self.issuer);
        options.set_access_synchronized(Some(false));
        options
    }
}

fn failed(error: KeychainError) -> VaultError {
    VaultError::Keychain(Box::new(error))
}

impl SecretItem for SystemItem {
    fn read(&self) -> Result<Option<Vec<u8>>, VaultError> {
        match generic_password(self.options()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.code() == NOT_FOUND => Ok(None),
            Err(error) => Err(failed(error)),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), VaultError> {
        set_generic_password_options(bytes, self.options()).map_err(failed)
    }

    fn delete(&self) -> Result<(), VaultError> {
        match delete_generic_password_options(self.options()) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == NOT_FOUND => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }
}
