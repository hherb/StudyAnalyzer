//! The state Tauri manages: where the database lives (each command opens its
//! own connection), the app's directories, the running lookup, answer and
//! download (so they can be stopped), the API keys, the context windows learnt
//! this launch and the loaded search model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bmlib::embed::Embedder;
use studyanalyzer_core::config::{
    DB_FILE_NAME, EMBEDDER_SUBDIR, EMBEDDING_MODEL, RETRIEVAL_DEADLINE,
};
use studyanalyzer_core::embedder::{load_embedder, ModelSlot};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::llm_settings::SecretStore;
use studyanalyzer_core::store::open_store;

use crate::error::CommandError;

/// Sub-directory of the cache holding bmlib's full-text cache.
pub const FULLTEXT_CACHE_SUBDIR: &str = "fulltext";
/// Sub-directory of the cache for temporary PDF files.
pub const SCRATCH_SUBDIR: &str = "scratch";

/// Everything the commands share.
pub struct AppState {
    /// The app's data directory (database), read by `connect()`.
    pub data_dir: PathBuf,
    /// The app's cache directory (full-text cache, scratch).
    pub cache_dir: PathBuf,
    /// The running lookup by identifier, so `cancel_lookup` can stop it.
    pub lookup: Mutex<Option<Limit>>,
    /// The running answer, so `stop_answer` can stop it.
    pub answer_stop: StopSlot,
    /// The running PICO extraction, so `stop_pico` can stop it; one at a time
    /// ([`StopSlot::try_begin`]).
    pub pico_stop: StopSlot,
    /// The running search-model download, so `cancel_download` can stop it.
    pub download_stop: StopSlot,
    /// Where the API keys are kept (the OS keychain in the app).
    pub secrets: Box<dyn SecretStore>,
    /// Each model's context window as its provider gave it this launch
    /// (`None`: the provider states none), so it is asked once per model.
    pub windows: Mutex<HashMap<String, Option<u64>>>,
    /// The search model, kept under app data (not the cache, so a cache clear
    /// does not cost another 1.1 GB download), loaded once needed. Unloaded
    /// before the app exits: on Metal, llama.cpp aborts at exit while a model
    /// is loaded.
    pub embedder: ModelSlot<Embedder>,
}

/// Locks `m`, using the value as is if a panic poisoned it. Only for the
/// plain slots here (a lookup, a stop flag, a window cache), each valid
/// whatever the panicking thread was doing; the embedder's slot instead drops
/// its model after a panic (`ModelSlot`).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The stop flag of the one running operation of a kind (an answer, a
/// download). Each run gets a fresh flag, so a stop meant for an earlier run
/// never stops a later one.
#[derive(Default)]
pub struct StopSlot(Mutex<Option<Arc<AtomicBool>>>);

