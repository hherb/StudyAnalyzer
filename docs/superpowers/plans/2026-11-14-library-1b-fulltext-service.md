# Shared paper library, step 1b: `FullTextService` on the library — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** With a library set, bmlib's `FullTextService` looks a paper up in
the library before the network, saves the JATS and PDFs it downloads there
instead of in `FullTextCache`, and tells the caller which library files a
result stands on, or why the library could not be used. Rust and Python on
par.

**Architecture:** One new module per language, `fulltext/library_store`, holds
everything the service needs from the library: mapping a request to
`PaperIds`, picking a paper's JATS or PDF, reading it digest-checked, saving
and noting uses, with each `LibraryError` turned into one warning and a
`library_failure` string. The service gains an optional store beside its
optional cache. It keeps three seams: the lookup before the cache check, the
JATS and PDF saves, and the PDF the chain would download. The cache keeps
the *PDF still owed* mark, the abstract sidecar and, without a library or
when the library fails, everything it holds today.

**Tech Stack:** Rust (bmlib's `library` and `fulltext` modules; no new
dependency) and Python 3.11 (`bmlib.library`, `bmlib.fulltext`).

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(§4.3–§4.6, §5.1). Plan 1a (`2026-11-13-library-1a-bmlib-module.md`) and
bmlib's `docs/manual/library.md` describe the library API as built (PR #754).

**As built (bmlib PR #763).** Where this plan and #763 differ, #763's API,
bmlib's `docs/manual/library.md` and `docs/DECISIONS.md` (*FullTextService
on a library (step 1b)*, which holds the reasons) are authoritative. Write
step 2 and 1c against them:

- A library JATS that is another paper's is a `library_failure`, and that
  lookup keeps nothing in the library (Task 3's "warns, the lookup goes on"
  and its test are superseded). One that does not parse or has no body is
  a warning only.
- `find_source` serves a file only if a record listing it names the paper
  without contradicting the request (absent agrees). A file ranks by the
  earliest `saved_at` among those records; a malformed one ranks last.
- A lookup scans the library once: `scan`, `find_source_in` and
  `note_use_in` are public and share it. `save` still rescans per file
  saved.
- `note_use` returns `Result<bool, String>` (a use was written);
  `path_of` returns `Result<PathBuf, String>`; Python's raise
  `LibraryStoreFailure`. Python's `library_ids` refuses a value that is
  not text (dropped), which Rust's `&str` cannot be given.
- A held library PDF also makes Tier 0 skip the caller's `pdf` entries,
  and a read-only library still serves it (with `library_failure`).
- A library JATS hit without a PDF seeks it once, in the lookup that first
  notes the JATS in this installation's record (with a cache and an
  `identifier`, `fetch_pdf_with_jats` on), not through the cache mark
  alone.
- The shared fixture has 12 scenarios, not 7.

## Decided for this plan (2026-11-14)

