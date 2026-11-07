//! Which model answers, how big its window may be, and where API keys come from.
//!
//! The model and window live in the `settings` table. Keys never do: they sit
//! behind [`SecretStore`] (the OS keychain in the app), with the provider's
//! environment variable as the fallback. No key is ever logged, put in an
//! error, or handed to the frontend (only whether one exists).

use std::fmt;
use std::sync::Arc;

use bmlib::db::Db;
use bmlib::http::UreqClient;
use bmlib::llm::{parse_model_string, provider_specs, LlmClient, ProviderSpec, DEFAULT_PROVIDER};
use bmlib::secrets::API_KEYS_ACCOUNT;
/// The key store (bmlib #634): one keychain item in the app, a map in tests.
pub use bmlib::secrets::{MemorySecrets, SecretError, SecretErrorKind, SecretStore};
use serde::Serialize;

use crate::config::{CONTEXT_TOKENS_KEY, KEYCHAIN_SERVICE, LLM_MODEL_KEY};
use crate::store::{delete_setting, get_setting, set_setting, StoreError};

/// What can go wrong reading or changing the settings.
#[derive(Debug)]
pub enum SettingsError {
    /// The value is not acceptable (the text says why).
    Invalid(String),
    /// The database failed.
    Store(StoreError),
    /// The secret store failed.
    Secret(SecretError),
    /// The provider needs a key, the keychain could not be read, and the
    /// environment variable is not set either.
    KeyUnavailable {
        /// The provider that needs the key.
        provider: String,
        /// The environment variable that could supply it.
        env_var: String,
        /// Why the keychain could not be read (the platform's message, no key).
        reason: String,
        /// What will help, from [`keychain_advice`].
        advice: &'static str,
    },
}

/// What will help when the keychain cannot be read, by bmlib's kind (#689):
/// a damaged item is replaced by saving a key, two matching items need one
/// removed by hand, and a store failure (or a kind bmlib adds later) needs
/// the keychain unlocked, access allowed, or a keychain where there is none.
pub fn keychain_advice(kind: SecretErrorKind) -> &'static str {
    match kind {
        SecretErrorKind::UnreadableItem => {
            "The item is damaged: saving a key replaces it and drops the keys stored there."
        }
        SecretErrorKind::AmbiguousItem => {
            "More than one keychain item matches: remove one in the keychain; saving a key does not help."
        }
        _ => "Unlock the keychain or allow access, or set one up if this system has none; saving a key does not help.",
    }
}

impl fmt::Display for SettingsError {
    /// Renders the error for logs and for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Invalid(m) => write!(f, "{m}"),
            SettingsError::Store(e) => write!(f, "{e}"),
            SettingsError::Secret(e) => write!(f, "{e}"),
            SettingsError::KeyUnavailable {
                provider,
                env_var,
                reason,
                advice,
            } => write!(
                f,
                "{provider} needs an API key, and the keychain could not be read ({reason}). {advice} Setting {env_var} also works."
            ),
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
    /// Where the key the provider would be called with comes from; `None`
    /// when there is none (or the provider needs none).
    pub key_source: Option<KeySource>,
    /// The environment variable that can supply the key.
    pub key_env: Option<String>,
}

/// Where a provider's key comes from. The keychain wins over the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// Saved in the OS keychain by the user.
    Keychain,
    /// The provider's environment variable (`ProviderInfo::key_env`).
    Env,
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
    /// Set when the keychain could not be read: `has_key` then reflects the
    /// environment only. A message for the user; it never holds a key.
    pub keychain_error: Option<String>,
}

/// Where `spec`'s key comes from: the keychain if it holds one, else the
/// environment, else nowhere. Only whether a key exists is looked at.
///
/// # Errors
/// The keychain's [`SecretError`] when it cannot be read.
pub fn key_source(
    spec: &ProviderSpec,
    secrets: &dyn SecretStore,
) -> Result<Option<KeySource>, SecretError> {
    if secrets.get(spec.name)?.is_some() {
        return Ok(Some(KeySource::Keychain));
    }
    Ok(env_source(spec))
}

/// `Some(Env)` when `spec`'s environment variable holds a key, else `None`.
fn env_source(spec: &ProviderSpec) -> Option<KeySource> {
    spec.resolve_api_key(None).map(|_| KeySource::Env)
}

/// The key for a provider: the keychain's, else its environment variable's.
pub fn api_key(
    spec: &ProviderSpec,
    secrets: &dyn SecretStore,
) -> Result<Option<String>, SecretError> {
    let stored = secrets.get(spec.name)?;
    Ok(spec.resolve_api_key(stored.as_deref()))
}

