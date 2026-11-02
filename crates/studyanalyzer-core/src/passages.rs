//! Finding the passages of a paper that best match a question, by embedding,
//! through bmlib's chunk store.

use std::fmt;

use bmlib::db::Db;
use bmlib::embed::store::{
    index_paper, search, ChunkEmbedder, ChunkHit, StoreError as ChunkStoreError,
};
use bmlib::fulltext::canonical::CanonicalText;

use crate::config::RETRIEVAL_PASSAGES;

/// Why passages could not be found.
#[derive(Debug)]
pub enum PassageError {
    /// The user cancelled indexing; nothing new was stored.
    Cancelled,
    /// bmlib's chunk store failed.
    Store(ChunkStoreError),
}

impl fmt::Display for PassageError {
    /// A sentence for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PassageError::Cancelled => write!(f, "Indexing the paper was cancelled"),
            PassageError::Store(e) => write!(f, "Could not find passages in the paper: {e}"),
        }
    }
}

impl std::error::Error for PassageError {}

impl From<ChunkStoreError> for PassageError {
    /// `Cancelled` keeps its own variant; the rest is wrapped.
    fn from(e: ChunkStoreError) -> Self {
        match e {
            ChunkStoreError::Cancelled => PassageError::Cancelled,
            other => PassageError::Store(other),
        }
    }
}

/// The key a canonical text's chunk set is stored under.
pub fn index_key(canonical_id: i64) -> String {
    format!("canonical:{canonical_id}")
}

/// Logs a failed step for `key`: `info` for a cancel, `warn` otherwise.
fn log_failure(key: &str, step: &str, e: &ChunkStoreError) {
    if matches!(e, ChunkStoreError::Cancelled) {
        log::info!("{key}: {step} cancelled by the user");
    } else {
        log::warn!("{key}: {step} failed: {e}");
    }
}

/// The [`RETRIEVAL_PASSAGES`] passages of `canonical` closest to `question`.
/// A paper not yet indexed, or indexed from another text, model or chunker, is
/// indexed first (`on_progress(done, total)` returns false to cancel).
pub fn find_passages(
    db: &mut dyn Db,
    canonical_id: i64,
    canonical: &CanonicalText,
    question: &str,
    embedder: &mut dyn ChunkEmbedder,
    on_progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Vec<ChunkHit>, PassageError> {
    let key = index_key(canonical_id);
    match search(db, &key, canonical, question, embedder, RETRIEVAL_PASSAGES) {
        Err(
            e @ (ChunkStoreError::NotIndexed(_)
            | ChunkStoreError::Stale(_)
            | ChunkStoreError::Corrupt(_)),
        ) => {
            if matches!(e, ChunkStoreError::NotIndexed(_)) {
                log::info!("indexing {key}: {e}");
            } else {
                log::warn!("re-indexing {key}: {e}");
            }
            index_paper(db, &key, canonical, embedder, Some(on_progress)).map_err(|e| {
                log_failure(&key, "re-index", &e);
                PassageError::from(e)
            })?;
            search(db, &key, canonical, question, embedder, RETRIEVAL_PASSAGES).map_err(|e| {
                log_failure(&key, "search after indexing", &e);
                PassageError::from(e)
            })
        }
        other => Ok(other?),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::EMBEDDING_MODEL;
    use crate::store::open_memory_store;
    use bmlib::embed::store::BoxError;
    use bmlib::embed::{ModelFile, EMBEDDING_DIM};
    use bmlib::fulltext::canonical::canonical_from_markdown;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    /// Embeds by word hashes, so texts sharing words are close.
    pub(crate) struct FakeEmbedder;

    /// A unit vector with one weight per hashed word.
    fn embed(text: &str) -> Vec<f32> {
        let mut v = vec![0f32; EMBEDDING_DIM];
        for w in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            let mut h = DefaultHasher::new();
            w.to_lowercase().hash(&mut h);
            v[(h.finish() as usize) % EMBEDDING_DIM] += 1.0;
        }
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1.0);
        v.iter().map(|x| x / n).collect()
    }

    impl ChunkEmbedder for FakeEmbedder {
        fn model(&self) -> Option<&ModelFile> {
            Some(&EMBEDDING_MODEL)
        }
        fn embed_documents(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, BoxError> {
            Ok(texts.iter().map(|t| embed(t)).collect())
        }
        fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, BoxError> {
            Ok(embed(query))
        }
    }

    /// A paper with long, distinct sections so it makes several chunks.
    fn paper(tag: &str) -> CanonicalText {
        let filler = |w: &str| format!("{w} ").repeat(300);
        canonical_from_markdown(&format!(
            "# Trial {tag}\n\n## Methods\n\n{}\n\n## Results\n\n{}\n\n## Discussion\n\n{}\n",
            filler("alpha"),
            filler("hazard ratio"),
            filler("limitations")
        ))
    }

    /// The key is the canonical id under a fixed prefix.
    #[test]
    fn index_key_is_prefixed() {
        assert_eq!(index_key(7), "canonical:7");
    }

    /// The first call indexes and finds; the second needs no indexing.
    #[test]
    fn indexes_then_finds() {
        let mut db = open_memory_store().unwrap();
        let text = paper("a");
        let mut seen = 0;
        let hits = find_passages(
            &mut db,
            1,
            &text,
            "hazard ratio",
            &mut FakeEmbedder,
            &mut |_, _| {
                seen += 1;
                true
            },
        )
        .unwrap();
        assert!(seen > 0);
        assert!(!hits.is_empty());
        let top = &hits[0];
        assert!(text.text[top.span.start..top.span.end].contains("hazard"));
        let mut again = 0;
        let second = find_passages(
            &mut db,
            1,
            &text,
            "hazard ratio",
            &mut FakeEmbedder,
            &mut |_, _| {
                again += 1;
                true
            },
        )
        .unwrap();
        assert_eq!(again, 0);
        assert_eq!(second.len(), hits.len());
    }

    /// A set indexed from another text under the same key is re-indexed.
    #[test]
    fn stale_reindexes() {
        let mut db = open_memory_store().unwrap();
        index_paper(&mut db, &index_key(1), &paper("a"), &mut FakeEmbedder, None).unwrap();
        let text = paper("b");
        let mut seen = 0;
        let hits = find_passages(
            &mut db,
            1,
            &text,
            "limitations",
            &mut FakeEmbedder,
            &mut |_, _| {
                seen += 1;
                true
            },
        )
        .unwrap();
        assert!(seen > 0);
        assert!(text.text[hits[0].span.start..hits[0].span.end].contains("limitations"));
    }

    /// A progress callback returning false cancels.
    #[test]
    fn cancel_keeps_nothing_new() {
        let mut db = open_memory_store().unwrap();
        index_paper(&mut db, &index_key(1), &paper("a"), &mut FakeEmbedder, None).unwrap();
        let r = find_passages(
            &mut db,
            1,
            &paper("b"),
            "alpha",
            &mut FakeEmbedder,
            &mut |_, _| false,
        );
        assert!(matches!(r, Err(PassageError::Cancelled)));
        assert!(r.unwrap_err().to_string().contains("cancelled"));
        // The old set survives: searching the old text indexes nothing.
        let hits = find_passages(
            &mut db,
            1,
            &paper("a"),
            "alpha",
            &mut FakeEmbedder,
            &mut |_, _| panic!("re-indexed"),
        )
        .unwrap();
        assert!(!hits.is_empty());
    }
}
