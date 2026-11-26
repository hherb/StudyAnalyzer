//! The state Tauri manages: where the database lives (each command opens its
//! own connection), the app's directories, the running lookup, answer and
//! download (so they can be stopped), the API keys, the model limits learnt
//! this launch and the loaded search model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bmlib::embed::Embedder;
use bmlib::llm::ModelLimits;
use studyanalyzer_core::bookmark::Access;
use studyanalyzer_core::config::{
    DB_FILE_NAME, EMBEDDER_SUBDIR, EMBEDDING_MODEL, LIBRARY_LABEL_FALLBACK, RETRIEVAL_DEADLINE,
};
use studyanalyzer_core::embedder::{load_embedder, ModelSlot};
use studyanalyzer_core::library::{configured_with, Configured};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::llm_settings::SecretStore;
use studyanalyzer_core::store::open_store;

use crate::error::CommandError;
use crate::grants::FileGrants;

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
    /// The running PRISMA assessment, so `stop_prisma` can stop it; one at a
    /// time ([`StopSlot::try_begin`]).
    pub prisma_stop: StopSlot,
    /// The running evidence weight assessment, so `stop_evidence` can stop
    /// it; one at a time ([`StopSlot::try_begin`]).
    pub evidence_stop: StopSlot,
    /// The appraisal whose paired run holds `evidence_stop`, while one does
    /// (`None` while the slot is free or a phase 2 run holds it): so setting
    /// that appraisal aside is refused, and any other appraisal says its run
    /// waits.
    pub evidence_appraisal: Mutex<Option<i64>>,
    /// The running search-model download, so `cancel_download` can stop it.
    pub download_stop: StopSlot,
    /// Where the API keys are kept (the OS keychain in the app).
    pub secrets: Box<dyn SecretStore>,
    /// Each model's context window and output limit as its provider gave them
    /// this launch (either field `None` when the provider does not state it),
    /// so it is asked once per model.
    pub limits: Mutex<HashMap<String, ModelLimits>>,
    /// The search model, kept under app data (not the cache, so a cache clear
    /// does not cost another 1.1 GB download), loaded once needed. Unloaded
    /// before the app exits: on Metal, llama.cpp aborts at exit while a model
    /// is loaded.
    pub embedder: ModelSlot<Embedder>,
    /// The files the user chose or dropped, by grant (the webview can only pass a grant back, never a path).
    pub grants: FileGrants,
    /// How this installation's records in the shared library name it
    /// ([`library_label`]).
    pub library_label: String,
    /// Held while the library setting changes, waiting sources settle, a
    /// paper opens (whose sources settle into the library it read) or the
    /// manual clear lists or removes, so one runs at a time ([`AppState::try_library_op`]).
    pub library_op: Mutex<()>,
    /// The library folder's security-scoped access, started at launch from
    /// its bookmark (macOS), or on a later focus when the launch could not
    /// (`resume_again` in lib.rs, #126), and stopped at exit by dropping it.
    pub library_access: Mutex<Option<Access>>,
    /// What the launch could not keep or reach of the library folder
    /// (`library_setting::Resumed::Held`'s `unsaved`, a `Resumed::Lost`
    /// reason, or why `resume` failed). Every report of the library ends an
    /// unreachable reason with it, and the status shows it while the library
    /// is reachable; forgotten once the setting changes (#130, #134), and
    /// replaced by each focus's try while the launch could not reach it.
    pub library_launch_note: Mutex<Option<String>>,
}

/// This installation's label in the shared library's records: the app and
/// the machine's name, else the app alone.
pub fn library_label() -> String {
    let host = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    if host.is_empty() {
        LIBRARY_LABEL_FALLBACK.to_string()
    } else {
        format!("{LIBRARY_LABEL_FALLBACK} on {host}")
    }
}

/// Locks `m`, using the value as is if a panic poisoned it. Only for the
/// plain slots here (a lookup, a stop flag, a window cache), each valid
/// whatever the panicking thread was doing; the embedder's slot instead drops
/// its model after a panic (`ModelSlot`).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| {
        log::warn!("a slot's lock was poisoned by a panic; using its value as is");
        e.into_inner()
    })
}

/// The stop flag of the one running operation of a kind (an answer, a
/// download). Each run gets a fresh flag, so a stop meant for an earlier run
/// never stops a later one.
#[derive(Default)]
pub struct StopSlot(Mutex<Option<Arc<AtomicBool>>>);

impl StopSlot {
    /// Starts a run with a fresh, unset flag and makes it the one
    /// [`StopSlot::stop`] sets, replacing any still registered. Guard the
    /// claim with [`Release`], so a run that panics cannot leave its flag
    /// behind for a later stop to find.
    pub fn begin(&self) -> Claim {
        let flag = Arc::new(AtomicBool::new(false));
        *lock(&self.0) = Some(flag.clone());
        Claim(flag)
    }