/// The key to call `spec`'s provider with: the keychain's, else the
/// environment's. An unreadable keychain degrades to the environment (logged
/// as a warning); with no key there either it is
/// [`SettingsError::KeyUnavailable`], so the user learns the keychain failed.
/// `Ok(None)` is a readable keychain with no key: the call then reports the
/// missing key itself.
pub fn key_for_asking(
    spec: &ProviderSpec,
    secrets: &dyn SecretStore,
) -> Result<Option<String>, SettingsError> {
    match api_key(spec, secrets) {
        Ok(key) => Ok(key),
        Err(e) => {
            log::warn!(
                "keychain unreadable, using the environment's key for {}: {e}",
                spec.name
            );
            match (spec.resolve_api_key(None), spec.api_key_env) {
                (Some(key), _) => Ok(Some(key)),
                // No variable: bmlib's `requires_api_key` says it needs no key.
                (None, None) => Ok(None),
                (None, Some(env_var)) => Err(SettingsError::KeyUnavailable {
                    provider: spec.name.to_string(),
                    env_var: env_var.to_string(),
                    reason: e.message().to_owned(),
                    advice: keychain_advice(e.kind()),
                }),
            }
        }
    }
}

/// A client for `model`'s provider with its key (see [`key_for_asking`]),
/// and the provider's name. A local provider's keychain entry is not read.
///
/// # Errors
/// [`SettingsError::Invalid`] for an unknown provider; as [`key_for_asking`].
pub fn llm_client(
    model: &str,
    secrets: &dyn SecretStore,
) -> Result<(String, LlmClient), SettingsError> {
    let (provider, _) = parse_model_string(Some(model), DEFAULT_PROVIDER);
    let specs = provider_specs();
    let spec = specs.get(provider.as_str()).ok_or_else(|| {
        SettingsError::Invalid(format!(
            "the model {model:?} names no known provider ({provider:?})"
        ))
    })?;
    let mut client = LlmClient::new(Arc::new(UreqClient::new()));
    if spec.requires_api_key() {
        client.api_key = key_for_asking(spec, secrets)?;
    }
    Ok((provider, client))
}

/// The environment variable that could supply `provider`'s key, when the
/// provider needs one and `client` (from [`llm_client`]) has none; `None` for
/// a provider that needs no key, has one, or is unknown. bmlib folds a missing
/// key into a failed request, so a caller checks this before sending anything.
pub fn missing_key(provider: &str, client: &LlmClient) -> Option<&'static str> {
    let specs = provider_specs();
    let spec = specs.get(provider)?;
    // A keyed provider names its variable (`requires_api_key` needs one).
    spec.api_key_env
        .filter(|_| spec.requires_api_key() && client.api_key.is_none())
}

/// Removes a provider's API key (absent is fine); refuses an unknown provider.
pub fn remove_api_key(secrets: &dyn SecretStore, provider: &str) -> Result<(), SettingsError> {
    if !provider_specs().contains_key(provider) {
        return Err(SettingsError::Invalid(format!(
            "unknown provider {provider:?}"
        )));
    }
    Ok(secrets.delete(provider)?)
}

/// Stores a provider's API key (trimmed); refuses an unknown provider or a blank key.
/// A damaged keychain item is logged first: bmlib's `set` then replaces it.
pub fn save_api_key(
    secrets: &dyn SecretStore,
    provider: &str,
    key: &str,
) -> Result<(), SettingsError> {
    if !provider_specs().contains_key(provider) {
        return Err(SettingsError::Invalid(format!(
            "unknown provider {provider:?}"
        )));
    }
    let key = key.trim();
    if key.is_empty() {
        return Err(SettingsError::Invalid("the API key is empty".into()));
    }
    // bmlib's notice of a replaced item goes to stderr, which a packaged app
    // drops. A loaded cache answers this for free but misses an item damaged
    // since; otherwise it is one extra keychain read (`set` re-reads too).
    // Another kind, if it lasts, fails the `set` below, which reports it; but
    // `set` reads again, so a keychain unlocked in between can still save.
    if let Err(e) = secrets.get(provider) {
        if e.kind() == SecretErrorKind::UnreadableItem {
            log::warn!(
                "the keychain item is damaged ({e}); saving {provider}'s key replaces it \
                 with one holding just this key"
            );
        } else {
            log::info!(
                "the keychain could not be read before saving {provider}'s key ({e}); \
                 the save reads it again"
            );
        }
    }
    Ok(secrets.set(provider, key)?)
}

