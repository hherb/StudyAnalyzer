# Phase 2d: the hand-off — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Another app hands StudyAnalyzer a paper: a `.studyanalyzer` file
(identifiers, title, sender, and the full text inside it) opened with the app,
or a `studyanalyzer://open?…` link (identifiers only). Either way it reaches
the running app (single instance), is stored at once, and opens as soon as
nothing else is running. File paths reach Rust only from the system (dialog,
drop, an opened file); the webview never names one.

**Architecture:** As 2a–2c: logic in `crates/studyanalyzer-core` (the format,
its storage, opening it), thin glue in `src-tauri` (plugins, run events,
commands), Svelte 5 in `src/`. A received hand-off is parsed and validated in
Rust, stored in a new `handoffs` table, and announced to the webview by an
event; the webview opens it with a command when idle. A file the user chooses
or drops becomes a one-time *grant* in Rust; commands take the grant's id.

**Tech Stack:** Rust 2021, Tauri 2 (`tauri-plugin-single-instance` 2 with
its `deep-link` feature, `tauri-plugin-deep-link` 2; both Apache-2.0 OR MIT),
`base64` and `url` (both already in `Cargo.lock`, now direct dependencies of
the core crate), bmlib at `a491838` (PR #66), Svelte 5 + TypeScript + Vite,
vitest.

**Spec:** `docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`
(§4.1, §4.2, §8, §10, §11). Builds on HANDOVER.md *Phase 2a* (open, place,
persist, `Mismatch`, the contract) and *Phase 2b* (dependents, #65).

### Builder's calls this plan settles (spec §11)

- **The container: JSON, the full text base64 inside** (not a zip). Every
  sender's platform writes JSON and base64 with its standard library (Swift
  `JSONEncoder` + `Data.base64EncodedString()`, Kotlin, Python), a zip
  reader is attack surface (bombs, entry paths) we need not have, and the
  33% overhead is paid once per hand-off. The format is documented for
  senders in `docs/handoff-format.md` and pinned by fixtures (Task 2):

  ```json
  {
    "format": "studyanalyzer-handoff",
    "version": 1,
    "sender": "MedicalFactChecker",
    "title": "Optional title the sender already has",
    "identifiers": { "pmid": "…", "doi": "…", "pmcid": "…" },
    "full_text": {
      "format": "pdf | jats | html | markdown | text",
      "data": "<base64 of the file's bytes, standard alphabet, padded>",
      "file_name": "optional, a name only",
      "source": "optional, where the sender got it, e.g. Europe PMC"
    }
  }
  ```

  `sender` is required and non-empty; `title`, every identifier and
  `full_text` are optional, but a hand-off must carry a valid identifier or
  full text. Text formats are bytes too (base64), decoded by 2a's strict
  `decode_text`, so a hand-off's text reads exactly as the same file opened
  by hand.
- **Versioning:** `version` is an integer. Adding an optional field does not
  bump it, and unknown fields are ignored (a newer sender still works with an
  older app). A breaking change bumps it; a `version` above
  `HANDOFF_VERSION` (1) is refused naming both numbers (*made by a newer
  version of {sender}; update StudyAnalyzer*). A missing or different
  `format` is *not a hand-off file*. A duplicate key is malformed.
- **Trust boundary (§4.1).** The file is read up to one byte past
  `MAX_HANDOFF_BYTES` (refused past it, before parsing). Each identifier is
  checked on its own (`identifiers::check_handed`: as `check_fetched`, but
  the PMID field takes a `PPR` accession, as §4.1 allows); an invalid one is
  dropped with a notice shown on open, as a JATS file's own invalid
  identifier is (2a). `file_name` is a name, never a path: one with `/`,
  `\`, a control character, or past `MAX_HANDOFF_LABEL_CHARS` is malformed,
  as is a `sender` or `source` with a control character or past that cap,
  and a `title` past `MAX_HANDOFF_TITLE_CHARS`. Nothing in the file is
  dereferenced: `source` is shown, `file_name` is the title hint and nothing
  else, and the kind comes from `full_text.format` (a `pdf` whose bytes do
  not start as a PDF is 2a's `IntakeError::NotPdf`). Nothing is truncated:
  an over-long field refuses the file, naming the field and the cap.
- **The link** `studyanalyzer://open?pmid=…&doi=…&pmcid=…`: scheme
  `studyanalyzer`, host `open`, at most `MAX_LINK_CHARS`; each value is
  percent-decoded (the `url` crate) and checked as in the file; a repeated
  key or an empty value is malformed; an unknown key is ignored and logged
  (forward compatible). No sender, no title.
- **How Rust trusts a path (the open question in HANDOVER *Next*).** Paths
  exist only in Rust. The file dialog runs in Rust (`pick_file`, through
  `tauri_plugin_dialog`'s Rust API); a drop is read from Tauri's
  `WindowEvent::DragDrop` in Rust; an opened file comes from
  `RunEvent::Opened` (macOS, iOS) or the command line (Linux, Windows, and
  a second instance's through `tauri-plugin-single-instance`). A chosen or
  dropped file becomes a **grant**: an id the webview passes to
  `open_file` / `attach_file`, good once, and replaced by the next pick or
  drop (one paper at a time). A dropped `.studyanalyzer` file is received as
  a hand-off instead. The webview loses the dialog permission
  (`dialog:default` leaves `capabilities/default.json`), so a script in the
  page can neither choose a path nor open one. A path from a command line
  is only ever read as a hand-off file (extension `.studyanalyzer`, then
  parsed): any program can launch the app with arguments, and one that can
  name a file can already read it.
- **A hand-off is stored on arrival, opened when idle.** The received file
  is read, parsed and stored (the `handoffs` table, migration 8) before the
  webview hears of it, so the copy is made while the granted access lasts
  (§4.1), nothing lives only in memory, and a hand-off that launched the app
  (the event fires before the webview listens) is still found. A refused
  hand-off is stored too, as a row holding only its refusal, so the user
  sees why even on a cold start. The webview opens the newest pending
  hand-off it has not tried this launch, once nothing runs (`busy` and
  `interrogatorBusy` both false, the gate navigation already uses). So a
  hand-off never saves a text while an ask or a PICO run is under way, and
  **#65 cannot be reached from the UI**. The issue stays open for the
  maintainer's call on what the error should say. Every other pending row
  is listed on the start screen (*Handed over*) with *Open* and *Discard*.
- **When a row goes.** `open_handoff` deletes the row when the open ends in
  an outcome (`Ready`, `NeedsFullText`) or in an error that a retry would
  repeat (the document's own fault, a mismatch, a cancel). It keeps the row
  on `needs_email`, where the email screen's save re-runs the open, and on
  our own faults (`internal`, `store`), where the error is shown and the row
  waits for *Open* or the next launch. A shown refusal row is deleted once
  shown.
- **Opening a hand-off** reuses `open`:
  - **With full text**, it converts as a file (`origin` is
    `Handoff::origin()`: *{sender}, from {source}*, or *{sender}*). The
    hand-off's identifiers are the claim, like typed ones; a JATS document's
    own identifiers fill gaps. One that differs from the claim of the same
    kind refuses the hand-off with `OpenError::HandoffMismatch(why)`, storing
    nothing. Then 2a's `find_paper` / `place` / `persist` run as for a file:
    a contradiction with the stored paper is `Mismatch`, or lands apart
    under `KEEP_MISMATCHED_KEY`, and the same document again stores no copy.
    The hand-off's title is the record title (`convert`'s `record_title`).
  - **With identifiers only**, it is a lookup by those identifiers.
    `open_identifier` is split into parsing and an
    `open_by_identifiers(ctx, ids, title, notes)` that both use, so the
    contact email, the limit, *Cancel* and the "typed identifier wins" rule
    hold unchanged. The hand-off's title is used when the record has none,
    and its notices join the record's notes.
- **Single instance:** desktop only (`#[cfg(desktop)]`), registered as the
  first plugin; its callback reads the second instance's arguments as
  above, and focuses the main window. On Linux and Windows the deep-link
  feature forwards a link argument to `on_open_url`, so the callback skips
  arguments that start with `studyanalyzer:`. Links arrive through
  `app.deep_link().on_open_url` and, at launch, `get_current()`; files
  through `RunEvent::Opened` (only its `file:` URLs, links are the plugin's)
  and the arguments. `register_all()` runs on Linux and in Windows debug
  builds, as the plugin's guide says. No webview permission is added; all of
  it is Rust-side.
- **Bundle:** `bundle.fileAssociations` declares extension `studyanalyzer`,
  name *StudyAnalyzer hand-off*, role `Viewer`, MIME type
  `application/vnd.bmlibrarian.studyanalyzer+json`, and on macOS an
  `exportedType` with identifier `com.bmlibrarian.studyanalyzer.handoff`
  conforming to `public.json`. `plugins.deep-link.desktop.schemes` is
  `["studyanalyzer"]`. No new entitlement: macOS grants a sandboxed app the
  file it is asked to open. Android's `content://` route (§4.1) is phase 4.
- **The hand-off code stays here.** It reads a format only StudyAnalyzer
  receives. When a second sender wants a *writer* (bmlibrarian_lite in
  Python, MedicalFactChecker's mobile builds through `bmlib-ffi`), the
  writer goes to bmlib under the bmlib-first rule, against Task 2's fixtures.

## Global Constraints

- Everything in 2a's–2c's Global Constraints still holds: gates before every commit (CLAUDE.md § Gates), constants in `config.rs`, doc comments, tests on outward-facing items, no truncation, every error logged and shown, sandbox-ready, code points in Rust.
- Constants (`config.rs`): `HANDOFF_FORMAT = "studyanalyzer-handoff"`, `HANDOFF_VERSION: u64 = 1`, `HANDOFF_EXTENSION = "studyanalyzer"`, `HANDOFF_MANIFEST_BYTES = 1024 * 1024`, `MAX_HANDOFF_BYTES = MAX_PDF_BYTES.div_ceil(3) * 4 + HANDOFF_MANIFEST_BYTES` (the largest PDF in base64, plus room for the rest), `MAX_HANDOFF_LABEL_CHARS = 200`, `MAX_HANDOFF_TITLE_CHARS = 2000`, `MAX_LINK_CHARS = 2048`, `LINK_SCHEME = "studyanalyzer"`, `LINK_HOST = "open"`, `LINK_ORIGIN = "a studyanalyzer:// link"`, `PAPER_FILE_EXTENSIONS` (moved from `api.ts`'s `FILE_EXTENSIONS`, built from the existing `*_EXTENSIONS` plus `pdf` and `txt`).
- Event names (`src-tauri/src/handoff.rs`, mirrored in `api.ts`, pinned in the contract): `FILES_DROPPED_EVENT = "files-dropped"` (payload `GrantedFile[]`), `HANDOFF_EVENT = "handoff-received"` (no payload).
- A new command goes in `handler()`, `commands()`, `RETURN_TYPES`, `api.ts` and the vitest call list together; a new shape is a key in `contract()` plus `keysMatch`/`valuesMatch` rows.
- Never fall back to the abstract: a hand-off's full text passes `assess` as any file's does; without full text it is a lookup (JATS or PDF, never the abstract).

## Review Focus

1. **A hand-off while something runs** (an answer streaming, a PICO run, a lookup, a re-derive): it waits, is listed, and opens once idle; no text is saved mid-ask (#65). Tests: Task 6 (`nextHandoff_waits_while_busy`, `nextHandoff_newest_untried`); Task 7's hand check.
2. **A hand-off that launches the app** (double-clicked with the app closed; a link with the app closed): the event fires before the webview listens, the stored row is opened at mount. Tests: Task 3 (`saved_handoff_is_pending`), Task 6 (`nextHandoff_newest_untried` on a mount-time list); Task 7's hand check (cold start, file and link).
3. **A hostile or broken hand-off** (oversize; not JSON; another format marker; a newer version; bad base64; `file_name` `../../x.pdf` or with a NUL; a `pdf` that is HTML; an invalid PMID beside a valid DOI; no identifier and no text; a link with `pmid=1&pmid=2`): refused or the field dropped with a notice, nothing read from any path in it, and the refusal shown even on a cold start. Tests: Task 2 (one per case), Task 3 (`refused_handoff_is_listed`).
4. **The paper is already stored** (the same hand-off twice; its text contradicts its own identifiers; it contradicts the stored paper): a repeat reopens the paper with no second copy (§8); a self-contradiction is `HandoffMismatch`, nothing stored; a stored-paper contradiction is `Mismatch`. Tests: Task 4 (`same_handoff_twice_one_copy`, `handoff_text_contradicts_its_ids`, `handoff_contradicts_stored_paper`, `ids_only_handoff_is_lookup`).
5. **A script in the webview names a path** (`invoke("open_file", { grant: 7 })` with a guessed id; an old grant reused): refused, nothing read. Tests: Task 1 (`grant_is_good_once`, `pick_replaces_grants`, `unknown_grant_is_refused`).

---

### Task 1: Paths stay in Rust: file grants, the Rust dialog, drops

**Files:**
- Create: `src-tauri/src/grants.rs`
- Modify: `src-tauri/src/state.rs` (`AppState.grants`), `commands.rs` (`pick_file`; `open_file`, `attach_file` take a grant), `lib.rs` (`on_window_event`, `handler()`), `contract.rs`, `capabilities/default.json` (drop `dialog:default`), `crates/studyanalyzer-core/src/config.rs` (`PAPER_FILE_EXTENSIONS`), `src/lib/api.ts`, `src/App.svelte`, `src/lib/contract.test.ts`, `src/lib/fixtures/ipc_contract.json` (regenerated)

**Interfaces:**
- Produces: `pub struct GrantedFile { pub id: u64, pub name: String }` (Serialize; `name` is the file name for display); `pub struct FileGrants` with `pub fn replace(&self, paths: Vec<PathBuf>) -> Vec<GrantedFile>` (forgets every earlier grant) and `pub fn take(&self, id: u64) -> Option<PathBuf>`; ids from a counter, never reused in a launch. Commands `pick_file() -> Option<GrantedFile>` (`None` on cancel; filter *Papers* with `PAPER_FILE_EXTENSIONS`), `open_file(grant: u64)`, `attach_file(paper_id: String, grant: u64)`, both `-> OpenOutcome`; an unknown or used grant is `invalid_input` *This file is no longer available; choose or drop it again.* The drop handler emits `FILES_DROPPED_EVENT` with `replace(paths)` (paths ending `.studyanalyzer` go to Task 5's `receive_file` instead, once it exists). `api.ts`: `pickFile(): Promise<GrantedFile | null>` calls the command; `openFile(grant: number)`, `attachFile(paperId, grant)`; `onFilesDropped(cb: (files: GrantedFile[]) => void): Promise<() => void>` (`listen` from `@tauri-apps/api/event`) replaces `onDragDropEvent`.

- [ ] **Step 1: Write the failing tests** in `grants.rs`: `grant_is_good_once` (`take` gives the path once, then `None`); `pick_replaces_grants` (after a second `replace`, the first's ids give `None`); `unknown_grant_is_refused` (`open_file` through the mock runtime with id 999 → `invalid_input`, no read); `granted_name_is_file_name` (`/a/b/paper.pdf` → `"paper.pdf"`). In `contract.test.ts`: `open_file` and `attach_file` take `grant` (number), `pick_file` returns `GrantedFile | null`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer grants` and `pnpm run test:web`. Expected: FAIL (no `grants` module; contract mismatch).
- [ ] **Step 3: Implement.** `pick_file` runs `app.dialog().file().add_filter("Papers", &PAPER_FILE_EXTENSIONS).blocking_pick_file()` inside `blocking` (never on the main thread), then `FilePath::into_path()` (an error is `internal`, logged). The drop handler is `Builder::on_window_event` matching `WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. })` (Tauri 2.12 emits drops on a webview window as a window event, `manager/window.rs`); an emit failure is logged. `App.svelte`'s `onDrop` takes `GrantedFile[]`, keeps its one-at-a-time and screen rules, and passes `grant.id`. Remove `@tauri-apps/plugin-dialog` from `api.ts` and `dialog:default` from the capability (the Rust plugin stays registered). `UPDATE_CONTRACT=1 cargo test -p studyanalyzer contract`, then fix `api.ts`.
- [ ] **Step 4: Run** all StudyAnalyzer gates. Expected: PASS.
- [ ] **Step 5: Commit** `feat: file paths stay in Rust (grants for the dialog and drops)`.

### Task 2: The hand-off format

**Files:**
- Create: `crates/studyanalyzer-core/src/handoff.rs`, `crates/studyanalyzer-core/tests/handoff_contract.rs`, `crates/studyanalyzer-core/tests/fixtures/handoff/` (`cases.json` and the `.studyanalyzer` files it names), `docs/handoff-format.md`
- Modify: `config.rs` (the constants above), `identifiers.rs` (`check_handed`), `lib.rs`, `crates/studyanalyzer-core/Cargo.toml` (`base64`, `url`, at the versions `Cargo.lock` already holds)

**Interfaces:**
- Consumes: `Identifiers`, `IdentifierError`, `SourceKind` (2a).
- Produces:
  - `pub fn check_handed(pmid: Option<&str>, doi: Option<&str>, pmc_id: Option<&str>) -> (Identifiers, Vec<IdentifierError>)`, sharing `check_fetched`'s body (a private helper taking the PMID check).
  - `pub struct Handoff { pub sender: Option<String>, pub title: Option<String>, pub identifiers: Identifiers, pub notices: Vec<String>, pub document: Option<HandedDocument> }` and `pub struct HandedDocument { pub kind: SourceKind, pub file_name: Option<String>, pub source: Option<String>, pub bytes: Vec<u8> }`, both `Debug, Clone, PartialEq`. `notices` holds one sentence per dropped identifier.
  - `impl Handoff { pub fn origin(&self) -> String }`: *{sender}, from {source}*, *{sender}*, or `LINK_ORIGIN` (*a studyanalyzer:// link*) when there is no sender.
  - `pub enum HandoffError { TooLarge { len: usize, max: usize }, NotAHandoff, Newer { version: u64 }, Malformed(String), Nothing { refused: Vec<String> }, Link(String) }` with `Display` (a sentence the user can read; `Malformed` names the field) and `std::error::Error`.
  - `pub fn parse_handoff(bytes: &[u8]) -> Result<Handoff, HandoffError>`, `pub fn parse_link(link: &str) -> Result<Handoff, HandoffError>`.

- [ ] **Step 1: Write the failing tests** in `handoff.rs`:
  - `full_handoff_parses` (every field; `bytes` equal the decoded data; `kind` from `format`).
  - `ids_only_handoff_parses`, `ppr_pmid_is_kept`.
  - `invalid_id_dropped_with_notice` (bad PMID, valid DOI: the DOI kept, one notice naming the PMID).
  - `nothing_valid_is_refused` (`Nothing`, carrying the reasons).
  - `oversize_refused_before_parse` (`MAX_HANDOFF_BYTES + 1` bytes of `{` → `TooLarge`).
  - `not_json_is_malformed`, `other_format_is_not_a_handoff`, `newer_version_refused` (`version: 2` → `Newer { version: 2 }`, message names 2 and 1), `duplicate_key_is_malformed`, `unknown_field_ignored`.
  - `bad_base64_is_malformed`, `unknown_text_format_is_malformed`.
  - `path_in_file_name_is_malformed` (`../x.pdf`, `a\\b.pdf`, `a\u{0}.pdf`), `long_title_is_malformed` (`MAX_HANDOFF_TITLE_CHARS + 1`), `control_char_in_sender_is_malformed`, `empty_sender_is_malformed`.
  - `origin_names_sender_and_source`.
  - `link_parses` (all three ids, percent-encoded DOI `10.1000%2Fxyz` → `10.1000/xyz`), `link_wrong_scheme_or_host`, `link_repeated_key`, `link_empty_value`, `link_unknown_key_ignored`, `link_too_long`, `link_without_ids` (`Nothing`).

  In `identifiers.rs`: `check_handed_takes_ppr` (the PMID field keeps `PPR123`; `check_fetched` still refuses it).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core handoff`. Expected: FAIL (no module).
- [ ] **Step 3: Implement** with serde structs mirroring the JSON (`Option` for optional fields, no `deny_unknown_fields`); `version` is read before the rest, so a newer file with a changed shape still says `Newer`. A `pdf` document's magic is checked in Task 4 by the existing conversion, not here.
- [ ] **Step 4: The contract fixtures and the senders' document.** `cases.json` lists each fixture file with its expected outcome (`ok` with the identifiers, kind, sender and title it yields, or the `HandoffError` variant name). It covers a JATS hand-off, a PDF hand-off (2a's small test PDF), an identifiers-only one, a newer version and a path-bearing `file_name`. `tests/handoff_contract.rs` reads every case. `docs/handoff-format.md` states the format, the versioning rule, the caps, the link, and *where these fixtures live, for a sender's own tests*.
- [ ] **Step 5: Run** `cargo test -p studyanalyzer-core` and the Rust gates. Expected: PASS.
- [ ] **Step 6: Commit** `feat(core): the hand-off file and link format`.

### Task 3: Hand-offs in storage

**Files:**
- Create: `crates/studyanalyzer-core/src/handoff_store.rs`
- Modify: `schema.rs` (migration 8), `lib.rs`

**Interfaces:**
- Consumes: `Handoff`, `HandedDocument` (Task 2), `StoreError`, `Db`.
- Produces: migration 8, `CREATE TABLE handoffs (id INTEGER PRIMARY KEY, received_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, sender TEXT, title TEXT, pmid TEXT, doi TEXT, pmc_id TEXT, notices_json TEXT NOT NULL DEFAULT '[]', kind TEXT, file_name TEXT, source TEXT, bytes BLOB, refusal TEXT)`. `pub struct HandoffSummary { pub id: i64, pub received_at: String, pub sender: Option<String>, pub title: Option<String>, pub identifiers: Identifiers, pub has_full_text: bool, pub refusal: Option<String> }` (Serialize; `received_at` as SQLite writes it, read by the frontend's `parseSqliteUtc`). `pub fn save_handoff(db: &mut dyn Db, h: &Handoff) -> Result<i64, StoreError>`, `pub fn save_refused_handoff(db: &mut dyn Db, refusal: &str) -> Result<i64, StoreError>`, `pub fn pending_handoffs(db: &mut dyn Db) -> Result<Vec<HandoffSummary>, StoreError>` (newest first, by `id`), `pub fn load_handoff(db: &mut dyn Db, id: i64) -> Result<Option<Handoff>, StoreError>` (`None` for an unknown id or a refusal row), `pub fn delete_handoff(db: &mut dyn Db, id: i64) -> Result<bool, StoreError>`. `kind` is stored as `SourceKind`'s existing stored name (as `sources.kind`).

- [ ] **Step 1: Write the failing tests:** `saved_handoff_is_pending` (summary fields; `has_full_text`); `load_round_trips` (`load_handoff` equals what was saved, bytes included); `refused_handoff_is_listed` (a refusal row lists with `refusal` set and loads as `None`); `pending_newest_first`; `delete_handoff_removes_it` (`true`, then `false`); `migration_8_on_a_v7_database` (as the earlier migration tests do).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core handoff_store`. Expected: FAIL.
- [ ] **Step 3: Implement**, one statement per function, inside the caller's connection (no transaction needed for single statements).
- [ ] **Step 4: Run** the Rust gates. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): hand-offs stored on arrival`.

### Task 4: Opening a hand-off

**Files:**
- Modify: `crates/studyanalyzer-core/src/open.rs`, `src-tauri/src/error.rs`

**Interfaces:**
- Consumes: `Handoff` (Task 2); `convert`, `find_paper`, `place`, `persist`, `open_identifier` (2a).
- Produces: `OpenRequest::Handoff(Handoff)`; `OpenError::HandoffMismatch(String)` with `Display` *The document {sender} handed over is a different paper than the identifiers sent with it ({why}), so it was not stored.*; `CommandError` maps it to `invalid_input`. `attach` refuses a `Handoff` as it refuses an `Identifier`. `fn open_by_identifiers(ctx: &mut OpenContext, ids: Identifiers, title: Option<&str>, notes: Vec<String>) -> Result<OpenOutcome, OpenError>`, with `open_identifier` now `parse_identifier` then this.

- [ ] **Step 1: Write the failing tests** in `open.rs` (the fake source and PDF backend 2a's tests use):
  - `handoff_with_jats_opens` (Ready; the paper holds the hand-off's identifiers and the JATS's own; origin stored as `Handoff::origin()`).
  - `handoff_title_names_untitled_paper` (plain text without a title takes the hand-off's).
  - `same_handoff_twice_one_copy` (two opens → one source, same paper).
  - `handoff_reopens_paper_by_id` (a paper opened by PMID earlier gets the hand-off's text, §8).
  - `handoff_text_contradicts_its_ids` (hand-off PMID 1, JATS PMID 2 → `HandoffMismatch`, nothing stored).
  - `handoff_contradicts_stored_paper` (`Mismatch`, nothing stored).
  - `handoff_notices_shown` (a dropped identifier's notice is in `Ready.notices`).
  - `ids_only_handoff_is_lookup` (the fake source is asked with all three ids; the record-less title falls back to the hand-off's; its notices end a `NeedsFullText` reason).
  - `handoff_pdf_that_is_not_pdf` (`Intake(NotPdf)`).
  - `attach_refuses_handoff`.
  
  Existing `open_identifier` tests pass unchanged.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core open`. Expected: the new tests FAIL.
- [ ] **Step 3: Implement.** In `open`, a `Handoff` with a document is converted by `convert(ctx, kind, &origin, file_name, title, bytes, &PdfLimit::default())`, its `ids` merged with the claim (a differing field → `HandoffMismatch`), its notices prefixed with the hand-off's, then the rest of `open`'s file path (with no identifiers at all, it finds its paper by its bytes through `holder_of_source`, as a file does). A `Handoff` without one goes to `open_by_identifiers`. A `pdf` kind is checked with `detect_kind(Some("x.pdf"), &bytes)` before converting, so a non-PDF says `NotPdf`.
- [ ] **Step 4: Run** the Rust gates. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): open a hand-off`.

### Task 5: Receiving: single instance, the link, opened files, commands

**Files:**
- Create: `src-tauri/src/handoff.rs`
- Modify: `src-tauri/Cargo.toml` (`tauri-plugin-single-instance = { version = "2", features = ["deep-link"] }`, `tauri-plugin-deep-link = "2"`), `lib.rs` (plugins first, `setup`, `RunEvent::Opened` in `app.run`, the drop handler from Task 1 sending `.studyanalyzer` paths to `receive_file`, `handler()`), `commands.rs`, `contract.rs`, `tauri.conf.json` (`bundle.fileAssociations`, `plugins.deep-link`), `src/lib/api.ts`, `src/lib/contract.test.ts`, the contract fixture

**Interfaces:**
- Consumes: `parse_handoff`, `parse_link` (Task 2), `save_handoff`, `save_refused_handoff`, `pending_handoffs`, `load_handoff`, `delete_handoff` (Task 3), `OpenRequest::Handoff` (Task 4), `read_chosen_file`'s capped read (2a).
- Produces: `pub fn receive_file<R: Runtime>(app: &AppHandle<R>, path: PathBuf)`, `pub fn receive_link<R: Runtime>(app: &AppHandle<R>, link: &str)`, `pub fn receive_args<R: Runtime>(app: &AppHandle<R>, args: Vec<String>)` (skips the program name, `studyanalyzer:` arguments, and anything not ending `.studyanalyzer`, each skip logged), and the pure `pub fn receive(db: &mut dyn Db, read: Result<Vec<u8>, String>) -> Result<i64, StoreError>` they share (parse, then `save_handoff` or `save_refused_handoff`). Each spawns its work with `tauri::async_runtime::spawn_blocking`, then emits `HANDOFF_EVENT` (a failure logged). Commands: `pending_handoffs() -> Vec<HandoffSummary>`, `open_handoff(id: i64) -> OpenOutcome` (registers a lookup like `open_identifier`, so *Cancel* works; needs the contact email only when the hand-off has no full text; deletes the row per the rule above), `discard_handoff(id: i64) -> ()`. `api.ts`: `pendingHandoffs`, `openHandoff`, `discardHandoff`, `onHandoff(cb: () => void): Promise<() => void>`.

- [ ] **Step 1: Write the failing tests:** in `handoff.rs`: `receive_stores_a_valid_file`, `receive_stores_a_refusal` (bad JSON → a refusal row, its text the `HandoffError`'s), `receive_stores_a_read_failure` (`Err("permission denied")` → a refusal row naming it), `args_skip_non_handoffs` (a pure helper `handoff_paths(args: &[String]) -> Vec<PathBuf>`: `["app", "studyanalyzer://open?pmid=1", "/x/a.pdf", "/x/b.studyanalyzer"]` → `["/x/b.studyanalyzer"]`). In `commands.rs` through the mock runtime: `open_handoff_deletes_on_outcome`, `open_handoff_keeps_on_needs_email`, `discard_removes`. In `contract.test.ts`: the three commands' arguments and returns, `HandoffSummary`'s keys, both event names.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer handoff` and `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement.** Read a file with the existing capped reader at `MAX_HANDOFF_BYTES` (one byte past is `TooLarge`, recorded as a refusal). In `setup`: `app.deep_link().on_open_url(…)` calling `receive_link` for each URL, `get_current()` for launch links, `receive_args(std::env::args().collect())` on Linux and Windows, and `register_all()` under `#[cfg(any(target_os = "linux", all(debug_assertions, windows)))]`. `app.run`'s handler gains `RunEvent::Opened { urls }`: `file:` URLs through `Url::to_file_path`, then `receive_file`. The single-instance callback calls `receive_args` and focuses `main`. Log one line per received hand-off (its source: link, opened file, argument, drop) so a double delivery would show in the log.
- [ ] **Step 4: Run** all StudyAnalyzer gates and `pnpm tauri build --debug` once (the bundle config must validate). Expected: PASS; the built `Info.plist` holds `CFBundleDocumentTypes`, `UTExportedTypeDeclarations` and `CFBundleURLTypes` for `studyanalyzer`.
- [ ] **Step 5: Commit** `feat: receive hand-off files and links (single instance)`.

### Task 6: The frontend

**Files:**
- Create: `src/lib/handoff.ts`, `src/lib/handoff.test.ts`
- Modify: `src/App.svelte`, `src/components/StartScreen.svelte`

**Interfaces:**
- Consumes: Task 5's `api.ts` functions; App's `busy`, `interrogatorBusy`, `run`, `land`, the email flow.
- Produces: `export function nextHandoff(pending: HandoffSummary[], tried: ReadonlySet<number>, busy: boolean): HandoffSummary | null` (the newest not tried, or `null` while busy); `export function handoffCaption(h: HandoffSummary): string` (title, else the first identifier, else *untitled*; then *from {sender}* when known; then the received time through `parseSqliteUtc` and `CREATED_AT_FORMAT`); `export function handoffBusyText(h: HandoffSummary): string` (`LOOKING` when there is no full text, else the opening text App already shows).

- [ ] **Step 1: Write the failing tests:** `nextHandoff_waits_while_busy`, `nextHandoff_newest_untried` (three pending, the newest tried → the second), `nextHandoff_none`, `nextHandoff_refusal_counts` (a refusal row is returned too: App shows it and discards it), `handoffCaption_falls_back`, `handoffBusyText_lookup_vs_file`.
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement** the functions, then wire `App.svelte`. Load `pendingHandoffs()` at mount and on `onHandoff`. An `$effect` on `pending`, `busy` and `interrogatorBusy` takes `nextHandoff`, marks it tried, and either shows its `refusal` in the error banner and discards it, or `land(await run(() => api.openHandoff(id)))` with `handoffBusyText`, then reloads the list. `needs_email` keeps the existing email flow; its pending re-run becomes a function (`pendingRetry`), so the save re-runs either a lookup or the hand-off. `StartScreen` lists the other pending hand-offs under *Handed over*, each with *Open* and *Discard* (disabled while busy).
- [ ] **Step 4: Run** all StudyAnalyzer gates. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): open hand-offs when idle; the Handed over list`.

### Task 7: Check by hand and hand over

**Files:**
- Modify: `HANDOVER.md`, `ROADMAP.md`, `CLAUDE.md` (*State of the repository*), `README.md` (the hand-off, linking `docs/handoff-format.md`)

- [ ] **Step 1: Check it by hand** with a built, sandboxed app in `/Applications` (deep links only work there on macOS; owed to the maintainer if no one can click the window):
  - Choosing and dropping a file still opens and attaches.
  - A `.studyanalyzer` file from Task 2's fixtures, double-clicked with the app closed and again with it open (one window), dropped on the window, and sent by `open -a StudyAnalyzer file.studyanalyzer`.
  - `open "studyanalyzer://open?pmid=…"` with the app closed and open.
  - A hand-off while an answer streams, and while a PICO run goes: it waits, then opens.
  - Two hand-offs in a row: the newest opens, the other is listed.
  - A refused file on a cold start: the reason is shown.
  - The identifiers-only link with no contact email: the email screen, then the lookup.
  - Quit with a hand-off pending, reopen: it opens.
  - Finder shows the file type's name; dark mode.
- [ ] **Step 2: Run** every gate. Expected: PASS.
- [ ] **Step 3: Update the docs:**
  - 2d landed, and phase 2 is done.
  - What phase 3 builds on: the format document, the fixtures, the macOS route.
  - #65 unreachable from the UI, still open for the maintainer's call.
  - The click-through owed.
  - Anything found, filed as issues.
- [ ] **Step 4: Commit** `docs: phase 2d landed`, push, open the PR.

### Changed while building

Record here every point the build settles differently from this plan, with
the commit.
