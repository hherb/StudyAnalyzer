//! API keys in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Linux Secret Service). Kept small and self-contained: it moves to bmlib later.

use std::collections::HashMap;
use std::sync::Mutex;

use keyring::v1::Entry;
use studyanalyzer_core::config::KEYCHAIN_SERVICE;
use studyanalyzer_core::llm_settings::{SecretError, SecretStore};

/// What the OS keychain answered for one provider this launch.
type Cache = HashMap<String, Option<String>>;

/// The keychain, with each provider's answer remembered for the launch so the
/// OS prompts (macOS asks per access) once per provider.
#[derive(Default)]
pub struct KeyringSecrets {
    cache: Mutex<Cache>,
}

/// A keyring failure as a [`SecretError`]: the platform's message, which holds no key.
fn secret_error(e: keyring::Error) -> SecretError {
    SecretError(e.to_string())
}

/// The keychain entry for `provider`.
fn entry(provider: &str) -> Result<Entry, SecretError> {
    Entry::new(KEYCHAIN_SERVICE, provider).map_err(secret_error)
}

impl KeyringSecrets {
    /// Locks the cache; a poisoned lock only means another thread panicked, the map is still valid.
    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl SecretStore for KeyringSecrets {
    /// The cached answer, else the keychain's (an absent entry is `None`).
    fn get(&self, provider: &str) -> Result<Option<String>, SecretError> {
        if let Some(hit) = self.cache().get(provider) {
            return Ok(hit.clone());
        }
        let found = match entry(provider)?.get_password() {
            Ok(k) => Some(k),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => return Err(secret_error(e)),
        };
        self.cache().insert(provider.to_string(), found.clone());
        Ok(found)
    }

    /// Writes the keychain, then the cache.
    fn set(&self, provider: &str, key: &str) -> Result<(), SecretError> {
        entry(provider)?.set_password(key).map_err(secret_error)?;
        self.cache()
            .insert(provider.to_string(), Some(key.to_string()));
        Ok(())
    }

    /// Removes the keychain entry (absent is fine), then the cache's.
    fn delete(&self, provider: &str) -> Result<(), SecretError> {
        match entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(secret_error(e)),
        }
        self.cache().insert(provider.to_string(), None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cached answer is returned without touching the OS keychain.
    #[test]
    fn cached_answer_skips_the_keychain() {
        let s = KeyringSecrets::default();
        s.cache().insert("p".into(), Some("k".into()));
        s.cache().insert("q".into(), None);
        assert_eq!(s.get("p").unwrap().as_deref(), Some("k"));
        assert_eq!(s.get("q").unwrap(), None);
    }
}
