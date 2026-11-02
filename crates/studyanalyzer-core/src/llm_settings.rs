//! Which model answers, how big its window may be, and where API keys come from.
//!
//! The model and window live in the `settings` table. Keys never do: they sit
//! behind [`SecretStore`] (the OS keychain in the app), with the provider's
//! environment variable as the fallback. No key is ever logged, put in an
//! error, or handed to the frontend (only whether one exists).

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use bmlib::db::Db;
use bmlib::llm::{parse_model_string, provider_specs, ProviderSpec, DEFAULT_PROVIDER};
use serde::Serialize;

use crate::config::{CONTEXT_TOKENS_KEY, LLM_MODEL_KEY};
use crate::store::{delete_setting, get_setting, set_setting, StoreError};

/// A secret store failed. The text is the platform's message and never holds a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretError(pub String);

impl fmt::Display for SecretError {
    /// Renders the platform's message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "secret store: {}", self.0)
    }
}

impl std::error::Error for SecretError {}

/// Somewhere API keys are kept, one per provider name.
pub trait SecretStore: Send + Sync {
    /// The stored key for `provider`, if any.
    fn get(&self, provider: &str) -> Result<Option<String>, SecretError>;
    /// Stores (replacing) the key for `provider`.
    fn set(&self, provider: &str, key: &str) -> Result<(), SecretError>;
    /// Removes the key for `provider`; removing an absent one is not an error.
    fn delete(&self, provider: &str) -> Result<(), SecretError>;
}

/// An in-memory [`SecretStore`] for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    /// Reads from the map.
    fn get(&self, provider: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().expect("secrets lock").get(provider).cloned())
    }
    /// Writes to the map.
    fn set(&self, provider: &str, key: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .expect("secrets lock")
            .insert(provider.into(), key.into());
        Ok(())
    }
    /// Removes from the map.
    fn delete(&self, provider: &str) -> Result<(), SecretError> {
        self.0.lock().expect("secrets lock").remove(provider);
        Ok(())
    }
}

/// What can go wrong reading or changing the settings.
#[derive(Debug)]
pub enum SettingsError {
    /// The value is not acceptable (the text says why).
    Invalid(String),
    /// The database failed.
    Store(StoreError),
    /// The secret store failed.
    Secret(SecretError),
}

