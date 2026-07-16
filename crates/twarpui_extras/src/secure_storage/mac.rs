//! Implementations of the [`SecureStorage`] service for the macOS platform.

use anyhow::anyhow;
use security_framework::item::{ItemClass, ItemSearchOptions, Reference, SearchResult};
use security_framework::os::macos::{
    keychain::SecKeychain, keychain_item::SecKeychainItem, passwords::SecKeychainItemPassword,
};

use super::Error;

/// `errSecItemNotFound` from the macOS Security framework.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// Implementation of the SecureStorage service using macOS Security
/// framework keychains.
pub struct SecureStorage {
    /// The name of the service under which to store the values.
    service_name: String,
}

impl SecureStorage {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
        }
    }
}

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        let keychain = SecKeychain::default()?;
        match keychain.set_generic_password(self.service_name.as_str(), key, value.as_bytes()) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Updating in place fails when the existing item was written
                // by a build with a different code signature: the item's ACL
                // denies this binary. Deleting doesn't need read access, so
                // replace the orphaned item and retry — this is what makes
                // re-saving a key in Settings self-heal after a resign.
                match self.delete_item_without_reading(key) {
                    Ok(()) | Err(Error::NotFound) => {}
                    Err(err) => return Err(err),
                }
                keychain
                    .set_generic_password(self.service_name.as_str(), key, value.as_bytes())
                    .map_err(Into::into)
            }
        }
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        let (password, _) = self.get_password_item(key)?;
        String::from_utf8(password.as_ref().to_vec())
            .map_err(|err| Error::DecodeError(err.utf8_error()))
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        // Search by reference only (no password load) so removal works even
        // on items whose ACL denies this binary reading the secret.
        self.delete_item_without_reading(key)
    }
}

impl SecureStorage {
    /// Deletes the item for `key`, if any, without loading its password data.
    /// Reading the secret is ACL-gated to the binaries that wrote it, but a
    /// ref-only search + delete is not, so this succeeds even on items
    /// orphaned by a code-signature change.
    fn delete_item_without_reading(&self, key: &str) -> Result<(), Error> {
        let results = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&self.service_name)
            .account(key)
            .load_refs(true)
            .search()
            .map_err(|err| {
                if err.code() == ERR_SEC_ITEM_NOT_FOUND {
                    Error::NotFound
                } else {
                    Error::Unknown(anyhow!(err))
                }
            })?;
        for result in results {
            if let SearchResult::Ref(Reference::KeychainItem(item)) = result {
                item.delete();
            }
        }
        Ok(())
    }

    fn get_password_item(
        &self,
        key: &str,
    ) -> Result<(SecKeychainItemPassword, SecKeychainItem), Error> {
        let keychain = SecKeychain::default()?;
        keychain
            .find_generic_password(&self.service_name, key)
            .map_err(|err| {
                // Only a genuine errSecItemNotFound means the item is absent.
                // Anything else (e.g. an ACL denial after the app is rebuilt
                // with a new code signature) must not masquerade as NotFound,
                // or callers that treat NotFound as "the secret is gone" will
                // destroy state for an item that still exists.
                if err.code() == ERR_SEC_ITEM_NOT_FOUND {
                    Error::NotFound
                } else {
                    Error::Unknown(anyhow!(err))
                }
            })
    }
}

impl From<security_framework::base::Error> for Error {
    fn from(value: security_framework::base::Error) -> Self {
        Error::Unknown(anyhow!(value))
    }
}
