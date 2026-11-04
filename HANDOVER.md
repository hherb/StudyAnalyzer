# HANDOVER

_Last updated: 2026-11-04 (bmlib pin `a491838`, bmlib PR #653's merge; phase 2c merged, PR #59; the quote guard, PR #62; #55's fix, PR #64)._

Working notes for picking up in-flight work. Each section is one self-contained
slice: what's known, where to start, and how to verify. Remove a section once
its slice has landed, unless it holds calls the next phase builds on (as
*Phase 2a* and *Phase 2b* do for 2c); add a new section when handing off new work. Planned
work not yet started is in `ROADMAP.md`.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*. Phase 2 is four plans (2a scaffold, storage
  and intake; 2b Interrogator; 2c PICO; 2d hand-off), the maintainer's call
  (2026-10-29).
- **Phase 1 is done**: every bmlib item (spec §7, plus item 11) is merged,
  the last on 2026-10-31 (bmlib PR #586). All ship in **bmlib 0.12.0 / Rust
  crate 0.3.0** (release PR #610; tagged v0.12.0 and published to PyPI and
  crates.io). The git pin is past that tag (below); it can move to a
  crates.io release that includes it (spec §11). bmlib's own HANDOVER, ROADMAP and CHANGELOG hold the detail.
- **The bmlib pin (`rev` in the root `Cargo.toml`) is `a491838`**, bmlib
  main after **PR #653** (#563: `canonical_from_html` refuses a page still
  parsing after `HTML_PARSE_SECONDS`, 30 s, as `IntakeError::HtmlTimeout`,
  the document's fault, though a failed re-derive does not record it, since
  it depends on the machine; a large page can run past it, **bmlib #654**) and
  **PR #650** (#635: *Stop* ends an answer while a local model still reads
  the prompt, through `chat_stream_with_cancel`; the abandoned request is not
  interrupted, bmlib #649). Since the 0.12.0 tag `d2ee803` it brought, among
  others: **#648** (#632: `find_passage` rejects a near-verbatim match that
  changes a number, a negation or a direction; `PassageMatch.contradiction`), **#644**
  (#640–#642: the PICO agent sends `num_ctx` and honours a stop), **#643**
  (#633: a refused download is not retried), **#639** (`PicoAgent::prepare`
  once), **#637** (part of #628: `PDF_CANONICAL_VERSION` 6), #631, #630,
  #629, #627, #626 (`FullTextError::Exhausted { faults, absences }`), #622,
  #615 and the MuPDF switch (#596). bmlib's own HANDOVER and CHANGELOG hold
  the detail.

  A stored PDF text older than `pdf-6` re-derives on its next open unless a
  conversation turn or a PICO citation points into it (`open::has_dependents`).
- **Phase 2b is merged** (PR #51, 2026-11-02), and its review follow-ups
  #50, #52 and #53 (PR #54, 2026-11-03). #54's review left #55, fixed
  by PR #64 (below).
- **Phase 2c (PICO) is merged** (PR #59, 2026-11-03; plan
  `docs/superpowers/plans/2026-11-03-phase2c-pico.md`, whose *Changed while
  building* lists every deviation). Its review left #58, #60, #61.
- **The quote guard is merged** (PR #62, 2026-11-04): a pin move to `514cda2`,
  `MIN_QUOTE_SIMILARITY` 0.99 by re-measurement (*Phase 2b*), a refused
  quote's reason stored and shown, and fixes for #61 and #58.

## Phase 2a (merged)

**Phase 2a landed** as PR #10 (2026-10-29), after the plan's PR #6. Plan:
`docs/superpowers/plans/2026-10-29-phase2a-scaffold-and-intake.md`. 10 web
tests, 117 Rust tests, 1 ignored real-PDF test. Its reviews left #11–#14.
What it does: open a paper from a
file (dialog or drop), a paste or a PMID/DOI/PMC ID; retrieve full text
(JATS first, else a cached PDF; never the abstract); decode any encoding
strictly (BOM → UTF-8 → declared → chardetng guess, shown to the user);
convert to bmlib's canonical text; run `assess_full_text` (a failed check
opens with a banner and a stored "this is the whole paper"); store everything
in SQLite (originals as BLOBs, a connection per command, WAL, IMMEDIATE
transactions); show the paper in style A, light and dark.

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
  - The debugging setting `KEEP_MISMATCHED_KEY` (Settings, "Open a
    mismatching document as a paper of its own"; `OpenContext.keep_mismatched`)
    opens such a document on the first other stored paper its identifiers
    name that it does not contradict (where a repeat landed), else a new one,
    with a notice naming the paper it contradicts. On a lookup the typed
    paper keeps a note of where it went.
  - **The same document again stores no copy** (#27): when the paper's
    latest canonical text equals the new one and came from a source of the
    same kind and bytes, `open::persist` stores no second source or text,
    only this open's notices (replacing that source's) and the title and
    opened time. So a repeated upload or attach onto the same paper, or a
    lookup under `KEEP_MISMATCHED_KEY`, keeps the text (and its "this is the
    whole paper") as it was. When that text is another (in practice older)
    converter's reading of the same kind, bytes and file stem, the new text
    is stored on the stored source and keeps the confirmation, as a re-derive
    does (#43; `StoredCopy::Older`). With no converter bump only a rename that
    changes the text (a plain text's name is its title) is stored anew; after
    a bump a renamed file of any kind is (a `shortcut:` in `stored_copy`).
    A document stored anew over a confirmed text says so in the outcome
    (`REPLACED_CONFIRMED_NOTICE`, not stored). A stored text that cannot be
    read no longer blocks a file open: the document is stored afresh, with a
    notice.
  - **A file or paste without identifiers finds its paper by its bytes**
    (#44, the maintainer's call 2026-11-01): `open` lands it on the paper
    that last stored a source of the same kind and identical bytes
    (`store::holder_of_source`), else a new paper. Other bytes, even a
    fragment of the same text, make another paper. A paste of bytes stored
    as a file is converted with that file's name as its title hint, so a
    plain text keeps its title and confirmation.
- **A lookup asks for the record first** (#36): `retrieval::resolve` calls
  bmlib's `lookup_record` (PubMed, OpenAlex, Europe PMC as far as the
  identifiers allow; a PPR accession as `ppr_id`, never as a PMID) and logs
  each source's outcome. A source whose request failed or whose answer did not
  read (`request_failed`, `unreadable`) is *unanswered*; the lookup is retried
  only while the record so far leaves nothing to ask the chain by, so a typed
  DOI or PMID costs one try. `request_for` builds the `FullTextRequest`: the
  identifiers typed win, the record's fill what they lack once they pass
  `identifiers::check_fetched` (the typed checks), and its `fulltext_sources`
  go along (converted from JSON, a `shortcut:` until **bmlib #624**).
  `Resolved::notes` names, for the user, a preprint's journal version, a
  source that served another paper or a Bookshelf chapter, a record
  identifier that is invalid or differs from the typed one, a link that did
  not read, and a refused lookup. They are notices of the document stored on
  the paper typed (not one kept apart), or end the `NeedsFullText` reason
  and are kept in `papers.refusal`, so a reopen shows them. The record's title
  names a new or still untitled paper, and a document without a title of its
  own; it is **not** the converter's title hint, so a re-derive (file name
  only, none for a lookup) gives the same text. A PPR no source names by PMID,
  PMC ID or DOI, and links no full text for, is `NotFound`. Any `NotFound` or
  `Unreachable` while a record source was unanswered is `Unreachable`,
  naming it. `Identifiers` still keeps a PPR in `pmid` (a `shortcut:`); only
  `retrieval::split_pmid` tells them apart. The record's identifiers are not
  stored on the paper; the JATS's own still are (#39).
- **Retrieval** tells "no free full text" (not retried) from "could not reach
  the sources" (retried) by bmlib's types (#565, #603):
  - `FullTextError::Exhausted` with no `faults` is final, with faults a retry.
  - With a DOI or PMID, bmlib answers `Ok` with an abstract or a bare link
    even when every tier faulted. Such a result whose `tier_faults` is not
    empty is an outage (`retrieval::faulted_exhaustion`).
  - Faults behind full text or a PDF are logged.
  - `FullTextSource::fetch` forwards all of bmlib's log lines to the app log.
  - Only `"No identifiers provided"` is still matched by its wording.
- **Re-deriving** passes the stored file name as the title hint again and
  carries the user's "this is the whole paper" over to the new text.
- **Re-derive**: a newer converter re-derives from the source the stored text
  names, only while nothing points into it (`open::needs_rederive`'s
  `has_dependents`; 2b's rule is under *Phase 2b*). A failed attempt is recorded on the text
  (schema migration 2: `rederive_failed_version`, `rederive_error`) and not
  repeated until the converter changes again (#22); the old text opens with a
  notice each time. A scratch-file fault (`IntakeError::Io`) or a PDF engine
  fault passes, so it is shown but not recorded.
- **PDF faults** (#11, with bmlib #598): `IntakeError::Pdf` holds bmlib's
  `PdfError`; `IntakeError::is_the_documents` says whether another file is
  the remedy. Upload maps a file fault (unreadable, encrypted) to
  `invalid_input` and an engine fault to `internal`. A lookup keeps the paper
  with "could not be read" only for the document's own faults; an engine,
  scratch-file or canonical-text fault is an error, storing nothing.
- **Notices**: `OpenOutcome::Ready.notices` carries what the user should know
  about an open, shown in the paper banner: a failed re-derive, a JATS file's
  own identifier that is invalid (`intake::jats_identifiers`), one that
  clashes with the paper's stored one or another paper's
  (`store::add_identifiers` returns them), or a mismatch kept under the
  debugging setting (`open::place`), or what MuPDF read past without failing
  (`intake::engine_notice`, bmlib #599: a repaired xref, a page cut short, a
  bogus font; a run of one repeated message is given once with its count,
  every message kept). A successful re-derive puts its own engine notice, or
  none, in place of the stored one (found by `config::ENGINE_NOTICE_LEAD`). A
  PDF that reads to no text says it in the `IntakeError::Empty` error, and a
  text bmlib refuses in `Canonical`. Texts stored before the pin bump show
  none until re-derived (`pdf-3` did not move).
  The notices of an open that stores a source are stored with it (schema
  migration 3: `sources.notices_json`) and shown again on every open of a
  text derived from it, as written then: a paper they name by title may have
  been renamed since, the identifier they name has not. If the paper view
  fails to load right after an open, they go into the error banner.
- **`retry::with_backoff`** blocks its thread (use `spawn_blocking`); 2c wraps
  PICO's Rust `chat()` calls in it while **bmlib #561** stands (2b streams,
  and bmlib retries opening a stream itself).
- **A lookup by identifier has a total limit and a Cancel** (#7):
  `limit::Limit` (deadline `RETRIEVAL_DEADLINE`, 120 s, plus a shared cancel
  switch) and `limit::LimitedClient`, an `HttpClient` that refuses a request
  once the limit has stopped and clips each one's timeout to the time left
  (`get`, `get_with_headers`, `post_json`; it refuses `post_json_stream`). A
  running request is not interrupted, so a cancel waits for it (at most
  bmlib's 45 s). A free PDF whose download failed (`pdf_download_failure`,
  #601) with no JATS beside it is never "no full text": a retryable cause
  (failed request, 5xx, 408, 429, a cut-short download) is retried, then
  `Unreachable` naming the link; a final one (a web page served, other 4xx,
  cache not writable) is `NotFound` naming the link. An unreadable cached PDF
  is a final `NotFound` naming the fault and the link (bmlib converts no PDF
  here, `with_convert_pdfs(false)`). A cancel is `OpenError::Cancelled`,
  stores nothing, and stops MuPDF mid-page (bmlib #600, `Limit::pdf_limit`);
  only reopening a stored paper is not stopped. Navigation stays disabled
  while busy so Cancel stays reachable; `AppState::begin_lookup`/`end_lookup`
  track the one lookup. 2b's answer and download stop through a `StopSlot`.
- **Small hardening** (#13): `store::open_store` refuses a database a newer
  app wrote (`StoreError::Newer`); `open_memory_store` begins `IMMEDIATE` as
  the file store does (both through `store::prepare`); `PaperId`'s field is
  crate-private (`as_str()`; outside code gets one from `paper_by_id`);
  `PaperView.canonical_id` goes back with `confirm_full_text`, which confirms
  that text of that paper only, so a text a re-derive stored meanwhile stays
  unconfirmed; a `.pdf` that is not a PDF is `IntakeError::NotPdf`; a chosen
  file is read no further than one byte past `MAX_PDF_BYTES`.
- **`view::utf16_offsets` / `span_to_utf16`** are the only code-point → UTF-16
  conversion; the Interrogator's citation spans use them.
- **The Rust↔TS JSON contract is pinned** (#12, #17, #19, #41):
  `src-tauri/src/contract.rs` serialises every shape `src/lib/api.ts` mirrors
  into `src/lib/fixtures/ipc_contract.json`; `src/lib/contract.test.ts` checks
  keys and value types at compile time (`check:web`), tags and error kinds at
  run time, and slices every span. **A new shape** is a key in `contract()`
  plus `keysMatch`/`valuesMatch` rows. `commands()` lists each command's
  argument keys and JSON types, sent to the real handlers through Tauri's mock
  runtime (each key also dropped and mistyped). **A new command** goes in
  `lib.rs`'s `handler()`, `commands()`, `api.ts` and the vitest call list
  together. The fixture's `returns` is each handler's declared `Ok` type
  (every command returns `Result<_, CommandError>`) mapped through
  `RETURN_TYPES`. After a deliberate change: `UPDATE_CONTRACT=1 cargo test -p
  studyanalyzer contract`, then fix `api.ts`. `CommandError.kind` is the enum
  `ErrorKind`; the frontend refuses a kind it does not know. The banner's
  missing headings come from Rust (`FullTextStatus.missing_headings`, a copy
  of bmlib's private rule pinned by `missing_headings_agree_with_bmlib`).

## Phase 2b (merged)

**Phase 2b landed** as PR #51 (2026-11-02), plan
`docs/superpowers/plans/2026-11-02-phase2b-interrogator.md`. With its
follow-ups: 57 web tests, 335 Rust tests, 3 ignored (a real PDF, the
threshold's margins and model quotes). What it does: ask a question of
the paper with a chosen model, stream the answer, check every quote against
the paper and highlight the paper's own span; conversations stored per
canonical text; whole-paper or retrieval mode by the model's window; the
search model downloaded on first need; provider, model, window and API keys
in Settings. The hand click-through is owed (below).

Calls later phases build on:
- **Dependents**: a canonical text with a conversation turn
  (`conversations::has_turns`, the `has_dependents` answer) is never
  re-derived, and the same file opened again under a newer converter keeps it
  (`open::persist` turns `StoredCopy::Older` into `Same`). A different document
  stores a new text; earlier conversations stay on their text and are
  read-only (`read_only` = not the current conversation). An empty
  conversation is no dependent; a chunk set is none either. It is keyed by
  canonical text id (`passages::index_key`), and `store::save_canonical`
  drops every older text's set when it stores a newer one (#53, the
  maintainer's call 2026-11-03): only the latest text is searched, and a set
  can always be rebuilt. An ask that indexed an older text drops its set
  again once its search is done, failed or not, if a newer text was saved
  meanwhile (`store::drop_superseded_chunk_set`, #55; a save between the
  ask's indexing and its search still fails that ask, **#65**). A PICO citation keeps its text
  too (*Phase 2c*).
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
  labelled cases on `article.xml` and the CC BY trial PMC13712513): the guard
  catches every changed numeral (digits, signs), negation and direction, so only rejects it
  lets through bound the threshold (`print_margins`). An arm swap with the
  numbers in place passes it (0.9856, **bmlib #651**, as does a spelled-out
  number) and lies among benign edits (dropped, doubled or added words, `+/-`
  for ±: 0.9758–0.9910), so by
  plan 2b Task 4's overlap rule it is 0.99 and seven true accepts carry
  `"expected": false`. A refused quote's reason is stored
  (`quotes.contradiction`, migration 7, as bmlib names it) and shown beside
  it (`contradictionNote`; an unknown reason raw). The guard only checks a
  window that reached the threshold, so a short changed quote is plain "not
  found". PICO uses the same value; its agent drops the reason (**bmlib #647**).
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
  `from_chat`: a missing key is `needs_api_key`, others `llm`); `TooLong` is
  `invalid_input`, naming the fix.
- **Events go over a Tauri `Channel`**: `Channel<AnswerEvent>` (`CheckingModel`
  before the search model is hashed and loaded, `Indexing { done, total }`,
  `Waiting` once the model is asked, `Delta { text }`) for `ask`, `Channel<DownloadProgress>`
  (`done`, `total` bytes) for `download_embedder`. The frontend drops events
  from a run that is no longer the live one.
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
  stops a later one (`stop_answer`, `cancel_download`). A DB connection per
  command, never held behind a lock while streaming.
- **The embedder** (`embedder.rs`): `ModelSlot<E>` holds the pinned
  `EMBEDDING_MODEL` (F16, 1.1 GB) under the app data directory's `models/`,
  verified and loaded once per launch, downloaded with backoff. The app
  unloads it on `RunEvent::Exit` (llama.cpp aborts at exit on Metal with a
  model loaded). **`needs_embedder` is the authority**: a corrupt file of the
  pinned size reads `downloaded: true` in `embedder_status`, so the UI offers
  the download on `needs_embedder` anyway. The `embed` feature pulls
  llama.cpp and CMake into every target (a note for phase 4).
- **Keys**: one keychain item (service `com.bmlibrarian.studyanalyzer`,
  account `api-keys`, a JSON object provider → key), read once per launch
  and cached, since macOS prompts per item read. `llm_settings::key_source`
  says `keychain`, `env` or none (Settings shows *saved* / *from
  `<ENV_VAR>`*, the provider's own variable / *none*). A failed keychain read degrades to the env var, never blocks
  (`LlmSettings.keychain_error` is shown). The store moves to bmlib:
  **bmlib #634**.
- **The window**: `prompt::running_window(setting, maximum)` = the smaller
  of those known. Fit estimates `FIT_CHARS_PER_TOKEN` (3) code points a token
  plus `ANSWER_TOKENS` (4096) for the answer. Unknown window and no setting →
  retrieval mode, checked against `UNKNOWN_WINDOW_FIT_TOKENS` (8192) and sent
  with no `num_ctx`; otherwise `num_ctx` is the running window on every
  request. Retrieval: the question alone, `RETRIEVAL_PASSAGES` (8). A prompt
  that still does not fit is refused, never cut. Temperature 0, no other
  parameter; per-model parameters are **#49**.

## Phase 2c (merged)

A *PICO* tab between *Interrogator* and *Original*: population,
intervention, comparison, outcome, study type and sample size, each part
backed by quotes `find_passage` located, highlighted on the paper. 72 web
tests, 350 core and 37 app Rust tests at its merge (79, 355 and 39 since
PR #64). Calls the later analysis tabs build on:
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
  located in the whole text. The check reads the abstract (`abstract_text`,
  JATS sub-sections included) and says so (`from_abstract`).
- **Retry and stop**: `RequestFailed` retried by `with_backoff`, never once
  stopped; the stop is read after every call and before `save_run`, so a
  stopped run stores nothing (a failure under way is logged, not shown).
  `run_pico` claims `AppState.pico_stop` with `StopSlot::try_begin` (one run
  at a time; its `Claim` is opaque) and frees it through `state::Release` in
  `commands::with_claim`, a panic included. A stop command returns whether it
  reached a run; `api.stopWhilePending` re-sends one Tauri handled before its
  async command registered (#61; the answer, the download and the lookup's
  Cancel too), and checks after each wait that its own run is still pending,
  so a later run never gets it. Every run registers before its blocking task
  is queued and frees its slot on every path, a panic included (`Release`,
  `state::EndLookup` for the lookup). `pico_store::run`
  hands back the text it read (`StoredPicoRun.text`), so a view reads it once
  (#58). A missing key is refused before
  anything is sent (`llm_settings::missing_key`).
- **The tab host keeps the run's state** (`Interrogator.svelte`): the tabs
  are `{#if}` branches, so `PicoTab`/`PicoCard` only show. Auto-run once per
  host mount (`shouldAutoRun`); *Extract PICO* whenever there is no run on an
  askable text and none going (`offerRun`). A PICO run makes the host `busy`
  (App's navigation waits) but not the chat. `older` follows the tab shown.
  Where a failed run's error goes is `runFailure` (`pico.ts`); a stored run
  whose view fails says it was stored, and the old card never stands in for it.

## Next

1. **What a page cut short should be** (**bmlib #605**, the maintainer's
   call): today it is a notice like any other engine message. **bmlib #619**
   (on a failed read, Rust drops MuPDF's messages; Python's error does not
   carry them) would let a failed PDF say what the engine reported before it
   failed.
2. **#31** (two transitive Dependabot alerts) waits on Tauri and
   vite-plugin-svelte releases. A Svelte component documents itself in a
   `/** … */` at the top of its script (#14).
3. **Open issues.** bmlib: **#634** (the secret store
   moves to bmlib), **#636** (`Method`, `Result`, `Discussions`, `Experimental` match
   no section pattern), **#646** (the condenser's model retries a 401/404 as
   an outage), **#609** (no progress while a long paper is condensed; the
   tab says it takes minutes), **#647** (PICO drops a refused quote's reason),
   **#651** (the guard misses an arm swap and a spelled-out number; the
   threshold waits on it). Ours: **#49** (per-model parameters, owed
   below), **#60** (the PICO card hides the confidences it
   stores, a UI call), **#65** (an ask fails with a passage error if a newer
   text is saved mid-ask; 2d, what the user sees is a call). bmlib **#577** (PICO reply changes
   that leave no trace) gained two cases, **#646** the PICO chat path.
   Not hit by plan 2b Task 4's measurement: **bmlib #528**, **#529**.
4. **Phase 2d** is unblocked (its pin move landed, above): its plan comes
   first, and must settle how Rust trusts that a path came from the dialog or
   a drop, and #65.

## Owed by the maintainer

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (`find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).
- **Click through phase 2a**, if not done before the merge (plan Task 9
  Step 5; no agent could click the window): open a PDF, a JATS file, a saved
  HTML page, a pasted Markdown text; a PMID with free full text and one
  without; Wi-Fi off for the unreachable path; dark mode. (#7's *Cancel* was
  clicked through on 2026-10-31 and works.)
- **Click through phase 2b** (plan Task 10 Step 3; no agent could click the
  window), with `pnpm dev`, a real cloud model and a local Ollama model: a
  whole-paper question with citations (click → scroll, current dark, earlier
  pale); a follow-up; a question the paper does not answer; *Stop*
  mid-answer, during *Finding passages* and while a local model reads a
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
  paper on a small window (condensed caption, quotes still highlight; a
  question asked meanwhile is answered); *Stop* during condensation (nothing
  stored, *Extract PICO* offered); leaving the tab mid-run and back (the same
  run, no second); a double-click on *Extract PICO* (one run); Wi-Fi off
  during condensation (the error after the retries, the previous run kept);
  *Re-run* with another model (both listed); an earlier run on an older text
  (the banner's confirm held back); no model / no key (a message and a
  button, no jump to Settings); an unconfirmed text, then confirming it with
  the tab open (*Extract PICO* offered); a narrow window (*PICO | Paper*);
  dark mode; quit and reopen (the run shows at once, no new call).
- **Decide bmlib #651 and #647**: whether the guard should catch a swap of
  named arms and a spelled-out number (then `MIN_QUOTE_SIMILARITY` can drop
  towards 0.98 and keep dropped-word quotes), and whether PICO records a
  refused quote's reason.
- **Decide #65** before 2d: what an ask shows when the paper's text is
  replaced while it runs (a distinct "ask again" error, or one retry on the
  latest text).
- **Decide #49**: per-model parameters (spec §6.6); 2b sends temperature 0
  and nothing else.
- **Decide how large `num_ctx` is on Ollama**: 2b sends the running window,
  which is the model's full maximum unless Settings lowers it (often 131072),
  so a large local model may fail to load or spill to CPU. Sizing it from the
  prompt estimate instead could undercount (CJK, dense notation) and let
  Ollama cut the paper silently. Check it in the click-through first.
- **Decide bmlib #560**: how strict `assess_full_text` is (exact heading
  matches only, landing-page headings like `Cited by` excluded, `Methods
  summary` as Methods). It moves the shared fixture in both languages.
- **Decide #20**: MuPDF is AGPL but Artifex's, so "the maintainer holds the
  copyright" no longer covers store distribution (spec §7). Blocks store
  submission only.
- **Decide #39**: store a looked-up record's identifiers on the paper, or
  keep only the typed ones and a JATS document's own.
- **Decide bmlib #561**: should Rust's `chat()` retry a failed call as
  Python's SDKs do (and Python's Ollama provider too)?

## Landed (context)

Phase 1 items merged in bmlib, all in **0.12.0** (published); the open issues
on each are listed in `ROADMAP.md`:

- **Item 8**, chunks stored and searched (PR #586): `bmlib::embed::store`, used
  by `passages.rs` and `embedder.rs`. F16 ↔ Q8_0 makes every paper stale.
- **Item 5**, a record by identifier (PRs #580, #584):
  `bmlib::publications::lookup` / `lookup_record` (*Phase 2a*, #36).
- **Item 7**, the PICO agent (PR #575), used by 2c.
- **Item 11**, context windows (PR #568): `LlmClient::context_window(model)`
  gives the maximum or `None`, never a guess (*Phase 2b*, the window).
- **Item 6**, streamed chat (PR #562): only `openai`, `deepseek` and `ollama`
  report usage on a stream.
- **Items 4a–4c**, canonical text (PRs #547, #556 and MuPDF's #596/#597,
  #559): every title, heading and block a code-point span; a table row a
  `table_row` block of tab-separated cells; a PDF's text as extracted (a table
  quote may fail to locate, §5.2). For 2d prefer `canonical_from_markdown`.
- **Item 9**, retrieval returns its source (PR #545): `FullTextResult.jats_xml`;
  `jats_xml=None` is "no structured source".
- **Item 2**, `find_passage` (PRs #526, #544), fixture `passage_match.json`.
- **Items 1 and 10**, the embedder and our GGUF (PRs #531, #534, #535): F16 and
  Q8_0 at `hherb/snowflake-arctic-embed-l-v2.0-gguf`, pinned by revision, size
  and sha256.