- **`save` keeps bytes once** (the maintainer, bmlib #755 item 7). When the
  digest is already in the library, `Library.save` puts the use into the
  folder that holds those bytes, even if the identifiers find another
  folder. The file is not written again. This amends design §4.3's
  "callers look it up first": `save` does it for every caller.
- **An unreachable or read-only library does not stop a lookup** (the
  maintainer). The service warns, sets `library_failure` on the result, and
  falls back to the cache exactly as without a library. The app reads the
  flag and keeps the source as *waiting for the library* (design §4.6, step
  2). The cache is a staging area here, not a second store: the app copies
  the bytes out, as StudyAnalyzer does today.
- **The library is keyed by the request's identifiers, not `identifier`.**
  `pmid`, `doi` and `pmc_id` are canonicalised by `PaperIds.canonical`
  field by field. A value it refuses is dropped with a WARNING naming the
  field, and the rest are kept. A request with no valid identifier does not
  use the library: it runs as without one, with no flag, because that is
  not a failure.
- **A JATS hit is parsed and rendered on every lookup.** No HTML render is
  stored for a library paper, so no stale-render rule applies to it.
  Python's stamp logic stays for the cache path.
- **The service handles only the kinds it fetches: JATS and PDF.** The
  library's HTML, Markdown and text files are not used by the service. The
  app finds them through the library API itself (StudyAnalyzer's intake
  already converts those kinds). *Amends design §4.5's last sentence* for
  bmlib; step 2 does that part.
- **`save` still rescans per call.** A lookup costs one scan, plus one per
  file saved (at most a JATS and a PDF). The `shortcut:` stays, its
  trigger reworded to "a scan's cost shows in a lookup's time". A scan
  passed in would go stale after the lookup's own first save and create a
  second folder.
- **`file_path` points into the library** for a library PDF, after the
  bytes passed `Library.read`'s digest check. The extractor reads that
  path, as it reads a cache path today.

## Global Constraints

- Work in the bmlib worktree `~/src/bmlib-1b`, branch `feat/library-fulltext`
  from `origin/main`. bmlib's `HANDOVER.md` and `docs/SESSION-RULES.md` apply.
- **Rust and Python on par in the same PR**, held by shared fixtures (Task 1's
  `library_ids` cases, Task 4's scenarios).
- Without a library, behaviour is unchanged. Every existing service and cache
  test passes untouched.
- **Nothing is deleted or overwritten** in the library (design §2). The
  service only calls `save` and `record_use`.
- Every library error is logged once at WARNING, with what was being done and
  the error, and is reported on the result. It is never swallowed and never
  raised out of `fetch_fulltext`.
- Every public item documented, AGPL-3 header on new files, ruff line length
  100, mypy clean, `cargo doc -D warnings`.
- Gates: CLAUDE.md § Gates (bmlib, Rust and Python) plus `uv run mypy`.

## Review Focus

1. **A library file whose bytes no longer match** (edited, half-synced). The
   lookup warns with `library_failure`, ignores the file, and runs the chain.
   It never serves the file. Task 3 tests this.
2. **The JATS in the library belongs to another paper** (a wrong download
   saved by another app). The JATS's own identifiers contradict the request,
   so the file is not served. The lookup warns and runs the chain (design
   §4.4: a record is a claim, not proof). Task 3 tests this.
3. **The library is set but the request has only `identifier`.** The lookup
   runs as without a library, with no `library_failure`. Task 3 tests this.
4. **A split paper** (PMID-only folder and DOI-only folder) **gets its PDF
   downloaded again** by a PMID-only caller. The bytes land once, in the
   folder that already holds them (Task 1's save scenario, and Task 3's
   "split paper" test).
5. **The library becomes read-only mid-lookup** (the JATS saved, the PDF
   refused). The result keeps the library JATS. The PDF goes to the cache
   with `library_failure` set, and `library_files` lists only the JATS.
   Task 4 tests this.

---

### Task 1: The library side, `save` by digest and the request's identifiers

**Files:**
- Modify: `rust/bmlib/src/library/save.rs` (`save`: the `existing` folder), `bmlib/library/save.py` (`save_file`)
- Create: `rust/bmlib/src/fulltext/library_store.rs`, `bmlib/fulltext/library_store.py`
- Modify: `rust/bmlib/src/fulltext/mod.rs`, `bmlib/fulltext/__init__.py` (export)
- Modify: `tests/fixtures/library_cases.json` (a `save_scenarios` case), new `tests/fixtures/fulltext_library_ids.json`
- Test: `rust/bmlib/tests/library_store.rs`, `tests/test_fulltext_library_store.py`, the existing library contract tests

**Interfaces:**
- Produces (Rust):
  - `pub fn library_ids(pmid: &str, doi: Option<&str>, pmc_id: Option<&str>) -> (PaperIds, Vec<&'static str>)`: the canonical ids and the field names dropped.
  - `pub struct LibraryStore { library: Library, who: Installation }` with `pub fn new(library: Library, who: Installation) -> Self`.
- Produces (Python): `library_ids(pmid: str, doi: str | None, pmc_id: str | None) -> tuple[PaperIds, list[str]]`, and `LibraryStore(library: Library, who: Installation)`.

- [ ] **Step 1: Add the save scenario.** It goes in
  `library_cases.json` → `save_scenarios`, named `"a split paper's bytes
  are kept once"`. Install A saves `"pdf bytes split"` with `{"doi":
  "10.1/s"}`. Install B saves the same text with `{"pmid": "222"}`. Expect
  one paper folder, holding one PDF, with both records in it, and B's
  record carrying the PMID. The existing contract runners in both languages
  pick it up.
- [ ] **Step 2: Add the fixture `fulltext_library_ids.json`**, at least
  these cases:
  - all three valid, so the ids come out canonical (`" 0038012345 "` →
    `"38012345"`);
  - an invalid DOI (`"not-a-doi"`), which is dropped with
    `dropped == ["doi"]` while the PMID is kept;
  - an empty PMID with no others, which gives empty ids and nothing
    dropped;
  - `pmc_id` `"pmc123"` → `"PMC123"`;
  - a `PPR` preprint PMID, which is kept.

  Write a test in each language that loads the fixture and asserts each
  case's `ids` and `dropped`.
- [ ] **Step 3: Run both new tests and the contract tests.** Expected: the
  scenario and the ids tests FAIL (the functions are undefined; save
  writes a second folder).
- [ ] **Step 4: Implement.** In `save`, the target folder is found by digest
  first (`find_digest(scan, digest)`), then by identifiers, then claimed.
  `write_once` already accepts the same bytes present. Remove the "only
  with no identifiers" comment and fix the doc comments that state it.
  `library_ids` tries each field alone through `PaperIds::canonical`.
- [ ] **Step 5: Run every library and store test in both languages.** Expected: PASS.
- [ ] **Step 6: Commit** `library: save keeps bytes once across a split paper (#755 item 7); fulltext: library_ids`.

### Task 2: Reading and writing a paper through the store

**Files:**
- Modify: `rust/bmlib/src/fulltext/library_store.rs`, `bmlib/fulltext/library_store.py`
- Test: the same two test files

**Interfaces:**
- Consumes: `library_ids`, `LibraryStore` (Task 1).
- Produces (Rust; Python mirrors each with the same name, returning the
  value and raising `LibraryStoreFailure(message)`, a new `Exception`
  subclass in `library_store.py`, where Rust returns `Err(message)`):
  - `pub struct Held { pub file: FileRef, pub bytes: Vec<u8> }`.
  - `pub fn find_source(&self, ids: &PaperIds, kind: SourceKind) -> Result<Option<Held>, String>`. It looks in the paper's group (`find`) for the files of `kind` that some record lists and that are on disk. It takes the latest by `saved_at` across records, with ties going to the lower digest, and reads it digest-checked. A `LibraryError` from scan or read becomes `Err(message)`, where the message names the paper's ids, the file and the error.
  - `pub fn keep(&self, paper: &PaperInfo, bytes: &[u8], kind: SourceKind, origin: &str) -> Result<FileRef, String>`, which wraps `save`.
  - `pub fn note_use(&self, paper: &PaperInfo, file: &FileRef) -> Result<(), String>`. It wraps `record_use` and is skipped when this installation's record already lists the digest.
  - `pub fn path_of(&self, file: &FileRef) -> PathBuf`, which is `root/papers/<folder>/<name>` through the library's `component` checks.
  - `pub fn paper_info(ids: PaperIds, article: Option<&JATSArticle>) -> PaperInfo`. It takes the title, the year (parsed as `i32`, else `None`) and the first author's `surname`, else their collab name. The JATS's own ids are merged in where the request lacks them; a value the JATS gives that `library_ids` refuses is dropped.

- [ ] **Step 1: Write the failing tests.** Use a temporary library made by
  `open_library` and these cases:
  - `find_source` returns `None` for an unknown paper.
  - It returns the JATS saved under a DOI when asked by that DOI, and when
    asked by the PMID of a linked folder.
  - It returns the later of two PDFs.
  - A file overwritten on disk gives `Err` containing `"digest"`.
  - An unreadable root gives `Err`.
  - `keep`, then `find_source`, round-trips.
  - `note_use` adds this installation to `used_by` once; a second call
    leaves the record's bytes unchanged.
  - `path_of` reads back the bytes.
  - `paper_info` from the `sample_article.xml` fixture gives its title,
    year and first surname.
- [ ] **Step 2: Run the tests.** Expected: FAIL (methods undefined).
- [ ] **Step 3: Implement** the five functions in both languages.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** `fulltext: LibraryStore finds, reads, keeps and notes library sources`.

### Task 3: The lookup reads the library first

**Files:**
- Modify: `rust/bmlib/src/fulltext/service.rs`, `rust/bmlib/src/fulltext/models.rs`; `bmlib/fulltext/service.py`, `bmlib/fulltext/models.py`
- Test: `rust/bmlib/tests/service_library.rs` (new; reuse `tests/service.rs`'s `ScriptedClient`, moved into `tests/common/` if it is not shared yet), `tests/test_fulltext_service_library.py` (new)

**Interfaces:**
- Consumes: Task 2's store.
- Produces:
  - Rust: `FullTextService::with_library(self, library: Library, who: Installation) -> Self` and `fn library(&self) -> Option<&LibraryStore>`.
  - Python: `FullTextService(..., library: Library | None = None, installation: Installation | None = None)`. A `ValueError` is raised when exactly one of the two is given.
  - `FullTextResult` gains `library_files: Vec<FileRef>` (Python `list[FileRef]`, default empty), the library files the result stands on.
  - `FullTextResult` gains `library_failure: Option<String>` (Python `str | None`).
  - The `source` of a library hit is `"library"`.

Flow, at the head of `fetch_fulltext`, before the cache check:
1. Run `library_ids` on the request. If the ids are empty, skip the library.
2. Call `find_source(ids, Jats)`. If it holds a file, parse it as Tier 0
   parses caller XML, and check that its own identifiers agree with the
   request's. Both sides go through `library_ids` and are compared field
   by field; an absent field agrees.
   - If they agree and the JATS has a body, render it and return a
     `fulltext` result: `source "library"`, `jats_xml`, and `library_files`
     holding the JATS. If `find_source(ids, Pdf)` holds a PDF, also set
     `file_path` from `path_of` and add the PDF to `library_files`. Call
     `note_use` for each file used.
   - If the result has no PDF and the service has a cache and an
     `identifier`, it goes through `seek_pdf_again` as a cache hit does,
     since the mark stays in the cache. *(As built: it seeks once when the
     JATS is first noted; see the note at the top.)*
3. A contradiction, a body-less JATS, or an `Err` sets `library_failure` (an
   `Err`) or warns (a contradiction or no body), and the lookup goes on. On
   `Err`, the rest of this call uses no library (the cache path as today).
   *(As built: a contradiction is a `library_failure` and keeps nothing.)*
4. With no usable JATS, the library's PDF (if any) is held for Task 4. The
   cache check and the chain run.

- [ ] **Step 1: Write the failing tests** (both languages, scripted HTTP
  answering nothing unless the test says so):
  - `jats_in_library_ends_the_lookup`: no request is made; `source ==
    "library"`; the HTML holds the fixture's title; `jats_xml` equals the
    bytes; `library_files` lists the JATS; this installation's record now
    lists it.
  - `jats_and_pdf_in_library`: `file_path` is the library path, and
    `library_files` has two entries.
  - `tampered_jats_is_not_served`: the network JATS is used, and
    `library_failure` contains `"digest"`.
  - `jats_of_another_paper_is_not_served`: the request's PMID is `999`, and
    the JATS's own PMID differs. Expect a WARNING naming both, and the chain
    to run. *(As built: `library_failure` is set too.)*
  - `identifier_only_request_skips_the_library`: there is no
    `library_failure`, and the cache is used as today.
  - `unreachable_library_falls_back_to_the_cache`: the root is removed
    after `open`. Expect `library_failure` to be set and the cache hit to
    be served.
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement** the flow above, plus the two fields and the
  builder or constructor argument. Every early return still goes through
  the existing tier-fault bookkeeping.
- [ ] **Step 4: Run the new tests, then the whole service suite in both
  languages.** Expected: PASS, and no existing test changed.
- [ ] **Step 5: Commit** `fulltext: a lookup reads the library first (design §4.5)`.

### Task 4: Downloads go to the library; the library's PDF replaces a download

**Files:**
- Modify: the two service files (`cache_html`, `download_and_cache_pdf` / `save_pdf_to_cache`, `pdf_for_jats`, the no-JATS PDF tiers in `retrieve`)
- Create: `tests/fixtures/fulltext_library_scenarios.json`
- Test: `rust/bmlib/tests/service_library.rs`, `tests/test_fulltext_service_library.py`

**Interfaces:**
- Consumes: Tasks 2–3.

Rules:
- **JATS with a body, from any tier, goes to the library.** It is saved
  with `keep(paper_info(ids, Some(&article)), xml, Jats, origin)` and the
  file is added to `library_files`. The origin is the tier's `source` name,
  such as `"europepmc"` or `"pmc_open_data"`. The parsed article is passed
  out of `fetch_jats_xml` and its siblings instead of being dropped.
  - With a library, the cache gets no `jats/` or `html/` entry.
  - On `Err`, the result gets `library_failure` and `cache_html` runs as
    today.
- **A PDF the chain downloads goes to the library.** The bytes go through
  `keep(paper_info(ids, None), bytes, Pdf, url)`, after the `%PDF` and md5
  checks. `file_path` is set to `path_of`, and the extractor reads it.
  - On `Err`, the PDF goes through `save_pdf_to_cache` as today, with
    `library_failure` set.
  - With a library set, a PDF is downloaded even without an `identifier`.
    Today a PDF without a cache key is left as a link.
  - A JATS saved in the same call gives its `paper_info` to the PDF save.
- **The library's PDF, held from Task 3, replaces a download.** In
  `pdf_for_jats` it is the PDF beside JATS, with no candidate fetched; the
  mark is cleared as on a successful download. When no JATS tier succeeds,
  it is returned before Tier 1d, so Europe PMC's render and Unpaywall are
  not fetched. Either way it is added to `library_files` with `note_use`.
- The abstract sidecar and the *PDF still owed* mark stay in the cache,
  keyed by `identifier` as today.

- [ ] **Step 1: Write the fixture.** Each scenario gives:
  - `library`: the files to save first, each with kind, fixture file,
    paper ids and app;
  - `request`: `pmid`, `doi`, `pmc_id`, `identifier`;
  - `network`: the URL patterns the scripted client answers, each with a
    fixture file or a status;
  - `read_only_after`: an optional number of saves after which the library
    turns read-only (permissions);
  - `expect`: `source`, `content_kind`, the `library_files` kinds in order,
    `library_failure` (`null` or a substring), the URLs requested (by
    pattern, in order), and the library's folder count and per-folder file
    kinds afterwards.

  Scenarios:
  1. Empty library, Europe PMC JATS. The JATS is saved into one new folder
     named from the article.
  2. Empty library, `fetch_pdf_with_jats`, JATS plus Open Data PDF. Both
     land in one folder.
  3. The library holds only a PDF; the network has JATS. The JATS is saved
     beside it, the PDF is not fetched, and `library_files` is `[jats,
     pdf]`.
  4. The library holds only a PDF; the network has no JATS. The library's
     PDF is returned, and Unpaywall and the Europe PMC render are not
     requested.
  5. A split paper: the PDF is in a DOI-only folder, the request is
     PMID-only, and Unpaywall serves the same bytes (the library has no
     link from PMID to DOI). Afterwards there is one folder for those
     bytes. This is Review Focus 4.
  6. The library goes read-only after the JATS save. The PDF lands in the
     cache with `library_failure` set, and `library_files` is `[jats]`.
     This is Review Focus 5.
  7. A request with `identifier` and no PMID, DOI or PMC ID: the library is
     untouched.
- [ ] **Step 2: Write a runner in each language** that builds each
  scenario's library in a temporary directory, runs `fetch_fulltext`, and
  asserts `expect`. Run it. Expected: FAIL on 1–6.
- [ ] **Step 3: Implement** the rules above.
- [ ] **Step 4: Run the runner and the full suites in both languages.** Expected: PASS.
- [ ] **Step 5: Commit** `fulltext: downloads are kept in the library; its PDF replaces a download`.

### Task 5: Documentation, gates, PR

**Files:**
- Modify: `docs/manual/library.md`, `rust/bmlib/docs/manual/library.md` (a "FullTextService on a library" section, doc-tested in Rust), the fulltext manuals' cache sections, `CHANGELOG.md` (Unreleased), `HANDOVER.md`, `docs/DECISIONS.md` ("library": the bullets of *Decided for this plan*), `save.rs`/`save.py` (the reworded `shortcut:`)

- [ ] **Step 1: Write the manuals' section.** Cover building a service with
  a library, the lookup order, what `library_files` and `library_failure`
  mean, and what stays in the cache.
- [ ] **Step 2: Run every gate** in CLAUDE.md § Gates for bmlib (Rust and
  Python), plus `uv run mypy` and `uvx ruff@0.15.20`, with
  `set -o pipefail`. Expected: all green, with the test counts higher than
  on `origin/main`.
- [ ] **Step 3: Update bmlib's HANDOVER.** 1b is done; 1c (FFI) is next.
  The FFI needs `with_library`, `library_files` and `library_failure`
  converted.
- [ ] **Step 4: Commit, push, and open the PR** to bmlib `main`. It links
  #755 (item 7 closed by Task 1) and names this plan.
