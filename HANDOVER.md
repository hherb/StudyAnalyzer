# HANDOVER

_Last updated: 2026-11-14 (this session: library step 2c, the manual clear, PR #123. Earlier sessions are in git history)._

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
- **The bmlib pin (`rev` in the root `Cargo.toml`) is `33f859c4`, bmlib
  PR #773's merge commit** (2026-11-14), past the published **bmlib
  0.13.0 / Rust crate 0.4.0** (2026-11-10). It brings the library steps
  1a–1c (#754, #763, #764) and their follow-ups #767–#769 (`find_digest` by
  kind, a durable `save`, an evicted file read as *not downloaded*), PMC's
  AWS Open Data bucket as a JATS source and the PDF beside JATS checked by
  md5 (#749, #752; a mismatch is `checksum-mismatch`, worded by
  `retrieval::describe_download_failure`), and evidence weight phase 1
  (#743, for the later *risk of bias* tab). What each pin brought is in git
  history (`git log -p Cargo.toml`). Every pinned bmlib PR is merged; open
  there now: #774 (the FFI's Kotlin name clash) and #775 (MECA over S3),
  other sessions' work.

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
  - `open` refuses (stores nothing of) a document whose identifier
    contradicts the stored value of the paper it matches
    (`store::contradiction`, `OpenError::Mismatch`); on a lookup the reason is
    kept on the paper (migration 4: `papers.refusal`).
  - A lookup lands on the paper of the identifier typed, so a fetched value
    that differs from the typed one, or from one that paper holds, is a
    mismatch, and a repeated lookup lands in the same place.
  - Debugging setting `KEEP_MISMATCHED_KEY` (Settings): such a document opens
    on the first other stored paper its identifiers name and do not
    contradict, else a new one, with a notice naming the contradicted paper;
    on a lookup the typed paper notes where it went.
  - **The same document again stores no copy** (#27): same text, kind and
    bytes → `open::persist` stores only this open's notices, title and time.
    An older converter's reading of the same source gets the new text, keeping
    the confirmation (#43). A rename that changes the text is stored anew (any
    rename after a converter bump: a `shortcut:` in `stored_copy`), saying so
    over a confirmed text (`REPLACED_CONFIRMED_NOTICE`).
  - **A file or paste without identifiers finds its paper by its bytes**
    (#44): it lands on the paper that last stored a source of the same kind
    and identical bytes (`store::holder_of_source`), else a new paper. Other
    bytes, even a fragment of the same text, make another paper.
- **A lookup asks for the record first** (#36): `retrieval::resolve` calls
  bmlib's `lookup_record` (PubMed, OpenAlex, Europe PMC; a PPR as `ppr_id`),
  retried only while a source is unanswered and nothing else can be asked.
  `request_for`: typed identifiers win, the record's fill gaps once
  `identifiers::check_fetched` passes them. `Resolved::notes` become notices on
  the typed paper or end the `NeedsFullText` reason (`papers.refusal`). A PPR
  still sits in `Identifiers.pmid` (a `shortcut:`; `retrieval::split_pmid`
  tells them apart). Record identifiers are not stored on the paper (#39).
- **Retrieval** tells "no free full text" (final) from "could not reach the
  sources" (retried) by bmlib's types (#565, #603): `Exhausted` with `faults`,
  or an abstract-only `Ok` with `tier_faults` (`retrieval::faulted_exhaustion`),
  is an outage. Only `"No identifiers provided"` is matched by its wording.
- **Re-derive**: a newer converter re-derives from the stored source only
  while nothing points into it (`open::needs_rederive`'s `has_dependents`); a
  failure is recorded (migration 2) and not retried until the converter
  changes again (#22); a scratch-file or engine fault is shown, not recorded.
- **HTML** over 2 M code points or 30 s is `IntakeError::Html` (a re-derive
  records `TooLarge`; the cap also refuses `data:`-image pages, bmlib **#656**).
  **PDF faults** (#11): `PdfError::is_the_documents` says whether another file
  helps; a failed read ends with MuPDF's messages (#619).
- **Notices** (the paper banner: a failed re-derive, a clashing JATS
  identifier, a kept mismatch, what MuPDF read past) are stored with the
  source (migration 3) and shown on every open of a text derived from it.
- **`retry::with_backoff`** blocks its thread (use `spawn_blocking`); 2c wraps
  PICO's Rust `chat()` calls in it while **bmlib #561** stands.
- **A lookup has a total limit and a Cancel** (#7): `limit::Limit`
  (`RETRIEVAL_DEADLINE`, 120 s, shared cancel) and `limit::LimitedClient`
  (refuses once stopped, clips each timeout; a running request ends within
  bmlib's 45 s). A failed free-PDF download with no JATS is `Unreachable`
  (retryable) or `NotFound` (final), naming the link, never "no full text". A
  cancel stores nothing (`OpenError::Cancelled`) and stops MuPDF mid-page
  (`Limit::pdf_limit`); navigation stays disabled while busy so Cancel stays
  reachable; `AppState::begin_lookup`/`end_lookup` track the one lookup.
- **Hardening** (#13): a newer app's database is refused; a chosen file is
  read at most one byte past `MAX_PDF_BYTES`; `view::span_to_utf16` and
  `utf16_offsets` are the only code-point → UTF-16 conversion.
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
  changed quote is plain "not found". PICO uses the same value; since bmlib
  #647 (PR #105) its agent keeps a contradicted quote
  apart with its reason, and a reported part with no located quote and at
  least one contradicted one is `contradicted`:
  stored in `pico_parts.quotes_contradicted_json` (migration 9), shown on the
  card with `contradictionNote`.
- **The Original tab's PDF beside JATS** (PR #105, bmlib #739). A looked-up
  paper whose text is JATS keeps the PDF bmlib found with it as a source
  naming the JATS's (`sources.pdf_beside`, migration 10; `save_pdf_beside`,
  once per source). `original_pdf` serves the text's own
  PDF, else the one beside its source; `PaperView.has_original_pdf` shows
  the tab (the `paper_view` command goes through `stored_paper_view` for it).
  A PDF that failed to download, or a saved copy that is not a PDF,
  unreadable or over `MAX_PDF_BYTES`, is not stored; why is a notice stored
  with the JATS's source, so a reopen says it again. A JATS kept apart from
  the paper typed (`keep_mismatched`) keeps no PDF: bmlib sought it by the
  identifiers typed. Not covered: a paper opened before this (a lookup of a
  paper with text reopens it from storage, and bmlib serves a cached JATS hit
  as it is, without a PDF), an upload or hand-off of JATS (no lookup, so no
  PDF), a JATS that cannot be read drops the PDF beside it (**#106**, the
  maintainer's call). Since bmlib #745 the notice names a failed search
  (`PDF_SEARCH_FAILED`) and, when `pdf_failure_may_pass`, adds "The fault
  may be a passing one." It promises nothing of a reopen, which never looks
  the paper up again (the maintainer's call, 2026-11-11). **Bot screens**
  (2026-11-12): MDPI's, Europe PMC's `?pdf=render` and so Unpaywall's links
  answer bmlib 403 while a browser gets the PDF; impersonating a browser was
  refused (it would reach bmlib's bulk callers). bmlib PR #749 (#748, in
  our pin) seeks the PDF first in PMC's AWS Open Data bucket (NLM's, for
  programmatic access), clicked through on its branch: DOI
  10.3390/nu17213408 opened with its PDF from there. A paper stored before
  the pin keeps its 403 notice (a reopen never looks it up again). A paper
  with no PMC ID still meets the screen; an "add the PDF you downloaded"
  route is not built (not filed; the maintainer's call).
- **`interrogate::ask(ctx, paper, question, stop, on_event)`** (and
  `ask_loading_embedder`, which loads a downloaded model on a new launch and
  asks once more) returns the conversation id. A stopped or broken stream is
  stored as a `Stopped` / `Failed` turn with what arrived and no citations
  checked; a failure or a stop before any text stores nothing (a stop is
  `Cancelled`). An answer cut at its `reply_tokens` (`LENGTH_STOP_REASONS`) is
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
  local model still reads the prompt, and since bmlib PR #714 the server
  stops reading it too: the Interrogator streams through an `LlmClient` that
  wraps a plain `UreqClient`, which honours `post_json_stream_with_cancel`
  (PICO does not stream; its stop is read between calls, *Phase 2c*). A
  client that wraps another must forward that method; `LimitedClient`
  (lookups only) refuses streams.
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
  `llm_settings::keychain_advice(kind)` (#689) is what the banner, the ask's
  `KeyUnavailable` and `save_api_key`'s log say helps: a damaged item
  (`UnreadableItem`) is replaced by saving a key, two matching items
  (`AmbiguousItem`) need one removed by hand, anything else needs the
  keychain unlocked.
- **The window**: `prompt::running_window(setting, maximum)` = the smaller
  of those known. Fit estimates `FIT_CHARS_PER_TOKEN` (3) code points a token
  plus the reply's room: `prompt::reply_tokens(provider, window, max_output)`
  = a quarter of the window, clamped to 4096–32768, never above the model's
  output limit (bmlib `ModelLimits`, cached per launch in `AppState.limits`);
  4096 when that limit is unknown, except Ollama. It is also every call's
  `max_tokens` (a thinking model's reasoning counts against it). Unknown window and no setting →
  retrieval mode, checked against `UNKNOWN_WINDOW_FIT_TOKENS` (8192) and sent
  with no `num_ctx`; otherwise `num_ctx` is the running window on every
  request. Retrieval: the question alone, `RETRIEVAL_PASSAGES` (8). A prompt
  that still does not fit is refused, never cut. Temperature 0 (Anthropic is sent
  none, bmlib #709), no other parameter; per-model parameters are **#49**.
- **The model picker** (bmlib #680; a menu since PR #100, the webview's
  datalist offered nothing): Settings lists the provider's models (`list_models`
  → bmlib `LlmClient::list_models`: every page, sorted, nothing filtered) as a
  menu with *Other…* to type a name; a stored model the provider does not list
  reads *‹model› (not listed by ‹provider›)*. Re-listed on a provider change,
  on open and after a key is saved or removed ("Listing…" meanwhile). A failed listing
  is a note under the field (`CommandError::from_list_models`: a bad provider
  `invalid_input`, a missing or refused key `needs_api_key`); an empty one a
  note and an `info` log line.

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
  `UNKNOWN_WINDOW_FIT_TOKENS`) less the reply's room, the longest prompt and
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
  (`handoff_store`, `receive_file` / `receive_args` / `receive_link`); a
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
  `EarlyOpened` queue that `receive_held` drains. Links come the same way
  (`urls_opened`, held in `EarlyLinks` before setup). Deep links work only
  from `/Applications`.
- **Single instance and arguments**: `second_instance` resolves a relative
  path against the second process's own directory; launch arguments are read
  with `args_os()` (lossy, never a panic). `read_chosen_file` takes only a
  regular file (a FIFO would block a thread) and a cap. Arrival failures
  show in a non-blocking dialog (`report_arrival_failure`). Files that come
  before `setup` ran (macOS's early opened files, any second launch's
  arguments) wait in `EarlyOpened` with their route; `setup` takes them once
  the state is made, or reports them as not received when it failed, and
  taking closes the queue. Every link argument is received (`link_args`),
  held in `EarlyLinks` before setup; one that does not parse is stored as a
  refusal. `register_all` runs off the main thread and a failure is shown
  (#71; Linux and Windows unrun).
- **Links by the app's own routes** (#94): on desktop the app does not
  listen to the deep-link plugin's events. Tauri 2.12.5 defers a `listen` or
  emit that finds its handler map locked until an emit runs a Rust handler,
  so a listener registered while a second launch's link is emitted could
  never fire. Links come from the command line (Linux, Windows: launch and
  `second_instance`) and `RunEvent::Opened` (macOS, iOS), held like files in
  `EarlyLinks` (`handoff::Early`, like `EarlyOpened`) until `receive_held`;
  single-instance runs without its `deep-link` feature. The plugin stays for
  `register_all` and Android, where its listener is registered first in
  `setup` and `get_current()` is read after it (`links_at_setup`), with a
  `shortcut:` on the deferral (unrun: Android is not built).
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
2. **#31** (two transitive Dependabot alerts) waits on Tauri 3 (glib) and a
   vite-plugin-svelte without deepmerge (both still so on 2026-11-07). A
   Svelte component documents itself in a `/** … */` at the top of its
   script (#14).
3. **Open issues.** bmlib's are in ROADMAP (*Phase 1*); the nearest are
   **#651**, **#577** (point 3 and dict-shaped quotes stay open). #686 is
   closed and in the pin (the Windows key item's naming; check it on a
   Windows machine). Ours: **#106** (a JATS that cannot be read drops the
   PDF beside it), **#49** (per-model
   parameters), **#65** (an ask fails if a newer text is saved mid-ask; unreachable
   from the UI now), **#70** (memory at the cap, phase 4), **#74** (iOS:
   keyring 4.2's v1 API refuses iOS and its Apple store needs `protected`;
   still so on 2026-11-07, a bmlib change for phase 4), **#78** (decide:
   refuse bidi marks and invisible format characters in labels too).
4. **Phase 3 is now the shared paper library** (design
   `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`, approved
   2026-11-13): one user-chosen folder of papers that every bmlib app reads
   and writes, nothing deleted automatically, then *Open in StudyAnalyzer* on
   it. Four steps (design §8):
   1. **bmlib**, in three PRs: **1a** the `library` module, merged as
      bmlib PR #754; **1b** `FullTextService` on it, merged as #763 (in our
      pin); **1c** the library through bmlib-ffi, merged as bmlib PR #764
      (plans `docs/superpowers/plans/2026-11-1{3,4}-library-1*.md`,
      each with an *As built* note). 1c exposes only the `library` module, not
      `with_library` (MedicalFactChecker fetches in Swift). BioMedLit adds
      `extension LibraryError: CustomStringConvertible {}` for bmlib's error
      text; in Kotlin it is `toString()`. Follow-ups: bmlib **#755** (items
      1–6), **#761** (reads follow symlinks), **#762** (a non-UTF-8 library
      JATS), **#765** (the FFI skips `save_scenarios`), **#766** (`save`
      does not check a folder's JATS; StudyAnalyzer checks it itself).
   2. **StudyAnalyzer**, in four PRs (the maintainer's call, 2026-11-14;
      plans `docs/superpowers/plans/2026-11-14-library-2{a-core,b-setting}.md`):
      - **2a, merged** (PR #116): sources by digest. Calls later PRs build on:
        - **Migration 11** moved the bytes to `source_bytes`; `sources.place`
          is `app`, `waiting` or `library` (`store::SourcePlace`), `sha256`
          the reference, `library_note` why a source still waits.
        - **`library::Configured`** (`None`, `Ready`, `Unreachable`) from the
          settings keys `library_path` and `library_installation`, opened per
          command (`AppState::library`). With a library chosen, every new
          source is stored `waiting` and `open::finish` settles it after the
          commit (`library::settle`). Reads go through `library::read_source`
          (digest checked). `foreign_jats` refuses a folder whose JATS names
          another paper (bmlib #766). An empty folder becomes a library only
          the first time it is opened (one `library_opened:<folder>` key per
          folder, kept when the setting changes). Reads find the file
          by digest **and kind** (bmlib's `find_digest`, since #767); a
          `save` is durable when it returns (#768).
      - **2b, merged** (PR #117): the Library setting.
        Calls 2b′ builds on:
        - **`library_setting.rs`** (core): `status`, `choose(root, bring)`,
          `clear(bring)`, `move_in`, each returning `Changed { copied, moved,
          failed: [PaperProblem] }`. `choose` validates by `Library::open`
          (a folder of other files is `SettingError::Refused`, nothing
          changed); bringing papers from another library reads each
          `library` row back (`store::bring_back`) as `waiting` and settles
          it into the new one paper by paper, and the setting changes only
          after that. With no library set, waiting rows become `app`.
        - **Tauri**: `library_status`, `choose_library(bring)` (the folder
          dialog in Rust; the path never reaches the webview),
          `clear_library(bring)`, `move_into_library`. `AppState.library_op`
          lets one change run at a time; settling on `Focused(true)` uses
          `try_library_op` and skips when nothing waits. Launch and focus
          settles emit `library-settled` (`{moved, left, error}`) when a
          source moved, one waits, or settling failed.
        - **UI**: `LibrarySetting.svelte` in Settings (wording in
          `src/lib/library.ts`); App shows the unreachable status line (read
          at mount, on focus, on leaving Settings, on each `library-settled`)
          and *N files moved into the paper library.*
        - **Every open holds `library_op`** (`with_context`), so an open never
          settles into a library a switch is leaving. Opens wait for each
          other too, and a lookup holds it through its fetch and retries, so
          a change can wait for a slow lookup. A copy reads the old library
          from one scan (`library::read_scanned`); a file the new one does
          not take goes back to the old (`mark_in_library`). Choosing a
          library folder this app opened before that lost its marker is
          refused.
        - **A file a change leaves in the old folder** (declined, unreadable,
          refused by the new one) keeps that folder in its row's
          `library_note` (`store::note_left_behind`, `note_library`), and a
          read that misses the file names it. `Changed`'s `copied` and
          `moved` never overlap. A database failure stops a copy before the
          switch.
        - **Left from review** (minor): the launch settle's
          `library-settled` usually fires before the webview listens, so its
          count is not shown; a source that keeps failing is tried again on
          every focus (a scan and its JATS reads each time); the Settings
          screen is not refreshed by `library-settled`; the same folder by
          another spelling (a symlink, a trailing slash) counts as another,
          so it is copied out and back; the mobile build leaves
          `LIBRARY_DIALOG_TITLE` unused. From the final review (issue
          #118): a database failure after the switch reports the error but
          not what the change had done (the setting is reread, so the new
          folder shows); the setting does not list the files left in another
          folder (a read names it); the focus settle's decisions sit in a
          Tauri closure without a test.
      - **2b′, merged** (PR #120; plan
        `docs/superpowers/plans/2026-11-14-library-2b-prime-bookmark.md`):
        the macOS security-scoped bookmark. Calls later PRs build on:
        - **`bookmark.rs`** (core, `objc2-foundation`): `create(dir)`,
          `resolve(data)` → `Resolved { path, stale, access }`; dropping
          `Access` stops the access. Off macOS both refuse; callers gate.
        - **The setting** stores it as `library_bookmark` (base64) in
          `choose`'s transaction (a failed bookmark refuses the choice before
          the folder is marked or anything changes; a folder in a Trash is
          refused too); `clear` removes it. `library_setting::resume` runs in
          `setup_state`: it follows a folder moved between launches (the
          maintainer's call, 2026-11-14; a rename while running reads as
          unreachable until the next launch), renews a stale bookmark, and
          bookmarks a path set before 2b′. A folder that resolves into a
          Trash (`config::TRASH_FOLDERS`) is never followed there (emptying
          it would delete the library): it is unreachable, path and bookmark
          kept, so putting it back recovers on the next launch. That reason,
          or why the bookmark did not resolve (ending "choose the folder
          again"), is kept in `library_bookmark_fault`, which `configured`
          adds to the unreachable status ("…; at launch: …"); a failed write
          of it is logged and the launch still learns why.
          `AppState.library_access` holds the access until `RunEvent::Exit`
          (`on_exit`).
        - Entitlements: `files.user-selected.read-write` (was read-only) and
          `files.bookmarks.app-scope`.
        - **Left from review** (issue **#119**, rare): a `false` start in a
          sandbox reads as held; a database error mid-`resume` drops the
          access; resolution may mount a network share at launch; a stored
          bookmark that fails is not remade from a still-reachable path; a
          stale bookmark that fails to renew is only logged.
        - **bmlib #769 (dataless files)**: option 3 (the maintainer,
          2026-11-14), in the pin since bmlib PR #773. Every library read
          turns the thread's materialisation off, so an evicted file or
          record is `NotDownloaded` at once and stays evicted;
          `read_scanned` shows bmlib's own *not downloaded* message.
      - **2c, in review** (PR #123; plan
        `docs/superpowers/plans/2026-11-14-library-2c-clear.md`): the manual
        clear. Calls later PRs build on:
        - **`library_clear.rs`** (core): `list(db, label)` → `Listing` (one
          `ListedPaper` per folder, each `ListedFile` with its size from the
          folder, `used_by` labels and `used_here`; the scan's problems as
          `Debug` text), `remove(db, label, folder, name)` (a fresh scan must
          still list it, else `SettingError::Library`). Tauri:
          `library_papers`, `remove_library_paper`, `remove_library_file`,
          each holding `library_op`. UI: `LibraryPapers.svelte`, on request,
          with an in-page confirmation (`removeQuestion`).
        - **A reopen brings a library source's bytes back** (`open::persist`,
          `bring_back`), so a file the library lost returns; the settle
          finds a file still there and keeps it.
        - **`foreign_jats` passes over a listed JATS not on disk**: a removed
          one had kept the paper's sources waiting for good.
        - **Left** (shortcuts in the plan): folders of one paper are listed
          apart, not grouped; the confirmation's users are those of the list
          as read.
   3. **MedicalFactChecker (macOS)**, in bmlibrarian_lite: re-pin the
      xcframework, the setting, its fetch through the library (keeping the
      JATS), the move, the clear. 1c is merged, so it can start.
   4. **Open in StudyAnalyzer**: `StudyAnalyzerHandoff` in BioMedLit, always
      a file (never the link), sending what StudyAnalyzer cannot fetch itself
      (whatever it has when offline).

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
  names the old folder); *No library* with the copy ticked (`select place,
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