    /// Starts a run only when none is running: a claim on a fresh, unset flag,
    /// or `None` while another run holds the slot. Checked and set under one
    /// lock, so two calls racing (a double-click) cannot both start. Guard the
    /// claim with [`Release`]; a slot claimed this way must never be
    /// [`StopSlot::begin`]-ed, which would replace the claim.
    pub fn try_begin(&self) -> Option<Claim> {
        let mut slot = lock(&self.0);
        if slot.is_some() {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(flag.clone());
        Some(Claim(flag))
    }

    /// Forgets `flag` once its run is over, unless a newer run replaced it.
    pub fn end(&self, flag: &Arc<AtomicBool>) {
        let mut slot = lock(&self.0);
        if slot.as_ref().is_some_and(|f| Arc::ptr_eq(f, flag)) {
            *slot = None;
        }
    }

    /// Whether a run holds the slot.
    pub fn busy(&self) -> bool {
        lock(&self.0).is_some()
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

/// A run's hold on a [`StopSlot`], from [`StopSlot::begin`] or
/// [`StopSlot::try_begin`] only (its flag is private), so [`Release`] can
/// guard nothing but a real claim.
pub struct Claim(Arc<AtomicBool>);

/// Ends a run's claim on its [`StopSlot`] when dropped, so the slot is freed
/// on every path out of the run, a panic's unwind included (a slot claimed by
/// [`StopSlot::try_begin`] would otherwise refuse every later run, and one
/// from [`StopSlot::begin`] would take the next stop meant for a later run).
pub struct Release<'a> {
    /// The slot the run claimed.
    slot: &'a StopSlot,
    /// The run's flag, the one the slot holds.
    flag: Arc<AtomicBool>,
}

impl<'a> Release<'a> {
    /// Guards `claim`, taken from `slot` by [`StopSlot::try_begin`].
    pub fn new(slot: &'a StopSlot, claim: Claim) -> Self {
        Release {
            slot,
            flag: claim.0,
        }
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

/// Ends a lookup's registration when dropped, a panic's unwind included, so
/// a crashed lookup cannot take the next Cancel meant for a later one.
pub struct EndLookup<'a> {
    /// The state the lookup is registered in.
    state: &'a AppState,
    /// The lookup's limit, the one the slot holds.
    limit: Limit,
}

impl<'a> EndLookup<'a> {
    /// Guards `limit`, returned by [`AppState::begin_lookup`] on `state`.
    pub fn new(state: &'a AppState, limit: Limit) -> Self {
        EndLookup { state, limit }
    }

    /// The lookup's limit.
    pub fn limit(&self) -> &Limit {
        &self.limit
    }
}

impl Drop for EndLookup<'_> {
    /// Ends the lookup's registration.
    fn drop(&mut self) {
        self.state.end_lookup(&self.limit);
    }
}

impl AppState {
    /// Opens a fresh connection to the paper database (one per command, so a
    /// slow open never blocks another command).
    pub fn connect(&self) -> Result<rusqlite::Connection, CommandError> {
        Ok(open_store(&self.data_dir.join(DB_FILE_NAME))?)
    }

    /// The shared paper library the settings name, opened (or why it cannot
    /// be used, ending with the launch's note, #134), or none.
    pub fn library(&self, db: &mut rusqlite::Connection) -> Result<Configured, CommandError> {
        let note = self.launch_note();
        Ok(configured_with(db, &self.library_label, note.as_deref())?)
    }

    /// What the launch could not keep or reach of the library folder
    /// (`library_launch_note`), for every caller that reports the library.
    pub fn launch_note(&self) -> Option<String> {
        lock(&self.library_launch_note).clone()
    }

    /// Waits for, then holds, the library lock (taken by a change of the
    /// setting, a settle of waiting sources, every paper open, and the manual
    /// clear's list and removals).
    pub fn library_op(&self) -> MutexGuard<'_, ()> {
        lock(&self.library_op)
    }

    /// The library lock if nothing holds it now (a change, a settle, an open
    /// or the clear): settling when the app becomes active skips a turn rather than
    /// wait. A lock poisoned by a panic is taken as it is (it guards no data).
    pub fn try_library_op(&self) -> Option<MutexGuard<'_, ()>> {
        match self.library_op.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
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
    /// replaced it. Commands use [`EndLookup`], which calls this on every path.
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
            prisma_stop: StopSlot::default(),
            evidence_stop: StopSlot::default(),
            evidence_appraisal: Mutex::new(None),
            download_stop: StopSlot::default(),
            secrets: Box::new(studyanalyzer_core::llm_settings::MemorySecrets::default()),
            limits: Mutex::new(HashMap::new()),
            grants: FileGrants::default(),
            library_label: "StudyAnalyzer on a test".to_string(),
            library_op: Mutex::new(()),
            library_access: Mutex::new(None),
            library_launch_note: Mutex::new(None),
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

    /// The label names the app, and the machine when it has a name.
    #[test]
    fn library_label_names_the_app() {
        let label = library_label();
        assert!(label.starts_with(LIBRARY_LABEL_FALLBACK), "{label}");
    }

    /// The library lock is held by one change at a time.
    #[test]
    fn library_op_is_exclusive() {
        let s = state();
        let held = s.try_library_op().expect("free at first");
        assert!(s.try_library_op().is_none());
        drop(held);
        assert!(s.try_library_op().is_some());
    }

    /// No library setting is no library; a folder named in the settings is
    /// opened with this state's label.
    #[test]
    fn library_follows_the_settings() {
        let s = state();
        let mut db = studyanalyzer_core::store::open_memory_store().unwrap();
        assert!(matches!(s.library(&mut db).unwrap(), Configured::None));
        let dir = std::env::temp_dir().join(format!("sa-state-lib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = studyanalyzer_core::config::LIBRARY_PATH_KEY;
        studyanalyzer_core::store::set_setting(&mut db, key, dir.to_str().unwrap()).unwrap();
        match s.library(&mut db).unwrap() {
            Configured::Ready(lib) => assert_eq!(lib.who.label, s.library_label),
            other => panic!("not ready: {other:?}"),
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// An unreachable library ends with the stored launch fault, unless the
    /// launch has a note: then that fault is an earlier launch's (#132) and
    /// the note ends it instead (#134).
    #[test]
    fn library_ends_with_the_launch_note_over_the_fault() {
        use studyanalyzer_core::config::{LIBRARY_BOOKMARK_FAULT_KEY, LIBRARY_PATH_KEY};
        use studyanalyzer_core::store::set_setting;
        let s = state();
        let mut db = studyanalyzer_core::store::open_memory_store().unwrap();
        let gone = std::env::temp_dir().join(format!("sa-state-gone-{}", uuid::Uuid::new_v4()));
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        let why = |s: &AppState, db: &mut rusqlite::Connection| match s.library(db).unwrap() {
            Configured::Unreachable(why) => why,
            other => panic!("not unreachable: {other:?}"),
        };
        assert!(why(&s, &mut db).ends_with("; at launch: earlier"));
        *lock(&s.library_launch_note) = Some("not kept".into());
        let now = why(&s, &mut db);
        assert!(
            now.ends_with("; at launch: not kept") && !now.contains("earlier"),
            "{now}"
        );
    }

    /// Stop reaches the running run only; each run starts unset; ending an
    /// older run leaves a newer one stoppable.
    #[test]
    fn stop_slot_per_run() {
        let slot = state().answer_stop;
        assert!(!slot.stop());
        let first = slot.begin().0;
        assert!(slot.stop());
        assert!(first.load(Ordering::SeqCst));
        let second = slot.begin().0;
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
        let first = slot.try_begin().unwrap().0;
        assert!(slot.try_begin().is_none());
        assert!(slot.stop());
        assert!(first.load(Ordering::SeqCst));
        slot.end(&first);
        let second = slot.try_begin().unwrap().0;
        assert!(!second.load(Ordering::SeqCst));
        assert!(!Arc::ptr_eq(&first, &second));
    }

    /// A claim is released when its guard drops, a panic's unwind included,
    /// so a crashed run never leaves the slot held.
    #[test]
    fn release_survives_a_panic() {
        let slot = StopSlot::default();
        let claim = slot.try_begin().unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _release = Release::new(&slot, claim);
            panic!("a run failed");
        }));
        assert!(unwound.is_err());
        assert!(slot.try_begin().is_some());
    }

    /// A run from `begin` that panics leaves no flag behind: a later stop
    /// finds no run instead of setting the dead run's flag.
    #[test]
    fn begun_run_released_on_panic() {
        let slot = StopSlot::default();
        let claim = slot.begin();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _release = Release::new(&slot, claim);
            panic!("a run failed");
        }));
        assert!(unwound.is_err());
        assert!(!slot.stop());
    }

    /// A lookup that panics leaves nothing to cancel.
    #[test]
    fn lookup_ended_on_panic() {
        let s = state();
        let limit = s.begin_lookup();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _end = EndLookup::new(&s, limit);
            panic!("a lookup failed");
        }));
        assert!(unwound.is_err());
        assert!(!s.cancel_lookup());
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
