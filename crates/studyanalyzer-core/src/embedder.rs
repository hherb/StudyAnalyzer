//! The search model on this device: whether it is downloaded, loading it once
//! per launch, downloading it, and lending it to an ask.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use bmlib::embed::store::{BoxError, ChunkEmbedder};
use bmlib::embed::{download_model_from, verify_model, Embedder, ModelError, ModelFile};

use crate::retry::with_backoff;

/// Why the search model could not be downloaded, checked or loaded.
#[derive(Debug)]
pub enum EmbedderError {
    /// The user cancelled the download; nothing was kept.
    Cancelled,
    /// The download failed (network, disk, or a file that did not verify).
    Download(ModelError),
    /// The file on disk could not be read to check it.
    Check(ModelError),
    /// The verified file could not be loaded; the text says why.
    Load(String),
}

impl fmt::Display for EmbedderError {
    /// A sentence for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedderError::Cancelled => write!(f, "The download was cancelled."),
            EmbedderError::Download(e) => {
                write!(f, "The search model could not be downloaded: {e}")
            }
            EmbedderError::Check(e) => write!(f, "The search model could not be checked: {e}"),
            EmbedderError::Load(m) => write!(f, "The search model could not be loaded: {m}"),
        }
    }
}

impl std::error::Error for EmbedderError {}

/// Loads a verified model file; [`load_embedder`] in the app, a fake in tests.
pub type Loader<E> = fn(&ModelFile, &Path) -> Result<E, EmbedderError>;

/// Loads `path` as `model` with bmlib (`load_model`, never `load`, so the
/// embedder carries the identity the chunk store labels with).
pub fn load_embedder(model: &ModelFile, path: &Path) -> Result<Embedder, EmbedderError> {
    Embedder::load_model(model, path).map_err(|e| EmbedderError::Load(e.to_string()))
}

/// Whether a failed download attempt is worth another: a network or disk
/// fault, unless the user has cancelled. A mismatch is not (bmlib deleted the
/// file; the source is wrong), nor is a cancel.
pub fn retryable(e: &ModelError, stop: &AtomicBool) -> bool {
    matches!(e, ModelError::Io(_) | ModelError::Http(_)) && !stop.load(Ordering::SeqCst)
}

/// A download's result for the user: once `stop` is set any failure is the
/// cancel (it may have struck during a backoff wait), as is bmlib's own cancel.
fn download_outcome(
    result: Result<PathBuf, ModelError>,
    stop: &AtomicBool,
) -> Result<PathBuf, EmbedderError> {
    match result {
        Ok(path) => Ok(path),
        Err(_) if stop.load(Ordering::SeqCst) => Err(EmbedderError::Cancelled),
        Err(ModelError::Cancelled) => Err(EmbedderError::Cancelled),
        Err(e) => Err(EmbedderError::Download(e)),
    }
}

/// The search model of the app: the pinned file, the directory it is kept in,
/// and the loaded embedder once it is needed.
pub struct ModelSlot<E> {
    slot: Mutex<Option<E>>,
    model: ModelFile,
    dir: PathBuf,
    load: Loader<E>,
}

impl<E> ModelSlot<E> {
    /// A slot for `model` kept in `dir`, nothing loaded yet.
    pub fn new(model: ModelFile, dir: PathBuf, load: Loader<E>) -> Self {
        ModelSlot {
            slot: Mutex::new(None),
            model,
            dir,
            load,
        }
    }

    /// The pinned model file.
    pub fn model(&self) -> &ModelFile {
        &self.model
    }

    /// Where the model file is kept.
    pub fn path(&self) -> PathBuf {
        self.dir.join(self.model.file)
    }

