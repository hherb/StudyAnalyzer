# HANDOVER

_Last updated: 2026-11-06 (this session: #71, hand-off receipt on Linux and Windows; earlier today the API-key store moved to bmlib, bmlib #634, PR #687, here in PR #92, merged; the 2b click-through under way)._

Working notes: what is in flight, the calls later phases build on (*Phase
2a*–*2d*), what is next and what the maintainer owes. Planned work not yet
started is in `ROADMAP.md`.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*. Phase 2 is four plans (2a scaffold, storage
  and intake; 2b Interrogator; 2c PICO; 2d hand-off), the maintainer's call
  (2026-10-29).
- **Phase 1 is done**: every bmlib item (spec §7, plus item 11) is merged,
  the last on 2026-10-31 (bmlib PR #586), all in **bmlib 0.12.0 / Rust crate
  0.3.0** (published). The git pin is past that tag (below); it can move to a
  crates.io release that includes it (spec §11). bmlib's own HANDOVER, ROADMAP
  and CHANGELOG hold the detail.
- **The bmlib pin (`rev` in the root `Cargo.toml`) is `26f69db9`**, the
  branch of bmlib PR #687 (#634, the key store), unmerged: **merge #687,
  then re-pin to its merge on main**. Every other pinned bmlib PR is merged
  (bmlib's CHANGELOG has them); the ones this code leans on are named below.

  A stored PDF text older than `pdf-7` (bmlib #636) or Markdown text older
  than `markdown-2` (#548, #549) re-derives on its next open unless a
  conversation turn or a PICO citation points into it (`open::has_dependents`).
- **Phase 2 is built and merged**: 2a (PR #10), 2b (PR #51; follow-ups
  #54, #64), 2c (PR #59), the quote guard (PR #62: `MIN_QUOTE_SIMILARITY`
  0.99, a refused quote's reason stored and shown) and 2d (PR #75; its
  reviews' #67–#69 and #72 fixed in PR #77). Each plan in
  `docs/superpowers/plans/` lists its deviations under *Changed while
  building*. The click-throughs are owed.

## Phase 2a (merged)

PR #10 (2026-10-29; plan `docs/superpowers/plans/2026-10-29-phase2a-scaffold-and-intake.md`):
open from a file, a paste or a PMID/DOI/PMC ID; full text (JATS, else a PDF;
never the abstract); strict decoding (BOM → UTF-8 → declared → chardetng,
shown); canonical text; `assess_full_text`; SQLite (WAL, a connection per command).

Calls made while building, for later phases to know:
- **Identifiers** are stored canonical: PMID digits without leading zeros or
  `PPR`+digits; DOI lower-cased; PMC `PMC`+ASCII digits without leading zeros.
  A JATS source's own identifiers are recorded too, so a paper opened by PMID
  and later by its DOI is one paper. An identifier another paper holds is
  skipped with a warning and a notice naming that paper (see Notices), except
  on `attach`: a JATS file whose own identifiers contradict the paper or
  belong to another paper is refused (`store::identifier_conflict`,
  `OpenError::WrongPaper`). Since #25:
  - `open` refuses a document whose identifier contradicts the stored value
    of the paper it matches (`store::contradiction`, `OpenError::Mismatch`,
    carrying the document's own notices). Nothing of the document is stored.
    On a lookup the paper needs full text, `reason` says why, and the reason
    is kept on the paper (schema migration 4: `papers.refusal`) for when it
    is reopened from Papers.
  - A lookup lands on the paper of the identifier typed, not on whichever
    paper the fetched document's other identifiers name. So a fetched value
    that differs from the typed one, or from one the typed paper already
    holds, is a mismatch, and a repeated lookup lands in the same place.
  - The debugging setting `KEEP_MISMATCHED_KEY` (Settings;
    `OpenContext.keep_mismatched`) opens such a document on the first other
    stored paper its identifiers name that it does not contradict, else a new
    one, with a notice naming the paper it contradicts. On a lookup the typed
    paper keeps a note of where it went.
  - **The same document again stores no copy** (#27): when the paper's
    latest canonical text equals the new one and came from a source of the
    same kind and bytes, `open::persist` stores only this open's notices, the
    title and opened time, so the text and its "this is the whole paper" stay.
    When that text is an older converter's reading of the same source, the
    new text is stored on it and keeps the confirmation (#43;
    `StoredCopy::Older`). With no converter bump only a rename that changes
    the text (a plain text's name is its title) is stored anew; after a bump a
    renamed file of any kind is (a `shortcut:` in `stored_copy`). A document
    stored anew over a confirmed text says so (`REPLACED_CONFIRMED_NOTICE`).
    A stored text that cannot be read no longer blocks a file open.
  - **A file or paste without identifiers finds its paper by its bytes**
    (#44): it lands on the paper that last stored a source of the same kind
    and identical bytes (`store::holder_of_source`), else a new paper. Other
    bytes, even a fragment of the same text, make another paper.
- **A lookup asks for the record first** (#36): `retrieval::resolve` calls
  bmlib's `lookup_record` (PubMed, OpenAlex, Europe PMC as far as the
  identifiers allow; a PPR accession as `ppr_id`, never as a PMID) and logs
  each source's outcome. A source whose request failed or whose answer did not
  read is *unanswered*; the lookup is retried only while the record so far
  leaves nothing to ask the chain by. `request_for` builds the
  `FullTextRequest`: the identifiers typed win, the record's fill what they
  lack once they pass `identifiers::check_fetched`, and its `fulltext_sources`
  go along as they are (typed since bmlib #624). `Resolved::notes` names, for
  the user, a preprint's journal version, a source that served another paper,
  an invalid or differing record identifier, and a refused lookup; they are
  notices of the document stored on the paper typed, or end the `NeedsFullText`
  reason and are kept in `papers.refusal`. The record's title names a new or
  untitled paper, and is **not** the converter's title hint. A PPR no source
  links full text for is `NotFound`; any `NotFound` or `Unreachable` while a
  record source was unanswered is `Unreachable`. `Identifiers` still keeps a
  PPR in `pmid` (a `shortcut:`); only `retrieval::split_pmid` tells them apart.
  The record's identifiers are not stored on the paper (#39).
- **Retrieval** tells "no free full text" (not retried) from "could not reach
  the sources" (retried) by bmlib's types (#565, #603):
  `FullTextError::Exhausted` with no `faults` is final, with faults a retry;
  an `Ok` that is only an abstract or a bare link with non-empty `tier_faults`
  is an outage (`retrieval::faulted_exhaustion`). Faults behind full text or a
  PDF, and all of bmlib's log lines, go to the app log. Only `"No
  identifiers provided"` is still matched by its wording.
- **Re-derive**: a newer converter re-derives from the stored source (file
  name as title hint, "this is the whole paper" kept) only while nothing
  points into it (`open::needs_rederive`'s `has_dependents`, *Phase 2b*). A
  failed attempt is recorded (migration 2: `rederive_failed_version`,
  `rederive_error`) and not repeated until the
  converter changes again (#22), the old text opening with a notice; a
  scratch-file or PDF engine fault is shown, not recorded.
- **HTML pages** over 2 M code points or parsing past 30 s are
  `IntakeError::Html` (paste instead); a re-derive records `TooLarge`, never the machine-dependent
  `TimedOut`. The cap refuses a cheap `data:`-image page too (bmlib **#656**).
- **PDF faults** (#11, with bmlib #598): `IntakeError::Pdf` holds bmlib's
  `PdfError`; `IntakeError::is_the_documents` says whether another file is
  the remedy. Upload maps a file fault to `invalid_input` and an engine fault
  to `internal`. A lookup keeps the paper with "could not be read" only for
  the document's own faults; an engine, scratch-file or canonical-text fault
  is an error, storing nothing. A failed read's message ends with what MuPDF
  said (`PdfError.engine_messages`, bmlib #619, runs folded as in
  `engine_notice`); a `/Title` bmlib could not read is logged, not shown.
- **Notices**: `OpenOutcome::Ready.notices` carries what the user should know
  about an open, shown in the paper banner: a failed re-derive, a JATS file's
  own identifier that is invalid (`intake::jats_identifiers`) or clashes with
  the paper's or another paper's (`store::add_identifiers` returns them), a
  mismatch kept under the debugging setting, or what MuPDF read past without
  failing (`intake::engine_notice`: a repaired xref, a page cut short; a run
  of one repeated message is given once with its count). A successful
  re-derive puts its own engine notice, or none, in place of the stored one
  (found by `config::ENGINE_NOTICE_LEAD`). The notices of an open that stores
  a source are stored with it (migration 3: `sources.notices_json`) and shown
  again on every open of a text derived from it, as written then. If the
  paper view fails to load right after an open, they go into the error banner.
- **`retry::with_backoff`** blocks its thread (use `spawn_blocking`); 2c wraps
  PICO's Rust `chat()` calls in it while **bmlib #561** stands.
- **A lookup has a total limit and a Cancel** (#7): `limit::Limit`
  (`RETRIEVAL_DEADLINE`, 120 s, a shared cancel switch) and
  `limit::LimitedClient` (refuses a request once stopped, clips each timeout;
  a running request finishes, at most bmlib's 45 s). A failed free-PDF
  download with no JATS is `Unreachable` (retryable) or `NotFound` (final),
  naming the link, never "no full text". A cancel stores nothing
  (`OpenError::Cancelled`) and stops MuPDF mid-page (`Limit::pdf_limit`).
  Navigation stays disabled while busy so Cancel stays reachable;
  `AppState::begin_lookup`/`end_lookup` track the one lookup.
- **Small hardening** (#13): a newer app's database is refused
  (`StoreError::Newer`); `confirm_full_text` confirms `PaperView.canonical_id`
  only; a chosen file is read at most one byte past `MAX_PDF_BYTES`; `view::utf16_offsets`/`span_to_utf16` are the only
  code-point → UTF-16 conversion.
- **The Rust↔TS JSON contract is pinned** (#12, #17, #19, #41):
  `src-tauri/src/contract.rs` serialises every shape `src/lib/api.ts` mirrors
  into `src/lib/fixtures/ipc_contract.json`; `src/lib/contract.test.ts` checks
  keys and value types at compile time (`check:web`), tags and error kinds at
  run time. **A new shape** is a key in `contract()` plus `keysMatch`/
  `valuesMatch` rows. `commands()` lists each command's argument keys and JSON
  types, sent to the real handlers through Tauri's mock runtime. **A new
  command** goes in `lib.rs`'s `handler()`, `commands()`, `api.ts` and the
  vitest call list together. After a deliberate change: `UPDATE_CONTRACT=1
  cargo test -p studyanalyzer contract`, then fix `api.ts`. The frontend
  refuses an `ErrorKind` it does not know. The banner's missing headings come
  from Rust (`FullTextStatus.missing_headings`, pinned to bmlib's private rule
  by `missing_headings_agree_with_bmlib`).

## Phase 2b (merged)

**Phase 2b landed** as PR #51 (2026-11-02), plan
`docs/superpowers/plans/2026-11-02-phase2b-interrogator.md`. What it does:
ask a question of the paper with a chosen model, stream the answer, check
every quote against the paper and highlight the paper's own span;
conversations stored per canonical text; whole-paper or retrieval mode by the
model's window; the search model downloaded on first need; provider, model,
window and API keys in Settings. The click-through is owed (below).

Calls later phases build on:
- **Dependents**: a canonical text with a conversation turn
  (`conversations::has_turns`, the `has_dependents` answer) is never
  re-derived, and the same file opened again under a newer converter keeps it
  (`open::persist` turns `StoredCopy::Older` into `Same`). A different document
  stores a new text; earlier conversations stay on their text and are
  read-only. An empty conversation is no dependent; a chunk set is none
  either. A chunk set is keyed by canonical text id (`passages::index_key`),
  and `store::save_canonical` drops every older text's set when it stores a
  newer one (#53): only the latest text is searched. An ask that indexed an
  older text drops its set again once its search is done, if a newer text was
  saved meanwhile (`store::drop_superseded_chunk_set`, #55; a save between
  the ask's indexing and its search still fails that ask, **#65**). A PICO
  citation keeps its text too (*Phase 2c*).
- **Quotes**: the model writes `<q>exact words</q>` inline (no tools on a
  stream). `answer::parse_answer` splits pieces; `answer::locate(pieces,
  canonical, min_similarity)` runs `find_passage` on the whole canonical text
  and returns `Result<Located, InvalidSimilarity>`. An unlocated quote keeps
  its place with `number: None` and `find_passage`'s near-miss similarity
  (shown as *N quotes could not be found in the paper*, each with its closest
  match, `closestNote`, rounded down so a near miss never reads as passing). The shown wording is
  `canonical.span_text(span)`, never the model's.
- **`MIN_QUOTE_SIMILARITY` is 0.99** (was 0.999 until bmlib #632's guard),
  set by `tests/quote_threshold.rs` on `tests/fixtures/quotes.json` (106
  labelled cases): the guard catches every changed numeral, negation and
  direction, so only rejects it lets through bound the threshold
  (`print_margins`). An arm swap with the numbers in place passes it (0.9856,
  **bmlib #651**, as does a spelled-out number) and lies among benign edits
  (dropped, doubled or added words, `+/-` for ±: 0.9758–0.9910), so by plan 2b
  Task 4's overlap rule it is 0.99 and seven true accepts carry `"expected":
  false`. A refused quote's reason is stored (`quotes.contradiction`,
  migration 7) and shown beside it (`contradictionNote`; an unknown reason
  raw). The guard only checks a window that reached the threshold, so a short
  changed quote is plain "not found". PICO uses the same value; its agent
  drops the reason (**bmlib #647**).
- **`interrogate::ask(ctx, paper, question, stop, on_event)`** (and
  `ask_loading_embedder`, which loads a downloaded model on a new launch and
  asks once more) returns the conversation id. A stopped or broken stream is
  stored as a `Stopped` / `Failed` turn with what arrived and no citations
  checked; a failure or a stop before any text stores nothing (a stop is
  `Cancelled`). An answer cut at `ANSWER_TOKENS` (`LENGTH_STOP_REASONS`) is
  `Complete`, quotes located, with the cut-off notice in `error` (so read
  `status`, not `error`, to tell a failure). `AskError`: `NeedsModel`,
  `EmptyQuestion`, `NoFullText`, `NotFullText`, `NeedsEmbedder`, `TooLong {
  window, has_history }`, `Cancelled`, `Chat`, `Passages`, `Store`, `Embedder`,
  `Internal`. `CommandError::from_ask` maps them (`Chat` through
  `from_chat`: a missing or refused key, HTTP 401/403, is `needs_api_key`,
  no such model, HTTP 404, `invalid_input`, others `llm`); `TooLong` is
  `invalid_input`, naming the fix.
- **Events go over a Tauri `Channel`**: `Channel<AnswerEvent>`
  (`CheckingModel`, `Indexing { done, total }`, `Waiting`, `Delta { text }`)
  for `ask`, `Channel<DownloadProgress>` for `download_embedder`. The
  frontend drops events from a run that is no longer the live one.
- **The Interrogator always shows** a paper with text (#50): a text that
  neither passed the full-text check nor was confirmed shows the earlier
  conversations, with `askBlockedNote` in place of the question box; `ask`
  refuses it in Rust either way. So the paper screen has one layout. The
  banner's confirm waits while an earlier text is on screen (it names the
  latest), and the Interrogator is keyed on the text, so a newer one remounts it.
- **A paper view at a given text**: `view::stored_paper_view` / the
  `paper_view_at` command shows the text an earlier conversation was held on;
  `interrogator_view` and `conversation_view` carry `TurnView`s with
  UTF-16 citation spans.
- **Stop**: `StopSlot` (`src-tauri/src/state.rs`) gives every ask and every
  download a fresh `AtomicBool`, so a stop meant for an earlier run never
  stops a later one (`stop_answer`, `cancel_download`). *Stop* works while a
  local model still reads the prompt; the server reads on (bmlib **#649**).
  A DB connection per command, never held behind a lock while streaming.
- **The embedder** (`embedder.rs`): `ModelSlot<E>` holds the pinned
  `EMBEDDING_MODEL` (F16, 1.1 GB) under the app data directory's `models/`,
  verified and loaded once per launch, downloaded with backoff, unloaded on
  `RunEvent::Exit` (llama.cpp aborts at exit on Metal with a model loaded).
  **`needs_embedder` is the authority**: a corrupt file of the pinned size
  reads `downloaded: true`, so the UI offers the download anyway. The `embed`
  feature pulls llama.cpp and CMake into every target (a note for phase 4).
- **Keys**: bmlib's `secrets::KeyBundle::keyring(KEYCHAIN_SERVICE)` (bmlib
  #634): one item `api-keys`, `{provider: key}`, read once per launch.
  `llm_settings::key_source` says `keychain`, `env` or none; a failed read
  degrades to the env var (`LlmSettings.keychain_error` is shown).
- **The window**: `prompt::running_window(setting, maximum)` = the smaller
  of those known. Fit estimates `FIT_CHARS_PER_TOKEN` (3) code points a token
  plus `ANSWER_TOKENS` (4096) for the answer. Unknown window and no setting →
  retrieval mode, checked against `UNKNOWN_WINDOW_FIT_TOKENS` (8192) and sent
  with no `num_ctx`; otherwise `num_ctx` is the running window on every
  request. Retrieval: the question alone, `RETRIEVAL_PASSAGES` (8). A prompt
  that still does not fit is refused, never cut. Temperature 0, no other
  parameter; per-model parameters are **#49**.
- **The model picker** (2026-11-06, bmlib #680): Settings' model field is a
  text input with a `<datalist>` of the provider's models (`list_models`
  command → bmlib `LlmClient::list_models`: every page, sorted, nothing
  filtered), re-listed on a provider change, on open and after a key is saved
  or removed; "Listing…" shows while it runs (bmlib retries each page). A
  failed listing is a note under the field (a blank, colon-bearing or unknown
  provider is `invalid_input`, a missing or refused key `needs_api_key`,
  `CommandError::from_list_models`); an empty listing is a note and an `info`
  log line; a typed name always works. A datalist filters by what is typed, so a filled field
  shows only its match until cleared.

## Phase 2c (merged)

A *PICO* tab between *Interrogator* and *Original*: population,
intervention, comparison, outcome, study type and sample size, each part
backed by quotes `find_passage` located, highlighted on the paper. Calls the
later analysis tabs build on:
- **Storage** (`pico_store.rs`, migration 6): `pico_runs` → four
  `pico_parts` in `PICO_PARTS` order → `pico_citations` (spans only; the
  wording is re-read from the text). `PicoOutcome` is `NotSuitable(check)` or
  `Extracted { check: Option, extraction }` (`None`: forced); `save_run`
  refuses a shape its check contradicts, a text not the paper's and a span
  outside it; `run()` reads every contradiction (and a column of the wrong
  type) back as `Corrupt`. Every run is kept; `latest_run` is the newest on a text.
- **Dependents**: `open::has_dependents` = `has_turns || has_citations`; a
  run citing nothing is none. A new analysis that stores spans adds its own
  check there.
- **Budget, never a cut** (`pico::budget_chars`): window (else
  `UNKNOWN_WINDOW_FIT_TOKENS`) less `PICO_MAX_TOKENS`, the longest prompt and
  the title, times `FIT_CHARS_PER_TOKEN`, saturating; under
  `PICO_MIN_BUDGET_CHARS` is `TooLong`. Over it the paper is condensed once
  (`prepare`), whole or failed (`with_continue_on_error(false)`); quotes are
  located in the whole text. **Progress** (bmlib #609): the condenser's
  reports become `PicoEvent`s (`Condensing { batch, batches, level }` per
  part sent, `Condensed` when it ends, failed or not, and `Other { stage,
  message }` for a stage this version does not know, logged and shown;
  `PicoEvent::from_progress`) through `PicoContext.on_event`, sent on `run_pico`'s
  `Channel<PicoEvent>`; the tab shows `runningNote` (`pico.ts`), and the
  host drops a late event from an earlier run. The check reads the abstract (`abstract_text`,
  JATS sub-sections included) and says so (`from_abstract`).
- **Retry and stop**: `RequestFailed { retryable: true }` retried by
  `with_backoff` (a 401/404 is `retryable: false`, bmlib #659, and maps by
  its `status` as a chat failure does, #662), never once
  stopped. A refusal while a long paper is condensed carries its status too
  (bmlib #616, PR #666, in our pin). The window lookup
  before a model's first call (`cached_window`) maps a refused key (401,
  403) to `needs_api_key` by its status (bmlib #664), for a chat too; a
  `Transport` 404 stays `llm` (a wrong address, not a missing model). The
  stop is read after every call and before `save_run`, so a stopped run
  stores nothing. `run_pico` claims `AppState.pico_stop` with
  `StopSlot::try_begin` (one run at a time) and frees it through
  `state::Release` in `commands::with_claim`, a panic included. A stop command
  returns whether it reached a run; `api.stopWhilePending` re-sends one Tauri
  handled before its async command registered (#61; the answer, the download
  and the lookup's Cancel too), and checks that its own run is still pending,
  so a later run never gets it. `pico_store::run` hands back the text it read
  (`StoredPicoRun.text`), so a view reads it once (#58). A missing key is
  refused before anything is sent (`llm_settings::missing_key`).
- **The tab host keeps the run's state** (`Interrogator.svelte`): the tabs
  are `{#if}` branches, so `PicoTab`/`PicoCard` only show. Auto-run once per
  host mount (`shouldAutoRun`); *Extract PICO* whenever there is no run on an
  askable text and none going (`offerRun`). A PICO run makes the host `busy`
  (App's navigation waits) but not the chat. `older` follows the tab shown.
  Where a failed run's error goes is `runFailure` (`pico.ts`); a stored run
  whose view fails says it was stored, and the old card never stands in for it.
- **The card shows what the model said** (#60): each part's confidence beside
  its heading (*confidence not given* when the model gave none), a *Mean
  confidence* row (bmlib's mean of the reported parts), and after the check's
  notes, on both shapes, `findingsNote` (*Check: intervention
  study yes, comparison group not given, confidence 90%*).

## Phase 2d (merged)

A paper arrives as a `.studyanalyzer` file or a `studyanalyzer://open?…`
link, from the Finder, a drop, the command line or another app; the second
launch hands over to the first (single instance). Calls phase 3 builds on:
- **The format** is `docs/handoff-format.md` (for sender authors), pinned by
  `crates/studyanalyzer-core/tests/fixtures/handoff/` (`cases.json` and five
  `.studyanalyzer` files, read by the Rust tests). A sender must pass them.
- **Grants**: the webview can neither choose nor open a path. `open_file` /
  `attach_file` take grants only, Rust shows the dialog, and the dialog
  permission is gone. Tauri 2.12 still reports dropped paths to the webview
  (`tauri://drag-drop`); `dragDropEnabled` stays on, as Rust's
  `WindowEvent::DragDrop` needs it.
- **Arrival → `handoffs` table (migration 8) → `handoff-received` → opened
  when idle.** Every route stores the file's bytes at once
  (`handoff_store`, `receive_file` / `receive_args` / `receive_links`); a
  refused one is stored as a refusal row with its reason. The webview opens
  the newest row only when `busy || interrogatorBusy || flows > 0 || error`
  is false and Settings is not shown (`nextHandoff`); the others wait in the
  *Handed over* list. A hand-off with no identifier and no text, or over
  `MAX_HANDOFF_BYTES`, is refused with the reason shown, never silently.
- **When a row goes** (`keeps_row`, on the `CommandError.kind`): after the paper
  loaded. `needs_email`, a lookup needing full text and `OpenError::Mismatch` (read
  before it is `invalid_input`: *Keep mismatched*, then *Open*) keep it; an unknown id
  or refusal row is `invalid_input`, left (shown, then discarded); a gone row's discard is `Ok`.
- **macOS routes**: a Finder double-click is `RunEvent::Opened`, which can
  come before `setup` on a cold launch; the path then waits in the
  `EarlyOpened` queue that `setup_handoffs` drains. The deep-link plugin
  takes links (its `get_current()` holds one that came before `setup`) and
  forwards `file:` URLs too, which `receive_links` skips so a file is
  received once. Deep links work only from `/Applications`.
- **Single instance and arguments**: `second_instance` resolves a relative
  path against the second process's own directory; launch arguments are read
  with `args_os()` (lossy, never a panic). `read_chosen_file` takes only a
  regular file (a FIFO would block a thread) and a cap. Arrival failures
  show in a non-blocking dialog (`report_arrival_failure`). Files that come
  before `setup` ran (macOS's early opened files, any second launch's
  arguments) wait in `EarlyOpened` with their route; `setup` takes them once
  the state is made, or reports them as not received when it failed, and
  taking closes the queue. A link argument is the deep-link plugin's only as
  the sole argument on Linux and Windows; any other is reported
  (`unreceived_links`). `register_all` runs off the main thread and a failure
  is shown (#71; Linux and Windows unrun; the link-side race is **#94**).
- **`OpenRequest::Handoff`** carries identifiers, the optional document,
  title and the file's own notices. `OpenError::HandoffMismatch { sender,
  why, notices }` is a document whose own identifiers differ from those sent
  with it (nothing stored; the hand-off's and the document's notices end its
  message, as `Mismatch`'s do, #68); a link names `LINK_ORIGIN` as sender.
  Without a document it runs `open_by_identifiers`, as a typed identifier
  does, and the other identifiers sent go to the paper found
  (`add_identifiers`), so a later open by any of them lands on the same
  paper. Any other failure of `open_handoff_with` ends its message with the
  hand-off's notices, since the row usually goes with it.
- **Labels** (`sender`, `full_text.file_name`, `full_text.source`) refuse a
  control character and the bidi embeddings, overrides and isolates
  (`config::BIDI_CONTROLS`, #69); a title may carry them, and
  `handoffCaption` shows it inside FSI…PDI so they stop before the sender.

## Next

1. **What a page cut short should be** (**bmlib #605**, the maintainer's
   call; today a notice).
2. **#31** (two transitive Dependabot alerts) waits on Tauri and
   vite-plugin-svelte releases. A Svelte component documents itself in a
   `/** … */` at the top of its script (#14).
3. **Open issues.** bmlib's are in ROADMAP (*Phase 1*); the nearest are
   **#647**, **#651**, **#577**, **#665**, **#686** (Windows: Rust and
   Python name the key item differently), **#688** (Rust's `eprintln!`
   warnings never reach the app log; `save_api_key` logs a damaged item
   itself meanwhile) and **#689** (a non-UTF-8 key item cannot be replaced by
   saving; damaged vs locked cannot be told apart). Ours: **#49** (per-model parameters), **#65** (an ask fails if a
   newer text is saved mid-ask; unreachable from the UI now), and from 2d's
   reviews **#70** (memory at the cap, phase 4), **#94** (a second launch's
   link during setup can replace or duplicate the launch link), **#74** (iOS: the keyring crate's v1 stores
   need its `protected` store; now through bmlib's `keyring` feature), **#78** (decide: refuse bidi marks and invisible format characters
   in labels too).
4. **Phase 3** (MedicalFactChecker's sending side) once 2d is
   clicked through; it builds on *Phase 2d* above.

## Owed by the maintainer

- **Click-through of phase 2b, under way** (2026-11-06): round 1 (local
  model: citations, follow-up, unanswerable question, status line, *Stop*)
  passed; no key is in the database (checked). Next: round 2 (cloud model and
  keys) with the new model picker (provider switch lists; no key → a note;
  key saved → the list), then retrieval mode and the first download (the
  search model is not downloaded yet). **bmlib #681** (Rust's Gemini chat
  path has an extra `/v1`) is decided by one live Gemini chat in round 2.
  Keys now go through bmlib's store (same item, same crate): round 2 checks
  it, and a key saved before still reads. With the keychain unreadable (for example a
  damaged item), the Settings banner names the item and says a save replaces it.

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
  dev`, a real cloud model and a local Ollama model, beyond round 1 (above): *Stop* during *Finding passages* and while a local model reads a
  long prompt (no text yet) (nothing stored, question kept);
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
  once); `studyanalyzer <file> studyanalyzer://open?pmid=…` (the link's dialog);
  an AppImage without xdg-utils (the registration dialog).
- **Decide #73: a hand-off claiming a PPR preprint accession in its PMID field
  whose JATS has a numeric PMID** is refused as a mismatch (the rule
  `store::contradiction` applies); should a preprint's journal version land
  instead?
- **Decide: a hand-off arriving while Settings is open** waits until
  Settings is left (the plan said it would navigate away).
- **Decide bmlib #651 and #647**: whether the guard should catch a swap of
  named arms and a spelled-out number (then `MIN_QUOTE_SIMILARITY` can drop
  towards 0.98 and keep dropped-word quotes), and whether PICO records a
  refused quote's reason.
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