/// The user's context-window limit in tokens, if set.
pub fn context_tokens(db: &mut dyn Db) -> Result<Option<u64>, SettingsError> {
    match get_setting(db, CONTEXT_TOKENS_KEY)? {
        None => Ok(None),
        Some(t) => Ok(Some(t.parse::<u64>().map_err(|_| {
            SettingsError::Store(StoreError::Corrupt(format!(
                "setting {CONTEXT_TOKENS_KEY} is {t:?}, not a number"
            )))
        })?)),
    }
}

/// Reads the model, the window limit and each provider's key status.
pub fn llm_settings(
    db: &mut dyn Db,
    secrets: &dyn SecretStore,
) -> Result<LlmSettings, SettingsError> {
    let model = get_setting(db, LLM_MODEL_KEY)?;
    let context_tokens = context_tokens(db)?;
    let mut providers = Vec::new();
    let mut keychain_error: Option<String> = None;
    for spec in provider_specs().values() {
        let needs_key = spec.requires_api_key();
        // Local providers are not asked; after one failure the keychain is not asked again.
        let source = if !needs_key {
            None
        } else if keychain_error.is_some() {
            env_source(spec)
        } else {
            match key_source(spec, secrets) {
                Ok(source) => source,
                Err(e) => {
                    log::warn!("keychain unreadable, showing environment keys only: {e}");
                    keychain_error = Some(format!(
                        "The keychain item {API_KEYS_ACCOUNT} of {KEYCHAIN_SERVICE} could not be read ({e}); \
                         only keys in the environment are shown. {}",
                        keychain_advice(e.kind())
                    ));
                    env_source(spec)
                }
            }
        };
        providers.push(ProviderInfo {
            name: spec.name.to_string(),
            default_model: spec.default_model.to_string(),
            needs_key,
            has_key: source.is_some(),
            key_source: source,
            key_env: spec.api_key_env.map(str::to_string),
        });
    }
    Ok(LlmSettings {
        model,
        context_tokens,
        providers,
        keychain_error,
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

    /// A store whose every call fails, as a locked or denied keychain does.
    struct Broken;
    impl SecretStore for Broken {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            Err(SecretError::new("locked"))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::new("locked"))
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::new("locked"))
        }
    }

    /// A failing keychain degrades Settings to env-only key status, with a message.
    #[test]
    fn broken_keychain_degrades() {
        let mut db = open_memory_store().unwrap();
        let s = llm_settings(&mut db, &Broken).unwrap();
        let msg = s.keychain_error.expect("error reported");
        assert!(msg.contains("locked"));
        // Names the item so the user can find it, and says what helps.
        assert!(msg.contains(KEYCHAIN_SERVICE) && msg.contains(API_KEYS_ACCOUNT));
        assert!(msg.contains(keychain_advice(SecretErrorKind::Store)));
        assert_eq!(s.providers.len(), provider_specs().len());
        let specs = provider_specs();
        for p in &s.providers {
            assert_eq!(
                p.has_key,
                specs[p.name.as_str()].resolve_api_key(None).is_some() && p.needs_key
            );
            assert_eq!(p.key_source.is_some(), p.has_key);
            assert_ne!(p.key_source, Some(KeySource::Keychain));
        }
        assert!(api_key(&specs["openai"], &Broken).is_err());
    }

    /// A store whose reads fail with one kind, as a damaged, doubled or
    /// locked item does.
    struct FailsWith(SecretErrorKind);
    impl SecretStore for FailsWith {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            Err(SecretError::with_kind(self.0, "failed"))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::with_kind(self.0, "failed"))
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::with_kind(self.0, "failed"))
        }
    }

    /// The banner warns that a save replaces the item only when it does: on
    /// a damaged item, not on a doubled one or a locked keychain (#689).
    #[test]
    fn the_banner_says_what_helps_for_each_kind() {
        let mut db = open_memory_store().unwrap();
        let mut banner = |kind| {
            llm_settings(&mut db, &FailsWith(kind))
                .unwrap()
                .keychain_error
                .expect("error reported")
        };
        let damaged = banner(SecretErrorKind::UnreadableItem);
        assert!(damaged.contains("saving a key replaces it"), "{damaged}");
        let doubled = banner(SecretErrorKind::AmbiguousItem);
        assert!(doubled.contains("remove one"), "{doubled}");
        let locked = banner(SecretErrorKind::Store);
        assert!(locked.contains("Unlock"), "{locked}");
        for msg in [doubled, locked] {
            assert!(!msg.contains("replaces it"), "{msg}");
        }
    }

    /// The ask's error carries the same advice as the banner.
    #[test]
    fn the_ask_says_what_helps() {
        let spec = ProviderSpec {
            api_key_env: Some("STUDYANALYZER_TEST_ADVICE_ENV"),
            ..provider_specs()["anthropic"].clone()
        };
        let kind = SecretErrorKind::AmbiguousItem;
        let e = key_for_asking(&spec, &FailsWith(kind)).unwrap_err();
        assert!(e.to_string().contains(keychain_advice(kind)), "{e}");
    }

    /// A save over a doubled item fails and says so; only a damaged item is
    /// replaced by a save.
    #[test]
    fn save_over_a_doubled_item_fails() {
        let e =
            save_api_key(&FailsWith(SecretErrorKind::AmbiguousItem), "openai", "k1").unwrap_err();
        assert!(matches!(e, SettingsError::Secret(_)), "{e}");
    }

    /// A healthy keychain reports no error.
    #[test]
    fn healthy_keychain_has_no_error() {
        let mut db = open_memory_store().unwrap();
        let s = llm_settings(&mut db, &MemorySecrets::default()).unwrap();
        assert_eq!(s.keychain_error, None);
    }

    /// save_api_key refuses blank keys and unknown providers, and stores a good key trimmed.
    #[test]
    fn save_api_key_validates() {
        let secrets = MemorySecrets::default();
        for (p, k) in [("openai", ""), ("openai", "  \n"), ("nosuch", "k")] {
            assert!(matches!(
                save_api_key(&secrets, p, k),
                Err(SettingsError::Invalid(_))
            ));
        }
        assert_eq!(secrets.get("openai").unwrap(), None);
        save_api_key(&secrets, "openai", " k1 \n").unwrap();
        assert_eq!(secrets.get("openai").unwrap().as_deref(), Some("k1"));
    }

    /// A store that cannot be read but can be written, as a damaged item is.
    #[derive(Default)]
    struct Unreadable(MemorySecrets);
    impl SecretStore for Unreadable {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            Err(SecretError::with_kind(
                SecretErrorKind::UnreadableItem,
                "not a valid key list",
            ))
        }
        fn set(&self, p: &str, k: &str) -> Result<(), SecretError> {
            self.0.set(p, k)
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::with_kind(
                SecretErrorKind::UnreadableItem,
                "not a valid key list",
            ))
        }
    }

    /// The read before a save (to log a replaced item) never blocks the save.
    #[test]
    fn save_over_an_unreadable_store_still_saves() {
        let secrets = Unreadable::default();
        save_api_key(&secrets, "openai", "k1").unwrap();
        assert_eq!(secrets.0.get("openai").unwrap().as_deref(), Some("k1"));
    }

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
        assert_eq!(context_tokens(&mut db).unwrap(), Some(8192));
        clear_context_tokens(&mut db).unwrap();
        assert_eq!(
            llm_settings(&mut db, &secrets).unwrap().context_tokens,
            None
        );
        assert_eq!(context_tokens(&mut db).unwrap(), None);
    }

    /// A keychain that cannot be read degrades to the environment's key;
    /// with none there either the error says the keychain failed, and names
    /// the variable, never a key; a provider with no variable gets no key.
    #[test]
    fn unreadable_keychain_degrades_when_asking() {
        let spec = ProviderSpec {
            api_key_env: Some("STUDYANALYZER_TEST_DEGRADE_ENV"),
            ..provider_specs()["anthropic"].clone()
        };
        std::env::set_var("STUDYANALYZER_TEST_DEGRADE_ENV", "k-env");
        assert_eq!(
            key_for_asking(&spec, &Broken).unwrap().as_deref(),
            Some("k-env")
        );
        std::env::remove_var("STUDYANALYZER_TEST_DEGRADE_ENV");
        let e = key_for_asking(&spec, &Broken).unwrap_err();
        assert!(matches!(e, SettingsError::KeyUnavailable { .. }), "{e}");
        let text = e.to_string();
        assert!(
            text.contains("keychain") && text.contains("STUDYANALYZER_TEST_DEGRADE_ENV"),
            "{text}"
        );
        let healthy = MemorySecrets::default();
        assert_eq!(key_for_asking(&spec, &healthy).unwrap(), None);
        let no_variable = ProviderSpec {
            api_key_env: None,
            ..spec
        };
        assert_eq!(key_for_asking(&no_variable, &Broken).unwrap(), None);
    }

    /// A client for a known provider carries its stored key (none for a local
    /// one, whose keychain entry is not read); an unknown provider is refused.
    #[test]
    fn llm_client_for_the_model() {
        let secrets = MemorySecrets::default();
        secrets.set("anthropic", "k1").unwrap();
        let (provider, client) = llm_client("anthropic:claude", &secrets).unwrap();
        assert_eq!(
            (provider.as_str(), client.api_key.as_deref()),
            ("anthropic", Some("k1"))
        );
        let (provider, client) = llm_client("ollama:m", &Broken).unwrap();
        assert_eq!((provider.as_str(), client.api_key), ("ollama", None));
        let Err(e) = llm_client("nosuch:model", &secrets) else {
            panic!("an unknown provider was accepted");
        };
        assert!(matches!(e, SettingsError::Invalid(_)) && e.to_string().contains("nosuch"));
    }

    /// A keyed provider without a key names its variable; a local provider,
    /// or a keyed one whose key came from the keychain or the environment,
    /// names none.
    #[test]
    fn missing_key_for_keyed_provider() {
        let secrets = MemorySecrets::default();
        let spec = &provider_specs()["anthropic"];
        let (_, mut client) = llm_client("anthropic:claude", &secrets).unwrap();
        // The environment may hold a key on a developer's machine; drop it here.
        client.api_key = None;
        assert_eq!(missing_key("anthropic", &client), spec.api_key_env);
        assert!(spec.api_key_env.is_some());
        client.api_key = Some("from the environment".into());
        assert_eq!(missing_key("anthropic", &client), None);
        secrets.set("anthropic", "k1").unwrap();
        let (_, client) = llm_client("anthropic:claude", &secrets).unwrap();
        assert_eq!(missing_key("anthropic", &client), None);
        let (_, client) = llm_client("ollama:m", &secrets).unwrap();
        assert_eq!(missing_key("ollama", &client), None);
    }

    /// Every provider that needs a key names the variable that can supply it,
    /// so a message asking for one never names an empty variable (#58).
    #[test]
    fn every_keyed_provider_names_its_variable() {
        for (name, spec) in provider_specs() {
            if spec.requires_api_key() {
                assert!(spec.api_key_env.is_some_and(|v| !v.is_empty()), "{name}");
            }
        }
    }

    /// Removing a key refuses an unknown provider and removes a known one's.
    #[test]
    fn remove_api_key_checks_the_provider() {
        let secrets = MemorySecrets::default();
        secrets.set("anthropic", "k1").unwrap();
        assert!(matches!(
            remove_api_key(&secrets, "nosuch"),
            Err(SettingsError::Invalid(_))
        ));
        remove_api_key(&secrets, "anthropic").unwrap();
        assert_eq!(secrets.get("anthropic").unwrap(), None);
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

    /// The source is the keychain when it holds a key (even with the variable
    /// set), the environment when only it does, none without either; an
    /// unreadable keychain is an error.
    #[test]
    fn key_source_for_each_place() {
        let spec = ProviderSpec {
            api_key_env: Some("STUDYANALYZER_TEST_SOURCE_ENV"),
            ..provider_specs()["openai"].clone()
        };
        let secrets = MemorySecrets::default();
        std::env::remove_var("STUDYANALYZER_TEST_SOURCE_ENV");
        assert_eq!(key_source(&spec, &secrets).unwrap(), None);
        std::env::set_var("STUDYANALYZER_TEST_SOURCE_ENV", "k-env");
        assert_eq!(key_source(&spec, &secrets).unwrap(), Some(KeySource::Env));
        secrets.set(spec.name, "k1").unwrap();
        assert_eq!(
            key_source(&spec, &secrets).unwrap(),
            Some(KeySource::Keychain)
        );
        std::env::remove_var("STUDYANALYZER_TEST_SOURCE_ENV");
        assert_eq!(
            key_source(&spec, &secrets).unwrap(),
            Some(KeySource::Keychain)
        );
        assert!(key_source(&spec, &Broken).is_err());
    }

    /// A stored key shows as has_key, appears in no settings row and not in the serialised settings.
    #[test]
    fn key_never_in_settings() {
        let mut db = open_memory_store().unwrap();
        let secrets = MemorySecrets::default();
        secrets.set("anthropic", "sk-secret-123").unwrap();
        set_model(&mut db, "anthropic:claude").unwrap();
        let s = llm_settings(&mut db, &secrets).unwrap();
        assert!(s.providers.iter().any(|p| p.name == "anthropic"
            && p.has_key
            && p.key_source == Some(KeySource::Keychain)));
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
