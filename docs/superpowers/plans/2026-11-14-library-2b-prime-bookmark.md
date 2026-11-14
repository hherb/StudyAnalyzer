# Shared paper library, step 2b′: the macOS bookmark — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sandboxed macOS build keeps its library folder across launches:
choosing the folder stores a security-scoped bookmark, launch resolves it and
starts access, and exit stops it.

**Architecture:** A new core module, `crates/studyanalyzer-core/src/bookmark.rs`,
wraps `NSURL`'s bookmark calls (macOS; elsewhere it is never called). The
setting stores the bookmark beside the path (`library_setting::choose` writes
it in the same transaction; `clear` removes it). At launch,
`library_setting::resume` resolves it, follows a moved folder (the
maintainer's call, 2026-11-14), renews a stale bookmark and returns the
access, which `AppState` holds until `RunEvent::Exit`. When resolution fails,
`library::configured` adds the reason to its `Unreachable` message.

**Tech Stack:** Rust, `objc2-foundation` 0.3 (MIT/Apache, already in the
tree through Tauri; a direct macOS-only dependency now). bmlib stays at
`02c7fad0`.

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(§4.1, §4.6, §5.2).

## Global Constraints

- Entitlements: `com.apple.security.files.user-selected.read-write` (was
  read-only) and `com.apple.security.files.bookmarks.app-scope`.
- "access started at launch, stopped at exit" (§4.1).
- "a bookmark that no longer resolves … the app shows it, in the setting and
  as a banner while it lasts" (§4.6): this is the existing `Unreachable`
  status line, with the reason.
- The path never reaches the webview; the bookmark doesn't either.
- Linux, Windows and phones keep the path alone (the bookmark is macOS-only code).

## Out of scope

- bmlib #769 (dataless files): option 3 was chosen (the maintainer's call,
  2026-11-14). It is its own bmlib PR, in Rust and Python.
- Following a rename while the app runs: a rename reads as unreachable until
  the next launch.

## Review Focus

- A library chosen before 2b′ (path, no bookmark): at launch, `resume`
  makes the bookmark from the path while it can still be reached.
- A folder renamed or moved between launches: the app opens it under its new
  path, and `library_opened` is remembered for that path.
- A folder deleted between launches: resolution fails, and the status line
  says why.
- A stale bookmark (the volume was remounted): it is made again and stored.
- A failed bookmark creation in `choose`: nothing changes (refused before
  any copy).

---

### Task 1: `bookmark.rs`

**Files:**
- Create: `crates/studyanalyzer-core/src/bookmark.rs`
- Modify: `crates/studyanalyzer-core/src/lib.rs` (`pub mod bookmark;`),
  `crates/studyanalyzer-core/Cargo.toml` (`[target.'cfg(target_os = "macos")'.dependencies]
  objc2-foundation = { version = "0.3", default-features = false, features = ["std", "NSURL", "NSData", "NSError", "NSString", "NSArray"] }`, plus `objc2 = "0.6"` if `Retained` needs it).

**Interfaces (produces):**
- `pub fn create(dir: &Path) -> Result<Vec<u8>, String>`: bookmark data
  with `NSURLBookmarkCreationOptions::WithSecurityScope`.
- `pub struct Access`: holds the resolved `Retained<NSURL>` on macOS (empty
  elsewhere). `Drop` calls `stopAccessingSecurityScopedResource` once if
  access started. `Send + Sync`.
- `pub struct Resolved { pub path: PathBuf, pub stale: bool, pub access: Access }`
- `pub fn resolve(data: &[u8]) -> Result<Resolved, String>`: options
  `WithSecurityScope | WithoutUI`, then `startAccessingSecurityScopedResource`.
  A `false` start is logged (`warn`), not an error: an unsandboxed process
  needs no grant.
- Off macOS, both functions return `Err("bookmarks are macOS-only")`.
  Callers cfg-gate them.
- `String` errors carry `NSError`'s `localizedDescription`.

- [x] **Step 1: tests** (`#[cfg(all(test, target_os = "macos"))]`):
  `a_bookmark_resolves_to_its_folder` (create on a tempdir, resolve, compare
  `canonicalize` of both: `/var` vs `/private/var`; `!stale`);
  `a_bookmark_follows_a_renamed_folder`; `garbage_is_an_error` (`b"nope"`);
  `a_deleted_folder_does_not_resolve`.
- [x] **Step 2:** run `cargo test -p studyanalyzer-core bookmark`: FAIL.
- [x] **Step 3:** implement.
- [x] **Step 4:** the same run passes. If unsandboxed creation with security
  scope fails, stop and report: the design's route needs rethinking.
- [x] **Step 5:** commit.

### Task 2: the setting stores the bookmark; `resume` at launch

