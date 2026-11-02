//! API keys in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Linux Secret Service). Kept small and self-contained: it moves to bmlib later.
//!
//! Every provider's key lives in ONE keychain item, a JSON object
//! `{provider: key}`, read once per launch: the OS then asks once, not once per
//! provider (macOS prompts per access).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use keyring::v1::Entry;
use studyanalyzer_core::config::{KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};
use studyanalyzer_core::llm_settings::{SecretError, SecretStore};

/// The one item, as a plain read and write: the only part that touches the OS.
pub trait Vault: Send + Sync {
    /// The item's text, or `None` when it does not exist yet.
    fn read(&self) -> Result<Option<String>, SecretError>;
    /// Replaces the item's text.
    fn write(&self, json: &str) -> Result<(), SecretError>;
}

/// The OS keychain item.
pub struct KeyringVault;

/// A keyring failure as a [`SecretError`]: the platform's message, which holds no key.
fn secret_error(e: keyring::Error) -> SecretError {
    SecretError(e.to_string())
}

/// The keychain entry holding all keys.
fn entry() -> Result<Entry, SecretError> {
    Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(secret_error)
}

impl Vault for KeyringVault {
    /// Reads the item (an absent one is `None`).
    fn read(&self) -> Result<Option<String>, SecretError> {
        match entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_error(e)),
        }
    }
    /// Writes the item.
    fn write(&self, json: &str) -> Result<(), SecretError> {
        entry()?.set_password(json).map_err(secret_error)
    }
}

/// The keys, `{provider: key}`.
type Keys = BTreeMap<String, String>;

/// A [`SecretStore`] over one [`Vault`] item, read once and then cached for the launch.
pub struct Bundle<V: Vault> {
    vault: V,
    /// `None` until the first successful read; a failed read is retried next call.
    cache: Mutex<Option<Keys>>,
}

/// The API keys in the OS keychain.
pub type KeyringSecrets = Bundle<KeyringVault>;

impl KeyringSecrets {
    /// A store over the real keychain.
    pub fn keyring() -> Self {
        Bundle::new(KeyringVault)
    }
}

impl<V: Vault> Bundle<V> {
    /// A store over `vault`, nothing read yet.
    fn new(vault: V) -> Self {
        Bundle {
            vault,
            cache: Mutex::new(None),
        }
    }

    /// The cache, loaded from the vault if this is the first use, and the keys
    /// in it. A poisoned lock only means another thread panicked; the map is
    /// still valid. An item that is not a key list is an error, unless
    /// `replace_unreadable` (a key is about to be saved): then, logged, the
    /// keys start empty and the cache stays unloaded until they are written.
    fn loaded(
        &self,
        replace_unreadable: bool,
    ) -> Result<(MutexGuard<'_, Option<Keys>>, Keys), SecretError> {
        let mut guard = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(keys) = guard.as_ref() {
            let keys = keys.clone();
            return Ok((guard, keys));
        }
        let keys = match self.vault.read()? {
            None => Keys::new(),
            // The parse error is dropped, here and in the log: it could quote the item's text.
            Some(text) => match serde_json::from_str(&text) {
                Ok(keys) => keys,
                Err(_) if replace_unreadable => {
                    log::warn!(
                        "the keychain item {KEYCHAIN_ACCOUNT} is not a valid key list; replacing it with the key being saved"
                    );
                    return Ok((guard, Keys::new()));
                }
                Err(_) => {
                    return Err(SecretError(format!(
                        "the keychain item {KEYCHAIN_ACCOUNT} is not a valid key list"
                    )))
                }
            },
        };
        *guard = Some(keys.clone());
        Ok((guard, keys))
    }

    /// Writes `keys` to the vault, and only on success makes them the cache.
    fn store(
        guard: &mut MutexGuard<'_, Option<Keys>>,
        vault: &V,
        keys: Keys,
    ) -> Result<(), SecretError> {
        let json = serde_json::to_string(&keys)
            .map_err(|_| SecretError("the keys could not be encoded".into()))?;
        vault.write(&json)?;
        **guard = Some(keys);
        Ok(())
    }
}

impl<V: Vault> SecretStore for Bundle<V> {
    /// The provider's key from the cache.
    fn get(&self, provider: &str) -> Result<Option<String>, SecretError> {
        Ok(self.loaded(false)?.1.get(provider).cloned())
    }