impl fmt::Display for SettingsError {
    /// Renders the error for logs and for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Invalid(m) => write!(f, "{m}"),
            SettingsError::Store(e) => write!(f, "{e}"),
            SettingsError::Secret(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<StoreError> for SettingsError {
    /// Wraps a storage error.
    fn from(e: StoreError) -> Self {
        SettingsError::Store(e)
    }
}

impl From<SecretError> for SettingsError {
    /// Wraps a secret-store error.
    fn from(e: SecretError) -> Self {
        SettingsError::Secret(e)
    }
}

/// One provider as Settings shows it. It says whether a key exists, never what it is.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    /// The provider's name (the part before the first `:` of a model string).
    pub name: String,
    /// The model used when none is named.
    pub default_model: String,
    /// Whether the provider cannot be used without a key.
    pub needs_key: bool,
    /// Whether a key is available, from the keychain or the environment.
    pub has_key: bool,
    /// The environment variable that can supply the key.
    pub key_env: Option<String>,
}

/// Everything the Settings screen shows about the model.
#[derive(Debug, Clone, Serialize)]
pub struct LlmSettings {
    /// The chosen `provider:model`, if any (no model is hard-wired).
    pub model: Option<String>,
    /// The user's context-window limit in tokens, if set.
    pub context_tokens: Option<u64>,
    /// Every provider bmlib supports.
    pub providers: Vec<ProviderInfo>,
}

/// The key for a provider: the keychain's, else its environment variable's.
pub fn api_key(
    spec: &ProviderSpec,
    secrets: &dyn SecretStore,
) -> Result<Option<String>, SecretError> {
    let stored = secrets.get(spec.name)?;
    Ok(spec.resolve_api_key(stored.as_deref()))
}

/// Reads the model, the window limit and each provider's key status.
pub fn llm_settings(
    db: &mut dyn Db,
    secrets: &dyn SecretStore,
) -> Result<LlmSettings, SettingsError> {
    let model = get_setting(db, LLM_MODEL_KEY)?;
    let context_tokens = match get_setting(db, CONTEXT_TOKENS_KEY)? {
        None => None,
        Some(t) => Some(t.parse::<u64>().map_err(|_| {
            SettingsError::Store(StoreError::Corrupt(format!(
                "setting {CONTEXT_TOKENS_KEY} is {t:?}, not a number"
            )))
        })?),
    };
    let mut providers = Vec::new();
    for spec in provider_specs().values() {
        let needs_key = spec.requires_api_key();
        providers.push(ProviderInfo {
            name: spec.name.to_string(),
            default_model: spec.default_model.to_string(),
            needs_key,
            // Local providers are not asked, so the keychain is not prompted for them.
            has_key: needs_key && api_key(spec, secrets)?.is_some(),
            key_env: spec.api_key_env.map(str::to_string),
        });
    }
    Ok(LlmSettings {
        model,
        context_tokens,
        providers,
    })
}

/// Remembers the chosen `provider:model`; refuses an empty model or an unknown provider.
pub fn set_model(db: &mut dyn Db, model: &str) -> Result<(), SettingsError> {
    let model = model.trim();
    let (provider, name) = parse_model_string(Some(model), DEFAULT_PROVIDER);
    if model.is_empty() || name.is_empty() {
        return Err(SettingsError::Invalid("the model is empty".into()));
    }
    if !provider_specs().contains_key(provider.as_str()) {
        return Err(SettingsError::Invalid(format!(
            "unknown provider {provider:?}"
        )));
    }
    Ok(set_setting(db, LLM_MODEL_KEY, model)?)
}

/// Remembers a context-window limit in tokens; refuses 0.
pub fn set_context_tokens(db: &mut dyn Db, tokens: u64) -> Result<(), SettingsError> {
    if tokens == 0 {
        return Err(SettingsError::Invalid(
            "the context window must be at least 1 token".into(),
        ));
    }
    Ok(set_setting(db, CONTEXT_TOKENS_KEY, &tokens.to_string())?)
}

/// Forgets the context-window limit.
pub fn clear_context_tokens(db: &mut dyn Db) -> Result<(), SettingsError> {
    Ok(delete_setting(db, CONTEXT_TOKENS_KEY)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::open_memory_store;

    /// A chosen model is read back, whitespace trimmed.
    #[test]
    fn model_remembered() {
        let mut db = open_memory_store().unwrap();
        set_model(&mut db, " ollama:gemma4:31b ").unwrap();
        let s = llm_settings(&mut db, &MemorySecrets::default()).unwrap();
        assert_eq!(s.model.as_deref(), Some("ollama:gemma4:31b"));
        assert_eq!(s.context_tokens, None);
        assert!(s
            .providers
            .iter()
            .any(|p| p.name == "ollama" && !p.needs_key));
    }

    /// An unknown provider or an empty model is refused, and nothing is stored.
    #[test]
    fn unknown_provider_refused() {
        let mut db = open_memory_store().unwrap();
        for bad in ["nosuch:model", "", "  ", "ollama:"] {
            assert!(matches!(
                set_model(&mut db, bad),
                Err(SettingsError::Invalid(_))
            ));
        }
        assert_eq!(get_setting(&mut db, LLM_MODEL_KEY).unwrap(), None);
    }

    /// A window of 0 is refused; a set one is read back and can be cleared.
    #[test]
    fn zero_window_refused() {
        let mut db = open_memory_store().unwrap();
        assert!(matches!(
            set_context_tokens(&mut db, 0),
            Err(SettingsError::Invalid(_))
        ));
        set_context_tokens(&mut db, 8192).unwrap();
        let secrets = MemorySecrets::default();
        assert_eq!(
            llm_settings(&mut db, &secrets).unwrap().context_tokens,
            Some(8192)
        );
        clear_context_tokens(&mut db).unwrap();
        assert_eq!(
            llm_settings(&mut db, &secrets).unwrap().context_tokens,
            None
        );
    }

    /// The keychain's key beats the environment's; without it the environment answers.
    #[test]
    fn keychain_wins_over_env() {
        let spec = ProviderSpec {
            api_key_env: Some("STUDYANALYZER_TEST_KEY_ENV"),
            ..provider_specs()["openai"].clone()
        };
        std::env::set_var("STUDYANALYZER_TEST_KEY_ENV", "k2");
        let secrets = MemorySecrets::default();
        secrets.set(spec.name, "k1").unwrap();
        assert_eq!(api_key(&spec, &secrets).unwrap().as_deref(), Some("k1"));
        secrets.delete(spec.name).unwrap();
        assert_eq!(api_key(&spec, &secrets).unwrap().as_deref(), Some("k2"));
        std::env::remove_var("STUDYANALYZER_TEST_KEY_ENV");
        assert_eq!(api_key(&spec, &secrets).unwrap(), None);
    }

    /// A stored key shows as has_key, appears in no settings row and not in the serialised settings.
    #[test]
    fn key_never_in_settings() {
        let mut db = open_memory_store().unwrap();
        let secrets = MemorySecrets::default();
        secrets.set("anthropic", "sk-secret-123").unwrap();
        set_model(&mut db, "anthropic:claude").unwrap();
        let s = llm_settings(&mut db, &secrets).unwrap();
        assert!(s
            .providers
            .iter()
            .any(|p| p.name == "anthropic" && p.has_key));
        assert!(!serde_json::to_string(&s).unwrap().contains("sk-secret-123"));
        assert!(!format!("{s:?}").contains("sk-secret-123"));
        for key in ["llm_model", "context_tokens"] {
            let v = get_setting(&mut db, key).unwrap().unwrap_or_default();
            assert!(!v.contains("sk-secret-123"));
        }
        let rows = bmlib::db::fetch_all(&mut db, "SELECT key, value FROM settings", &[]).unwrap();
        for r in rows {
            assert!(!r.get_str("value").unwrap().contains("sk-secret-123"));
        }
    }
}
