# HANDOVER

_Last updated: 2026-10-28._

Working notes for picking up in-flight work. Each section is one self-contained
slice: what's known, where to start, and how to verify. Remove a section once
its slice has landed; add a new section when handing off new work. Planned
work not yet started is in `ROADMAP.md`.

---

## State

- **This repository holds the design only**: no code yet. It is on GitHub
  at `hherb/StudyAnalyzer` (public, AGPL-3.0); `main` has the design, session
  docs, README and CI scaffold (PR #1). CI (`.github/workflows/ci.yml`)
  is a scaffold whose steps start running once `Cargo.toml` / `package.json`
  exist. The design (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is
  marked *draft for approval*.
- **Phase 1 runs in bmlib** (`~/src/bmlib`). bmlib keeps its own HANDOVER,
  ROADMAP and CHANGELOG for that work, and its `docs/SESSION-RULES.md` apply
  there. This file only tracks where phase 1 stands.

## In flight

**Item 4b, canonical text from PDF**: bmlib **PR #556** (branch
`feat/canonical-pdf`, worktree `~/src/bmlib-pdf`), all gates green in both
languages, awaiting merge. `canonical_from_segmented(document)` takes the
segmenter's `SegmentedDocument` (`"pdf-1"`). For phase 2: pass the paper's
known title to `segment_document()` as `metadata["title"]`; front matter and
the no-headings fallback are untitled sections; the text is as extracted, so
a line-break hyphen reads `dose- response`, a title read off page 1 is often
printed again in the front matter, and a table quote may fail to locate
(§5.2). PyMuPDF (Python) and PDFium (Rust) can segment one PDF differently;
the app uses Rust's.

## Next

**The next phase 1 item.** In the order of `ROADMAP.md`: **4c** (HTML).
**Item 6** (streaming) is independent and can go at any point.

Start each item in bmlib: read bmlib's `HANDOVER.md` and
`docs/SESSION-RULES.md`, write the item's plan in
`~/src/bmlib/docs/superpowers/plans/`, and branch off bmlib's `main`. Verify
with bmlib's gates in both languages (`CLAUDE.md` § Gates). When the PR
merges, move the item out of `ROADMAP.md` and into *Landed* below.

## Owed by the maintainer

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (`find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).

## Landed (context)

Phase 1 items merged in bmlib, all **unreleased** (bmlib 0.11.0 predates them):

- **Item 3, RAG chunker**: PR #554. `chunk_canonical(canonical,
  max_chars=1000, overlap_chars=100)` takes a `CanonicalText` (not its
  string) and returns chunks that are each a code-point span into the
  canonical text plus the index of their section. For phase 2: a chunk never
  crosses a section and never passes `max_chars` (an over-long sentence is cut
  at whitespace, or mid-token at the cap where a piece has none); the title
  and headings are not chunked, so prefix the heading when embedding if
  wanted; store `CHUNKER_VERSION` with each chunk beside the canonical text's
  `converter_version`.
- **Item 4a, canonical text (model, JATS, Markdown, plain text)**: PR #547.
  `bmlib.fulltext.canonical` lays a paper out as one string, the title and
  every heading and block a code-point span. Item 4 is three PRs (the
  maintainer's call, 2026-10-28): 4a, then 4b PDF and 4c HTML. The JATS text
  keeps everything `to_html()` shows but the metadata lines, references
  included. For phase 2: store `CanonicalText.to_dict()` with its
  `converter_version`; a table row is a `table_row` block with tab-separated
  cells; Markdown keeps a section's `#` level after the title (so its top
  sections may be level 2, where JATS starts at 1). Open: bmlib #548, #549,
  #552, #553.
- **Item 9, retrieval returns its source**: PR #545. `FullTextResult.jats_xml`
  holds the JATS bytes as served; the cache keeps them in `jats/`; a PDF stays
  at `file_path`. An old HTML-only cache entry is served with `jats_xml=None`,
  not re-fetched. So **StudyAnalyzer gives `FullTextService` its own cache
  directory** in app storage, and treats `jats_xml=None` on a `fulltext`
  result as "no structured source".

- **Item 2, passage matcher**: PR #526, `find_passage` in Python and Rust,
  fixture `tests/fixtures/passage_match.json`, plus the bmlibrarian oracle. Open
  issues: #528, #529. PR #544 fixed #533 (the timing tests now bound windows
  scored, not wall-clock time).
- **Item 1, embedder**: PR #531 (`bmlib.embed`; Rust over `llama-cpp-2`,
  Python over llama-cpp-python as `bmlib[embed]`), then #534 (refuses NaN,
  infinite or zero vectors) and #535 (error types matched across the two
  languages). Shared contract `embedding_contract.json`.
- **Item 10, our own GGUF**: in PR #531.
  `scripts/convert_embedding_model.py`; F16 and Q8_0 published at
  `hherb/snowflake-arctic-embed-l-v2.0-gguf`, pinned by revision, size and
  sha256.

Phase 2 depends on bmlib through a path or git dependency until a release
includes these (spec §11).
