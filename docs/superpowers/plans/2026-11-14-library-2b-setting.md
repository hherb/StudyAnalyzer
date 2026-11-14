# Shared paper library, step 2b: the Library setting — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user chooses the library folder in Settings, moves the papers
the app keeps into it, switches to another folder or to none (bringing this
installation's files along if asked), and sees which sources still wait. An
unreachable library shows as a status line while it lasts. Waiting sources
move in when the app becomes active, and the app says how many moved.

**Architecture:** A new core module, `crates/studyanalyzer-core/src/library_setting.rs`,
holds the status, the choice, the clear and the move. Each change is a
setting change plus `library::settle` over rows that `store.rs` re-marks.
`library.rs` (959 lines) does not grow. `src-tauri` gains four commands,
one event, and a lock so that one library change runs at a time. The
frontend gains `LibrarySetting.svelte` inside Settings, `src/lib/library.ts`
(its pure wording, tested), and the status line in `App.svelte`.

**Tech Stack:** Rust, Tauri 2 (`tauri-plugin-dialog`'s `blocking_pick_folder`),
Svelte 5. bmlib stays at `02c7fad0`. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(§4.1, §4.6, §4.7, §5.2).

## Step 2 now in four PRs (the maintainer's call, 2026-11-14)

| PR | Scope |
|---|---|
| 2a | Sources by digest (PR #116, merged). |
| **2b** (this plan) | The setting with the path stored as today (dev, Linux, Windows; macOS unsandboxed); the move §4.7; switching to a folder or none; the waiting list; the unreachable status line; settling on activation. |
| 2b′ | The macOS security-scoped bookmark and entitlements; what bmlib #769 decides about dataless files. Until it lands, a **sandboxed** build cannot reach the folder after a relaunch. |
| 2c | The manual clear (§4.8). |

**The dataless device check is done** (2026-11-14, macOS 27.2, iCloud
Drive; filed as **bmlib #769**). A plain read of an evicted file under its
real name downloads it: 3 MB in 1.1 s online, correct bytes. `st_flags`
carries `SF_DATALESS`, and with materialisation off
(`setiopolicy_np`) the read fails at once with `EDEADLK`. Offline was not
tested.

## Decided for this plan (builder's calls, within the design)

- **The webview never holds the folder's path.** `choose_library` shows
  the dialog in Rust and applies the choice in the same command, as
  `pick_file` does. Whether to bring papers along is asked before the dialog,
  as a checkbox beside *Choose…*, unchecked by default (§4.7: "never
  automatic"). The path is shown in Settings as a label only.
- **"Bring them along" means one thing per case:**
  - From **no library** to a folder: every `app` row becomes `waiting`
    and is settled in (§4.7's move).
  - From **library A** to B: for each paper, its `library` rows are read
    from A (digest checked), kept as `waiting` with their bytes, and settled
    into B, one paper at a time, so the database never holds the whole
    library. The setting changes only after the copy, so a crash midway
    leaves the old setting and rows that are still readable from A.
  - To **no library**: `library` rows are read from the old library and kept
    as `app`.
  - Declined: `library` rows stay as they are and read as unavailable where
    the new place lacks them (§4.7). A failed copy leaves the row as it was
    and names the paper.
- **Waiting rows follow the setting.** On a switch to B they settle into B.
  On a switch to none they become `app`, their note cleared (HANDOVER's
  *Left for 2b*).
- **The folder is checked by `Library::open`** before anything changes: it
  makes an empty folder a library, uses a library and refuses anything else
  with bmlib's sentence (`invalid_input`). `LIBRARY_OPENED_KEY` is set to the
  new path at once.
- **One library change at a time.** `AppState.library_op: Mutex<()>`.
  `choose_library`, `clear_library` and `move_into_library` lock it.
  Settling on activation uses `try_lock` and skips a turn if it is busy.
  Opens do not take it (a shortcut: an open during a switch may settle into
  the old library; the Settings screen holds hand-offs back meanwhile).
- **Settling on activation**: `WindowEvent::Focused(true)` runs the
  launch's settle off the main thread when a waiting row exists (a cheap
  count first). When something moved or is left, it emits
  `library-settled` (`{moved, left}`). App shows *N files moved into the
  paper library.* as a status line, which goes on the next action. The
  launch's settle emits the same event.
- **The unreachable status line** lasts while `library_status().unreachable`
  is set. App reads the status at mount, on `window` focus and on leaving
  Settings, and shows *The paper library cannot be reached: ‹why›. Papers
  open from what the app keeps, and new files wait in the app until it is
  back.* It cannot be dismissed (it is a state, not an event).
- **The waiting list** is in the setting: title, kind and why, one row per
  waiting source.
- **No progress or Cancel for a move or switch** (a shortcut: the button
  reads *Moving…*; a large library takes as long as copying its files).

## Global Constraints

- The webview never passes a path (CLAUDE.md; plan 2d's grants).
- Nothing is deleted from any library folder by 2b; the old folder is never
  changed by a switch (§4.7).
- Every failure is logged and shown, naming the paper (§4.6, §4.7).
- A new command goes in `lib.rs`'s `handler()`, `contract.rs`'s
  `commands()`, `api.ts` and the vitest call list together; a new shape is
  a key in `contract()` plus `keysMatch`/`valuesMatch` rows
  (`UPDATE_CONTRACT=1 cargo test -p studyanalyzer contract`).

## Review Focus

1. A chosen folder that holds the user's own files is refused, and nothing
   changes (setting, rows) → test in Task 2.
2. A switch whose old library is unreachable while bringing files along
   changes the setting and reports every paper it could not copy. It never
   claims to have copied them → test in Task 2.
3. A switch to the folder already set does nothing harmful: no rows
   re-marked, nothing copied twice → test in Task 2.
4. Activation while a switch runs does not settle into the old library
   (`try_lock`) → test in Task 3 (the lock helper).
5. Going to none with bring-along, where one file is missing from the old
   library: the others become `app`, and the missing one stays `library`
   and is named → test in Task 2.

---

### Task 1: store helpers

**Files:** `crates/studyanalyzer-core/src/store.rs`.

**Produces:**
- `count_by_place(db) -> Result<PlaceCounts, StoreError>`, where
  `PlaceCounts { app: usize, waiting: usize, library: usize }`.
- `papers_with_place(db, place: SourcePlace) -> Result<Vec<PaperId>, StoreError>`.
- `sources_with_place(db, paper: &PaperId, place) -> Result<Vec<StoredSource>, StoreError>`.
- `app_to_waiting(db) -> Result<usize, StoreError>`: every `app` row
  becomes `waiting`.
- `waiting_to_app(db) -> Result<usize, StoreError>`: every `waiting` row
  becomes `app`, its note cleared.
- `bring_back(db, id: i64, bytes: &[u8], place: SourcePlace) -> Result<bool, StoreError>`:
  from `library` only; inserts the `source_bytes` row and sets the place
  (`Waiting` or `App`). False when the row was not `library`. A digest that
  does not match the bytes is `Corrupt`.

- [x] Tests: `place_counts_and_moves` (two app, one waiting; `app_to_waiting`
  → 3 waiting; `waiting_to_app` → 3 app, notes `None`);
  `bring_back_only_from_library` (a library row gets its bytes back and
  `source_bytes` returns them; an `app` row returns false; wrong bytes are
  `Corrupt`). Run `cargo test -p studyanalyzer-core store::`.
- [x] Implement; tests pass; commit.

### Task 2: `library_setting.rs`

**Files:** create `crates/studyanalyzer-core/src/library_setting.rs`; add
`pub mod library_setting;` to `lib.rs`.

**Consumes:** Task 1; `library::{configured, settle, read_source, Configured,
LibraryAt, Settled}`; `config::{LIBRARY_PATH_KEY, LIBRARY_OPENED_KEY}`.

**Produces:**
- `status(db, label: &str) -> Result<LibraryStatus, StoreError>`, where
  `LibraryStatus { path: Option<String>, unreachable: Option<String>,
  counts: PlaceCounts, waiting: Vec<WaitingSource> }` and
  `WaitingSource { paper_id: String, title: String, kind: SourceKind,
  note: Option<String> }`.
- `choose(db, label, root: &Path, bring: bool) -> Result<Changed, SettingError>`.
- `clear(db, label, bring: bool) -> Result<Changed, SettingError>`.
- `move_in(db, label) -> Result<Changed, SettingError>`: §4.7 run later
  (the library must be `Ready`, else `SettingError::Unreachable`).
- `Changed { copied: usize, moved: usize, failed: Vec<(PaperId, String)> }`:
  files read back from the old library, sources moved into the new one, and
  why each paper could not be copied or moved (`settle`'s `left` folded in).
- `SettingError { Store(StoreError), Refused(String) /* the folder */,
  Unreachable(String) }`.
- `installation` in `library.rs` becomes `pub(crate)`, so `choose` can make
  the new `LibraryAt` before the setting changes.

Order inside `choose`: `Library::open(root)` (refused → `Refused`, nothing
changed). Then, by the old `configured`:
- `None` and `bring`: `app_to_waiting`.
- `Ready(old)`, `bring` and a different root: for each of
  `papers_with_place(Library)`, `read_source` from `old` → `bring_back(…,
  Waiting)`, then `settle(db, &new, Some(paper))`.
- `Unreachable(why)` and `bring`: each paper with `library` rows goes into
  `failed` with `why`.

Then set `LIBRARY_PATH_KEY` and `LIBRARY_OPENED_KEY` to the root, and
`settle(db, &new, None)`. A root equal to the stored path skips the copy.

Order inside `clear`: with `bring` and the old library `Ready`, every
`library` row is read back as `app` (`bring_back(…, App)`); `Unreachable`
names each such paper in `failed`. Then `waiting_to_app`, and both
settings are deleted.

- [x] Tests, on temporary folders and the memory store (as in
  `library.rs`'s tests):
  - `choose_refuses_a_folder_of_user_files`: a folder holding `notes.txt`
    → `Refused`, `LIBRARY_PATH_KEY` unset, rows unchanged.
  - `choose_from_none_moves_app_sources_when_asked`: with `bring` → every
    row `library`, files on disk; without → rows stay `app`.
  - `switch_copies_this_installations_files`: A has two papers' files; a
    switch to B with `bring` → both in B, `copied == 2`, rows `library`, A
    unchanged (the same files are still there).
  - `switch_with_old_unreachable_names_each_paper`: A removed, then `bring`
    → setting is B, `failed` names both papers, `copied == 0`.
  - `switch_to_the_same_folder_copies_nothing`.
  - `clear_brings_files_back_and_names_a_missing_one`: one of two files
    removed from A, `clear(bring)` → one `app` with bytes, one `library`
    named in `failed`; `waiting` rows become `app`; setting unset.
  - `status_lists_waiting_sources`: an unreachable library → `unreachable`
    set; one waiting row with its title, kind and note.
  Run `cargo test -p studyanalyzer-core library_setting`.
- [x] Implement; tests pass; commit.

### Task 3: Tauri commands, the lock, activation

**Files:** `src-tauri/src/commands.rs`, `state.rs`, `lib.rs`, `error.rs`,
`contract.rs`, `src/lib/fixtures/ipc_contract.json`.

**Produces:**
- Commands: `library_status() -> LibraryStatus`;
  `choose_library(bring: bool) -> Option<Changed>` (`None`: dialog
  cancelled); `clear_library(bring: bool) -> Changed`;
  `move_into_library() -> Changed`. JSON: `Changed` as `{copied, moved,
  failed: [{paper_id, why}]}`; `WaitingSource.kind` as the stored kind
  name.
- `SettingError` → `CommandError`: `Refused` and `Unreachable` are
  `invalid_input` (the sentence names the folder), `Store` as today.
- `AppState.library_op: Mutex<()>`; `AppState::try_library_op()` returns
  `Option<MutexGuard>`.
- Event `library-settled`, payload `{moved: number, left: number}`. It is
  emitted by `settle_at_launch` and by `settle_on_focus` (from
  `on_window_event` on `Focused(true)`, only when
  `count_by_place().waiting > 0` and `try_library_op` gets the lock), and
  only when `moved > 0 || left > 0`.

- [x] Tests: `library_op_is_exclusive` (`try_library_op` is `None` while a
  guard is held); the contract test covers the new commands and shapes
  (`UPDATE_CONTRACT=1 cargo test -p studyanalyzer contract`, then the
  fixture diff shows exactly the four commands, `LibraryStatus`, `Changed`
  and the event payload); `error.rs`: `setting_errors_map_to_kinds`.
- [x] Implement; `cargo test --workspace` passes; commit.

### Task 4: the frontend

**Files:** create `src/lib/library.ts`, `src/lib/library.test.ts`,
`src/components/LibrarySetting.svelte`; modify `src/lib/api.ts`,
`src/lib/contract.test.ts`, `src/lib/api.test.ts` (call list),
`src/components/Settings.svelte`, `src/App.svelte`.

**Produces (in `library.ts`, pure):**
- `libraryCaption(s: LibraryStatus): string`: *No library folder: papers
  are kept in the app.* / *‹path›* / *‹path› cannot be reached: ‹why›*.
- `changeNote(c: Changed): string`: *3 files copied, 5 moved into the
  library.* plus one sentence per failure, *‹paper›: ‹why›*; *Nothing to
  move.* when all are zero.
- `settledNote(e: {moved, left}): string | null`: *2 files moved into the
  paper library.*, plus *1 still waits.*; null when both are zero.
- `unreachableNote(s: LibraryStatus): string | null`: the status line of
  *Decided* above.
- `bringLabel(s: LibraryStatus): string | null`: *Move the N papers kept in
  the app into it* (none set, `counts.app > 0`), *Copy this app's N files
  from the current library* (one set, `counts.library > 0`), else null (no
  checkbox).

- [x] Vitest tests for each function: the exact sentences above, with
  singular and plural.
- [x] `LibrarySetting.svelte`: the caption; *Choose…* / *Choose another…*
  with the `bringLabel` checkbox; *No library* (when one is set) with
  *Copy this app's files back into the app*; *Move N papers into the
  library* when `Ready` and `counts.app > 0`; the waiting list; the last
  `changeNote`; buttons disabled while a change runs. Every failure goes
  through `onerror`, already logged.
- [x] `App.svelte`: `unreachableNote` as a `role="status"` line (read at
  mount, on `window` focus and on leaving Settings); `library-settled`
  → `settledNote` as a status line, cleared by the next `run`.
- [x] `pnpm run check:web`, `pnpm run test:web` and `pnpm exec vite build`
  pass; commit.

### Task 5: docs and gates

- [x] HANDOVER: 2a merged; 2b as built (calls 2b′ builds on); 2b′ next
  with bmlib #769; the click-through owed (the steps of Task 4 with `pnpm
  dev`). ROADMAP: four PRs. CLAUDE.md unchanged (the library rule is
  already there).
- [x] Every gate in CLAUDE.md § Gates; commit, push, open the PR.

## Changed while building

- **Failed entries name the paper by title**: `PaperProblem { paper_id,
  title, why }`. The core types derive `Serialize`, so the commands return
  them as they are.
- **Wording counts files, not papers** (*Move the 3 paper files kept in the
  app into it*): the counts are of sources. `KIND_NAMES` and `moveLabel`
  joined `library.ts`. The vitest call list is in `contract.test.ts`.
- **`src-tauri/src/lib.rs`** raises the test build's `recursion_limit` to
  256: the contract's `json!` list of commands outgrew 128.
- **After review**:
  - **Every open takes `library_op`** (`with_context`), so a change of
    the setting waits for an open and an open for a change. An open can
    no longer settle a new source into a library a switch is leaving;
    the plan's shortcut is gone. Settings keeps *Back* disabled while a
    change runs, so its report of what failed is not lost.
  - **A copy scans the old library once** (`library::read_scanned`), not
    once per file.
  - **A file the new library does not take goes back to the old one**
    (`mark_in_library`: its file is still there, the setting has not
    switched yet) and is named. The database never holds more than one
    paper's bytes.
  - **A library folder that lost its marker is refused** when chosen
    again (an unplugged drive's empty mount point), as `configured` reads
    it.
  - With no library set, the caption names the files still in the folder
    set before. No copy is offered from a library that cannot be reached.
  - The folder dialog is desktop-only (`pick_folder`); on phones the
    command is `invalid_input` until phase 4.
  - Deferred, filed in HANDOVER: a launch settle's count emitted before
    the webview listens; a source that keeps failing re-tried on every
    focus; Settings not refreshed by `library-settled`; the same folder
    under another spelling (a symlink) treated as another.
