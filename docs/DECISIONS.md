# Calls later work builds on

The settled decisions each merged phase left behind, which later work must
respect. They moved here from `HANDOVER.md` on 2026-11-25; their history
before then is in that file's. Add a call here when a phase lands; what is
in flight and what is owed stays in `HANDOVER.md`.

## Phase 2a: storage and intake (plan `2026-10-29-phase2a-scaffold-and-intake.md`)

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
  `identifiers::check_fetched`). The record's valid identifiers for the
  fields the typed ones lack are stored on the paper too
  (`Resolved.record_ids`, `store::add_record_identifiers`), marked as the
  record's (`papers.record_ids_json`, migration 12): they find the paper
  but never refuse a document (`contradiction`), and a document's own,
  typed or handed-off identifier replaces one, with a notice (#39). A PPR
  sits in `Identifiers.pmid` (a `shortcut:`; `retrieval::split_pmid`). "No
  free full text" is told from an outage by bmlib's types
  (`retrieval::faulted_exhaustion`). `limit::Limit` (`RETRIEVAL_DEADLINE`,
  120 s) and `LimitedClient` give a total limit and a Cancel that stores
  nothing (`OpenError::Cancelled`).
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

## Phase 2b: the Interrogator (plan `2026-11-02-phase2b-interrogator.md`)

- **Dependents**: a text with a conversation turn or a PICO citation is
  never re-derived (`open::has_dependents`); a new analysis that stores
  spans adds its check there. Chunk sets are keyed by text id and only the
  latest text's is kept (`store::save_canonical`,
  `drop_superseded_chunk_set`). A set dropped between an ask's indexing and
  its search is `PassageError::TextReplaced`, "ask again" (#65).
- **Quotes**: the model writes `<q>…</q>` inline; `answer::parse_answer`,
  then `answer::locate` runs `find_passage` on the whole text. An unlocated
  quote keeps its place with `number: None` and its near miss
  (`closestNote`); the shown wording is always the paper's span. An answer
  with no `<q>` has its `“…”` of at least `FALLBACK_QUOTE_MIN_WORDS` (3)
  words taken instead (deepseek-flash ignores the format); an unmatched `“`
  stays text, and an unfound one keeps its words in the answer (#157).
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
  A JATS that cannot be read is replaced by the PDF beside it, with a
  notice; when that PDF could not be had or read, the reason adds why, and
  a fault of ours on it leaves the paper waiting (`open_by_identifiers`,
  #106).
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
  Temperature 0 (none to Anthropic); per-model parameters wait for a need
  (ROADMAP *Later*; #49 closed).
- **The model picker** lists the provider's models (`list_models`), with
  *Other…*; a failed listing is a note under the field.

## Phase 2c: the PICO tab (plan `2026-11-03-phase2c-pico.md`)

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

## PRISMA 2020 (plan `2026-11-20-prisma-2-tab.md`)

- **Storage is part by part** (`prisma_store.rs`, migration 13): `prisma_runs`
  → `prisma_parts` → `prisma_items` → `prisma_citations` (spans only). Every
  checklist item is shown from exactly one row, the one a non-`recheck` part
  stored; `prisma_store::run` (and so the view) shows no item row of a
  `recheck` part. A text with a PRISMA citation is never re-derived
  (`open::has_dependents`).
- **`save_part` does the follow-ups in its own transaction**, so a crash can
  never skip one and still let the run complete. Items a reply left out get a
  pending `missing` part; when that part's reply leaves some out again, a
  `missing` part is added already failed, naming them. On a condensed paper,
  each key item judged *not reported* gets a pending `recheck` part. A
  `recheck` part's answer is swapped with the item's shown row there too: the
  shown row takes the re-check's answer, the re-check's row keeps the one it
  replaced, and `Recheck.first_judgement` keeps the first. A re-check still
  missing its item gets a failed `recheck` follow-up. A failed part holds no
  items.
  Whether a part was condensed is its assessment's `condensed_from_chars`
  (`save_part` takes no separate flag).
- **Continue** first sets the run's failed parts back to pending
  (`reset_failed`; each keeps its error until it is saved or split, and the
  view still names it as failed), then runs every pending part, on the run's own text and
  model (`continue_uses_run_model` tests the command's choice). It is offered
  for any shown run that is incomplete, not only the current one. A part that
  is still missing items does not stop the drive: the rest runs and the run
  stays incomplete, with the failed part's error naming the items. A stop on
  the last part finishes the run. Opening the tab starts no run while an
  earlier run is incomplete (`earlierIncomplete`).
- **The budget covers the suitability check too**: `budget_chars` takes the
  longer of the longest part prompt and the check's prompt, because the check
  may send the whole paper. A halved `missing` part stays a re-ask.
- **`Part.of` grows** when a part is halved, re-asked or re-checked, so
  `index <= of` always holds. There is no `PRISMA_MISSING_REASKS`: the single
  re-ask is the part's kind.
- **PICO and PRISMA share one run-tab lifecycle**: `runTab.svelte.ts`
  (frontend; PRISMA reloads the view after a failed or stopped run) and
  `stop_slot`, `event_sink` and `stored_view` in `src-tauri`. A third tab
  should use them too.
  `contract.rs`'s `OPTIONAL_ARGS` lists the commands' optional arguments.
- **What the model did besides the checklist** (#152, surface diagnostics):
  the run view's `left_out` (the ids of every `missing` part), `halved` (the
  labels of `split` parts) and `unasked` (each part's `unasked_json`,
  migration 14, from bmlib #808), worded by `prisma.ts`'s `omissionNotes`
  above the notes.
- **Types** (#153): the stored words are `PartKind`, `PartStatus` and
  `RunStatus` (`as_str`/`parse`/`ALL`, serialised as the same words; TS's
  `RUN_STATUSES`); a part's error is `Option<PartError>` (a failed part
  without one, or a done or split part with one, is `Corrupt`), a shown
  item's re-check `Option<Recheck>`, which must name a `recheck` part of the
  same run. The run view's check carries `from_abstract` and `variant`;
  `FailedPartView`'s fields are plain strings. `create_run` takes a partial
  plan (tests); `run_prisma` plans all 42.
- **Continue in the frontend** is `RunTab.continueRun` (an earlier run stays
  shown, also after a stopped or failed Continue); `showRun` drops a stale
  run or text fetch when a later run was chosen (#154).

## Evidence weight (plan `2026-11-25-evidence-tab-2-tab.md`)

- **Storage is step by step** (`evidence_store.rs`, `evidence_rows.rs`,
  migration 15). A run starts with one pending `design` step. Saving the
  design adds the `outcomes` step, or ends the run as *not covered*. Saving
  the outcomes adds `plan_rob2(n)`'s steps, or ends the run as *no primary
  outcome*. So the driver (`evidence.rs`) runs only what is pending, as in
  PRISMA.
- **A failed step:** a design or outcomes failure ends the call. A failed
  domain or escalation ends the call only when it came from a request (the
  failure is `retryable`, or has an HTTP status): the next step would meet
  the same server. Any other failure (an unreadable reply, a failed
  condensation without a request fault) is stored, and the run goes on.
- **Budget:** one function measures the design prompt, the outcome prompt,
  D1's prompt and the D2-D5 prompts of every outcome known so far. The
  design and outcomes steps use it with no outcomes. The RoB 2 steps use it
  with the run's stored outcomes, so one `Rob2Digests` serves the whole
  RoB 2 stage. The design and outcome digests are not reused by the domains.
- **Continue on a run made under earlier rules** (any stored version, or
  the ladder settings, differing from the pinned bmlib's) is refused. Such a
  run is shown in full with *Re-run* only: rules must never mix within one
  run.
- **Continue first checks every done escalation** of a current-rules run:
  its outcome's domains are rebuilt and combined with the stored escalation,
  and an error or an overall that differs from the stored one is
  `Store(Corrupt)`, before anything is reset. The check runs only on
  current-rules runs, so old runs stay readable.
- **A step set back to pending loses its stored result** (a D5 with a failed
  second look, an escalation whose domains are re-run). It is replaced by
  the new attempt. A failed design, outcomes or domain step holds no
  result. A failed escalation keeps its outcome's row (overall not assessed,
  with the ladder error) until it is set back.
- **Stored results are read as text for the view** and turned into bmlib
  types only to hand them back to bmlib (an escalation, combining) on a run
  with the current versions. So an old run's view never depends on today's
  rules.
- **Question ids are checked against bmlib's `questions(domain)` only on
  runs with the current versions.** On an earlier-rules run any non-empty id
  is accepted, and the view gives bmlib's question text only for an id that
  `questions(domain)` still knows (else the id alone).
- **Earlier-rules runs are read without today's vocabulary checks.** Their
  designs, layouts, scales, D5 facts and verdicts, and ladder reasons may be
  any non-empty name, shown as stored (spec §6: never re-judged); on a
  current-rules run each must be a name today's bmlib writes. The plan check
  still compares every run with today's `plan_rob2`.
- **The paper weight is the largest stored `ladder_weight`** of the run's
  outcomes (it is what bmlib's `paper_weight` is), shown only for a complete
  `assessed` run with every step done. The view never calls `ladders()`; a
  test pins the equality with bmlib on a current run.
- **`first_shown_at`** is set only if still empty, when a run with a done
  domain step is served: by `evidence`, `evidence_run` and also by
  `run_evidence` on the view it returns (what the tab renders), so a run
  shown straight after finishing is marked too. It is stamped then even if
  the user switched tabs meanwhile: this errs towards "not blind", the
  strict-evidence side.
- **Continue sizes its budget with the run's stored outcomes first**, so a
  window too small is `TooLong` with nothing set back.
- **The commands are in `src-tauri/src/evidence_commands.rs`**, their own
  file, shaped as PRISMA's. One run at a time (`with_claim` on
  `evidence_stop`); Continue uses the run's stored model.
- **The view checks spans itself** (a citation whose span the paper's text
  cannot give is `Corrupt`, not a missing quote), and `DomainView.name` is
  bmlib's full domain title; the card's short labels come from
  `DOMAIN_LABELS` in the frontend.

## Phase 2d: the hand-off (plan `2026-11-04-phase2d-handoff.md`)

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
  (`config::BIDI_CONTROLS`, #69) and bidi marks, separators and invisible
  characters, tags included, ZWNJ, ZWJ and variation selectors allowed
  (`config::LABEL_INVISIBLES`, #78); `handoffCaption` isolates a title.

## Phase 3: the shared paper library (design `2026-11-13-shared-paper-library-design.md`)

Plans `docs/superpowers/plans/2026-11-1{3,4}-library-*.md`, each with an
*As built* note.
- **bmlib 1a–1c**: the `library` module (#754), `FullTextService` on it
  (#763), and the module through bmlib-ffi (#764; not `with_library`).
  Follow-ups open there: **#755** (items 1 and 4; `FileClash` fixed by
  bmlib #787, item 2 by #798), **#762** (a non-UTF-8 library JATS), **#778**
  (items 2–4; item 1 fixed by #798); #765 closed by bmlib #786, #796 by
  #799.
- **2a, sources by digest** (PR #116): migration 11 moved the bytes to
  `source_bytes`; `sources.place` is `app`, `waiting` or `library`
  (`store::SourcePlace`), `sha256` the reference, `library_note` why a
  source waits or which folder its file was left in. `library::Configured`
  (`None`, `Ready`, `Unreachable`) from the settings `library_path` and
  `library_installation`, per command (`AppState::library`). With a library
  set every new source is stored `waiting` and `open::finish` settles it
  (`library::settle`); reads go through `library::read_source` (digest and
  kind checked). bmlib's `save` refuses a folder whose JATS names another
  paper (#766), passing over one that reads as `Missing`; the source waits
  with bmlib's reason as its note. An empty folder becomes a library only
  the first time it is opened (`library_opened:<folder>`).
- **2b, the setting** (PR #117): `library_setting.rs` (`status`,
  `choose(root, bring)`, `clear(bring)`, `move_in`, each returning `Changed
  { copied, moved, failed }`); commands `library_status`,
  `choose_library(bring)` (the dialog in Rust), `clear_library(bring)`,
  `move_into_library`. `AppState.library_op` lets one change run at a time;
  every open holds it (`with_context`), a lookup through its fetch (a
  change can wait for a slow lookup; deliberate, revisit if it shows in
  use). The launch and focus settle (`settle_waiting` in `lib.rs`, its
  decisions in core's `library::settle_waiting` since PR #128) emits
  `library-settled`; a database failure partway is `SettleFailed`, keeping
  what moved (#129), so the event's `moved` stays true (`settled_event`).
  A file a change leaves in the old folder keeps that folder in
  `library_note` (`note_left_behind`); since PR #128 the setting lists such
  files (`LibraryStatus.elsewhere`) and choosing that folder again clears
  their notes (`store::clear_library_note`), and a database failure after
  the switch is `SettingError::Switched`, keeping `Changed`.
- **2b′, the macOS bookmark** (PR #120): `bookmark.rs` (`create`,
  `resolve` → `Resolved { path, stale, started, access }`). Stored as
  `library_bookmark` in `choose`'s transaction, removed by `clear`.
  `library_setting::resume` at launch follows a moved folder, renews a stale
  bookmark, bookmarks a path set before 2b′, never follows a folder into a
  Trash (`TRASH_FOLDERS`). It never mounts (#126): a library under a
  `/Volumes` volume that is not mounted (absent, or an empty folder an
  unclean eject left: `is_mount_point`, by device) is lost as a drive or
  share *not connected; connect it, then switch back to the app*. While the
  launch lost it, each focus that gets `library_op` resolves again
  (`resume_again` in `lib.rs`); a changed status (regained, or another
  reason) with nothing settled sends a 0/0 `library-settled`, so the status
  line rereads. A stored bookmark that fails is made again from the path
  when the folder there holds bmlib's marker (`remake`, #130; never an empty
  mount point), and a remake that fails too says so in the reason. A start
  that fails in a sandbox (`SANDBOX_ENV`) is lost; what it learnt is written
  in one transaction (PR #127). A failed write or a stale bookmark not
  renewed keeps the access, with why in `Resumed::Held`'s `unsaved`, held in
  `AppState.library_launch_note` (learnt at launch, or by a focus's
  `resume_again`), shown as `LibraryStatus.launch_note` on the status line
  and in Settings, and forgotten once the setting changes (#130). Why a
  bookmark is lost is also stored as `library_bookmark_fault`, read only
  without a launch note. A lost reason is always the launch note (written or
  not; shown while the folder is still usable too). Every report of the
  library takes the note (`configured_with(…, launch_note)`, via
  `AppState::launch_note`): an unreachable reason ends *; at launch: ‹note›*,
  over any stored fault (an earlier launch's, #132), and the status carries
  `launch_note` only while the library is reachable (#134). `resume` fails
  only reading the settings, so its note is the database failure.
  `AppState.library_access` holds the access until exit. Entitlements:
  `files.user-selected.read-write`, `files.bookmarks.app-scope`. Library
  reads turn materialisation off, so an evicted file is `NotDownloaded`
  (bmlib #769).
- **2c, the manual clear** (PR #123): `library_clear.rs` (`list`,
  `remove`); commands `library_papers`, `remove_library_paper`,
  `remove_library_file`, each holding `library_op`; a paper's users are its
  records' (`ListedPaper.used_by`), `users_unknown` when the scan could not
  read one; `LibraryPapers.svelte` with an in-page confirmation. A reopen
  brings a library source's bytes back only when the library does not hold
  the file whole (`library::holds`, PR #125).
- **Settle reports** (PR #144): what a settle tells before the webview
  listens is held (`SettleInbox` in `lib.rs`, merged) and handed over by
  `library_settle_listening`, which App calls once subscribed; Settings
  rereads on `library-settled`. `choose` stores the folder by its real path
  (`real_folder`; Windows keeps the spelling, a `shortcut:`), and compares
  it with a setting or marker check stored before then as `resume` does
  (`moved`, and the spelling given), so a link remembered earlier is still
  the same folder.
- **A folder that does not answer** (#149): `library::answers` looks at
  the library folder on a thread of its own and gives up after
  `LIBRARY_ANSWER_TIMEOUT` (5 s), first thing in `resume` (lost, *did not
  answer within 5s … check that it is connected, then switch back to the
  app*) and in `configured_with` (unreachable), so a share still mounted
  while its server is gone holds neither the launch's main thread nor
  `library_op` for longer. A caller that finds a look already running waits
  for its answer for what is left of the 5 s (callers without
  `library_op`, such as `library_status`, run beside a settle); one that
  finds it past the 5 s is told at once. No second look starts (one thread
  per such share). The launch's resume stays
  on the main thread: off it, an open could take `library_op` before the
  bookmark's access started. A read after the check can still block if the
  share dies between the two (`shortcut:` the check covers the first look
  only; bound the reads if it shows in use).
- **Left, minor, by choice**: a failing source is tried on every focus (so
  a fix made in Finder is picked up; one scan each); the clear lists folders
  of one paper apart (group them if split papers turn up), shows bmlib's
  `Debug` text for scan problems (surface diagnostics while developing), and
  a reopen puts back only the file the text was read from.
