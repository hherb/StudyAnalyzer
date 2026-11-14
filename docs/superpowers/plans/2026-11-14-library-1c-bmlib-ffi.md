# Shared paper library, step 1c: the library through bmlib-ffi — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Swift and Kotlin can open a library, scan it, find a paper or a
file, read a file with its digest checked, save, record a use, release, and
remove a file or a folder, all through bmlib's one Rust implementation.
MedicalFactChecker (step 3) builds on this.

**Architecture:** One new module, `rust/bmlib-ffi/src/library.rs`. It holds
owned FFI copies of the library's types, a `Library` object over
`bmlib::library::Library`, a `LibraryScan` object over `bmlib::library::Scan`,
and a few free functions. Values coming in from the app go through bmlib's
validating constructors. Values going out are copied field for field, with
exhaustive destructuring, so a new bmlib field fails to compile here.

**Tech Stack:** Rust, UniFFI `=0.32.2` (the crate's pin). No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(§5.1, §5.3, §6 approach A). The library API as built is bmlib's
`rust/bmlib/docs/manual/library.md` (PRs #754 and #763).

**As built (bmlib branch `feat/library-ffi`).** Where this plan and the PR
differ, the PR and bmlib-ffi's README (*The shared paper library*) are
authoritative:

- `folder_name`, `folder_name_numbered` and `sha256_hex` are **not
  exported**. `save` names the folder and hashes the bytes, so no app needs
  them. Task 1's free functions are `paper_ids_canonical` and
  `installation_new` only.
- **Swift needs one line from the app:**
  `extension LibraryError: CustomStringConvertible {}`. UniFFI 0.32.2
  generates `description` (the exported `Display`) for an error, but does
  not declare the conformance (it does for objects). Without the line,
  `errorDescription` prints the enum's structure. A compiled Swift check
  showed both forms. Kotlin's `toString()` is bmlib's text as generated.
- Task 2 Step 5 went further than `swiftc -parse`. The generated Swift
  typechecked against its header and module map. A small program linked to
  the host dylib caught `Library.open(root:)`'s `Unreachable` and printed it
  in bmlib's wording.
- All 20 `open` cases run through the FFI. None needed special files.
- Error fields are also compared in their `Debug` form, so a field swapped
  in both conversions cannot cancel out (Review Focus 2).

## Decided for this plan (2026-11-14)

- **Only the `library` module is exposed, not `FullTextService`'s use of
  it.** MedicalFactChecker fetches through its own Swift chain (design
  §5.3), and bmlib-ffi's `BmlibFullTextService` is a sketch behind the
  `http-sketch` feature, in no binary the apps ship (#492). So
  `with_library`, `library_files` and `library_failure` are not converted.
  That ends the item in StudyAnalyzer's HANDOVER ("1c must convert
  `with_library`…"). They are converted when the transport is designed.
- **`scan()` returns an object.** `LibraryScan` holds bmlib's `Scan` in
  Rust. It answers `folders()`, `problems()`, `find(ids)`,
  `find_digest(sha256)`, `group()` and `used_by(folder, sha256)`. This way
  a 4,000-paper scan is not copied across the boundary for every find. The
  scan is a snapshot: a save does not change it.
- **What comes in is validated.** `PaperIds` from the app goes through
  `PaperIds::canonical`, so raw identifiers are accepted and come back
  canonical. `Installation` goes through `Installation::new`. A refused
  value is the same `LibraryError` (`BadIdentifier`, `BadInstallation`)
  that bmlib returns. `paper_ids_canonical` and `installation_new` are also
  exported, so an app can check values before it stores them.
- **`PaperFolder` carries its merged `ids`.** This is `PaperFolder::ids()`
  computed once on the way out, so the app does not merge records itself.
  `used_by` is a `LibraryScan` method for the same reason: one
  implementation.
- **Errors are typed and keep bmlib's wording.** `LibraryError` is a
  `uniffi::Error` enum with one case per bmlib variant. Paths become
  strings (`Path::display`) and `&'static str` fields become `String`. It
  exports `Display`, which is bmlib's own text (the FFI error converts
  back for formatting). Swift's `errorDescription` and Kotlin's `message`
  then read as bmlib's. Field names avoid `message` (Kotlin).
- **Paths are strings.** Swift resolves the security-scoped bookmark and
  passes the path (design §6, approach A). A path that is not UTF-8 cannot
  reach Rust from Swift or Kotlin, so nothing is lost.
- **Integers keep bmlib's widths** where UniFFI has them: `year` `i32`,
  `size` `u64`, record `version` `u32`. Indices (`group`) are `u64`, since
  UniFFI has no `usize`; the conversion is lossless. Unsigned types in
  Kotlin are #496's open question, as for `parse_crossref_retraction_response`.
- **Calls block.** Every `Library` method does disk I/O on the calling
  thread. The README says to call them off the main thread. Nothing here is
  async.
- **`parse_record`, `render_record` and the layout constants are not
  exported.** Records are the library's business. An app never writes or
  reads one directly.

## Global Constraints

- Work in a bmlib worktree (`~/src/bmlib-library-ffi`, branch
  `feat/library-ffi` from `origin/main` at `02c7fad0`). bmlib's
  `HANDOVER.md` and `docs/SESSION-RULES.md` apply.
- Rust only: the FFI has no Python counterpart. The shared fixture is
  `tests/fixtures/library_cases.json`, the one Rust and Python already run.
  The FFI tests run its sections through the FFI's own functions, so the
  apps' suites (BioMedLit, step 3) can do the same.
- Nothing is deleted except by `remove_file` and `remove_folder`, which are
  bmlib's own calls (design §2).
- Every public item has a doc comment. Error fields are never named
  `message`. The crate's existing exports and the bindings' names are
  unchanged.
- Gates: CLAUDE.md § Gates for bmlib (Rust and Python), and both CI clippy
  lines for the crate (`-p bmlib-ffi --all-targets`, and with
  `--features http-sketch,bindgen`). Tests run as `cargo test -p bmlib-ffi
  --features http-sketch`.

## Review Focus

1. **A value that skips validation.** Every `TryFrom` from an FFI record
   into bmlib must call `PaperIds::canonical` or `Installation::new`.
   Task 1 tests a refused identifier and installation on `save`.
2. **A swapped field.** `folder`/`name`, `expected`/`found` and
   `sha256`/`name` are all `String`, so a swap compiles. The tests compare
   every field against bmlib's value.
3. **The error text.** `Display` on the FFI error must equal bmlib's for
   every variant. Task 1 tests each variant.

## Tasks

### Task 1: Types, conversions and free functions

**Files:**
- Create: `rust/bmlib-ffi/src/library.rs`, `rust/bmlib-ffi/src/library_tests.rs`
- Modify: `rust/bmlib-ffi/src/lib.rs` (`mod library;`, `pub use`)

The types: `SourceKind` (enum), `PaperIds`, `PaperInfo`, `Installation`,
`FileRef`, `FileEntry`, `LibraryRecord`, `PaperFolder` (records), the
`LibraryProblem` enum (one case per `scan::Problem`), and `LibraryError`.
The free functions: `paper_ids_canonical`, `installation_new`,
`folder_name`, `folder_name_numbered` and `sha256_hex`.

- [ ] **Step 1: Write the failing tests.** Run the fixture's
  `canonical_ids`, `installations` and `folder_names` sections through the
  free functions. Build every `LibraryError` variant and check that its
  conversion keeps every field and that `to_string()` is bmlib's.
- [ ] **Step 2: Run them.** Expected: they fail to compile.
- [ ] **Step 3: Implement** the types and the conversions both ways.
- [ ] **Step 4: Run them.** Expected: PASS.
- [ ] **Step 5: Commit** `ffi(library): the library's types and free functions`.

### Task 2: `Library` and `LibraryScan`

**Files:** the same two.

`Library::open(root)` is a constructor. Its methods are `root`, `scan`,
`read`, `save`, `record_use`, `release`, `remove_file` and
`remove_folder`. The `LibraryScan` methods are as decided above.

- [ ] **Step 1: Write the failing tests**, on a copy of
  `tests/fixtures/library/sample/`:
  - The fixture's `scan`, `find`, `find_digest`, `group` and `read`
    sections, through the objects.
  - The fixture's `open` sections that need no special files.
  - A round trip in an empty library: `save`, then `scan`
    (`find_digest` finds it), `read`, `record_use` by a second
    installation, `used_by` (both), `release`, `used_by` (one),
    `remove_file`, and `read` (`Missing`).
  - A `save` with a bad DOI and one with a bad installation. Both refuse
    with nothing written.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the crate's whole suite.** Expected: PASS.
- [ ] **Step 5: Generate the Swift and Kotlin bindings** from the host
  build (README, *Bindings from the host build*). Check that they
  generate, and that `swiftc -parse` accepts the Swift file.
- [ ] **Step 6: Commit** `ffi(library): Library and LibraryScan objects`.

### Task 3: Documentation, gates, PR

**Files:**
- Modify: `rust/bmlib-ffi/README.md` (the *What it exposes* table and a
  *Library* section: blocking calls, validation, the scan snapshot, error
  text), `CHANGELOG.md` (Unreleased), `HANDOVER.md` (1c done; step 2 and
  step 3 next, with the conversion item closed), `docs/DECISIONS.md` (the
  bullets of *Decided for this plan*)

- [ ] **Step 1: Write the docs.**
- [ ] **Step 2: Run every gate**, with `set -o pipefail`. Expected: all
  green.
- [ ] **Step 3: Commit, push, and open the PR** to bmlib `main`, naming this
  plan.
- [ ] **Step 4: Update StudyAnalyzer's HANDOVER and ROADMAP** (1c's PR, its
  *As built* differences in this plan).