    /// The loaded embedder. After a panic while it was in use (inside
    /// llama.cpp, say) it may be inconsistent, so it is dropped, to be loaded
    /// again when next needed.
    fn lock(&self) -> MutexGuard<'_, Option<E>> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                if guard.take().is_some() {
                    log::error!(
                        "the search model was dropped after a panic while in use; it is loaded again when next needed"
                    );
                }
                self.slot.clear_poison();
                guard
            }
        }
    }

    /// Whether the embedder is loaded.
    pub fn is_loaded(&self) -> bool {
        self.lock().is_some()
    }

    /// Whether the model is on this device: loaded, or a file of the pinned
    /// size. Cheap (no hashing); the checksum is checked when it is loaded.
    pub fn downloaded(&self) -> bool {
        self.is_loaded()
            || std::fs::metadata(self.path())
                .is_ok_and(|m| m.is_file() && m.len() == self.model.size)
    }

    /// Whether the embedder is loaded, loading it first when its file is on
    /// disk and passes its size and checksum check (about 1 s per GB, so call
    /// it on first need, not to show status). A file that fails the check is
    /// left for [`ModelSlot::download`], which replaces it: false then, and
    /// when there is no file.
    ///
    /// # Errors
    /// [`EmbedderError::Check`] when the file cannot be read;
    /// [`EmbedderError::Load`] when it cannot be loaded.
    pub fn ensure_loaded(&self) -> Result<bool, EmbedderError> {
        let mut slot = self.lock();
        if slot.is_some() {
            return Ok(true);
        }
        let path = self.path();
        if !path.is_file() {
            return Ok(false);
        }
        match verify_model(&path, &self.model) {
            Ok(()) => {}
            Err(e @ (ModelError::SizeMismatch { .. } | ModelError::ChecksumMismatch { .. })) => {
                log::warn!(
                    "the search model at {} failed its check and must be downloaded again: {e}",
                    path.display()
                );
                return Ok(false);
            }
            Err(e) => return Err(EmbedderError::Check(e)),
        }
        *slot = Some((self.load)(&self.model, &path)?);
        Ok(true)
    }

    /// Downloads the model from `url` into its directory (bmlib verifies it
    /// and keeps nothing unverified), then loads it. Network and disk faults
    /// are retried `attempts` times with backoff from `base_delay`.
    /// `on_progress(done, total)` sees every block; setting `stop` cancels.
    ///
    /// # Errors
    /// [`EmbedderError::Cancelled`] once `stop` is set; otherwise
    /// [`EmbedderError::Download`] or [`EmbedderError::Load`].
    pub fn download(
        &self,
        url: &str,
        stop: &AtomicBool,
        attempts: u32,
        base_delay: Duration,
        on_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), EmbedderError> {
        let result = with_backoff(
            attempts,
            base_delay,
            |e| retryable(e, stop),
            || {
                download_model_from(url, &self.model, &self.dir, &mut |done, total| {
                    on_progress(done, total);
                    !stop.load(Ordering::SeqCst)
                })
            },
        );
        let path = download_outcome(result, stop)?;
        let mut slot = self.lock();
        if slot.is_none() {
            // Just verified by the download: no second hash.
            *slot = Some((self.load)(&self.model, &path)?);
        }
        Ok(())
    }

    /// Takes the embedder out, to drop it before the app exits.
    pub fn unload(&self) -> Option<E> {
        self.lock().take()
    }

    /// The embedder for an ask, locked only while it embeds.
    pub fn shared(&self) -> SharedEmbedder<'_, E> {
        SharedEmbedder { slot: self }
    }
}

/// A slot's embedder, locked only for each embedding call, so an answer that
/// streams for a minute does not hold it. Its identity is the slot's pinned
/// model, the only one ever loaded into it.
pub struct SharedEmbedder<'a, E> {
    slot: &'a ModelSlot<E>,
}

impl<E: ChunkEmbedder> SharedEmbedder<'_, E> {
    /// Runs `f` on the loaded embedder; an error when none is loaded.
    fn with<T>(&self, f: impl FnOnce(&mut E) -> Result<T, BoxError>) -> Result<T, BoxError> {
        match self.slot.lock().as_mut() {
            Some(e) => f(e),
            None => Err("the search model is not loaded".into()),
        }
    }
}

