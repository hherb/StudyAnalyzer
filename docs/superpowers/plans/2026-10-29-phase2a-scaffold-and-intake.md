# Phase 2a: scaffold, storage and getting a paper in — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A macOS app that takes a paper in by file, paste or identifier, finds
its full text, turns it into bmlib's canonical text, stores everything in
SQLite, and shows the paper in style A, ready for phase 2b's Interrogator.

**Architecture:** A Cargo workspace at the repository root. All logic lives in
`crates/studyanalyzer-core` (pure functions over bmlib, testable without
Tauri, reusable by phase 4's mobile builds); `src-tauri` is a thin command
layer; the Svelte 5 frontend in `src/` renders what the commands return. Code
points are the one offset unit in Rust; `view.rs` converts to UTF-16 at the
boundary and nowhere else.

**Tech Stack:** Rust 2021, Tauri 2, bmlib (Rust, git dependency), Svelte 5 +
TypeScript + Vite, vitest, pnpm. The same toolchain as `~/src/hanzitutor`
(copy its `vite.config.ts`, `vitest.config.ts`, `svelte.config.js`,
`tsconfig.json` and script layout; drop everything speech- or kana-specific).

**Spec:** `docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`
(§4, §5, §6.5, §6.6, §8, §11). House rules: `CLAUDE.md` § Coding rules.

### Phase 2 is four plans

| Plan | Scope | Waits on |
|---|---|---|
| **2a (this)** | Scaffold, storage, start screen, file/paste/identifier intake, retrieval, canonical text, full-text check, paper pane, contact-email setting | nothing |
| 2b | Interrogator: chunks + embeddings, whole-paper vs retrieval mode, streamed answers, quote marking, `find_passage` threshold by measurement, citations and highlights, provider/model settings, keychain | bmlib item 11 (context-window table) |
| 2c | PICO tab | bmlib item 7 |
| 2d | `.studyanalyzer` hand-off file and `studyanalyzer://` link, single instance | 2a |

### Builder's calls this plan settles (spec §11)

- **Layout:** root Cargo workspace with members `crates/studyanalyzer-core`
  and `src-tauri`; frontend at the root, as in hanzitutor.
- **bmlib dependency:** git, pinned by commit, so CI (which has no
  `~/src/bmlib`) builds it:
  `bmlib = { git = "https://github.com/hherb/bmlib", rev = "140db3f8931949534b8a451b6ebf8a4cf1afe5a4", features = ["pdf", "html"] }`
  (default features `sqlite`, `http-ureq` stay on). To try an unmerged bmlib
  branch locally, use an uncommitted `[patch."https://github.com/hherb/bmlib"]`
  pointing at `~/src/bmlib/rust/bmlib`. Bump `rev` deliberately.
- **Original files** are stored as BLOBs in SQLite (no paths to keep valid).
- **Converter versions:** when bmlib's converter for a paper's format has a
  newer version than the stored canonical text's, and nothing points into the
  stored text yet, re-derive from the stored original on open. Once 2b stores
  chunks or citations, the old text is kept (2b extends the "points into"
  check).
- **Failed full-text check:** the paper opens with a banner naming what is
  missing; the user may confirm "this is the whole paper", stored per
  canonical text. 2b's Interrogator opens only for a passing or confirmed
  text. Never silently.
- **Metadata** (authors, journal, year) waits for bmlib item 5; 2a's title is
  the canonical text's title, else the sender's/user's, else `Untitled paper`.

## Global Constraints

- Bundle identifier `com.bmlibrarian.studyanalyzer`; product name `StudyAnalyzer`; licence `AGPL-3.0-or-later` (bmlib's); no dependency with an incompatible licence.
- Gates before every commit (CLAUDE.md): `pnpm run check:web`, `pnpm run test:web`, `pnpm exec vite build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- No magic numbers: every limit and default lives in `crates/studyanalyzer-core/src/config.rs`.
- Every function, type and method has a doc comment; every outward-facing one has a unit test.
- Never truncate the paper's text. Never fall back to the abstract.
- Every error is logged (`log` crate, `tauri-plugin-log`) and shown to the user.
- No path or URL received from outside the app is dereferenced (spec §4.1); a file path reaches Rust only from the app's own file dialog or drop event.
- Sandbox-ready: App Sandbox entitlements on, data only under Tauri's `app_data_dir()` / `app_cache_dir()`.
- Offsets are code points in Rust; UTF-16 only in what `view.rs` hands the frontend.
- Style A: paper `#fbfaf6` background, `#222` text, serif `Georgia, 'Iowan Old Style', serif`; navy `#1f3a5f` accents and active tab; rules `#e6e2d8`; UI chrome `-apple-system, system-ui, sans-serif`. A dark variant follows `prefers-color-scheme` (paper `#1b1d22`, text `#d7dae0`, accents `#61afef`, rules `#2c313a`).

## Review Focus

1. **A paper with astral characters** (an emoji, `𝛼`, a CJK extension name) before a heading: the frontend's slices must land on the same text Rust's spans name. Test in Task 7.
2. **The same paper reached twice by different identifiers** (PMID first, DOI later, or a PMC ID with and without `PMC`): one stored paper, not two. Test in Task 3.
3. **A non-UTF-8 file** (a Latin-1 saved page, a UTF-16 text file): refused with a message naming the encoding problem, never decoded lossily into wrong text. Test in Task 4.
4. **An oversized or very deeply nested HTML file**: refused by size before parsing, with the limit named, not a hang. Test in Task 4.
5. **The network down during identifier retrieval**: retried, then a message that says the sources could not be reached, distinct from "no free full text exists". Test in Task 5.

---

### Task 1: Scaffold the workspace, app shell and gates

**Files:**
- Create: `Cargo.toml` (workspace), `crates/studyanalyzer-core/Cargo.toml`, `crates/studyanalyzer-core/src/lib.rs`, `crates/studyanalyzer-core/src/config.rs`
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/Entitlements.plist`, `src-tauri/icons/*` (generate with `pnpm tauri icon` from a placeholder)
- Create: `package.json`, `pnpm-lock.yaml`, `index.html`, `vite.config.ts`, `vitest.config.ts`, `svelte.config.js`, `tsconfig.json`, `src/main.ts`, `src/App.svelte`, `src/app.css`, `src/lib/smoke.test.ts`
- Modify: `.gitignore` (add `node_modules/`, `dist/`, `target/`, `src-tauri/gen/`)
- Test: `crates/studyanalyzer-core/src/config.rs` (unit), `src/lib/smoke.test.ts`

**Interfaces:**
- Produces: workspace members `crates/studyanalyzer-core` (lib `studyanalyzer_core`) and `src-tauri` (package `studyanalyzer`); pnpm scripts `dev`, `build`, `vite:dev`, `vite:build`, `check:web` (`svelte-check --tsconfig ./tsconfig.json`), `test:web` (`vitest run`). `config.rs` starts with `pub const APP_NAME: &str = "StudyAnalyzer";` and gains a constant per later task.

- [ ] **Step 1: Write the workspace and core crate.** Root `Cargo.toml` with `[workspace] resolver = "2"`, members above, `[workspace.package] version = "0.1.0"`, `edition = "2021"`, `license = "AGPL-3.0-or-later"`, and `[workspace.dependencies]` for `bmlib` (the pinned git line above), `serde` (derive), `serde_json`, `uuid` (v4), `log`, and `rusqlite = { version = "0.40", features = ["bundled"] }` — bmlib's own version, so one SQLite is linked; it is there only to name `rusqlite::Connection`, which bmlib's `open_path` returns and does not re-export (bump it with bmlib's). Core depends on all of these.
- [ ] **Step 2: Write `src-tauri`.** Dependencies: `studyanalyzer-core` (path), `tauri` 2, `tauri-plugin-dialog` 2, `tauri-plugin-log` 2, `serde`, `serde_json`, `log`; build-dep `tauri-build` 2. `tauri.conf.json`: identifier `com.bmlibrarian.studyanalyzer`, product `StudyAnalyzer`, dev port 1420, one window `main` 1280×860 (min 900×640), CSP as hanzitutor's, `bundle.macOS.entitlements = "Entitlements.plist"`. `Entitlements.plist`: `com.apple.security.app-sandbox`, `com.apple.security.network.client`, `com.apple.security.files.user-selected.read-only`, all `true`. Capabilities: `core:default`, `dialog:default`, `log:default`.
- [ ] **Step 3: Write the frontend shell.** `App.svelte` shows the title `StudyAnalyzer` in the style A top bar; `app.css` defines the style A tokens of Global Constraints as CSS custom properties (`--paper-bg`, `--ink`, `--accent`, `--rule`, `--serif`, `--sans`, `--hl-current: #e8b931`, `--hl-past: #f6e7b0`) with the dark set under `@media (prefers-color-scheme: dark)`. `smoke.test.ts` asserts `1 + 1 === 2` so `test:web` has a suite until Task 9.
- [ ] **Step 4: Run every gate.** `pnpm install`, then the six gate commands. Expected: all pass (the first build fetches PDFium for bmlib's `pdf` feature). `pnpm dev` opens the window.
- [ ] **Step 5: Check CI still matches.** `.github/workflows/ci.yml` runs the same six commands once `Cargo.toml`/`package.json` exist; no change expected. If clippy or test needs a flag, change CI and CLAUDE.md § Gates together.
- [ ] **Step 6: Commit** `feat: scaffold the Tauri app and core crate`.

### Task 2: Identifiers

**Files:**
- Create: `crates/studyanalyzer-core/src/identifiers.rs`
- Modify: `crates/studyanalyzer-core/src/lib.rs`, `config.rs` (`MAX_IDENTIFIER_CHARS: usize = 256`)

**Interfaces:**
- Consumes: `bmlib::publications::retractions::canonical_pmid`, `bmlib::publications::storage::normalize_doi`, `bmlib::fulltext::service::normalise_pmc_id`.
- Produces:
  - `#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)] pub struct Identifiers { pub pmid: Option<String>, pub doi: Option<String>, pub pmc_id: Option<String> }` — every field already canonical.
  - `pub fn validate_identifiers(pmid: Option<&str>, doi: Option<&str>, pmc_id: Option<&str>) -> Result<Identifiers, IdentifierError>` — blank counts as absent; at least one must remain.
  - `pub fn parse_identifier(input: &str) -> Result<Identifiers, IdentifierError>` — one free-text identifier from the start screen, classified.
  - `pub enum IdentifierError { Empty, TooLong(usize), Invalid { kind: &'static str, value: String } }` with `Display` and `std::error::Error`.

Rules: PMID is `canonical_pmid` (digits, no leading zeros) **or** a preprint accession `PPR` + digits (spec §4.1). DOI is `normalize_doi` (strips `doi:` / `https://doi.org/`, lower-cases) and must then be `10.` + 4–9 digits + `/` + at least one non-whitespace character, with no whitespace anywhere. PMC is `normalise_pmc_id` (`PMC` + digits). `parse_identifier` tries PMC (`PMC` prefix, case-insensitive), then DOI (contains `10.` and `/`), then PMID. Anything longer than `MAX_IDENTIFIER_CHARS` is `TooLong` before any other check.

- [ ] **Step 1: Write failing tests** in `identifiers.rs`'s `#[cfg(test)] mod tests`:
  - `pmid_canonical`: `validate_identifiers(Some(" 00012345 "), None, None)` → `pmid == Some("12345")`.
  - `pmid_preprint`: `Some("PPR123456")` accepted as is.
  - `pmid_invalid`: `Some("12a45")` → `Invalid { kind: "PMID", .. }`.
  - `doi_normalised`: `Some("https://doi.org/10.1056/NEJMoa2034577")` → `"10.1056/nejmoa2034577"`.
  - `doi_invalid`: `"10.1/x"` (registrant too short), `"10.1056/a b"` → `Invalid { kind: "DOI", .. }`.
  - `pmc_bare_and_prefixed_agree`: `"7654321"` as PMC and `"PMC7654321"` give the same value.
  - `all_blank_is_empty`: `(Some(""), Some("  "), None)` → `Empty`.
  - `too_long`: a 257-character DOI → `TooLong(257)`.
  - `parse_classifies`: `"pmc7654321"`, `"doi:10.1000/xyz"`, `"34567"` land in `pmc_id`, `doi`, `pmid` respectively.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core identifiers`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the three items above in `identifiers.rs`.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): validate PMID, DOI and PMC identifiers`.

### Task 3: Storage

**Files:**
- Create: `crates/studyanalyzer-core/src/store.rs`, `crates/studyanalyzer-core/src/schema.rs`
- Modify: `lib.rs`, `config.rs` (`DB_FILE_NAME: &str = "studyanalyzer.sqlite3"`, `UNTITLED_PAPER: &str = "Untitled paper"`)

**Interfaces:**
- Consumes: `rusqlite::Connection` (implements bmlib's `Db`); `bmlib::db::{sqlite::open_path, sqlite::open_memory, migrations::{Migration, run_migrations}, operations::{execute, fetch_all, fetch_one}, transactions::transaction, Db, Value}`; `Identifiers` (Task 2); `bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment}`.
- Produces:
  - `pub fn migrations() -> Vec<Migration>` (in `schema.rs`); migration 1 creates the tables below.
  - `pub fn open_store(path: &Path) -> Result<Connection, StoreError>` (WAL, foreign keys, migrations run) and `pub fn open_memory_store() -> Result<Connection, StoreError>` for tests.
  - `pub struct PaperId(pub String)` (uuid v4 text), `#[derive(Serialize)] pub struct PaperSummary { pub id: String, pub title: String, pub identifiers: Identifiers, pub opened_at: String }`.
  - `pub fn find_paper(db: &mut dyn Db, ids: &Identifiers) -> Result<Option<PaperId>, StoreError>` — match PMID, then DOI, then PMC.
  - `pub fn create_paper(db: &mut dyn Db, ids: &Identifiers, title: Option<&str>) -> Result<PaperId, StoreError>`.
  - `pub fn add_identifiers(db: &mut dyn Db, paper: &PaperId, ids: &Identifiers) -> Result<(), StoreError>` — fills absent ones only; never overwrites a different stored value (logs a warning instead).
  - `pub fn set_title(db, paper, title: &str)`, `pub fn touch_opened(db, paper)`.
  - `pub fn save_source(db, paper, src: &NewSource) -> Result<i64, StoreError>` where `pub struct NewSource { pub kind: SourceKind, pub origin: String, pub file_name: Option<String>, pub bytes: Vec<u8> }` (`SourceKind` from Task 4; `origin` is `"upload"`, `"paste"` or the `FullTextResult::source`).
  - `pub fn latest_source(db, paper) -> Result<Option<StoredSource>, StoreError>` (`StoredSource { id: i64, kind, bytes }`).
  - `pub fn save_canonical(db, paper, source_id: i64, text: &CanonicalText, assessment: &FullTextAssessment) -> Result<i64, StoreError>`.
  - `pub fn latest_canonical(db, paper) -> Result<Option<StoredCanonical>, StoreError>` (`StoredCanonical { id, source_id, text: CanonicalText, assessment_json: String, confirmed_full_text: bool }`).
  - `pub fn confirm_full_text(db, canonical_id: i64) -> Result<(), StoreError>`.
  - `pub fn recent_papers(db) -> Result<Vec<PaperSummary>, StoreError>` — every paper, most recently opened first (no limit: no truncation).
  - `pub fn get_setting(db, key: &str) -> Result<Option<String>, StoreError>`, `pub fn set_setting(db, key: &str, value: &str) -> Result<(), StoreError>`.
  - `pub enum StoreError { Db(DbError), Corrupt(String) }` with `Display`, `Error`, `From<DbError>`.

Schema (migration 1, `schema_version` is bmlib's):

```sql
CREATE TABLE papers (id TEXT PRIMARY KEY, pmid TEXT UNIQUE, doi TEXT UNIQUE, pmc_id TEXT UNIQUE,
  title TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), opened_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE sources (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, origin TEXT NOT NULL, file_name TEXT, bytes BLOB NOT NULL, received_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE canonical_texts (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  source_id INTEGER NOT NULL REFERENCES sources(id), converter_version TEXT NOT NULL, canonical_json TEXT NOT NULL,
  assessment_json TEXT NOT NULL, confirmed_full_text INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

`canonical_json` is `serde_json::to_string(&CanonicalText)`; read back through `CanonicalText::from_value` (it validates) — a failure is `StoreError::Corrupt`. "Latest" means highest `id`.

- [ ] **Step 1: Write failing tests** in `store.rs` (each on `open_memory_store()`):
  - `migrations_apply_once`: opening twice on one file (tempdir) leaves `schema_version` with one row.
  - `same_paper_by_other_identifier` (Review Focus 2): create with `pmid 123`; `add_identifiers` with `doi 10.1000/x`; `find_paper` by `{doi: 10.1000/x}` alone returns the same id; `find_paper` by `{pmc_id: PMC9}` returns `None`.
  - `add_identifiers_never_overwrites`: stored `doi A`, adding `doi B` leaves `A`.
  - `canonical_round_trip`: save a source and `canonical_from_plain_text("Para one.\n\nPara two.", "T")` with its assessment; `latest_canonical` returns an equal `CanonicalText` and `confirmed_full_text == false`; after `confirm_full_text`, `true`.
  - `corrupt_canonical_is_reported`: overwrite `canonical_json` with `{}` via SQL → `latest_canonical` is `Err(StoreError::Corrupt(_))`.
  - `recent_is_complete_and_ordered`: 30 papers, touched in a known order → all 30 back, newest first.
  - `settings_round_trip`: `set_setting("contact_email", "a@b.c")` then `get_setting` → `Some("a@b.c")`; unknown key → `None`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core store`. Expected: FAIL.
- [ ] **Step 3: Implement** `schema.rs` and `store.rs` with the signatures above; multi-row writes go through `transaction`.
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): SQLite storage for papers, sources and canonical texts`.

### Task 4: Intake — any source to canonical text

**Files:**
- Create: `crates/studyanalyzer-core/src/intake.rs`, fixtures `crates/studyanalyzer-core/tests/fixtures/{article.xml,page.html,paper.md,paper.txt}` (small, hand-written; the JATS one has `<body>` with Methods and Results sections)
- Modify: `lib.rs`, `config.rs`

**Interfaces:**
- Consumes: `bmlib::fulltext::canonical::{canonical_from_jats, canonical_from_html, canonical_from_markdown, canonical_from_plain_text, canonical_from_segmented, assess_full_text, CanonicalText, FullTextAssessment}`, `bmlib::fulltext::jats_reader::parse`, `bmlib::fulltext::pdf_text::PdfTextExtractor`, `bmlib::fulltext::segmenter::SectionSegmenter` (`Default`, then `segment_document(&blocks, path, Some(title), serde_json::json!({}))`).
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")] pub enum SourceKind { Pdf, Jats, Html, Markdown, PlainText }` with `as_str()` / `parse()` for storage.
  - `pub fn detect_kind(file_name: Option<&str>, bytes: &[u8]) -> Result<SourceKind, IntakeError>` — `%PDF-` magic → Pdf; else by extension (`.xml`/`.nxml` → Jats, `.html`/`.htm` → Html, `.md`/`.markdown` → Markdown, `.txt` → PlainText); with no name (paste): text starting (after whitespace) with `<?xml` or `<article` → Jats, with `<!doctype html`/`<html` (case-insensitive) → Html, a line starting `#` + space → Markdown, else PlainText. An unknown extension is `Unsupported`.
  - `pub fn check_size(kind: SourceKind, len: usize) -> Result<(), IntakeError>`.
  - `pub fn decode_text(bytes: &[u8]) -> Result<&str, IntakeError>` — strict UTF-8 (a leading BOM stripped); anything else is `NotUtf8`.
  - `pub fn to_canonical(kind: SourceKind, bytes: &[u8], title_hint: Option<&str>, pdf: &dyn PdfTextExtractor, scratch_dir: &Path) -> Result<CanonicalText, IntakeError>` — checks size first; PDF bytes go to a uniquely named file in `scratch_dir`, removed afterwards whatever the outcome; plain text uses `title_hint.unwrap_or("")`.
  - `pub fn assess(text: &CanonicalText) -> Result<FullTextAssessment, IntakeError>`.
  - `pub fn current_converter_version(kind: SourceKind) -> String` (`SourceFormat::converter_version` with bmlib's `*_VERSION` constants).
  - `pub enum IntakeError { Unsupported(String), TooLarge { kind: SourceKind, len: usize, max: usize }, NotUtf8, Jats(String), Pdf(String), Empty, Canonical(String), Io(String) }` with `Display` (user-readable: `TooLarge` names both sizes in MB) and `Error`.
- `config.rs`: `MAX_PDF_BYTES = 100 * 1024 * 1024`, `MAX_HTML_BYTES = 5 * 1024 * 1024` (the parse is quadratic in nesting depth, bmlib 4c), `MAX_TEXT_BYTES = 20 * 1024 * 1024` (JATS, Markdown, plain).

- [ ] **Step 1: Write failing tests** in `intake.rs`, with a `FakePdf` implementing `PdfTextExtractor` that returns two headings (`Methods`, `Results`) and paragraphs as `TextBlock`s:
  - `detect_by_magic_and_name`: PDF magic wins over a `.txt` name; each extension maps as listed; `.docx` → `Unsupported`.
  - `detect_paste`: `"  <!DOCTYPE html>…"` → Html; `"# Title\n\ntext"` → Markdown; `"Plain words."` → PlainText.
  - `latin1_refused` (Review Focus 3): bytes `b"caf\xe9"` as PlainText → `Err(NotUtf8)`; UTF-16LE with BOM → `Err(NotUtf8)`; UTF-8 with BOM → Ok.
  - `oversize_html_refused_before_parse` (Review Focus 4): `check_size(Html, MAX_HTML_BYTES + 1)` → `TooLarge`; `to_canonical(Html, …)` on a buffer of `MAX_HTML_BYTES + 1` bytes of `<div>` returns `TooLarge` (and does so in well under a second).
  - `each_fixture_converts`: each fixture gives a `CanonicalText` whose `converter_version` equals `current_converter_version(kind)` and whose text contains a sentence from the fixture.
  - `jats_fixture_is_full_text`: `assess` on the JATS fixture's text → `is_full_text == true` (the fixture carries ≥ `MIN_BODY_WORDS` words; generate filler sentences in the fixture file, not at test time).
  - `pdf_via_extractor_cleans_scratch`: `to_canonical(Pdf, b"%PDF-1.4…", Some("T"), &FakePdf, tmp)` → sections include `Methods`; `tmp` is empty afterwards, also when `FakePdf` returns `Err`.
  - `empty_text_refused`: whitespace-only plain text → `Err(Empty)` (no blocks to read).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core intake`. Expected: FAIL.
- [ ] **Step 3: Implement** the items above.
- [ ] **Step 4: Run** again. Expected: PASS. Then one manual check with a real PDF and `PdfiumExtractor::bundled()` from a `#[ignore]`d test, `cargo test -p studyanalyzer-core -- --ignored real_pdf`, the PDF given by `STUDYANALYZER_TEST_PDF`.
- [ ] **Step 5: Commit** `feat(core): turn PDF, JATS, HTML, Markdown and text into canonical text`.

### Task 5: Retrieval by identifier

**Files:**
- Create: `crates/studyanalyzer-core/src/retrieval.rs`, `crates/studyanalyzer-core/src/retry.rs`
- Modify: `lib.rs`, `config.rs` (`RETRIEVAL_ATTEMPTS: u32 = 3`, `RETRY_BASE_DELAY: Duration = Duration::from_secs(2)`)

**Interfaces:**
- Consumes: `bmlib::fulltext::service::{FullTextService, FullTextRequest, FullTextError}`, `bmlib::fulltext::models::{FullTextResult, ContentKind}`, `Identifiers`, `SourceKind`.
- Produces:
  - `pub fn with_backoff<T, E>(attempts: u32, base_delay: Duration, retryable: impl Fn(&E) -> bool, f: impl FnMut() -> Result<T, E>) -> Result<T, E>` in `retry.rs`; delay doubles; logs each retry. 2b reuses it for `chat()` (bmlib #561).
  - `pub trait FullTextSource { fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError>; }`, implemented for `FullTextService`.
  - `pub enum Retrieved { Found { kind: SourceKind, origin: String, bytes: Vec<u8> }, NotFound { reason: String }, Unreachable { reason: String } }`.
  - `pub fn retrieve(source: &dyn FullTextSource, ids: &Identifiers, attempts: u32, base_delay: Duration) -> Retrieved`.
  - `pub fn build_service(cache_dir: PathBuf, email: &str) -> FullTextService` — `FullTextService::new(Arc::new(UreqClient::new()), email).with_cache(FullTextCache::new(Some(cache_dir)))` (its own cache in app storage, per bmlib item 9).

Rules: request `pmid` = `ids.pmid` or `""`, `doi`, `pmc_id`, `identifier` = DOI else PMID else PMC ID (the cache key). `Ok` with `ContentKind::Fulltext` and `jats_xml: Some(b)` → `Found { kind: Jats, bytes: b }`. `Ok` with `ContentKind::Extracted` and `file_path: Some(p)` → read `p` (bmlib's own cache under our cache dir, not an outside path) → `Found { kind: Pdf }`; an unreadable file is `Unreachable`. `Ok` with an `Abstract`, `None` or a link only → `NotFound` (never the abstract). `Err(Unavailable)` → `NotFound`, not retried. `Err(Other)` → retried; still failing → `Unreachable` with the last message.

- [ ] **Step 1: Write failing tests** with a `FakeSource` holding a queue of results:
  - `backoff_retries_then_succeeds`: `with_backoff(3, ZERO, |_| true, …)` failing twice then OK → OK after 3 calls; non-retryable error → 1 call.
  - `jats_found`: Fulltext + `jats_xml` → `Found { kind: Jats, origin: "europepmc", .. }`.
  - `abstract_is_not_found`: `ContentKind::Abstract` with `jats_xml` set → `NotFound`.
  - `unavailable_not_retried`: one `Err(Unavailable)` → `NotFound`, fake called once.
  - `network_down_is_unreachable` (Review Focus 5): three `Err(Other("connection refused"))` → `Unreachable` whose reason contains `connection refused`, fake called `RETRIEVAL_ATTEMPTS` times.
  - `cache_key_prefers_doi`: the request the fake saw has `identifier == Some(doi)`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core retrieval retry`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): retrieve full text by identifier with retries`.

### Task 6: Opening a paper

**Files:**
- Create: `crates/studyanalyzer-core/src/open.rs`
- Modify: `lib.rs`

**Interfaces:**
- Consumes: Tasks 2–5.
- Produces:
  - `pub enum OpenRequest { File { name: Option<String>, bytes: Vec<u8> }, Paste { text: String }, Identifier { input: String } }`.
  - `pub struct OpenContext<'a> { pub db: &'a mut dyn Db, pub source: &'a dyn FullTextSource, pub pdf: &'a dyn PdfTextExtractor, pub scratch_dir: &'a Path, pub attempts: u32, pub base_delay: Duration }`.
  - `#[derive(Serialize)] #[serde(tag = "status", rename_all = "snake_case")] pub enum OpenOutcome { Ready { paper_id: String }, NeedsFullText { paper_id: String, reason: String, unreachable: bool } }`.
  - `pub fn open(ctx: &mut OpenContext, request: OpenRequest) -> Result<OpenOutcome, OpenError>` — a new paper from a file or paste; for an identifier, an existing paper with a canonical text reopens without network.
  - `pub fn attach(ctx: &mut OpenContext, paper_id: &str, request: OpenRequest) -> Result<OpenOutcome, OpenError>` — the upload/paste fallback for a paper in `NeedsFullText` (Identifier is refused here).
  - `pub fn reopen(ctx: &mut OpenContext, paper_id: &str) -> Result<OpenOutcome, OpenError>` — from the recent list; re-derives when `needs_rederive` says so.
  - `pub fn needs_rederive(stored_version: &str, current_version: &str, has_dependents: bool) -> bool` — true only when versions differ and nothing points into the stored text (2a: `has_dependents` is always `false`; 2b passes the real answer).
  - `pub enum OpenError { Identifier(IdentifierError), Intake(IntakeError), Store(StoreError), UnknownPaper(String) }` with `Display`, `Error`, `From` for each.

Flow for `Ready`: store the source, convert, `assess`, store the canonical text, set the title (canonical title span text, else the file name without extension, else `UNTITLED_PAPER`), `touch_opened`. A failed full-text check is still `Ready`: the banner is the frontend's job, from the view (Task 7).

- [ ] **Step 1: Write failing tests** on `open_memory_store()`, a `FakeSource` and `FakePdf`:
  - `paste_opens_ready`: a Markdown paste → `Ready`; `latest_canonical` exists with `markdown-1`.
  - `identifier_found_then_reopened_offline`: JATS fixture via fake → `Ready`; a second `open` with the same PMID → `Ready` with the same `paper_id`, fake called once in total.
  - `identifier_not_found_then_attach`: `NotFound` → `NeedsFullText { unreachable: false }` and a paper row with the PMID; `attach` with the HTML fixture → `Ready`, same id.
  - `identifier_unreachable_flagged`: `Unreachable` → `NeedsFullText { unreachable: true }`.
  - `bad_identifier_creates_nothing`: `"abc"` → `Err(Identifier(_))`; `recent_papers` is empty.
  - `intake_error_rolls_back`: a Latin-1 file → `Err(Intake(NotUtf8))` and no paper or source row remains.
  - `rederive_rule`: `needs_rederive("jats-1","jats-2",false)` true; same versions false; `has_dependents` true → false.
  - `reopen_rederives_old_version`: store a canonical text, rewrite its `converter_version` to `jats-0` by SQL, `reopen` → a new canonical row with `current_converter_version(Jats)`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core open`. Expected: FAIL.
- [ ] **Step 3: Implement**, each request's writes inside one `transaction`.
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): open a paper from a file, paste or identifier`.

### Task 7: The paper view at the boundary

**Files:**
- Create: `crates/studyanalyzer-core/src/view.rs`
- Modify: `lib.rs`

**Interfaces:**
- Consumes: `StoredCanonical`, `CanonicalText`, `FullTextAssessment`'s JSON.
- Produces:
  - `pub fn utf16_offsets(text: &str) -> Vec<usize>` — entry `i` is the UTF-16 offset of code point `i`; one extra entry for the end.
  - `pub fn span_to_utf16(offsets: &[usize], span: TextSpan) -> Utf16Span`; `#[derive(Serialize)] pub struct Utf16Span { pub start: usize, pub end: usize }`.
  - `#[derive(Serialize)] pub struct PaperView { pub paper_id: String, pub title: String, pub text: String, pub title_span: Option<Utf16Span>, pub sections: Vec<SectionView>, pub converter_version: String, pub full_text: FullTextStatus }`; `SectionView { level: usize, heading: Option<Utf16Span>, blocks: Vec<BlockView> }`; `BlockView { kind: BlockKind, span: Utf16Span }`; `FullTextStatus { passed: bool, confirmed: bool, body_words: usize, section_types: Vec<String> }`.
  - `pub fn paper_view(paper_id: &str, title: &str, stored: &StoredCanonical) -> Result<PaperView, StoreError>`.
  - 2b reuses `utf16_offsets`/`span_to_utf16` for citation highlights; nothing else converts.

- [ ] **Step 1: Write failing tests:**
  - `ascii_is_identity`: offsets of `"abc"` are `[0,1,2,3]`.
  - `astral_counts_two` (Review Focus 1): for `"a😀b𝛼c"` the offsets are `[0,1,3,4,6,7]`; a span over `b` maps to `{3,4}`.
  - `view_spans_slice_same_text`: a canonical text built with `canonical_from_markdown("# T😀\n\n## Méthods 𝛼\n\nBody 😀 text.")`: for every block and heading, slicing the text's UTF-16 encoding at the view's span decodes to `canonical.span_text(span)`.
  - `status_from_assessment`: a failing assessment, unconfirmed → `passed: false, confirmed: false`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core view`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): paper view with UTF-16 spans for the frontend`.

### Task 8: Tauri commands

**Files:**
- Create: `src-tauri/src/commands.rs`, `src-tauri/src/state.rs`, `src-tauri/src/error.rs`
- Modify: `src-tauri/src/lib.rs`, `crates/studyanalyzer-core/src/config.rs` (`CONTACT_EMAIL_KEY: &str = "contact_email"`)

**Interfaces:**
- Consumes: Tasks 3–7; `bmlib::fulltext::pdfium_backend::PdfiumExtractor::bundled()`.
- Produces (all `#[tauri::command]`, `async`, blocking work in `tauri::async_runtime::spawn_blocking`):
  - `open_file(path: String) -> Result<OpenOutcome, CommandError>` — the path comes only from the dialog or a drop event; size checked from metadata before reading.
  - `open_paste(text: String)`, `open_identifier(input: String)` → `Result<OpenOutcome, CommandError>`.
  - `attach_file(paper_id: String, path: String)`, `attach_paste(paper_id: String, text: String)` → `Result<OpenOutcome, CommandError>`.
  - `reopen_paper(paper_id: String) -> Result<OpenOutcome, CommandError>`, `recent_papers() -> Result<Vec<PaperSummary>, CommandError>`, `paper_view(paper_id: String) -> Result<PaperView, CommandError>`, `confirm_full_text(paper_id: String) -> Result<(), CommandError>`.
  - `get_contact_email() -> Result<Option<String>, CommandError>`, `set_contact_email(email: String) -> Result<(), CommandError>` (must contain one `@` with text either side; else `invalid_input`).
  - `#[derive(Serialize)] pub struct CommandError { pub kind: &'static str, pub message: String }` with `From<OpenError>`, `From<StoreError>`; kinds: `invalid_input`, `too_large`, `unsupported_format`, `not_utf8`, `needs_email`, `storage`, `internal`. Every conversion logs at `error` (or `warn` for user input) before returning.
  - `AppState { db: Mutex<Connection>, data_dir: PathBuf, cache_dir: PathBuf, pdf: OnceLock<Result<PdfiumExtractor, String>> }`, built in `setup` from `app.path().app_data_dir()` / `app_cache_dir()`. Set `PDFIUM_BUNDLED_CACHE_DIR` to `cache_dir/pdfium` before the first `bundled()` call (the sandbox denies the default). `open_identifier` with no contact email → `needs_email`, no network.

- [ ] **Step 1: Write failing tests** in `error.rs`: `each_open_error_maps_to_a_kind` (one assertion per `OpenError`/`IntakeError` variant, e.g. `TooLarge` → `too_large`, message names the limit); `contact_email_checked` (`"a@b"` ok, `"ab"` and `"@b"` refused).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands, state and `lib.rs` wiring (`tauri-plugin-log` with stdout and a log file in the app's log dir; `tauri-plugin-dialog`).
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `feat(app): Tauri commands over the core`.

### Task 9: Frontend — start screen, fallback, paper pane, settings

**Files:**
- Create: `src/lib/api.ts` (typed `invoke` wrappers; the TS types mirror Tasks 6–8's `Serialize` shapes), `src/lib/paper.ts`, `src/lib/paper.test.ts`, `src/components/StartScreen.svelte`, `src/components/NeedsFullText.svelte`, `src/components/PaperPane.svelte`, `src/components/FullTextBanner.svelte`, `src/components/Settings.svelte`, `src/components/ErrorBanner.svelte`
- Modify: `src/App.svelte`, `src/app.css`; delete `src/lib/smoke.test.ts`

**Interfaces:**
- Consumes: Task 8's commands; `@tauri-apps/plugin-dialog` `open()`; `getCurrentWebview().onDragDropEvent`.
- Produces (pure, in `paper.ts`):
  - `sliceSpan(text: string, span: Utf16Span): string`.
  - `groupBlocks(blocks: BlockView[]): Array<{ kind: "paragraph"; span: Utf16Span } | { kind: "table"; rows: Utf16Span[] }>` — consecutive `table_row` blocks form one table.
  - `splitCells(row: string): string[]` — on `\t`.
  - `bannerText(status: FullTextStatus): string | null` — `null` when `passed` or `confirmed`; else names what was missing (no Methods heading / no Results or Discussion heading / fewer than 1,000 words, using `body_words`).

Screens (`App.svelte` holds `screen: "start" | "needs_full_text" | "paper" | "settings"` in `$state`):
- **Start:** recent papers (title, identifiers, opened date; click → `reopen_paper`), *Open a file…* (dialog filtered to pdf, xml, nxml, html, htm, md, markdown, txt) and drag-and-drop onto the window, a *Paste text* area with *Open*, an identifier field (*PMID, DOI or PMC ID*) with *Find*. A spinner with *Looking for the full text…* while `open_identifier` runs.
- **Needs full text:** the reason; when `unreachable`, *The sources could not be reached — check the connection and try again* with *Try again*; otherwise *No free full text was found*. Both offer *Upload a file* and *Paste the text* (→ `attach_*`). Never offers the abstract.
- **Paper:** the top bar (title, *← Papers*, *Settings*), `FullTextBanner` (with *This is the whole paper* → `confirm_full_text`), and `PaperPane` full width: title, headings `h2`/`h3` by level in `--accent`, paragraphs in `--serif`, table rows in a `<table>`. 2b splits this screen into chat | paper.
- **Settings:** contact email (explained: *Europe PMC and Unpaywall ask for an address with each request*); reached also on a `needs_email` error.
- `ErrorBanner` shows any `CommandError.message`, dismissible, on every screen.

- [ ] **Step 1: Write failing tests** in `paper.test.ts`: `sliceSpan` on `"a😀b"` with `{start:1,end:3}` → `"😀"`; `groupBlocks` on `[p, row, row, p]` → `[paragraph, table(2 rows), paragraph]`; `splitCells("a\tb\t")` → `["a","b",""]`; `bannerText` → `null` for passed, `null` for confirmed, text containing `Methods` when `section_types` lacks `methods`.
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement** `paper.ts`, then the components in Svelte 5 runes (`$state`, `$props`, `$derived`).
- [ ] **Step 4: Run** `pnpm run test:web`, `pnpm run check:web`, `pnpm exec vite build`. Expected: PASS.
- [ ] **Step 5: Check it by hand** with `pnpm dev`: open a PDF, a JATS file, a saved HTML page, a pasted Markdown text; a PMID with free full text (e.g. a PMC open-access trial) and one without; Wi-Fi off for the unreachable path; the system in dark mode. Each lands on the right screen; the paper reads in style A.
- [ ] **Step 6: Commit** `feat(ui): start screen, full-text fallback, paper pane and settings`.

### Task 10: Hand over

**Files:**
- Modify: `README.md` (how to run, build, test), `HANDOVER.md`, `ROADMAP.md`, `CLAUDE.md` (*State of the repository*: code exists)

- [ ] **Step 1:** Run all six gates once more. Expected: PASS.
- [ ] **Step 2:** Update the docs: 2a landed; 2b next once bmlib item 11 lands; anything found on the way filed as issues.
- [ ] **Step 3: Commit** `docs: phase 2a landed`, push, open the PR.
