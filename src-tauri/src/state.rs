//! The state Tauri manages: where the database lives (each command opens its
//! own connection), the app's directories, the PDF backend and the running
//! lookup (for `cancel_lookup`).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use studyanalyzer_core::config::{DB_FILE_NAME, RETRIEVAL_DEADLINE};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::store::open_store;

use crate::error::CommandError;
use bmlib::fulltext::models::TextBlock;
use bmlib::fulltext::pdf_text::PdfTextExtractor;
use bmlib::fulltext::pdfium_backend::PdfiumExtractor;

/// Sub-directory of the cache holding PDFium's unpacked library.
pub const PDFIUM_CACHE_SUBDIR: &str = "pdfium";
/// Sub-directory of the cache holding bmlib's full-text cache.
pub const FULLTEXT_CACHE_SUBDIR: &str = "fulltext";
/// Sub-directory of the cache for temporary PDF files.
pub const SCRATCH_SUBDIR: &str = "scratch";
/// The environment variable bmlib reads to relocate PDFium's cache.
pub const PDFIUM_CACHE_ENV: &str = "PDFIUM_BUNDLED_CACHE_DIR";

/// Everything the commands share.
pub struct AppState {
    /// The app's data directory (database), read by `connect()`.
    pub data_dir: PathBuf,
    /// The app's cache directory (full-text cache, scratch, PDFium).
    pub cache_dir: PathBuf,
    /// The PDF backend, bound on first use; the error text if binding failed.
    pub pdf: OnceLock<Result<PdfiumExtractor, String>>,
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

    /// The PDF backend, binding PDFium the first time. A failed binding is
    /// remembered and reported by every later PDF open; other formats work.
    pub fn pdf_extractor(&self) -> PdfBackend<'_> {
        let bound = self.pdf.get_or_init(|| {
            PdfiumExtractor::bundled().inspect_err(|e| log::error!("PDFium unavailable: {e}"))
        });
        PdfBackend(bound)
    }
}

/// Adapts the stored binding result to [`PdfTextExtractor`]: a failed binding
/// becomes the error of every extraction.
pub struct PdfBackend<'a>(&'a Result<PdfiumExtractor, String>);

impl PdfBackend<'_> {
    /// Why PDFium could not be bound, if it could not.
    pub fn binding_error(&self) -> Option<&str> {
        self.0.as_ref().err().map(String::as_str)
    }

    /// The working extractor, or the binding error.
    fn get(&self) -> Result<&PdfiumExtractor, String> {
        self.0.as_ref().map_err(|e| e.clone())
    }
}

impl PdfTextExtractor for PdfBackend<'_> {
    /// The backend's name, for logs.
    fn name(&self) -> &'static str {
        "pdfium"
    }

    /// The PDF's positioned text blocks, or the binding error.
    fn extract_blocks(&self, path: &Path) -> Result<Vec<TextBlock>, String> {
        self.get()?.extract_blocks(path)
    }

    /// The PDF's text page by page, or the binding error.
    fn page_texts(&self, path: &Path) -> Result<Vec<String>, String> {
        self.get()?.page_texts(path)
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
            pdf: OnceLock::new(),
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

    /// A failed binding is reported by both extraction calls, not a panic.
    #[test]
    fn failed_binding_is_reported() {
        let failed: Result<PdfiumExtractor, String> = Err("no pdfium".into());
        let b = PdfBackend(&failed);
        assert_eq!(b.extract_blocks(Path::new("x")).unwrap_err(), "no pdfium");
        assert_eq!(b.page_texts(Path::new("x")).unwrap_err(), "no pdfium");
    }
}