impl StopSlot {
    /// Starts a run with a fresh, unset flag and makes it the one
    /// [`StopSlot::stop`] sets, replacing any still registered.
    pub fn begin(&self) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        *lock(&self.0) = Some(flag.clone());
        flag
    }

    /// Starts a run only when none is running: a fresh, unset flag, or `None`
    /// while another run holds the slot. Checked and set under one lock, so two
    /// calls racing (a double-click) cannot both start.
    pub fn try_begin(&self) -> Option<Arc<AtomicBool>> {
        let mut slot = lock(&self.0);
        if slot.is_some() {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(flag.clone());
        Some(flag)
    }

    /// Forgets `flag` once its run is over, unless a newer run replaced it.
    pub fn end(&self, flag: &Arc<AtomicBool>) {
        let mut slot = lock(&self.0);
        if slot.as_ref().is_some_and(|f| Arc::ptr_eq(f, flag)) {
            *slot = None;
        }
    }

    /// Stops the running operation; false when none is running.
    pub fn stop(&self) -> bool {
        match lock(&self.0).as_ref() {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

/// Ends a run's claim on its [`StopSlot`] when dropped, so the slot is freed
/// on every path out of the run, a panic's unwind included (a slot claimed by
/// [`StopSlot::try_begin`] would otherwise refuse every later run).
pub struct Release<'a> {
    slot: &'a StopSlot,
    flag: Arc<AtomicBool>,
}

impl<'a> Release<'a> {
    /// Guards `flag`, the run's flag from `slot`.
    pub fn new(slot: &'a StopSlot, flag: Arc<AtomicBool>) -> Self {
        Release { slot, flag }
    }

    /// The run's stop flag.
    pub fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for Release<'_> {
    /// Ends the run's claim.
    fn drop(&mut self) {
        self.slot.end(&self.flag);
    }
}

impl AppState {
    /// Opens a fresh connection to the paper database (one per command, so a
    /// slow open never blocks another command).
    pub fn connect(&self) -> Result<rusqlite::Connection, CommandError> {
        Ok(open_store(&self.data_dir.join(DB_FILE_NAME))?)
    }

    /// The full-text cache directory.
    pub fn fulltext_dir(&self) -> PathBuf {
        self.cache_dir.join(FULLTEXT_CACHE_SUBDIR)
    }

    /// The scratch directory for temporary PDF files.
    pub fn scratch_dir(&self) -> PathBuf {
        self.cache_dir.join(SCRATCH_SUBDIR)
    }

    /// The search model's slot for app data directory `data_dir`.
    pub fn embedder_slot(data_dir: &std::path::Path) -> ModelSlot<Embedder> {
        ModelSlot::new(
            EMBEDDING_MODEL,
            data_dir.join(EMBEDDER_SUBDIR),
            load_embedder,
        )
    }

    /// Starts a lookup with the full time limit and makes it the one
    /// [`AppState::cancel_lookup`] stops. It replaces any lookup still
    /// registered, which then can no longer be cancelled; the UI runs one
    /// lookup at a time.
    pub fn begin_lookup(&self) -> Limit {
        let limit = Limit::new(RETRIEVAL_DEADLINE);
        *self.lookup_slot() = Some(limit.clone());
        limit
    }

    /// Forgets `limit` once its lookup is over, unless a newer lookup has
    /// replaced it.
    pub fn end_lookup(&self, limit: &Limit) {
        let mut slot = self.lookup_slot();
        if slot.as_ref().is_some_and(|l| l.same_lookup(limit)) {
            *slot = None;
        }
    }

    /// Cancels the running lookup; false when none is running.
    pub fn cancel_lookup(&self) -> bool {
        match self.lookup_slot().as_ref() {
            Some(limit) => {
                limit.cancel();
                true
            }
            None => false,
        }
    }

    /// A state over `data_dir` (the cache beside it) with in-memory keys and
    /// nothing running or loaded, for tests.
    #[cfg(test)]
    pub fn for_tests(data_dir: PathBuf) -> Self {
        AppState {
            cache_dir: data_dir.join("cache"),
            embedder: Self::embedder_slot(&data_dir),
            data_dir,
            lookup: Mutex::new(None),
            answer_stop: StopSlot::default(),
            pico_stop: StopSlot::default(),
            download_stop: StopSlot::default(),
            secrets: Box::new(studyanalyzer_core::llm_settings::MemorySecrets::default()),
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// The lookup slot; a panic while it was held cannot leave it
    /// inconsistent (it is one `Option`), so a poisoned lock is used as is.
    fn lookup_slot(&self) -> MutexGuard<'_, Option<Limit>> {
        lock(&self.lookup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studyanalyzer_core::limit::Stop;

    /// A state with no directories in use.
    fn state() -> AppState {
        AppState::for_tests(PathBuf::new())
    }

    /// Stop reaches the running run only; each run starts unset; ending an
    /// older run leaves a newer one stoppable.
    #[test]
    fn stop_slot_per_run() {
        let slot = state().answer_stop;
        assert!(!slot.stop());
        let first = slot.begin();
        assert!(slot.stop());
        assert!(first.load(Ordering::SeqCst));
        let second = slot.begin();
        assert!(!second.load(Ordering::SeqCst), "a new run starts unset");
        slot.end(&first);
        assert!(slot.stop());
        assert!(second.load(Ordering::SeqCst));
        slot.end(&second);
        assert!(!slot.stop());
    }

    /// The model goes under app data, not the cache.
    #[test]
    fn embedder_lives_in_app_data() {
        let s = AppState::for_tests(PathBuf::from("/data"));
        assert_eq!(
            s.embedder.path(),
            PathBuf::from("/data")
                .join(EMBEDDER_SUBDIR)
                .join(EMBEDDING_MODEL.file)
        );
    }

    /// A second run cannot start while one holds the slot; once it ends one
    /// can; a stop reaches the flag `try_begin` returned.
    #[test]
    fn try_begin_refuses_a_second() {
        let slot = StopSlot::default();
        let first = slot.try_begin().unwrap();
        assert!(slot.try_begin().is_none());
        assert!(slot.stop());
        assert!(first.load(Ordering::SeqCst));
        slot.end(&first);
        let second = slot.try_begin().unwrap();
        assert!(!second.load(Ordering::SeqCst));
        assert!(!Arc::ptr_eq(&first, &second));
    }

    /// A claim is released when its guard drops, a panic's unwind included,
    /// so a crashed run never leaves the slot held.
    #[test]
    fn release_survives_a_panic() {
        let slot = StopSlot::default();
        let flag = slot.try_begin().unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _release = Release::new(&slot, flag);
            panic!("a run failed");
        }));
        assert!(unwound.is_err());
        assert!(slot.try_begin().is_some());
    }

    /// Cancel reaches the running lookup; after it ends there is none to
    /// cancel; ending an older lookup leaves a newer one in place.
    #[test]
    fn lookup_cancel_and_end() {
        let s = state();
        assert!(!s.cancel_lookup());
        let first = s.begin_lookup();
        assert!(s.cancel_lookup());
        assert_eq!(first.remaining(), Err(Stop::Cancelled));
        let second = s.begin_lookup();
        assert!(
            second.remaining().is_ok(),
            "a new lookup starts uncancelled"
        );
        s.end_lookup(&first);
        assert!(s.cancel_lookup());
        assert_eq!(second.remaining(), Err(Stop::Cancelled));
        s.end_lookup(&second);
        assert!(!s.cancel_lookup());
    }
}