impl<E: ChunkEmbedder> ChunkEmbedder for SharedEmbedder<'_, E> {
    /// The slot's pinned model.
    fn model(&self) -> Option<&ModelFile> {
        Some(&self.slot.model)
    }
    /// Embeds documents with the loaded embedder.
    fn embed_documents(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, BoxError> {
        self.with(|e| e.embed_documents(texts))
    }
    /// Embeds a query with the loaded embedder.
    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, BoxError> {
        self.with(|e| e.embed_query(query))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::passages::tests::FakeEmbedder;

    /// A tiny pinned "model": the five bytes `model`.
    pub(crate) const TINY: ModelFile = ModelFile {
        repo: "test/tiny",
        revision: "0000000000000000000000000000000000000000",
        file: "tiny.gguf",
        size: 5,
        sha256: "9372c470eeadd5ecd9c3c74c2b3cb633f8e2f2fad799250a0f70d652b6b825e4",
    };

    /// Loads anything as the fake embedder.
    pub(crate) fn fake_load(_: &ModelFile, _: &Path) -> Result<FakeEmbedder, EmbedderError> {
        Ok(FakeEmbedder)
    }

    /// A slot for [`TINY`] in a fresh temporary directory.
    pub(crate) fn tiny_slot() -> ModelSlot<FakeEmbedder> {
        let dir = std::env::temp_dir().join(format!("sa-model-{}", uuid::Uuid::new_v4()));
        ModelSlot::new(TINY, dir, fake_load)
    }

    /// Writes `bytes` as the slot's model file.
    pub(crate) fn put_file(slot: &ModelSlot<FakeEmbedder>, bytes: &[u8]) {
        std::fs::create_dir_all(&slot.dir).unwrap();
        std::fs::write(slot.path(), bytes).unwrap();
    }

    /// A file of the pinned size counts as downloaded without being read;
    /// none, or one of another size, does not.
    #[test]
    fn downloaded_is_a_size_check() {
        let s = tiny_slot();
        assert!(!s.downloaded());
        put_file(&s, b"mode");
        assert!(!s.downloaded());
        put_file(&s, b"xxxxx");
        assert!(s.downloaded(), "same size, not hashed");
        assert!(!s.is_loaded());
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    /// A verified file is loaded once; a file that fails its check, or none,
    /// is not loaded.
    #[test]
    fn ensure_loaded_needs_a_verified_file() {
        let s = tiny_slot();
        assert!(!s.ensure_loaded().unwrap());
        put_file(&s, b"xxxxx");
        assert!(!s.ensure_loaded().unwrap(), "checksum mismatch");
        assert!(!s.is_loaded());
        put_file(&s, b"model");
        assert!(s.ensure_loaded().unwrap());
        assert!(s.is_loaded());
        std::fs::remove_file(s.path()).unwrap();
        assert!(
            s.ensure_loaded().unwrap(),
            "already loaded: the file is not read again"
        );
        assert!(s.unload().is_some() && !s.is_loaded());
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    /// Network and disk faults are retried, unless cancelled; a mismatch or a
    /// cancel never is.
    #[test]
    fn retry_predicate() {
        let go = AtomicBool::new(false);
        let stopped = AtomicBool::new(true);
        let io = || ModelError::Io(std::io::Error::other("disk"));
        assert!(retryable(&io(), &go));
        assert!(retryable(&ModelError::Http("down".into()), &go));
        assert!(!retryable(&io(), &stopped));
        assert!(!retryable(&ModelError::Http("down".into()), &stopped));
        let size = ModelError::SizeMismatch {
            expected: 1,
            actual: 2,
        };
        assert!(!retryable(&size, &go));
        let sum = ModelError::ChecksumMismatch {
            expected: "a".into(),
            actual: "b".into(),
        };
        assert!(!retryable(&sum, &go));
        assert!(!retryable(&ModelError::Cancelled, &go));
        assert!(!retryable(&ModelError::Invalid("x"), &go));
    }

    /// bmlib's cancel, and any failure once stop is set, is the user's cancel.
    #[test]
    fn cancel_maps_to_cancelled() {
        let go = AtomicBool::new(false);
        let stopped = AtomicBool::new(true);
        let http = || Err(ModelError::Http("down".into()));
        assert!(matches!(
            download_outcome(Err(ModelError::Cancelled), &go),
            Err(EmbedderError::Cancelled)
        ));
        assert!(matches!(
            download_outcome(http(), &stopped),
            Err(EmbedderError::Cancelled)
        ));
        assert!(matches!(
            download_outcome(http(), &go),
            Err(EmbedderError::Download(ModelError::Http(_)))
        ));
    }

    /// A download from a closed port fails after its retries; with stop set
    /// it is the cancel after one attempt; nothing is loaded either way.
    #[test]
    fn download_failure_and_cancel() {
        let s = tiny_slot();
        let url = "http://127.0.0.1:1/tiny.gguf";
        let mut seen = 0;
        let mut progress = |_: u64, _: u64| seen += 1;
        let go = AtomicBool::new(false);
        let e = s
            .download(url, &go, 2, Duration::ZERO, &mut progress)
            .unwrap_err();
        assert!(matches!(e, EmbedderError::Download(_)), "{e}");
        let stopped = AtomicBool::new(true);
        let e = s
            .download(url, &stopped, 3, Duration::from_secs(60), &mut progress)
            .unwrap_err();
        assert!(matches!(e, EmbedderError::Cancelled), "{e}");
        assert!(!s.is_loaded());
        assert_eq!(seen, 0);
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    /// A panic while the embedder was in use drops it (it reloads when next
    /// needed) and leaves the slot usable.
    #[test]
    fn poisoned_slot_drops_the_embedder() {
        let s = tiny_slot();
        put_file(&s, b"model");
        assert!(s.ensure_loaded().unwrap());
        std::thread::scope(|t| {
            let _ = t
                .spawn(|| {
                    let _held = s.slot.lock().unwrap();
                    panic!("llama.cpp went wrong");
                })
                .join();
        });
        assert!(!s.is_loaded());
        assert!(!s.slot.is_poisoned());
        assert!(s.ensure_loaded().unwrap());
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    /// The shared embedder embeds through the slot and names its model; with
    /// nothing loaded it fails rather than panics.
    #[test]
    fn shared_embedder_locks_per_call() {
        let s = tiny_slot();
        let mut shared = s.shared();
        assert_eq!(shared.model(), Some(&TINY));
        assert!(shared.embed_query("q").is_err());
        put_file(&s, b"model");
        assert!(s.ensure_loaded().unwrap());
        assert_eq!(shared.embed_documents(&["a", "b"]).unwrap().len(), 2);
        assert!(s.is_loaded(), "not held between calls");
        std::fs::remove_dir_all(&s.dir).unwrap();
    }
}
