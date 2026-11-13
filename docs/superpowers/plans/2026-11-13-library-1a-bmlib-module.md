# Shared paper library, step 1a: bmlib's `library` module — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `library` module in bmlib, Rust and Python on par, that opens a
user-chosen library folder, saves source files into it write-once, reads them
back digest-checked, and lists what each installation uses, held together by
shared contract fixtures.

**Architecture:** Plain files only (design §4.2–§4.3): a marker file, one
folder per paper, write-once source files named by kind and digest, one JSON
record per installation. Pure functions for names, identifiers, finding and
grouping; one small file-operations module that everything else goes
through (design §5.1: phase 4's Android swaps it). No index, no database:
`scan()` reads the folder, and apps cache it.

**Tech Stack:** Rust (std, `sha2`, `serde_json`, `chrono`,
`unicode-normalization`, bmlib's `atomic`; **no new dependency**), Python 3.11
stdlib (`hashlib`, `unicodedata`, `pathlib`, `json` through
`bmlib._strict_json`, `bmlib._atomic`).

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(in StudyAnalyzer). This plan is the first third of the design's §8 step 1;
**1b** (`FullTextService` uses the library, design §4.5) and **1c** (the
bmlib-ffi surface) get their own plans once this lands.

## Global Constraints

- Work in a **bmlib worktree** on branch `feat/shared-library` from
  `origin/main`, never the shared `~/src/bmlib` checkout; check
  `git worktree list` for a live session first. bmlib's `docs/SESSION-RULES.md`
  and `HANDOVER.md` apply.
- **Rust and Python on par, in the same PR**, held by
  `tests/fixtures/library_cases.json` and the tree `tests/fixtures/library/`.
- Every public item documented (Rust `cargo doc -D warnings`; Python
  docstrings, type hints), AGPL-3 header on every new file, ruff line length
  100, mypy clean.
- **Nothing deleted or overwritten without the caller asking** (design §2):
  no eviction; a source file is never rewritten; a name clash is an error.
- No feature gate: the module builds with `default-features = false`
  (bmlib-ffi's configuration).
- Constants (Rust `library::` / Python `bmlib.library.`), exact values:
  `LIBRARY_FORMAT = "bmlib-library"`, `LIBRARY_VERSION = 1`,
  `RECORD_FORMAT = "bmlib-library-record"`, `RECORD_VERSION = 1`,
  `MARKER_FILE = "bmlib-library.json"`, `PAPERS_DIR = "papers"`,
  `RECORDS_DIR = "records"`, `DIGEST_PREFIX_HEX = 12`,
  `MAX_FOLDER_NAME_BYTES = 150`, `MAX_TITLE_CHARS_IN_NAME = 60`,
  `MAX_AUTHOR_CHARS_IN_NAME = 40`, `MAX_NAME_SUFFIX = 1000`,
  `MAX_LABEL_CHARS = 200`, `IGNORED_NAMES = [".DS_Store", ".localized",
  "desktop.ini", "Thumbs.db"]` (and any name starting with `.`),
  `ICLOUD_PLACEHOLDER = ".{name}.icloud"`, `CORRUPT_SUFFIX` reused from
  `fulltext::cache` (`.corrupt`).
- Gates (bmlib CLAUDE.md, plus mypy): from the worktree root
  `uv run pytest tests/ -v`, `uvx ruff@0.15.20 check .`,
  `uvx ruff@0.15.20 format --check .`, `uv run mypy`; from `rust/`
  `cargo test`, `cargo test -p bmlib --no-default-features --test library_contract`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt --check`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
  Run chains with `set -o pipefail`.

## Review Focus

1. **Two writers at once** (two apps, or one app on two synced Macs): saving
   the same bytes for the same paper concurrently must end with one file, two
   records, no error. Task 5's `save_same_bytes_twice_is_one_file` and the
   folder race test pin it.
2. **A file name that already exists with other bytes**: never overwritten,
   `FileClash`. Task 5's `save_never_overwrites_other_bytes`.
3. **Case-insensitive file systems (APFS, NTFS)**: two papers whose names
   differ only by case collide on `create_dir`; the second takes ` (2)` unless
   their identifiers match. Task 5's `folder_clash_by_case_takes_suffix`.
4. **Names from records or callers used as paths** (`../x`, `a/b`, `.`):
   refused before any file operation. Task 3's `component_refuses_paths`.
5. **An online-only placeholder** (iCloud's `.<name>.icloud`): `NotDownloaded`,
   not `Missing`. Task 4's `read` cases.

---

### Task 1: Identifiers, kinds, installations and folder names (pure)

**Files:**
- Create: `rust/bmlib/src/library/mod.rs` (constants, re-exports, module docs),
  `rust/bmlib/src/library/ids.rs`, `rust/bmlib/src/library/names.rs`
- Create: `bmlib/library/__init__.py`, `bmlib/library/ids.py`, `bmlib/library/names.py`
- Modify: `rust/bmlib/src/lib.rs` (`pub mod library;`)
- Create: `tests/fixtures/library_cases.json` (sections `canonical_ids`,
  `installations`, `folder_names`)
- Test: `rust/bmlib/tests/library_contract.rs`, `tests/test_library_contract.py`

**Interfaces — Produces:**
- `SourceKind { Pdf, Jats, Html, Markdown, Text }`; `as_str()` →
  `"pdf" | "jats" | "html" | "markdown" | "text"` (the hand-off format's
  names); `extension()` → `"pdf" | "xml" | "html" | "md" | "txt"`;
  `SourceKind::parse(&str) -> Option<SourceKind>`. Python: `SourceKind(str, Enum)`.
- `PaperIds { pmid: Option<String>, doi: Option<String>, pmcid: Option<String> }`
  with `PaperIds::canonical(pmid: Option<&str>, doi: Option<&str>, pmcid:
  Option<&str>) -> Result<PaperIds, LibraryError>`, `is_empty()`,
  `shares_any(&PaperIds) -> bool`, `merged(&PaperIds) -> (PaperIds, Vec<&'static str>)`
  (gaps filled from the other; a conflicting value keeps `self`'s and names
  the field in the second item). Python: `canonical_ids(pmid, doi, pmcid) -> PaperIds`
  (frozen dataclass, same methods).
- `PaperInfo { ids: PaperIds, title: Option<String>, year: Option<i32>, first_author: Option<String> }`.
- `Installation { app: String, id: String, label: String }` with
  `Installation::new(app, id, label) -> Result<Installation, LibraryError>` and
  `record_file_name() -> String` = `"{app}-{id}.json"`.
- `folder_name(info: &PaperInfo) -> String`.
- `LibraryError` (Rust enum with `Display` and `std::error::Error` as bmlib's
  other error types implement them; Python: `LibraryError(Exception)` with
  one subclass per variant, same names):
  `Unreachable { path, reason }`, `NotALibrary { path, reason }`,
  `NewerLibrary { version }`, `ReadOnly { path }`, `BadIdentifier { field, value }`,
  `BadInstallation { field, reason }`, `BadName { name }`, `Missing { folder, name }`,
  `NotDownloaded { folder, name }`, `DigestMismatch { folder, name, expected, found }`,
  `FileClash { folder, name }`, `NewerRecord { folder, file, version }`,
  `Io { path, reason }`. Every variant names what the user must be told.

**Rules the fixture pins:**
- `pmid`: trimmed; ASCII digits → leading zeros stripped (all zeros refused);
  `PPR` + digits, any case → `PPR` + digits; else `BadIdentifier { field: "pmid" }`.
- `doi`: `publications::storage::normalize_doi` (Python `_normalize_doi`),
  then it must start with `10.` and contain `/`, else `BadIdentifier`.
- `pmcid`: trimmed, an optional `PMC` prefix in any case, ASCII digits,
  leading zeros stripped → `PMC` + digits; else `BadIdentifier`.
- Empty or whitespace-only → `None` for every field.
- `Installation`: `app` matches `^[a-z0-9]{1,40}$`, `id` `^[a-z0-9-]{1,64}$`,
  `label` 1–`MAX_LABEL_CHARS` characters with no control character and no
  bidi control (U+200E, U+200F, U+202A–U+202E, U+2066–U+2069).
- `folder_name`, in order:
  1. `id` = `PMID <pmid>` else `DOI <doi>` else `PMCID <pmcid>` else `no identifier`.
  2. `head` = year (only when 1000–9999) and `first_author` (cut to
     `MAX_AUTHOR_CHARS_IN_NAME` characters), joined by one space.
  3. `title` cut to `MAX_TITLE_CHARS_IN_NAME` characters at the last space at
     or before the limit (a hard cut when there is none), no ellipsis.
  4. `body` = `head – title` (U+2013 with a space each side); `head` or
     `title` alone when the other is empty. `name` = `body (id)`, or `(id)`
     when `body` is empty.
  5. Sanitise the whole name: NFC; each of `<>:"/\|?*`, every control
     character (Unicode Cc) and every bidi control above → one space; runs
     of whitespace → one space; trim; strip leading `.`; strip trailing `.`
     and spaces.
  6. If the part before the first `.`, upper-cased, is `CON`, `PRN`, `AUX`,
     `NUL`, `COM1`–`COM9` or `LPT1`–`LPT9`, prefix `_`.
  7. If the UTF-8 length exceeds `MAX_FOLDER_NAME_BYTES`, cut `body` at a
     character boundary so that `body (id)` fits, re-trim it; if ` (id)`
     alone does not fit, cut the whole name at a character boundary.

- [ ] **Step 1: Set up the worktree.** `git -C ~/src/bmlib worktree list`
  (stop and ask if a live session holds `feat/shared-library`), then
  `git -C ~/src/bmlib fetch origin && git -C ~/src/bmlib worktree add
  ~/src/bmlib-library -b feat/shared-library origin/main`. Read
  `docs/SESSION-RULES.md` there. Run the Rust and Python suites once to get
  the baseline test counts; note them.

- [ ] **Step 2: Write the fixture sections.** `canonical_ids`: at least
  `{"pmid":" 0038012345 "}` → `"38012345"`, `"ppr123"` → `"PPR123"`,
  `"000"` → error `pmid`, `"12a"` → error, `{"doi":"https://doi.org/10.1/AbC"}`
  → `"10.1/abc"`, `"doi:10.1/x"` → `"10.1/x"`, `"11.1/x"` → error `doi`,
  `{"pmcid":"pmc0012"}` → `"PMC12"`, `"12"` → `"PMC12"`, `"PMC"` → error,
  `"１２"` (full-width digits) → error, `"  "` → null. `installations`: a valid
  one; `app` `"Study"` → error `app`; `id` `"a/b"` → error `id`; a label with
  U+202E → error `label`; a 201-character label → error `label`.
  `folder_names`: the design's example
  (`2024`, `Smith`, `Metformin and cognition`, PMID `38012345` →
  `2024 Smith – Metformin and cognition (PMID 38012345)`); no title; no year
  and no author; nothing but a DOI (`DOI 10.1 x` after sanitising); no
  identifier; a 61-character title with a space at 55; one with no space; a
  title holding `a/b: c?`; `CON` as the title and nothing else; a name ending
  in `...`; a leading `.`; NFD input (`e` + U+0301) coming out NFC; a
  300-character DOI (whole-name cut, ≤150 bytes, valid UTF-8); a CJK title
  long enough to hit the byte cap with the id kept.

- [ ] **Step 3: Write the failing contract tests.**
  `canonical_ids_match_the_fixture`, `installations_match_the_fixture`,
  `folder_names_match_the_fixture` in Rust (`include_str!` of the fixture, as
  `tests/fulltext_source_contract.rs` does) and `test_canonical_ids`,
  `test_installations`, `test_folder_names` in Python (parametrised over the
  cases). Each asserts the exact value or the exact error `field`. Plus
  `folder_name_is_at_most_150_bytes` over every case.

- [ ] **Step 4: Run them; expect failure** (`cargo test --test
  library_contract` fails to compile; `uv run pytest
  tests/test_library_contract.py` fails on the import).

- [ ] **Step 5: Implement** the types, `PaperIds::canonical`, `Installation::new`,
  `folder_name` and `LibraryError` in the files above, both languages.
  Reuse `normalize_doi`; `unicode-normalization`'s `nfc()` / `unicodedata.normalize("NFC")`.

- [ ] **Step 6: Run both suites; expect PASS.** Then `cargo clippy` and
  `uvx ruff@0.15.20 check .` on the new files.

- [ ] **Step 7: Commit** `feat(library): identifiers, kinds, installations and folder names (Rust + Python)`.

### Task 2: Records

**Files:**
- Create: `rust/bmlib/src/library/record.rs`, `bmlib/library/records.py`
- Modify: `tests/fixtures/library_cases.json` (section `records`)
- Test: the two contract test files

**Interfaces:**
- Consumes: Task 1's `Installation`, `PaperInfo`, `PaperIds`, `SourceKind`.
- Produces: `FileEntry { sha256: String, kind: SourceKind, name: String,
  origin: String, size: u64, saved_at: String }`; `Record { installation:
  Installation, paper: PaperInfo, files: Vec<FileEntry> }`;
  `parse_record(text: &str) -> Result<Record, RecordProblem>` where
  `RecordProblem = Unreadable(String) | Newer(u32)`;
  `render_record(&Record) -> String`. Python `parse_record`, `render_record`,
  `RecordProblem` the same.

**The JSON, exactly** (two-space indent, keys in this order, a trailing
newline; absent optionals written as `null`; unknown fields ignored on read;
Rust `serde_json::to_string_pretty`, Python `json.dumps(indent=2,
ensure_ascii=False)`):

```json
{
  "format": "bmlib-library-record",
  "version": 1,
  "installation": {"app": "studyanalyzer", "id": "7c1e0b2a", "label": "StudyAnalyzer on Horst's MacBook"},
  "paper": {"pmid": "38012345", "doi": "10.1/x", "pmcid": null, "title": "…", "year": 2024, "first_author": "Smith"},
  "files": [{"sha256": "<64 hex>", "kind": "pdf", "name": "pdf-<12 hex>.pdf", "origin": "Europe PMC", "size": 1234, "saved_at": "2026-11-13T10:00:00Z"}]
}
```

`parse_record` refuses (as `Unreadable` with the reason) a wrong `format`,
a non-integer or non-positive `version`, an invalid installation (Task 1's
rules), identifiers that are not canonical, an unknown `kind`, a `sha256`
not 64 lowercase hex, a `name` that is not `{kind}-{sha256[..12]}.{ext}`, a
duplicate `sha256` in `files`; `version > RECORD_VERSION` is `Newer`.
Python reads through `bmlib._strict_json.strict_json_loads`.

- [ ] **Step 1: Write the `records` fixture section**: the example above
  (parses); the same with an extra field (parses, ignored); version 2
  (`Newer(2)`); `"format": "x"`; a bad installation; `"pmid": "007"` (not
  canonical); `"kind": "docx"`; a name whose hex differs from the digest; a
  duplicated file; malformed JSON. Each with the expected outcome.
- [ ] **Step 2: Write the failing tests** `records_match_the_fixture` (Rust)
  and `test_records` (Python), plus `render_then_parse_round_trips` and
  `rendering_is_byte_identical_across_languages`: the fixture holds the
  expected rendered text of the example, and both languages' `render_record`
  must produce it exactly.
- [ ] **Step 3: Run; expect FAIL.**
- [ ] **Step 4: Implement** `parse_record` / `render_record` in both languages.
- [ ] **Step 5: Run; expect PASS.**
- [ ] **Step 6: Commit** `feat(library): installation records, versioned (Rust + Python)`.

### Task 3: File operations and opening a library

**Files:**
- Create: `rust/bmlib/src/library/files.rs`, `bmlib/library/files.py` (the
  only code in the module that touches the file system)
- Create: `rust/bmlib/src/library/open.rs`, `bmlib/library/library.py`
  (`Library` and `open_library`)
- Modify: `tests/fixtures/library_cases.json` (section `open`)

**Interfaces:**
- Produces (files): `component(name: &str) -> Result<&str, LibraryError>`
  (one path component: not empty, not `.`/`..`, no `/` or `\`, no NUL;
  else `BadName`); `list_names(dir) -> io::Result<Vec<String>>` (sorted by
  bytes, `IGNORED_NAMES` and names starting with `.` left out, except that
  `has_placeholder(dir, name)` checks `ICLOUD_PLACEHOLDER` directly);
  `read_bytes(path)`, `write_once(path, bytes, sha256) -> Result<Written,
  LibraryError>` where `Written = New | AlreadyThere` (an existing file is
  read and its digest compared: equal → `AlreadyThere`, different →
  `FileClash`; absent → `atomic::atomic_write`); `create_dir_new(path) ->
  io::Result<bool>` (`false` when it already exists); `remove_file`,
  `remove_dir_all`. An `io::Error` maps to `ReadOnly` for `PermissionDenied`
  and `ReadOnlyFilesystem` on a write, `Io { path, reason }` otherwise.
- Produces (open): `Library::open(root: &Path) -> Result<Library, LibraryError>`,
  `Library::root(&self) -> &Path`. Python `open_library(root: Path) -> Library`.

`open`: root missing or not a directory → `Unreachable`. A marker present:
not JSON or `format` not `bmlib-library` → `NotALibrary`; `version >
LIBRARY_VERSION` → `NewerLibrary`; `papers/` created if missing. No marker:
the folder empty but for ignored names → write the marker
(`{"format": "bmlib-library", "version": 1}`, rendered like a record) and
create `papers/`; anything else → `NotALibrary` naming the first entry seen.

- [ ] **Step 1: Write the `open` fixture section**: each case lists entries
  to create in a temporary directory (`{"dir": "x"}`, `{"file": "y", "text": "…"}`)
  and the outcome: empty → created; only `.DS_Store` → created; a marker →
  opened; marker version 2 → `NewerLibrary`; marker with `"format": "x"` →
  `NotALibrary`; a user's `notes.txt` → `NotALibrary`; a marker without
  `papers/` → opened and `papers/` exists; root missing → `Unreachable`.
- [ ] **Step 2: Write the failing tests** `open_matches_the_fixture`,
  `component_refuses_paths` (`""`, `"."`, `".."`, `"a/b"`, `"a\\b"`,
  `"../x"`, `"x\0"` → `BadName`; `"2024 Smith"` passes), and
  `write_once_never_overwrites` (different bytes under an existing name →
  `FileClash`, the file's bytes unchanged), and in Python
  `test_write_once_publishes_atomically` (per bmlib's "a file is published,
  never written in place": patch `os.replace` and assert the target is still
  absent at that moment). Rust's `atomic_write` has its own tests; `write_once`
  only calls it.
- [ ] **Step 3: Run; expect FAIL.**
- [ ] **Step 4: Implement** `files` and `open` in both languages.
- [ ] **Step 5: Run; expect PASS.**
- [ ] **Step 6: Commit** `feat(library): open a library folder; one module for file operations`.

### Task 4: Scanning, finding, grouping, reading

**Files:**
- Create: `rust/bmlib/src/library/scan.rs`, `bmlib/library/scan.py`
- Create: the tree `tests/fixtures/library/sample/` (below)
- Modify: `tests/fixtures/library_cases.json` (sections `scan`, `find`,
  `find_digest`, `group`, `read`)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: `PaperFolder { folder: String, records: Vec<Record>, files_on_disk: Vec<String> }`
  with `ids() -> PaperIds` (the union of its records', first record wins a
  conflict) and `used_by(sha256: &str) -> Vec<&Installation>`;
  `Problem { UnreadableRecord { folder, file, reason }, NewerRecord { folder,
  file, version }, MisnamedRecord { folder, file }, UnrecordedFile { folder,
  name }, StrayEntry { name } }`; `Scan { folders: Vec<PaperFolder>,
  problems: Vec<Problem> }`; `Library::scan(&self) -> Result<Scan,
  LibraryError>`; free functions `find<'a>(scan: &'a Scan, ids: &PaperIds) ->
  Vec<&'a PaperFolder>` (folders whose ids share any with `ids`, in scan
  order), `find_digest(scan: &Scan, sha256: &str) -> Option<FileRef>` (a file
  on disk named `{kind}-{sha256[..12]}.{ext}`; the full digest is checked by
  `read`), `group(scan: &Scan) -> Vec<Vec<usize>>` (indices of folders
  connected by shared identifiers, each group and the list sorted), and
  `Library::read(&self, file: &FileRef) -> Result<Vec<u8>, LibraryError>`.
  `FileRef { folder: String, name: String, sha256: String, kind: SourceKind }`.
  `sha256_hex(bytes: &[u8]) -> String` (lowercase).

`scan` is ordered by folder name (bytes), never fails on one bad folder:
every oddity is a `Problem` (design: surface diagnostics). A record whose
file name is not its installation's `record_file_name()` is `MisnamedRecord`
and not used. `read`: `component` on both names; not found → `NotDownloaded`
when `has_placeholder`, else `Missing`; the digest differs →
`DigestMismatch` with both digests.

**The sample tree** (contents small text such as `%PDF-1.4 sample A\n`; the
fixture records each file's digest):
- `2024 Smith – Metformin and cognition (PMID 38012345)/`: `pdf-…` used by
  two installations (`studyanalyzer-aaaa`, `medicalfactchecker-bbbb`), a
  `jats-…` used by one; records with PMID and DOI.
- `Smith – Metformin and cognition (DOI 10.1 x)/`: a split of the same paper,
  one record with only the DOI, an `html-…` file.
- `2023 Lee – Statins (PMID 111)/`: a record of version 2 (problem), a
  malformed record (problem), a misnamed record (problem), a file on disk no
  record lists (problem), a `pdf-…` whose bytes do not match its name
  (`read` → `DigestMismatch`), and `.pdf-<hex>.pdf.icloud` with no file
  (`read` → `NotDownloaded`).
- `papers/stray.txt` (problem `StrayEntry`).

- [ ] **Step 1: Build the sample tree** and write its sections: `scan`
  (the folders, each record's installation, each file's `used_by` labels, the
  problems in order), `find` (PMID `38012345` → both Smith folders; DOI
  `10.1/x` → both; PMID `111` → Lee; PMCID `PMC9` → none), `find_digest`
  (the Smith PDF's digest → its `FileRef`; an unknown digest → null), `group`
  (`[[0], [1, 2]]`: by bytes `2023 Lee…` < `2024 Smith…` < `Smith –…`),
  `read` (good file → its bytes' digest; mismatch; placeholder; missing).
- [ ] **Step 2: Write the failing tests** `scan_matches_the_fixture`,
  `find_matches_the_fixture`, `find_digest_matches_the_fixture`,
  `group_matches_the_fixture`, `read_matches_the_fixture` (Rust and
  Python), the tree copied to a temporary directory first so nothing writes
  into the fixture.
- [ ] **Step 3: Run; expect FAIL.**
- [ ] **Step 4: Implement** `scan`, `find`, `find_digest`, `group`, `read`,
  `sha256_hex` in both languages.
- [ ] **Step 5: Run; expect PASS.**
- [ ] **Step 6: Commit** `feat(library): scan, find, group and read, digest-checked`.

### Task 5: Saving, releasing and removing

**Files:**
- Modify: `rust/bmlib/src/library/open.rs` (the `Library` methods),
  `bmlib/library/library.py`
- Modify: `tests/fixtures/library_cases.json` (section `save_scenarios`)

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: `Library::save(&self, who: &Installation, paper: &PaperInfo,
  bytes: &[u8], kind: SourceKind, origin: &str) -> Result<FileRef,
  LibraryError>`; `Library::release(&self, who: &Installation, file:
  &FileRef) -> Result<(), LibraryError>` (drops the file from this
  installation's record only; the file stays); `Library::remove_file(&self,
  file: &FileRef) -> Result<(), LibraryError>` and
  `Library::remove_folder(&self, folder: &str) -> Result<(), LibraryError>`
  (the manual clear; other installations' records are left as they are and
  will read `Missing`). Python methods of the same names.

`save`, in order:
1. `digest = sha256_hex(bytes)`; `name = "{kind}-{digest[..12]}.{ext}"`.
2. `scan`; the folder is the first of `find(scan, &paper.ids)` (scan order);
   with no identifiers, the folder of `find_digest(scan, digest)`; else a new
   folder: `folder_name(paper)`, claimed with `create_dir_new`. When that
   name exists, it is used if its records' ids share any with
   `paper.ids`, else ` (2)`, ` (3)` … up to `MAX_NAME_SUFFIX` (then `Io`).
3. `write_once(folder/name, bytes, digest)`.
4. This installation's record (`records/{app}-{id}.json`): absent → new;
   `Newer` → `NewerRecord` (never downgraded); unreadable → renamed to
   `+ CORRUPT_SUFFIX` with a `log::warn!` / `logger.warning` and replaced;
   otherwise merged: ids by `PaperIds::merged` (a conflict logged with the
   field), title/year/first_author fill gaps, the file entry appended unless
   its `sha256` is listed. Written through `atomic_write`.
5. Return the `FileRef`.

The file is written before the record, so a crash between leaves only an
unrecorded file (a `Problem`), which the next save of the same bytes records.

- [ ] **Step 1: Write `save_scenarios`**: each a list of steps (`save` with
  installation, paper, bytes as text, kind, origin; `release`;
  `remove_file`; `remove_folder`) and the expected tree afterwards (paths,
  and each record's content without `saved_at`, which is checked only for
  the `YYYY-MM-DDTHH:MM:SSZ` shape). Scenarios: first save into an empty
  library; the same bytes by a second installation (one file, two records);
  a second file for the same paper found by DOI only (lands in the existing
  folder, ids merged); a different paper whose name differs only in case
  (`folder_clash_by_case_takes_suffix`: ` (2)`); a paper with no identifiers
  saved twice (one folder, found by digest); `release` then `scan` (the file
  is used by one installation fewer and still on disk); `remove_file` then
  `read` (`Missing`); `remove_folder("../x")` (`BadName`, nothing removed).
- [ ] **Step 2: Write the failing tests** `save_scenarios_match_the_fixture`
  (Rust, Python), and: `save_same_bytes_twice_is_one_file` (two threads in
  Rust, two processes in Python via `multiprocessing`, saving the same bytes
  for the same paper 20 times each: one file, both records, no error);
  `save_never_overwrites_other_bytes` (a planted file with the target name
  and other bytes → `FileClash`, planted bytes unchanged);
  `newer_own_record_is_not_downgraded`; `corrupt_own_record_is_kept_aside`
  (the `.corrupt` file exists afterwards, a warning was logged);
  `read_only_library_says_so` (a directory made read-only → `ReadOnly`;
  skipped when running as root).
- [ ] **Step 3: Run; expect FAIL.**
- [ ] **Step 4: Implement** `save`, `release`, `remove_file`,
  `remove_folder` in both languages.
- [ ] **Step 5: Run; expect PASS.**
- [ ] **Step 6: Commit** `feat(library): save write-once, release, remove (the manual clear)`.

### Task 6: Documentation, the full gates, the PR

**Files:**
- Create: `rust/bmlib/docs/manual/library.md` (and its `struct Library;`
  entry in `lib.rs`'s `mod manual`), `docs/manual/library.md`
- Modify: `docs/manual/index.md`, `CHANGELOG.md` (Unreleased),
  `CLAUDE.md` (directory structure and a *Module notes* line for `library/`),
  `docs/DECISIONS.md` (write-once files; one record per installation; no
  automatic deletion; plain files, no database), `HANDOVER.md`, `ROADMAP.md`
  (bmlib's), and in StudyAnalyzer's `HANDOVER.md` / `ROADMAP.md` the PR number.

- [ ] **Step 1: Write both manuals**: what a library is (the layout with the
  example tree), the API, every error and what to tell the user, the
  never-delete rule, a crash between file and record, the Python ↔ Rust name
  map (`open_library` ↔ `Library::open`, exception classes ↔ variants).
- [ ] **Step 2: Update the other files above**; reflow edited paragraphs.
- [ ] **Step 3: Run every gate** in Global Constraints with `set -o
  pipefail`; all pass. Confirm the test counts rose by the new tests only
  (baseline from Task 1 Step 1).
- [ ] **Step 4: Commit**, push `feat/shared-library`, open the PR against
  bmlib `main`: what the module is, the link to the StudyAnalyzer design, the
  fixtures, that 1b and 1c follow. Then StudyAnalyzer's HANDOVER and ROADMAP
  name the PR (a docs commit on StudyAnalyzer's branch).
