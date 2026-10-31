//! The state Tauri manages: where the database lives (each command opens its
//! own connection), the app's directories and the running lookup (for
//! `cancel_lookup`).

use std::path::PathBuf;
use std::sync::Mutex;

use studyanalyzer_core::config::{DB_FILE_NAME, RETRIEVAL_DEADLINE};
use studyanalyzer_core::limit::Limit;
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

    /// The lookup slot; a panic while it was held cannot leave it
    /// inconsistent (it is one `Option`), so a poisoned lock is used as is.
    fn lookup_slot(&self) -> std::sync::MutexGuard<'_, Option<Limit>> {
        self.lookup.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studyanalyzer_core::limit::Stop;

    /// A state with no directories in use.
    fn state() -> AppState {
        AppState {
            data_dir: PathBuf::new(),
            cache_dir: PathBuf::new(),
            lookup: Mutex::new(None),
        }
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
