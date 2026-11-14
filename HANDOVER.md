# HANDOVER

_Last updated: 2026-11-14 (this session: #119 in PR #127, #118 in PR #128,
and this file condensed. Earlier sessions are in git history)._

Working notes: what is in flight, the calls later work builds on, what is
next and what the maintainer owes. Planned work not yet started is in
`ROADMAP.md`. Each merged phase's full notes are in git history (this file
before 2026-11-14) and in its plan's *Changed while building* / *As built*.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*.
- **Phase 1 is done** (every bmlib item of spec §7 and item 11, last merged
  2026-10-31 as bmlib PR #586, in bmlib 0.12.0 / Rust crate 0.3.0).
- **The bmlib pin** (`rev` in the root `Cargo.toml`) is `33f859c4`, bmlib
  PR #773's merge commit (2026-11-14), past bmlib 0.13.0 / Rust crate 0.4.0.
  It holds the library steps 1a–1c (#754, #763, #764) and their follow-ups
  (#771, #773: `find_digest` by kind, a durable `save`, an evicted file read
  as *not downloaded*), PMC's AWS Open Data bucket for JATS and the PDF
  beside it (#749, #752; a md5 mismatch is `checksum-mismatch`) and evidence
  weight phase 1 (#743, for a later *risk of bias* tab). bmlib #774 (the
  FFI's Kotlin name) and #775 (MECA over S3) merged after the pin; nothing
  of ours waits on them. What each pin brought: `git log -p Cargo.toml`.
- A stored PDF text older than `pdf-7` (bmlib #636) or Markdown text older
  than `markdown-2` re-derives on its next open unless a conversation turn
  or a PICO citation points into it (`open::has_dependents`).
- **Phase 2 is merged**: 2a (PR #10), 2b (PR #51), 2c (PR #59), the quote
  guard (PR #62) and 2d (PR #75, fixes in PR #77). Their click-throughs are
  owed (below).
- **Phase 3 (the shared paper library)**: bmlib's steps 1a–1c and
  StudyAnalyzer's steps 2a, 2b, 2b′ and 2c are merged (PRs #116, #117, #120,
  #123, #125). In review: **PR #127** (#119: a sandbox's ungranted access is
  *lost*; `resume`'s writes in one transaction) and **PR #128** (#118: a
  failure after a switch keeps what it did; the setting lists files left in
  another folder; the focus settle's decisions in core). Steps 3 and 4 are
  in bmlibrarian_lite (below).

## Calls later work builds on

### Phase 2a: storage and intake (plan `2026-10-29-phase2a-scaffold-and-intake.md`)

- **Identifiers** are stored canonical (PMID digits without leading zeros or
  `PPR`+digits; DOI lower-cased; `PMC`+digits). A JATS source's own
  identifiers are recorded too, so one paper opened by PMID and by DOI is
  one paper. An identifier another paper holds is skipped with a notice;
  `attach` refuses a JATS naming another paper (`OpenError::WrongPaper`).
  `open` refuses a document contradicting the stored paper
  (`store::contradiction`, `OpenError::Mismatch`; a lookup keeps the reason
  in `papers.refusal`); the debugging setting `KEEP_MISMATCHED_KEY` opens it
  elsewhere instead, with a notice.
- **The same document again stores no copy** (`open::persist`); an older
  converter's reading of it gets the new text and keeps the confirmation. A
  file or paste without identifiers finds its paper by its bytes
  (`store::holder_of_source`).
- **A lookup** asks for the record first (`retrieval::resolve` → bmlib
  `lookup_record`); typed identifiers win (`request_for`,
  `identifiers::check_fetched`). A PPR sits in `Identifiers.pmid` (a
  `shortcut:`; `retrieval::split_pmid`). "No free full text" is told from an
  outage by bmlib's types (`retrieval::faulted_exhaustion`). `limit::Limit`
  (`RETRIEVAL_DEADLINE`, 120 s) and `LimitedClient` give a total limit and a
  Cancel that stores nothing (`OpenError::Cancelled`).
- **Re-derive** only while nothing points into the text
  (`open::needs_rederive`); a failure is recorded and not retried until the
  converter changes. HTML over 2 M code points or 30 s is
  `IntakeError::Html`; `PdfError::is_the_documents` says whether another
  file helps.
- **Notices** (the paper banner) are stored with the source and shown on
  every open of a text derived from it.
- **`retry::with_backoff`** blocks its thread (`spawn_blocking`); PICO's
  `chat()` calls go through it while **bmlib #561** stands.
- **Hardening**: a newer app's database is refused; a chosen file is read at
  most one byte past `MAX_PDF_BYTES`; `view::span_to_utf16` and
  `utf16_offsets` are the only code-point → UTF-16 conversion.
- **The Rust↔TS contract**: `src-tauri/src/contract.rs` writes every shape
  `src/lib/api.ts` mirrors into `src/lib/fixtures/ipc_contract.json`;
  `contract.test.ts` checks keys and types. A new shape is a key in
  `contract()` plus `SameKeys` rows; a new command goes in `lib.rs`'s
  `handler()`, `commands()`, `api.ts` and the vitest call list together.
  After a deliberate change: `UPDATE_CONTRACT=1 cargo test -p studyanalyzer
  contract`, review the diff, rerun.

### Phase 2b: the Interrogator (plan `2026-11-02-phase2b-interrogator.md`)

- **Dependents**: a text with a conversation turn or a PICO citation is
  never re-derived (`open::has_dependents`); a new analysis that stores
  spans adds its check there. Chunk sets are keyed by text id and only the
  latest text's is kept (`store::save_canonical`,
  `drop_superseded_chunk_set`; **#65** stays).
- **Quotes**: the model writes `<q>…</q>` inline; `answer::parse_answer`,
  then `answer::locate` runs `find_passage` on the whole text. An unlocated
  quote keeps its place with `number: None` and its near miss
  (`closestNote`); the shown wording is always the paper's span.
  **`MIN_QUOTE_SIMILARITY` is 0.99**, set by `tests/quote_threshold.rs` on
  `tests/fixtures/quotes.json` (106 cases); an arm swap passes the guard
  (**bmlib #651**). A refused quote's reason is stored
  (`quotes.contradiction`) and shown (`contradictionNote`); PICO keeps it too
  (`pico_parts.quotes_contradicted_json`).
- **The Original tab's PDF beside JATS** (PR #105): a looked-up JATS paper
  keeps the PDF bmlib found with it (`sources.pdf_beside`,
  `save_pdf_beside`); `original_pdf` serves it; a failed one leaves a notice
  (`PDF_SEARCH_FAILED`, "may be a passing one" when
  `pdf_failure_may_pass`). A reopen never looks the paper up again. Bot
  screens (MDPI, Europe PMC) answer 403; impersonating a browser was
  refused; PMC's AWS bucket (bmlib #749) covers papers with a PMC ID. An
  "add the PDF you downloaded" route is not built (the maintainer's call).
  **#106** stays.
- **`interrogate::ask`** returns the conversation id; a stopped or broken
  stream is stored as `Stopped` / `Failed`; an answer cut at `reply_tokens`
  is `Complete` with a notice in `error` (read `status`). `AskError` maps
  through `CommandError::from_ask` (`from_chat`: 401/403 `needs_api_key`,
  404 `invalid_input`).
- **Events** go over a Tauri `Channel` (`AnswerEvent`, `DownloadProgress`,
  `PicoEvent`); the frontend drops events from a run no longer live.
  `StopSlot` gives every run a fresh `AtomicBool`; `api.stopWhilePending`
  re-sends a stop Tauri handled before its command registered (#61). A
  client that wraps another must forward `post_json_stream_with_cancel`.
  A DB connection per command, never held behind a lock while streaming.
- **The Interrogator always shows** a paper with text; an unconfirmed text
  shows earlier conversations and `askBlockedNote`. `paper_view_at` shows
  the text an earlier conversation was held on.
- **The embedder** (`embedder.rs`, `ModelSlot`): the pinned
  `EMBEDDING_MODEL` (F16, 1.1 GB), downloaded with backoff, unloaded on
  `RunEvent::Exit`; `needs_embedder` is the authority. The `embed` feature
  pulls llama.cpp and CMake into every target (phase 4).
- **Keys**: bmlib's `KeyBundle::keyring(KEYCHAIN_SERVICE)`, one item
  `api-keys`, read once per launch; env var fallback
  (`llm_settings::key_source`); `keychain_advice(kind)` says what helps.
- **The window**: `prompt::running_window`; `FIT_CHARS_PER_TOKEN` (3);
  `prompt::reply_tokens` (a quarter of the window, 4096–32768, never above
  the model's output limit) is every call's `max_tokens`. Unknown window and
  no setting → retrieval mode (`UNKNOWN_WINDOW_FIT_TOKENS`,
  `RETRIEVAL_PASSAGES`). A prompt that does not fit is refused, never cut.
  Temperature 0 (none to Anthropic); per-model parameters are **#49**.
- **The model picker** lists the provider's models (`list_models`), with
  *Other…*; a failed listing is a note under the field.

### Phase 2c: the PICO tab (plan `2026-11-03-phase2c-pico.md`)

- **Storage** (`pico_store.rs`): `pico_runs` → four `pico_parts` →
  `pico_citations` (spans only). `save_run` refuses a shape its check
  contradicts; `run()` reads a contradiction back as `Corrupt`. Every run
  is kept; `latest_run` is the newest on a text.
- **Budget, never a cut** (`pico::budget_chars`): over it the paper is
  condensed once (`prepare`), whole or failed; quotes are located in the
  whole text. Progress comes as `PicoEvent`s (`runningNote`).
- **Retry and stop**: `RequestFailed { retryable: true }` is retried by
  `with_backoff`, never once stopped; a refused key maps by its status. The
  stop is read after every call and before `save_run`. `run_pico` claims
  `AppState.pico_stop` (`StopSlot::try_begin`) and frees it in
  `commands::with_claim`.
- **The tab host keeps the run's state** (`Interrogator.svelte`): auto-run
  once per mount (`shouldAutoRun`), *Extract PICO* by `offerRun`, errors by
  `runFailure`. The card shows each part's confidence, *Mean confidence*
  and the check's `findingsNote`.

### Phase 2d: the hand-off (plan `2026-11-04-phase2d-handoff.md`)

- **The format** is `docs/handoff-format.md`, pinned by
  `crates/studyanalyzer-core/tests/fixtures/handoff/`; a sender must pass
  them.
- **Grants**: the webview neither chooses nor opens a path; `open_file` /
  `attach_file` take grants, Rust shows the dialog.
- **Arrival → `handoffs` table → `handoff-received` → opened when idle**
  (`nextHandoff`); every route stores the bytes at once; a refused one is a
  refusal row with its reason. `keeps_row` decides when a row goes.
- **Routes**: macOS's `RunEvent::Opened` (files and links, held in
  `EarlyOpened` / `EarlyLinks` before setup, drained by `receive_held`);
  Linux and Windows by the command line and `second_instance`. The app does
  not listen to the deep-link plugin's events on desktop (#94: Tauri 2.12.5
  defers a listener); deep links work only from `/Applications`.
- **`OpenRequest::Handoff`**: `OpenError::HandoffMismatch` for a document
  whose identifiers differ from those sent; without a document it is a
  lookup, and the other identifiers go to the paper found
  (`add_identifiers`). Labels refuse control and bidi characters
  (`config::BIDI_CONTROLS`, #69); `handoffCaption` isolates a title.

### Phase 3: the shared paper library (design `2026-11-13-shared-paper-library-design.md`)

Plans `docs/superpowers/plans/2026-11-1{3,4}-library-*.md`, each with an
*As built* note.
- **bmlib 1a–1c**: the `library` module (#754), `FullTextService` on it
  (#763), and the module through bmlib-ffi (#764; not `with_library`).
  Follow-ups open there: **#755**, **#761** (reads follow symlinks), **#762**
  (a non-UTF-8 library JATS), **#765** (the FFI skips `save_scenarios`),
  **#766** (`save` does not check a folder's JATS; we check it ourselves).
- **2a, sources by digest** (PR #116): migration 11 moved the bytes to
  `source_bytes`; `sources.place` is `app`, `waiting` or `library`
  (`store::SourcePlace`), `sha256` the reference, `library_note` why a
  source waits or which folder its file was left in. `library::Configured`
  (`None`, `Ready`, `Unreachable`) from the settings `library_path` and
  `library_installation`, per command (`AppState::library`). With a library
  set every new source is stored `waiting` and `open::finish` settles it
  (`library::settle`); reads go through `library::read_source` (digest and
  kind checked). `foreign_jats` refuses a folder whose JATS names another
  paper, passing over one that reads as `Missing`. An empty folder becomes a
  library only the first time it is opened (`library_opened:<folder>`).
- **2b, the setting** (PR #117): `library_setting.rs` (`status`,
  `choose(root, bring)`, `clear(bring)`, `move_in`, each returning `Changed
  { copied, moved, failed }`); commands `library_status`,
  `choose_library(bring)` (the dialog in Rust), `clear_library(bring)`,
  `move_into_library`. `AppState.library_op` lets one change run at a time;
  every open holds it (`with_context`), a lookup through its fetch (a
  change can wait for a slow lookup; deliberate, revisit if it shows in
  use). The launch and focus settle (`settle_waiting` in `lib.rs`, its
  decisions in core's `library::settle_waiting` since PR #128) emits
  `library-settled`. A file a change leaves in the old folder keeps that
  folder in `library_note` (`note_left_behind`); since PR #128 the setting
  lists such files (`LibraryStatus.elsewhere`) and choosing that folder
  again clears their notes (`store::clear_library_note`), and a database
  failure after the switch is `SettingError::Switched`, keeping `Changed`.
- **2b′, the macOS bookmark** (PR #120): `bookmark.rs` (`create`,
  `resolve` → `Resolved { path, stale, started, access }`). Stored as
  `library_bookmark` in `choose`'s transaction, removed by `clear`.
  `library_setting::resume` at launch follows a moved folder, renews a stale
  bookmark, bookmarks a path set before 2b′, never follows a folder into a
  Trash (`TRASH_FOLDERS`), and since PR #127 treats a start that fails in a
  sandbox (`SANDBOX_ENV`) as lost and writes what it learnt in one
  transaction (a failed write keeps the access). Why a bookmark is lost is
  `library_bookmark_fault`, added to the unreachable status.
  `AppState.library_access` holds the access until exit. Entitlements:
  `files.user-selected.read-write`, `files.bookmarks.app-scope`. Library
  reads turn materialisation off, so an evicted file is `NotDownloaded`
  (bmlib #769).
- **2c, the manual clear** (PR #123): `library_clear.rs` (`list`,
  `remove`); commands `library_papers`, `remove_library_paper`,
  `remove_library_file`, each holding `library_op`; `LibraryPapers.svelte`
  with an in-page confirmation. A reopen brings a library source's bytes
  back only when the library does not hold the file whole
  (`library::holds`, PR #125).
- **Left, minor** (shortcuts in the plans and code): the launch settle's
  event usually fires before the webview listens; a failing source is tried
  on every focus; Settings is not refreshed by `library-settled`; the same
  folder by another spelling counts as another; the mobile build leaves
  `LIBRARY_DIALOG_TITLE` unused; the clear lists folders of one paper apart,
  shows bmlib's `Debug` text for scan problems, and a reopen puts back only
  the file the text was read from.

## Next

1. **Phase 3 step 3, MedicalFactChecker (macOS)** in bmlibrarian_lite:
   re-pin the xcframework (its PR #779 re-pins bmlib), the setting, its
   fetch through the library (keeping the JATS), the move, the clear.
   BioMedLit adds `extension LibraryError: CustomStringConvertible {}` for
   bmlib's error text (Kotlin: `toString()`).
2. **Phase 3 step 4, Open in StudyAnalyzer**: `StudyAnalyzerHandoff` in
   BioMedLit, always a file (never the link), sending what StudyAnalyzer
   cannot fetch itself (whatever it has when offline).
3. **What a page cut short should be** (**bmlib #605**, the maintainer's
   call; today a notice).
4. **#31** (two transitive Dependabot alerts) waits on Tauri 3 (glib) and a
   vite-plugin-svelte without deepmerge (both still so on 2026-11-07).
5. **Open issues.** bmlib's are in ROADMAP (*Phase 1*); the nearest are
   **#651** and **#577** (point 3). #686 is in the pin (check the Windows key
   item's naming on Windows). Ours: **#126** (decide: may the launch's
   bookmark resolution mount a network share), **#106**, **#49**, **#65**,
   **#70** (memory at the cap, phase 4), **#74** (iOS keyring, a bmlib
   change for phase 4), **#78** (decide: refuse bidi marks in labels too).
   A Svelte component documents itself in a `/** … */` at the top of its
   script (#14).

## Owed by the maintainer

- **Click-through of step 2c** (`pnpm dev`, a library holding a few
  papers): *Show the library's papers* lists each with its size and *used
  by this app*; *Remove file* on a PDF this app uses: the question says the
  text and conversations stay, *Keep it* removes nothing, *Remove* deletes
  it from the folder and the list; that paper's Original tab then says the
  file is no longer in the paper library; open the same PDF again: the tab
  works and the file is back in the folder. *Remove paper*: the folder is
  gone in Finder. A paper MedicalFactChecker also uses (once step 3 writes
  records) names it in the question. Delete a paper folder in Finder, then
  *Remove* it from a list shown before: refused, and the list is read again.
  While a removal runs, *Choose…* and *No library* are disabled; unplug the
  library's drive with the list shown, then *Read the list again*: the
  banner says why and the list goes.
- **Click-through of step 2b′** (a sandboxed build in `/Applications`):
  choose a folder, quit, relaunch: the library is reachable and a library
  PDF's Original tab opens; rename the folder while quit, relaunch: it is
  followed (Settings shows the new path); move it to the Trash while quit,
  relaunch: the status line says it is in the Trash (put it back, relaunch:
  reachable); delete it for good (`rm -r`) while quit, relaunch: the status
  line ends "at launch: …; choose the folder again"; choose another folder:
  the status line goes. *Choose…* a folder inside the Trash: refused.
- **Click-through of steps 2a and 2b** (`pnpm dev`, unsandboxed): Settings
  → *Paper library*: a folder holding your own files is refused; *Choose…*
  an empty folder with *Move the N paper files kept in the app into it*
  ticked (*N files moved*, each paper in `papers/…` with a
  `records/studyanalyzer-….json`); open a PMID, a PDF and a paste (each
  lands there); the Original tab of a library PDF; rename the folder: the
  status line appears (on focus), a new paper opens with the *kept in the
  app for now* notice and is in the waiting list; rename it back and switch
  to the app: *1 file moved into the paper library.*; *Choose another…* with
  the copy ticked (the old folder unchanged); *Choose another…* without it
  (*Nothing was copied or moved.*; the Original tab of a PDF left behind
  names the old folder; the setting lists it under *Left in another library
  folder*, and choosing the old folder again empties that list, PR #128); *No library* with the copy ticked (`select place,
  count(*) from sources group by place` shows `app`).
- **PR #105's click-through, done 2026-11-12** but for two items: no Original
  tab without a PDF; DOI 10.3390/nu17213408 shows its PDF and keeps it on a
  reopen (on #749's branch; 10.3390/ijms262110631, looked up before it, got
  MDPI's and Europe PMC's 403s); a 403 notice without "may be a passing
  one", again on a reopen; PICO on a cloud and a local model; a page of
  800,010 `<` refused with "Paste its text instead". **Owed**: a PICO quote
  the paper contradicts, listed with its reason (none came up), and a 5xx
  or lost network ending in "The fault may be a passing one."
- **Click-through of PR #110**: type `ijms262110631` on the start screen. It
  should say `"ijms262110631" is not a valid PMID, DOI or PMC ID.`, while a
  bad `PMC…` value still names a PMC ID and a bad `10.…/…` one a DOI.

- **Click-through of phase 2b, under way** (2026-11-06): round 1 (local
  model: citations, follow-up, unanswerable question, status line, *Stop*)
  passed; no key is in the database (checked). Round 2 (2026-11-07) so far:
  PICO on `deepseek-flash` (fixed in PR #100, re-run passed) and the model
  menu (lists, choose, *Other…*, no-key note,
  *‹model› (not listed by ‹provider›)*).
  Next in round 2: one PICO run on the cloud model again (bmlib #696's quotes
  beside `reported: false`; #706's 600 s answer wait), a key saved → the
  list, then retrieval mode and the first download (the search model is not
  downloaded yet); one live Anthropic chat (sent no `temperature` since
  bmlib #709) and one live Gemini chat (bmlib #681's documented path, in
  our pin since #732). Keys go through bmlib's store: a key saved before
  still reads; with the keychain unreadable the Settings banner names the
  item and says what helps by kind (#689: damaged → a save replaces it;
  locked → unlock it, no save warning; an ask without a key adds the env
  var).

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (not hit by plan 2b Task 4's measurement;
  `find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).
- **Click through phase 2a** (plan Task 9 Step 5): open a PDF, a JATS file, a saved
  HTML page, a pasted Markdown text; a PMID with free full text and one
  without; Wi-Fi off for the unreachable path; dark mode. (#7's *Cancel* was
  clicked through on 2026-10-31 and works.)
- **The rest of phase 2b's click-through** (plan Task 10 Step 3), with `pnpm
  dev`, a real cloud model and a local Ollama model, beyond round 1
  (above): *Stop* during *Finding passages* and while a local model reads a
  long prompt (no text yet) (nothing stored, question kept; since bmlib
  PR #714 Ollama's log should show the request ending within about a
  second, not when the prompt is read);
  Wi-Fi off mid-stream and during the download (the offer stays to retry); a
  small context-window setting forcing retrieval (the download with *Cancel*,
  then *Finding passages*); a quote that fails to locate (ask for a
  paraphrase); *New conversation* and an earlier one (*on an earlier version
  of the text*); `needs_model` / `needs_api_key` opening Settings and *← Back*
  keeping the question; *Checking the search model…* and *Finding
  passages…* giving way to *Waiting for the model…* before the first text,
  and *Stopping…* (`applyEvent` is tested, its wiring is not); an unconfirmed text
  showing earlier conversations and the *Confirm above…* note, the box
  appearing on confirm, and no confirm button while an earlier text is shown; a PDF paper's
  *Original* tab; a narrow window for the phone switch; dark mode; quit with the search model loaded
  (no crash report) and reopen (everything still there); the Keychain prompt
  once per launch, not per provider; the key in Keychain Access, not in the
  database (`sqlite3 … "select * from settings"`); and the keychain in a
  sandboxed build.
- **Click through phase 2c** (plan Task 6 Step 3), with `pnpm dev`, a cloud
  model and a local Ollama model: an RCT from PMC (four verified parts; click
  → scroll, current dark, others pale; *Judged from its abstract*); a review
  (not suitable, rationale, *Run anyway*; then *Re-run* stays forced); a long
  paper on a small window (*Condensing the long paper, part n of m…*
  counting up, then *Reading the condensed paper…*; condensed
  caption, quotes still highlight; a question asked meanwhile is answered); *Stop* during condensation (nothing
  stored, *Extract PICO* offered); leaving the tab mid-run and back (the same
  run, no second); a double-click on *Extract PICO* (one run); Wi-Fi off
  during condensation (the error after the retries, the previous run kept);
  *Re-run* with another model (both listed); an earlier run on an older text
  (the banner's confirm held back); no model / no key (a message and a
  button, no jump to Settings); an unconfirmed text, then confirming it with
  the tab open (*Extract PICO* offered); a narrow window (*PICO | Paper*);
  dark mode; quit and reopen (the run shows at once, no new call); the check's
  findings line and *Mean confidence* (#60).
- **Click through phase 2d** (plan Task 7 Step 1; no agent could click the
  window), with a built, sandboxed app in `/Applications` (deep links work
  only there on macOS): choosing and dropping a file still opens and
  attaches; a `.studyanalyzer` file from the fixtures double-clicked with the
  app closed and again with it open (one window), dropped on the window, and
  sent by `open -a StudyAnalyzer file.studyanalyzer`; one double-clicked in
  Finder with the app closed (stored once, opened at mount: the
  early-opened queue); `open "studyanalyzer://open?pmid=…"` with the app
  closed and open; a hand-off while an answer streams and while a PICO run
  goes (it waits, then opens); one arriving while an error banner is shown
  (it waits until dismissed) and one while Settings is open (it waits until
  the user leaves); two in a row (the newest opens, the other is listed); a
  refused file on a cold start (the reason is shown and stays until
  dismissed); the identifiers-only link with no contact email (the email
  screen, then the lookup); quit with a hand-off pending, reopen (it opens);
  Finder shows the file type's name; dark mode. Once Linux or Windows can be
  run (#71): a second launch with a file while the first still starts (stored
  once); a second link while a link-launched app starts (both, once, #94); `studyanalyzer <file> studyanalyzer://open?pmid=…` (both received); `studyanalyzer 'studyanalyzer://[x'` (its refusal shown);
  an AppImage without xdg-utils (the registration dialog).
- **Decide #73: a hand-off claiming a PPR preprint accession in its PMID field
  whose JATS has a numeric PMID** is refused as a mismatch (the rule
  `store::contradiction` applies); should a preprint's journal version land
  instead?
- **Decide: a hand-off arriving while Settings is open** waits until
  Settings is left (the plan said it would navigate away).
- **Decide bmlib #651**: whether the guard should catch a swap of named
  arms and a spelled-out number (then `MIN_QUOTE_SIMILARITY` can drop
  towards 0.98 and keep dropped-word quotes). #647 (PICO records a refused
  quote's reason) landed in PR #105.
- **Decide** (one line each; the issue holds the argument): **#65** what an
  ask shows when the text is replaced mid-ask (an "ask again" error, or one
  retry on the latest; unreachable from the UI now); **#49** per-model
  parameters (§6.6; 2b sends temperature 0 only); **#39** store a looked-up
  record's identifiers on the paper, or only the typed and a JATS
  document's own; **#20** MuPDF's licence and store distribution (blocks
  store submission only); **bmlib #560** how strict `assess_full_text` is
  (moves the shared fixture); **bmlib #561** should Rust's `chat()` retry as
  Python's SDKs do.
- **Decide how large `num_ctx` is on Ollama**: 2b sends the running window,
  often the model's full 131072, so a large local model may fail to load or
  spill to CPU. Sizing it from the prompt estimate could undercount (CJK,
  dense notation) and let Ollama cut the paper silently. Check it in the
  click-through first.