**Files:**
- Modify: `crates/studyanalyzer-core/src/config.rs`: `LIBRARY_BOOKMARK_KEY = "library_bookmark"`
  (base64, standard alphabet, the `base64` crate already in core),
  `LIBRARY_BOOKMARK_FAULT_KEY = "library_bookmark_fault"`.
- Modify: `crates/studyanalyzer-core/src/library_setting.rs` (`choose`,
  `clear`, new `resume`), `crates/studyanalyzer-core/src/library.rs`
  (`configured`).

**Interfaces (produces):**
- `choose`: on macOS, after `Library::open` validates the folder and before
  anything changes, `bookmark::create(root)`; a failure is
  `SettingError::Refused("the folder … cannot be remembered: {e}")`. The
  transaction that sets `LIBRARY_PATH_KEY` sets `LIBRARY_BOOKMARK_KEY` and
  deletes `LIBRARY_BOOKMARK_FAULT_KEY`.
- `clear`: its transaction deletes both keys too.
- `pub enum Resumed { Nothing, Held(bookmark::Access), Lost(String) }`
- `pub fn resume(db: &mut dyn Db) -> Result<Resumed, StoreError>`:
  - Off macOS, or with no library set: `Nothing`.
  - A path but no bookmark: create one from the path. Success stores it and
    resolves it (`Held`). Failure is `Lost`.
  - Resolved: if its path is not the stored one, set `LIBRARY_PATH_KEY` to it
    and log `info` naming both. If `stale`, create the bookmark again from the
    resolved path and store it; a failure there is only a `warn`. Delete the
    fault key. Return `Held`.
  - Resolution fails: store the reason in `LIBRARY_BOOKMARK_FAULT_KEY`, log
    `warn`, return `Lost(reason)`.
- `configured`: when it returns `Unreachable(why)` and the fault key is set,
  `why` ends with `"; its bookmark did not resolve at launch: {fault}"`.

- [x] **Step 1: tests** (macOS-gated, in `library_setting.rs`):
  `choose_stores_a_bookmark` (key set, decodes, resolves to the folder);
  `clear_removes_the_bookmark`; `resume_without_a_library_is_nothing`;
  `resume_makes_a_bookmark_for_a_path_set_before`;
  `resume_follows_a_renamed_library` (choose, rename the folder,
  `resume` → `Held`, `LIBRARY_PATH_KEY` is the new path, and `configured` is
  `Ready`); `resume_of_a_deleted_folder_is_lost` (→ `Lost`, the fault key
  set, `configured`'s `Unreachable` message contains "did not resolve at
  launch"); `choose_clears_a_fault`. An existing test that sets
  `LIBRARY_PATH_KEY` by hand keeps working with no bookmark.
- [x] **Step 2:** run `cargo test -p studyanalyzer-core library`: the new ones FAIL.
- [x] **Step 3:** implement.
- [x] **Step 4:** run the whole core suite; it passes.
- [x] **Step 5:** commit.

### Task 3: hold the access in the app; entitlements; docs

**Files:**
- Modify: `src-tauri/src/state.rs`: `pub library_access: Mutex<Option<bookmark::Access>>`
  (and the test `AppState`).
- Modify: `src-tauri/src/lib.rs`: `setup_state` runs `resume` on a fresh
  connection after `open_store`. `Held(a)` → `Some(a)`. `Lost(why)` is
  logged (`warn`); the status line reports it through `configured`. A
  `StoreError` is logged (`error`) and launch goes on. `RunEvent::Exit`
  takes and drops the access, beside `unload_embedder` (rename that function
  to `on_exit`, or add a sibling).
- Modify: `src-tauri/Entitlements.plist` (Global Constraints).
- Modify: `HANDOVER.md`, `ROADMAP.md`. Tick this plan's tasks and add an
  *As built* note.

- [x] **Step 1:** implement. This is wiring, with no new branch logic; the
  core tests cover the decisions.
- [x] **Step 2:** run every StudyAnalyzer gate (CLAUDE.md § Gates); all pass.
- [x] **Step 3:** `pnpm tauri build` (signed or ad hoc). With
  `codesign -d --entitlements - <app>`, check that both entitlements are in
  the bundle.
- [x] **Step 4:** commit, push, open the PR.

**Owed by the maintainer (click-through, sandboxed build in `/Applications`):**
choose a folder, quit, relaunch: the library is reachable and a PDF's
Original tab opens. Rename the folder while quit, relaunch: it is followed.
Delete it while quit, relaunch: the status line says the bookmark did not
resolve. Choose another folder: the status line goes.

## As built

- As planned. `objc2` is a direct macOS dependency beside
  `objc2-foundation` (for `Retained` and `Bool`); both were in the tree.
- `resume`'s `moved` compares canonical paths too, so `/var` and
  `/private/var` are one folder.
- The stale branch has no unit test: macOS reports a bookmark stale only
  after a remount, which a test cannot cause.
- `unload_embedder` became `on_exit`, which also drops the access.