    /// Rewrites the item with the key added, then the cache. An item that is
    /// not a key list is replaced by one holding just this key.
    fn set(&self, provider: &str, key: &str) -> Result<(), SecretError> {
        let (mut guard, mut keys) = self.loaded(true)?;
        keys.insert(provider.to_string(), key.to_string());
        Self::store(&mut guard, &self.vault, keys)
    }

    /// Rewrites the item without the key (absent is fine), then the cache.
    fn delete(&self, provider: &str) -> Result<(), SecretError> {
        let (mut guard, mut keys) = self.loaded(false)?;
        if keys.remove(provider).is_none() {
            return Ok(());
        }
        Self::store(&mut guard, &self.vault, keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    /// An in-memory vault that counts reads and can be told to fail.
    #[derive(Default)]
    struct FakeVault {
        item: Mutex<Option<String>>,
        reads: AtomicU32,
        fail_write: AtomicBool,
    }

    impl Vault for FakeVault {
        fn read(&self) -> Result<Option<String>, SecretError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.item.lock().unwrap().clone())
        }
        fn write(&self, json: &str) -> Result<(), SecretError> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(SecretError("denied".into()));
            }
            *self.item.lock().unwrap() = Some(json.to_string());
            Ok(())
        }
    }

    /// A get after a set sees the new key, and a delete removes it.
    #[test]
    fn set_get_delete() {
        let s = Bundle::new(FakeVault::default());
        assert_eq!(s.get("openai").unwrap(), None);
        s.set("openai", "k1").unwrap();
        s.set("anthropic", "k2").unwrap();
        assert_eq!(s.get("openai").unwrap().as_deref(), Some("k1"));
        s.delete("openai").unwrap();
        assert_eq!(s.get("openai").unwrap(), None);
        assert_eq!(s.get("anthropic").unwrap().as_deref(), Some("k2"));
        let stored = s.vault.item.lock().unwrap().clone().unwrap();
        assert!(!stored.contains("k1") && stored.contains("k2"));
    }

    /// The vault is read once however many providers are asked about.
    #[test]
    fn vault_read_once() {
        let s = Bundle::new(FakeVault::default());
        for p in ["a", "b", "c", "a"] {
            s.get(p).unwrap();
        }
        assert_eq!(s.vault.reads.load(Ordering::SeqCst), 1);
    }

    /// A failed write leaves the cache, and so later gets, unchanged.
    #[test]
    fn failed_write_keeps_cache() {
        let s = Bundle::new(FakeVault::default());
        s.set("openai", "k1").unwrap();
        s.vault.fail_write.store(true, Ordering::SeqCst);
        assert!(s.set("openai", "k2").is_err());
        assert!(s.delete("openai").is_err());
        assert_eq!(s.get("openai").unwrap().as_deref(), Some("k1"));
    }

    /// An unreadable item is an error for `get` and `delete` that does not
    /// repeat the item's text, and is read again each time.
    #[test]
    fn unreadable_item_errors_without_content() {
        let v = FakeVault::default();
        *v.item.lock().unwrap() = Some("not json sk-leaky".into());
        let s = Bundle::new(v);
        let e = s.get("openai").unwrap_err();
        assert!(e.0.contains(KEYCHAIN_ACCOUNT) && !e.0.contains("sk-leaky"));
        assert!(s.delete("openai").is_err());
        assert!(s.get("openai").is_err());
        assert_eq!(s.vault.reads.load(Ordering::SeqCst), 3);
    }

    /// `set` over an unreadable item replaces it with just the new key, so
    /// saving a key in Settings is a way out.
    #[test]
    fn set_replaces_an_unreadable_item() {
        let v = FakeVault::default();
        *v.item.lock().unwrap() = Some("not json sk-leaky".into());
        let s = Bundle::new(v);
        s.set("openai", "k1").unwrap();
        assert_eq!(s.get("openai").unwrap().as_deref(), Some("k1"));
        let stored = s.vault.item.lock().unwrap().clone().unwrap();
        assert_eq!(stored, r#"{"openai":"k1"}"#);
    }

    /// A failed write over an unreadable item leaves it unreadable: `get`
    /// still reports it rather than answering from an empty map.
    #[test]
    fn failed_replace_keeps_the_error() {
        let v = FakeVault::default();
        *v.item.lock().unwrap() = Some("not json".into());
        v.fail_write.store(true, Ordering::SeqCst);
        let s = Bundle::new(v);
        assert!(s.set("openai", "k1").is_err());
        assert!(s.get("openai").is_err());
    }
}
