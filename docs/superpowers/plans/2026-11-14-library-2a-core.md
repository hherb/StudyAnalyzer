# Shared paper library, step 2a: StudyAnalyzer's sources by digest — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** With a library folder set, every source StudyAnalyzer stores
(a lookup's download, a file, a paste, a hand-off) goes into the shared
library. The database keeps its SHA-256 and kind, not its bytes. Every
read checks the digest. A source the library cannot take yet waits in the
app's storage and the paper's banner says so. Without a library, the app
behaves as it does today.

**Architecture:** A new core module, `crates/studyanalyzer-core/src/library.rs`,
sits between the store and bmlib's `library`. It holds the configured
library, moves waiting sources in, and reads a source's bytes wherever they
are. `store.rs` stays database-only: a source row says where its bytes are
(`place`), and the bytes of a source kept in the app live in a new table,
`source_bytes`. `open.rs` stores every new source as *waiting* when a library
is set and settles it after the transaction commits. Lookups go through
`FullTextService::with_library`.

**Tech Stack:** Rust, bmlib at `02c7fad0` (PR #763's merge), and one new
dependency in `src-tauri`, `gethostname` (Apache-2.0), for the record's label.

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(§4.4–§4.6, §5.2). The library API is bmlib's `rust/bmlib/docs/manual/library.md`.

## Step 2 in three PRs (the maintainer's call, 2026-11-14)

| PR | Scope |
|---|---|
| **2a** (this plan) | The pin moved to `02c7fad0`; migration 11; sources saved to and read from the library, with digests checked; lookups through `with_library`; hand-offs through the same path; the *waiting* state, its banner notice, and its move-in at launch and on every open. The library is the settings key `library_path` (no UI yet), which is enough for dev, Linux and Windows. |
| **2b** | The macOS security-scoped bookmark and entitlements (`files.user-selected.read-write`, `files.bookmarks.app-scope`); the Library setting in Settings (choose, the move of existing papers §4.7, switching folders or none, the list of waiting sources); the unreachable state in the banner; moving waiting sources in when the app becomes active; the macOS 14+ dataless-file device check (design §4.4). |
| **2c** | The manual clear (§4.8). |

## Decided for this plan (builder's calls, within the design)

- **The bytes move to their own table** instead of `sources.bytes` becoming
  nullable (design §5.2 says "nullable"). SQLite cannot relax `NOT NULL`
  without rebuilding `sources`, which `canonical_texts.source_id` and
  `sources.pdf_beside` reference, and a rebuild needs `foreign_keys` off,
  which cannot change inside the migration's transaction. Migration 11 copies
  the bytes into `source_bytes(source_id PRIMARY KEY, bytes BLOB NOT NULL)`
  and drops the column (SQLite 3.35+). A source in the library has no
  `source_bytes` row. Source rows also get smaller, which the comment on
  migration 10 asked for.
- **`sources.place`** is `app` (bytes here, no library), `waiting` (bytes
  here, to be moved in) or `library` (no bytes here; `sha256` and `kind` are
  the reference). `sources.sha256` is backfilled by migration 11 in Rust.
  `sources.library_note` holds why a waiting source is not in the library
  yet.
- **One path for every new source.** With a library set, `persist` and
  `keep_pdf_beside` store the source as `waiting` inside the open's
  transaction. After the commit, `library::settle` moves every waiting source
  in: it saves with `Library::save` (which keeps the bytes once by digest),
  marks the row `library` and drops its bytes. A failure stays `waiting`,
  with the reason in `library_note`. So a library that cannot be reached
  never stops an open (design §4.6), and 2b's move of existing papers (§4.7)
  is the same routine over `app` rows.
- **Settled at launch and after every open, attach and reopen.** Moving
  waiting sources in when the app becomes active belongs to 2b, with its UI.
- **The waiting notice is computed, not stored.** A `Ready` outcome whose
  text's source (or the PDF beside it) is still waiting ends with
  *This paper's file is kept in the app for now: …*, until it moves.
- **Before a source goes in, the paper's folders are checked.** If a JATS in
  any folder `find` returns for the paper names another paper (by
  `jats_identifiers`, the app's own intake rule), nothing is saved and the
  source waits, its note naming the folder. bmlib's `FullTextService` makes
  this check for a lookup but `Library::save` does not, so without it a file
  or a hand-off would be filed beside another paper's JATS. bmlib issue to
  file: `save` (or a public helper) should refuse such a folder for every
  app.
- **What needs the file says so.** A re-derive whose source cannot be read
  (unreachable, missing, damaged, not downloaded) keeps the old text with the
  failure notice and is not recorded, since the fault may pass. The *Original*
  tab returns a `storage` error naming the library's problem. Opening from
  the database never needs the file.
- **`plain_text` maps to the library's `text`** at the library boundary
  (`library::library_kind`); the stored name does not change.
- **The record's paper info** is the paper's canonical identifiers (each
  through bmlib's `library_ids`, a preprint accession left out) and its
  title. Year and first author are not known to the store, so folder names
  are `title (ID)`.
- **The installation id** is a lower-case UUID made on first use and kept
  under the settings key `library_installation`. The label is
  *StudyAnalyzer on ‹host name›*.
- **No index of the library** (design §9). Each read scans once
  (shortcut, noted in the code; bmlib measures about 0.12 s at 1,000 papers).
- **The library is opened per command** from the setting, as the database
  is. 2b replaces where the path comes from on macOS (the bookmark).

## Tasks

### Task 1: the pin

- [x] `rev = "02c7fad0"` in the root `Cargo.toml`. `FullTextResult` gains
  `library_files` and `library_failure`, so the two literals in the tests
  (`open.rs`, `retrieval.rs`) get them. Run `cargo test --workspace`.

### Task 2: migration 11 and the store

**Files:** `schema.rs`, `store.rs`, `config.rs`.

- [x] Migration 11 creates `source_bytes`, copies the bytes, adds `sha256`,
  `place` (default `app`) and `library_note`, backfills `sha256` row by row,
  drops `sources.bytes`, and indexes `(kind, sha256)`.
- [x] `SourcePlace` (`App`, `Waiting`, `Library`) with `as_str`/`parse`.
- [x] `save_source(db, paper, src, place)` writes the row with its digest,
  plus a `source_bytes` row. `save_pdf_beside` takes the place too.
- [x] `StoredSource` loses `bytes` and gains `sha256`, `place`, `origin` and
  `paper_id`. `source_bytes(db, id)` returns the bytes kept here, if any.
- [x] `holder_of_source(db, kind, sha256)` compares digests.
- [x] `original_pdf` becomes `original_pdf_source` and returns the source,
  not its bytes.
- [x] `waiting_sources`, `mark_in_library` (sets `library`, clears the note,
  drops the bytes; only from `waiting`), `note_waiting`.
- [x] Tests: the migration over a database at migration 1 with sources
  (digests, bytes moved, `place = app`), each new function, a corrupt
  `place`.

### Task 3: `library.rs`

- [x] `Configured` (`None`, `Ready(LibraryAt)`, `Unreachable(String)`) from
  the settings (`library_path`, `library_installation`) and a label.
- [x] `library_kind`, `paper_info(db, paper)`.
- [x] `read_source(db, configured, &StoredSource)`: the bytes here, else
  the file of its kind with its digest in a scan, then `Library::read`
  (digest-checked). Errors are sentences naming the paper and the file.
- [x] `settle(db, lib, paper)` and `settle_paper(db, configured, paper)`:
  the folder check, then `save`, then `mark_in_library`; a failure is
  `note_waiting` and a log line. `settle` returns how many moved and which
  are left (`Settled`).
- [x] `waiting_notice(db, paper)`: the banner sentence while the paper's text
  source or its PDF beside is waiting.
- [x] Tests on a temporary library: a save moves and drops the bytes; the
  same bytes twice are one file; an unreachable library leaves the source
  waiting with a note; a folder whose JATS names another paper is refused;
  a read checks the digest (a changed file is an error, a removed file is
  "no longer in the paper library"); `plain_text` is saved as `text`.

### Task 4: open, reopen, attach, lookups

**Files:** `open.rs`, `retrieval.rs`.

- [x] `OpenContext.library: &Configured`. New sources are `waiting` with a
  library set (`Ready` or `Unreachable`), else `app`.
- [x] `stored_copy` compares the kind and the digest.
- [x] A re-derive reads through `read_source`; a read failure is a failure
  that may pass.
- [x] After each commit, `open`, `attach` and `reopen` settle the paper and
  add `waiting_notice` to a `Ready` outcome.
- [x] `build_service(…, library: Option<LibraryAt>)` calls `with_library`;
  `Sources::fetch` logs `library_failure`.
- [x] Tests: an open with a library leaves no bytes in the database and the
  file in the library; an open with an unreachable library is `Ready`, with
  the waiting notice, and a later open after the folder returns moves it; a
  re-derive from the library; a re-derive with the file gone keeps the old
  text and records nothing; a paste of bytes already in the library finds
  its paper by digest.

### Task 5: Tauri

**Files:** `src-tauri/src/state.rs`, `lib.rs`, `commands.rs`, `error.rs`, `Cargo.toml`.

- [x] `AppState.library_label` from `gethostname`. `AppState::library(db)`
  gives the `Configured`.
- [x] `with_context` passes it to `build_service` and `OpenContext`.
- [x] `original_pdf` reads through `read_source`; a failure is `storage`
  with the library's sentence.
- [x] Setup spawns `settle` once the state is managed, and logs the count.

### Task 6: docs

- [x] CLAUDE.md's "received full text is copied into app storage
  immediately" says what happens with a library set.
- [x] HANDOVER (step 2a as built, 2b next) and ROADMAP.
- [x] File the bmlib issue about the folder check: **bmlib #766**.

## Changed while building

- **`OpenError::Library(String)`** is new: a re-derive whose file the
  library cannot give fails with it, and `rederive_failure_recurs` says it
  may pass. In `src-tauri` it maps to `storage`, as `LibraryFault` does, so
  `ErrorKind` and the IPC contract are unchanged (`Storage` now reads "the
  database failed, or the shared paper library cannot give a file").
- **`build_service` takes the `&Configured`** and uses only a `Ready`
  library. `Sources::fetch` logs `library_failure` and nothing else: the
  source found is stored as waiting either way, and `settle` notes its own
  failure. The case bmlib refuses (a folder whose JATS is another paper's)
  is refused again by `foreign_jats`.
- **`store::pdf_beside_source`** is public (the banner needs the PDF
  beside). **`store::paper_identifiers`** is new, for the record's paper
  info. A test-only `store::original_pdf` reads the bytes for the older
  tests.
- **The machine's name** comes from `gethostname` 1.1.0, in
  `src-tauri/src/state.rs` (`library_label`). A label bmlib refuses (empty,
  over 200 characters, control or bidi characters) falls back to
  `StudyAnalyzer` in core.
- **At launch**, `settle_at_launch` runs off the main thread once the state
  is managed, logging the count. Only an open shows a failure (the paper's
  banner); 2b's Settings lists the waiting sources.
- **After review** (PR #116):
  - `settle` and `read_source` re-read a source whose bytes are gone, so a
    source another thread moved in first is passed over or read from the
    library, not reported as corrupt.
  - A source the database fails on is noted and settling goes on with the
    others.
  - `finish` turns a settle failure into a notice: the paper is already
    stored.
  - `read_source` looks for the file of the source's own kind, because
    bmlib's `find_digest` ignores the kind (bmlib #767).
  - `foreign_jats` also checks the folder already holding the bytes (where
    `save` puts them). A JATS there that cannot be read now leaves the
    source waiting.
  - An empty folder becomes a library only the first time it is opened
    (`library_opened`). After that, a folder that has lost its marker, such
    as an unmounted drive's mount point, is unreachable.
  - `save` does not sync the folder before the bytes are dropped (bmlib
    #768).
