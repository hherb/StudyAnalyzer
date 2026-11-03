# Phase 2c: the PICO tab — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A *PICO* tab beside the Interrogator: the paper's population,
intervention, comparison and outcome (plus study type and sample size) as a
card, each part backed by quotes that `find_passage` located in the canonical
text and highlighted on the paper (current `#e8b931`, the run's others
`#f6e7b0`); run once on first open, stored, re-runnable, every run kept.

**Architecture:** bmlib's `PicoAgent` does the asking, the quote checking and
the map-reduce for long papers; this repository sizes its budget from the
running window, stores each run per canonical text, and shows it. As 2a and
2b: logic in `crates/studyanalyzer-core` (faked `JsonChat` in tests), thin
commands in `src-tauri`, Svelte 5 in `src/`. Spans are code points in Rust and
UTF-16 at the boundary (`view.rs`).

**Tech Stack:** Rust 2021, Tauri 2, bmlib at a new pin (Task 1), Svelte 5 +
TypeScript + Vite, vitest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`
(§2 anti-goals, §5.1, §6.1, §6.3, §6.4, §6.6, §8). Builds on HANDOVER.md
*Phase 2b* (calls 2c builds on) and *Landed* item 7 (the PICO agent).

### Builder's calls this plan settles (spec §11)

- **bmlib first (the maintainer's call, 2026-11-03).** Three additions go to
  bmlib before any code here (Task 1): the PICO agent's chat sends no
  `num_ctx` in either language, so Ollama would cut a whole paper silently;
  Rust has no production `ContextModel` over `LlmClient`, so
  `PicoAgent::pico_condenser` cannot be built; and a stop (below).
- **Stop between calls (the maintainer's call, 2026-11-03).** A run can be
  stopped; it ends before its next model call (a call under way runs to its
  end, its reply discarded) and stores nothing: `stop` is read after every
  call and once more just before `save_run`, so a stop pressed during a call
  that then succeeds still stores nothing. Long-paper progress cannot be
  shown (bmlib #609); the tab says a long paper takes minutes.
- **One run** is the suitability check, then the extraction if suitable. Not
  suitable → the run is stored with the check alone and the card says so
  plainly, with the model's rationale and *Run anyway*. *Run anyway* is a new
  run that skips the check (`force`). *Re-run* is a new run with the model now
  in Settings. Every run is kept with model and date; the newest run on the
  latest text is shown; earlier ones are listed as the Interrogator lists
  conversations. One run at a time: `run_pico` refuses while one is going
  (`invalid_input`, *A PICO extraction is already running.*), and the run's
  state lives in the tab host, not the tab, so leaving the tab and coming back
  neither loses it nor starts another.
- **What the check reads:** the first section whose heading, trimmed, equals
  bmlib's `ABSTRACT_HEADING` (`"Abstract"`) in any case, with its blocks and
  those of every following section of a deeper `level` (JATS writes the
  heading with no blocks and each abstract part as a sub-section,
  `canonical/jats.rs:96-104`), joined by blank lines; when there is no such
  section or the result is blank, the whole canonical text. The extraction
  always gets the whole `CanonicalText.text`, so citation offsets index it
  directly; the title is `span_text(title_span)` or empty.
- **Budget, never a cut (§6.4):** `PicoAgent`'s `condense_config.max_context_chars`
  is the running window (else `UNKNOWN_WINDOW_FIT_TOKENS`, as 2b) less
  `PICO_MAX_TOKENS`, less the longest PICO prompt's estimate, less the
  title's, times `FIT_CHARS_PER_TOKEN`, in saturating arithmetic
  (`context_tokens` is unbounded; `usize` is 32 bits on some phones). A
  budget under `PICO_MIN_BUDGET_CHARS = 4_000` (the size of bmlib's own
  default, `DEFAULT_MAX_CONTEXT_CHARS`; a tinier budget only buys many
  condensation calls that end in `CondensationFailed`) is refused as
  `TooLong`, naming the fix. A text over the budget is condensed;
  quotes are still checked against the whole paper (bmlib does this).
  `num_ctx` is sent whenever the window is known, by the chat and the
  condenser both.
- **A condensation is whole or it fails.** The config sets
  `with_continue_on_error(false)` (bmlib's default carries on past a failed
  batch and `condense_checked` accepts the `Partial` digest with a note, so a
  Wi-Fi blip would silently drop a section, against §6.4 and rule 6). Each
  condenser call retries a transport failure with backoff inside
  `LlmContextModel` (Task 1, rule 7), so `CondensationFailed` means retries
  ran out and is not retried again here. Any notes bmlib still attaches (the
  run's `notes`, the check's) and the condensation status are shown on the
  card, never only stored.
- **Known cost, bmlib #638:** each agent call condenses on its own, so a
  retried `extract` condenses again, and a paper with no abstract is
  condensed for the check and again for the extraction. Accepted until #638
  lets one digest serve both.
- **Parameters:** temperature `PICO_TEMPERATURE = 0.0` (2b's choice; #49 owns
  per-model parameters), `PICO_MAX_TOKENS: i64 = 4096` (bmlib's `max_tokens`
  is `i64`), `chat_json` retries left at bmlib's default. `RequestFailed` is
  retried with `retry::with_backoff` (`RETRIEVAL_ATTEMPTS`, whose doc already
  covers every failing network call, and `RETRY_BASE_DELAY`), never after a
  stop. A provider that needs a key and has none is refused as
  `needs_api_key` before anything is sent (bmlib folds a missing key into
  `RequestFailed`).
- **Threshold:** `MIN_QUOTE_SIMILARITY` (0.999, provisional on bmlib #632).
- **Labels (HANDOVER *Landed* item 7, spec §6.4):** `unverified` → *Unsure –
  model could not reproduce {Part} from text provided*; `not_reported` → *Not
  clearly stated*. Study type and sample size are the model's, never checked
  against the paper, and say so.
- **Dependents (§5.1):** a canonical text with a PICO citation is a dependent
  as one with a conversation turn is: never re-derived, kept when the same
  file comes under a newer converter. A run without citations (not suitable,
  or every part unverified or not reported) is none, so after a converter
  bump such a text re-derives and the tab auto-runs once on the new text (one
  model call; accepted, as the old run has nothing to highlight).
- **Failures store nothing.** The card keeps the previous run, and the error
  says what to try: `UnusableReply` and `Truncated` suggest another model
  (*Landed* item 7 suggests a larger `max_tokens`, but `PICO_MAX_TOKENS` is
  not a setting, so the model is what the user can change);
  `CondensationFailed` and `RequestFailed` give bmlib's message.
- **Auto-run needs a model.** Opening the tab with no model chosen shows
  *Choose a model in Settings to extract PICO* with a button, rather than
  jumping to Settings unasked.

## Global Constraints

- Everything in 2a's and 2b's Global Constraints still holds: gates before every commit (CLAUDE.md § Gates), constants in `config.rs`, doc comments, tests on outward-facing items, no truncation, every error logged and shown, sandbox-ready, code points in Rust.
- Never fall back to the abstract or the model's general knowledge: a run refuses a paper whose latest text neither passed the full-text check nor was confirmed (`not_full_text`), as `ask` does. The abstract feeds the suitability check only.
- A citation shows the paper's span and wording (`canonical.span_text`), never the model's copy; a part's quotes that were not found are listed under it, never dropped.
- Highlights: current `var(--hl-current)` (`#e8b931`), the run's other citations `var(--hl-past)` (`#f6e7b0`).
- Phone layout (below `PHONE_BREAKPOINT_PX = 700`): a *PICO | Paper* switch replaces the split; tapping a citation flips to *Paper*.
- No model hard-wired: with none chosen, a run is `needs_model`.
- A new command goes in `handler()`, `commands()`, `RETURN_TYPES`, `api.ts` and the vitest call list together.
- bmlib work (Task 1) happens in a bmlib **worktree**, never the shared `~/src/bmlib` checkout, under bmlib's HANDOVER.md and `docs/SESSION-RULES.md`; Rust and Python land in the same PR with shared contract fixtures; ruff via `uvx ruff@0.15.20`.

## Review Focus

1. **A paper too long for the window** (an 8k Ollama model, a 60-page trial): it is condensed, never cut, and never from a partial digest; quotes are located in the whole canonical text, not the digest; `num_ctx` is sent by the chat and the condenser. Tests: Task 1 (`num_ctx_is_sent`, both languages, chat and condenser), Task 3 (`budget_from_window`, `long_paper_is_condensed`, `quotes_located_in_whole_text`, `failed_batch_fails_the_run`, `pico_condenser_model_carries_window`).
2. **A quote that changes a number** (model `N = 150`, paper `N = 105`): the part loses that citation and lists the quote as not found; a shown citation's text is the paper's; sample size is labelled unchecked. Tests: Task 3 (`changed_number_is_not_cited`), Task 4 (`citation_text_is_the_papers`), Task 5 (`sampleSizeNote`).
3. **The text changes after a run** (a converter bump, a new file attached): a text with PICO citations is not re-derived; an earlier run is shown on its own text and its highlights never move. Tests: Task 2 (`text_with_pico_citations_not_rederived`, `same_file_after_bump_keeps_text_with_pico_run`, `run_without_citations_is_no_dependent`, `has_citations_counts_only_citations`), Task 4 (`earlier_run_uses_its_text`).
4. **Stop or a failure mid-run** (Wi-Fi off during condensation): nothing stored, the previous run still shown, a transient failure retried, a stopped one never; a stop during a call that succeeds stores nothing. Tests: Task 1 (`stop_before_call_sends_nothing`, chat and condenser; `context_model_retries_transport`), Task 3 (`stop_stores_nothing`, `stop_during_condensation`, `stop_during_successful_call`, `request_failed_is_retried`, `retries_run_out`, `stopped_is_not_retried`, `failure_keeps_previous_run`).
5. **Opening the tab** with no model, no key, an unconfirmed text, after a failed or stopped first run, or after leaving the tab mid-run: it auto-runs only when the latest text has no run, may be asked about and a model is chosen, once per paper opening, never in a loop, and never beside a run still going. Tests: Task 5 (`shouldAutoRun`); Task 6's hand check (leave the tab mid-run and come back).

---

### Task 1: bmlib — `num_ctx`, a stop, and a `ContextModel` over `LlmClient`

**Where:** a bmlib worktree on its own branch, under bmlib's rules; one PR,
Rust and Python on par, shared contract fixtures. Then, here, the pin.

**Interfaces (proposed; bmlib's review may rename — the names that land are
recorded under *Changed while building* below, and Tasks 3–4 use them):**
- Rust `bmlib::quality::LlmChat` gains `pub num_ctx: Option<NonZeroU32>` and
  `pub stop: Option<&'a AtomicBool>`, with `with_num_ctx` / `with_stop`;
  `num_ctx` is set on the `ChatRequest`; a set stop returns before the request
  is sent.
- Rust `bmlib::context_processor::llm_processor::LlmContextModel<'a> { client: &'a LlmClient, model: String, num_ctx: Option<NonZeroU32>, stop: Option<&'a AtomicBool> }`
  implementing `ContextModel` (`complete`: plain chat, an output-ceiling stop
  reason is an error, as the trait requires; `complete_json`: `json_mode`, a
  non-object is an error) — what Python's `LLMChunkProcessor` does over its
  agent. Each call retries a transport failure with exponential backoff
  (bmlib's attempts and base delay, rule 7), never after the stop is set; a
  set stop returns an error before any request.
- `PicoError::Stopped` (Python `PicoErrorKind.STOPPED`, kind string
  `"stopped"` in both): the stop was set before a model call, during
  condensation included (the condenser's stop error surfaces as `Stopped`,
  not `CondensationFailed`).
- Python: `PicoAgent(..., num_ctx=None, stop=None)` (a `threading.Event` or
  a zero-argument callable, bmlib's call), passed through `BaseAgent.chat` to
  `LLMClient.chat`, and to the condenser's calls.

- [ ] **Step 1:** Open bmlib issues for the three gaps (each naming StudyAnalyzer plan 2c), then follow bmlib's SESSION-RULES: failing tests first in both languages, including:
  - `num_ctx_is_sent`: the request reaching the provider carries it, from the chat and from the condenser.
  - `stop_before_call_sends_nothing`: no request after the stop is set, from the chat or mid-condensation; the agent returns `Stopped`. In Python, with the stop as an `Event` and as a callable.
  - `context_model_ceiling_is_error`, `context_model_non_object_is_error`, `context_model_retries_transport` (a transport failure then success → one result, two requests; failures past the attempts → an error).
  - A shared fixture both languages read, pinning the `num_ctx` each request carries (chat and condenser) and the `"stopped"` kind string.
- [ ] **Step 2:** Implement, run bmlib's Rust and Python gates (CLAUDE.md § Gates), open the PR, and wait for its merge. Update bmlib's HANDOVER, ROADMAP and CHANGELOG as its rules say.
- [ ] **Step 3:** Here: set `rev` in the root `Cargo.toml` to the merge commit; confirm `PDF_CANONICAL_VERSION` and the other converter versions are unchanged (bmlib #631 comes along, still `pdf-5`) or follow HANDOVER's re-derive rule if not; run every gate. Expected: PASS.
- [ ] **Step 4: Commit** `chore: bmlib to <rev> (PICO num_ctx, stop, ContextModel)`.

### Task 2: PICO runs in storage, and dependents

**Files:**
- Create: `crates/studyanalyzer-core/src/pico_store.rs`
- Modify: `schema.rs` (migration 6), `open.rs` (every `has_turns` call that decides a dependent: `persist`'s via `text_has_turns`, and `reopen_paper`'s; the doc at `open.rs:195` that names `has_turns`), `lib.rs`

**Interfaces:**
- Consumes: `PaperId`, `StoreError`, `bmlib::quality::{PicoSuitability, PicoExtraction, PicoPart, PicoPartState, PICO_PARTS}`, `store::canonical_by_id`.
- Produces:
  - `pub enum PicoOutcome { NotSuitable(PicoSuitability), Extracted { suitability: Option<PicoSuitability>, extraction: PicoExtraction } }` — the only two shapes a run has; `suitability: None` is a forced run. `save_run` refuses an `Extracted` whose check says not suitable, or a `NotSuitable` whose check says suitable (`StoreError::Corrupt`, never written).
  - `pub struct NewPicoRun { pub model: String, pub outcome: PicoOutcome }`.
  - `pub struct StoredPicoRun { pub id: i64, pub canonical_id: i64, pub model: String, pub created_at: String, pub outcome: PicoOutcome }`.
  - `pub struct PicoRunSummary { pub id: i64, pub canonical_id: i64, pub model: String, pub created_at: String }` (`Serialize`).
  - `pub fn save_run(db: &mut dyn Db, paper: &PaperId, canonical_id: i64, run: &NewPicoRun) -> Result<i64, StoreError>` — one IMMEDIATE transaction with its parts and citations.
  - `pub fn run(db: &mut dyn Db, run_id: i64) -> Result<Option<StoredPicoRun>, StoreError>` — citations checked against their text (`canonical_by_id`): outside it → `Corrupt`; each citation's `text` re-read from the paper.
  - `pub fn latest_run(db: &mut dyn Db, paper: &PaperId, canonical_id: i64) -> Result<Option<i64>, StoreError>` — highest id on that text.
  - `pub fn runs(db: &mut dyn Db, paper: &PaperId) -> Result<Vec<PicoRunSummary>, StoreError>` — every run, newest first.
  - `pub fn has_citations(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError>` — a citation row exists on a run of that text (not merely a run, nor an extracted run).
  - in `open.rs`: `fn has_dependents(db, canonical_id) -> Result<bool, StoreError>` = `has_turns || has_citations`, used wherever `has_turns` decides re-derivation today (`persist`'s call through `text_has_turns`, `open.rs:469`/`:528`, and `reopen_paper`'s, `:844`).

Migration 6:

```sql
CREATE TABLE pico_runs (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  canonical_id INTEGER NOT NULL REFERENCES canonical_texts(id), model TEXT NOT NULL,
  suitability_json TEXT, extracted INTEGER NOT NULL, study_type TEXT, sample_size TEXT,
  overall_confidence REAL, condensed_from_chars INTEGER, condensation_status TEXT, notes_json TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE pico_parts (id INTEGER PRIMARY KEY, run_id INTEGER NOT NULL REFERENCES pico_runs(id) ON DELETE CASCADE,
  part TEXT NOT NULL, description TEXT NOT NULL, state TEXT NOT NULL, confidence REAL,
  quotes_not_found_json TEXT NOT NULL, quotes_dropped INTEGER NOT NULL);
CREATE TABLE pico_citations (id INTEGER PRIMARY KEY, part_id INTEGER NOT NULL REFERENCES pico_parts(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, span_start INTEGER NOT NULL, span_end INTEGER NOT NULL,
  similarity REAL NOT NULL, exact INTEGER NOT NULL);
```

`suitability_json` is `PicoSuitability::to_json()` (no spans in it), read back
field by field. `extracted = 0` stores no parts and is `NotSuitable`, which
needs a stored check. An extracted run must have four parts in `PICO_PARTS`
order; anything else (fewer, more, out of order, none, an unknown `state`, a
span with `start > end` or outside the text) reads back `Corrupt`.

- [ ] **Step 1: Write failing tests** in `pico_store.rs` (paper and text as `conversations.rs`'s tests make them):
  - `run_round_trip`: a run with suitability and an extraction (one verified part with two citations, one unverified with a not-found quote, one not reported) reads back equal, citations in order, `text` the paper's.
  - `forced_run_has_no_check`: `suitability: None` reads back `None`.
  - `not_suitable_run_has_no_parts`.
  - `mismatched_outcome_refused`: `Extracted` with a not-suitable check, and `NotSuitable` with a suitable one → `Err(Corrupt(_))`, no row.
  - `latest_is_newest_on_that_text`; `runs` lists all, newest first.
  - `corrupt_rows_reported` (table-driven, as `conversations.rs`'s `corrupt_span_reported`): set by SQL, each → `Err(Corrupt(_))`: a span past the text's end; `start > end`; three parts; two parts swapped; `extracted = 1` with no parts; an unknown `state`; `extracted = 0` with no `suitability_json`.
  - `has_citations_counts_only_citations` (as `empty_conversation_is_no_dependent`): a not-suitable run → false; a run whose parts are all unverified or not reported → false; one verified part → true; another text's id → false.
  - in `open.rs` (Review Focus 3): `text_with_pico_citations_not_rederived` (as 2b's `text_with_turns_is_not_rederived`, with a run in place of a turn, the `reopen_paper` path); `same_file_after_bump_keeps_text_with_pico_run` (as `same_file_after_bump_keeps_text`, the `persist` path); `run_without_citations_is_no_dependent` (a not-suitable run → the text re-derives).
  - `schema.rs`: `versions_are_sequential` expects `[1, 2, 3, 4, 5, 6]`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): store PICO runs; a text with PICO citations is kept`.

### Task 3: Running PICO

**Files:**
- Create: `crates/studyanalyzer-core/src/pico.rs`
- Modify: `config.rs` (`PICO_TEMPERATURE: f64 = 0.0`, `PICO_MAX_TOKENS: i64 = 4096`, `PICO_MIN_BUDGET_CHARS: usize = 4_000`), `lib.rs`. The abstract heading is bmlib's `ABSTRACT_HEADING` (`fulltext::canonical::jats`, compared case-insensitively), not a copy; the attempts are `RETRIEVAL_ATTEMPTS`.

**Interfaces:**
- Consumes: Task 1's `LlmChat` (`num_ctx`, `stop`), `LlmContextModel`, `PicoError::Stopped`; `bmlib::quality::{PicoAgent, JsonChat}`, `ContextModel`, `ProcessingConfig`; Task 2; 2b's `interrogate::num_ctx`, `prompt::estimated_tokens`, `retry::with_backoff`, `MIN_QUOTE_SIMILARITY`, `UNKNOWN_WINDOW_FIT_TOKENS`, `FIT_CHARS_PER_TOKEN`, `RETRIEVAL_ATTEMPTS`, `RETRY_BASE_DELAY`; the full-text rule `ask` uses (`interrogate::full_text`, made `pub(crate)`).
- Produces:
  - `pub fn budget_chars(window: Option<u64>, title: &str) -> Option<usize>` — `(window.unwrap_or(UNKNOWN_WINDOW_FIT_TOKENS) − PICO_MAX_TOKENS − estimated_tokens(longest prompt) − estimated_tokens(title)) × FIT_CHARS_PER_TOKEN`, saturating at every step and converted with `usize::try_from(..).unwrap_or(usize::MAX)`, where the longest prompt is `PICO_SYSTEM_PROMPT` plus the longest of the suitability task + format, extraction task + format, and the two condensation prompts with `PICO_CONDENSE_QUERY` substituted; `None` when the result is under `PICO_MIN_BUDGET_CHARS`.
  - `pub fn suitability_text(canonical: &CanonicalText) -> String` — the abstract with its deeper sub-sections (above), else `canonical.text`.
  - `pub struct PicoContext<'a> { pub db: &'a mut dyn Db, pub chat: &'a mut dyn JsonChat, pub condenser_model: &'a dyn ContextModel, pub model: String, pub window: Option<u64>, pub retry_delay: Duration }` — the command passes `RETRY_BASE_DELAY`, tests `Duration::ZERO`.
  - `pub fn run_pico(ctx: &mut PicoContext, paper: &PaperId, force: bool, stop: &AtomicBool) -> Result<i64, PicoRunError>` — returns the stored run's id.
  - `pub enum PicoRunError { NoFullText, NotFullText, TooLong { window: u64 }, Cancelled, Pico(PicoError), Store(StoreError), Internal(String) }` (`Internal`: `PicoAgent::pico_condenser`'s `Err(String)`, which only a broken prompt constant can cause) with a user-readable `Display`: `TooLong` *The model's context window ({window} tokens) leaves no room for the paper. Choose a model with a larger window, or raise the context-window setting if it is lower than the model's.*; `Pico(UnusableReply)` *The model's reply could not be read ({msg}). Try another model.*; `Pico(Truncated)` *The model's reply reached the length limit ({PICO_MAX_TOKENS} tokens) and was cut off. Try another model.*; others bmlib's message.
  - `pub fn pico_chat<'a>(client: &'a LlmClient, model: &str, window: Option<u64>, stop: &'a AtomicBool) -> LlmChat<'a>` and `pub fn pico_condenser_model<'a>(…same…) -> LlmContextModel<'a>`, both with `num_ctx(window)` and the stop, so the command builds them in one place.

Flow: `full_text` (the paper's latest text, passed or confirmed); `budget_chars` (none → `TooLong`, its `window` the limit used, the floor when unknown); an agent over `ctx.chat` with `PicoAgent::pico_condenser(ctx.condenser_model, PICO_TEMPERATURE, PICO_MAX_TOKENS)` (its `Err` → `Internal`), `PICO_TEMPERATURE`, `PICO_MAX_TOKENS` and `ProcessingConfig::default().with_max_context_chars(budget).with_continue_on_error(false)`; unless `force`, `check_suitability(title, suitability_text)`; not suitable → store the check alone; else `extract(title, &canonical.text, MIN_QUOTE_SIMILARITY)` and store both. Each agent call goes through `with_backoff(RETRIEVAL_ATTEMPTS, ctx.retry_delay, |e| matches!(e, RequestFailed(_)) && !stop.load(SeqCst), …)`; the check is never re-asked when the extraction is retried. `stop` is read after each call and before `save_run`: set → `Cancelled`, the reply discarded. Any error with `stop` set, or `PicoError::Stopped`, is `Cancelled`. Every error is logged with the paper id (as `interrogate::fail`).

- [ ] **Step 1: Write failing tests** with a `FakeJsonChat` (scripted `Result<ChatJsonOutcome, ChatJsonError>` per call, recording the messages, optionally setting `stop` on a given call) and a `FakeModel: ContextModel` (one fixed short digest for any call, counting calls, optionally failing a given call or setting `stop`); text from `canonical_from_markdown` of a small trial (`## Abstract`, `## Methods` with `We enrolled 105 adults with asthma.`, `## Results`):
  - `budget_from_window`: `budget_chars(Some(100_000), title)` is `(100_000 − 4096 − estimated_tokens(longest) − estimated_tokens(title)) × 3`; `budget_chars(None, ..)` uses 8192; windows of 2048 and of `PICO_MAX_TOKENS` → `None`; a window whose budget is positive but under `PICO_MIN_BUDGET_CHARS` → `None`; `u64::MAX` does not overflow.
  - `check_reads_the_abstract`: the suitability request holds the abstract's words and not the Results'; an `ABSTRACT ` heading counts; a text with no `Abstract` heading, or one whose `Abstract` section and sub-sections are empty, sends the whole text.
  - `check_reads_a_jats_abstract`: on `tests/fixtures/real_rct.xml` through `canonical_from_jats`, the suitability request holds the abstract's sub-sections' words (the heading has no blocks of its own) and not the body's.
  - `suitable_then_extracted`: suitable, then an extraction quoting `We enrolled 105 adults with asthma.` → one stored run with both; the population part verified, its citation span's `span_text` the paper's.
  - `not_suitable_stops_at_check`: one chat call; the run stored with no extraction.
  - `forced_skips_check`: `force: true` → one call (the extraction); `suitability: None`.
  - `long_paper_is_condensed` (Review Focus 1): window 10 000 and a text over its budget → the extraction's request holds the fake digest, not the whole text.
  - `quotes_located_in_whole_text` (Review Focus 1): in that run, a quote from the paper but not in the digest is still verified, its span in `canonical.text`.
  - `failed_batch_fails_the_run` (Review Focus 1): the `FakeModel` fails one condensation call → `Err(Pico(CondensationFailed(_)))`, no extraction call, no run row (never a run from a partial digest).
  - `changed_number_is_not_cited` (Review Focus 2): an extraction quoting `We enrolled 150 adults with asthma.` → that quote in the part's `quotes_not_found`, no citation from it.
  - `too_small_window_refused`: `window: Some(PICO_MAX_TOKENS)` → `Err(TooLong { .. })`, no call.
  - `unconfirmed_text_refused`: → `Err(NotFullText)`, no call.
  - `request_failed_is_retried` (Review Focus 4): `RequestFailed` then success → stored, two calls (`retry_delay: Duration::ZERO`).
  - `retries_run_out` (Review Focus 4): a passed check, then `RequestFailed` on every extraction attempt → `Err(Pico(RequestFailed(_)))`, four chat calls (the check once, the extraction `RETRIEVAL_ATTEMPTS` times), no run row.
  - `stopped_is_not_retried`, `stop_stores_nothing` (Review Focus 4): `stop` set during the first call, which fails → `Err(Cancelled)`, one call, no run row.
  - `stop_during_successful_call` (Review Focus 4): `stop` set during a call that returns `Ok` → `Err(Cancelled)`, no row: (a) a not-suitable check is not stored; (b) after a suitable check, no extraction call; (c) during the extraction, the reply is discarded.
  - `stop_during_condensation` (Review Focus 4): the `FakeModel` sets `stop` inside `complete` → `Err(Cancelled)`, no extraction call, no row, no retry.
  - `failure_keeps_previous_run` (Review Focus 4): a stored run, then `UnusableReply` → `Err(Pico(_))`, `latest_run` still the first.
  - `pico_chat_carries_window`: `pico_chat(.., Some(8192), ..).num_ctx == NonZeroU32::new(8192)` and its stop is set; `None` → `None`.
  - `pico_condenser_model_carries_window` (Review Focus 1): the same for `pico_condenser_model`.
  - `display_names_the_fix`: `TooLong`'s message holds the window and the fix; `Pico(UnusableReply)`'s and `Pico(Truncated)`'s end *Try another model.*
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core pico`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): run PICO on the paper within the model's window`.

### Task 4: Views and commands

**Files:**
- Modify: `crates/studyanalyzer-core/src/view.rs`, `crates/studyanalyzer-core/src/llm_settings.rs`, `src-tauri/src/{commands.rs,state.rs,error.rs,lib.rs,contract.rs}`, `src/lib/fixtures/ipc_contract.json` (regenerated), `src/lib/api.ts`, `src/lib/contract.test.ts`

**Interfaces:**
- Consumes: Tasks 2–3; 2b's `CitationView`, `Utf16Span`, `utf16_offsets`, `span_to_utf16`, `stored_paper_view`, `StopSlot`, `cached_window`, `running_window`.
- Produces (core, `view.rs`, all `Serialize`):
  - `pub struct PicoPartView { pub part: String /* "population" … */, pub description: String, pub state: String /* "verified" | "unverified" | "not_reported" */, pub confidence: Option<f64>, pub citations: Vec<CitationView>, pub quotes_not_found: Vec<String> }` — citations numbered from 1 across the run, in part order.
  - `pub struct PicoExtractionView { pub parts: Vec<PicoPartView>, pub study_type: Option<String>, pub sample_size: Option<String>, pub overall_confidence: Option<f64>, pub condensed_from_chars: Option<i64>, pub notes: Vec<String> }`.
  - `pub struct SuitabilityView { pub is_suitable: bool, pub study_type: Option<String>, pub confidence: Option<f64>, pub rationale: Option<String>, pub condensed_from_chars: Option<i64>, pub notes: Vec<String> }`. Both views carry bmlib's notes (a condensation note among them) and the run's `condensation_status` string where it is not `completed`, for the card to show.
  - `pub struct PicoRunView { pub id: i64, pub canonical_id: i64, pub model: String, pub created_at: String, pub read_only: bool, pub suitability: Option<SuitabilityView>, pub extraction: Option<PicoExtractionView> }` (`read_only`: not the newest run on the latest text).
  - `pub struct PicoView { pub canonical_id: i64, pub current: Option<PicoRunView>, pub earlier: Vec<PicoRunSummary> }`.
  - `pub fn pico_run_view(db, run_id) -> Result<PicoRunView, StoreError>`, `pub fn pico_view(db, paper) -> Result<PicoView, StoreError>`.
- Produces (core, `llm_settings.rs`): `pub fn lacks_key(provider: &str, client: &LlmClient) -> bool` — the provider needs a key and the client has none.
- Produces (commands, each in `handler()`, `commands()`, `api.ts`, the vitest call list):
  - `pico(paperId) -> PicoView` (loads, never runs); `pico_run(runId: number) -> PicoRunView`.
  - `run_pico(paperId, force: boolean) -> PicoView` — refused as `invalid_input` (*A PICO extraction is already running.*) while `AppState.pico_stop` holds a flag (a second run would replace the first's flag, and *Stop* could no longer reach it); then `interrogate::ready_to_ask` (model chosen, else `needs_model`; then the text, `not_full_text`; mapped by `from_ask`), then `lacks_key` (`needs_api_key`) before `cached_window`, window as `ask_with` gets it; `AppState.pico_stop: StopSlot` begun before the blocking task as `ask` does (a `StopSlot::is_running` added, tested in `state.rs`).
  - `stop_pico() -> ()`, as `stop_answer`.
  - `CommandError::from_pico(PicoRunError, provider)`: `NoFullText`, `NotFullText` → `not_full_text`; `TooLong` → `invalid_input`; `Cancelled` and `Pico(Stopped)` → `cancelled`; `Store` → `storage`; `Internal` and `Pico(InvalidSimilarity | EmptyInput)` → `internal`; other `Pico` (`UnusableReply`, `Truncated`, `CondensationFailed`, `RequestFailed`, and any later variant: `PicoError` is `#[non_exhaustive]`) → `llm`, naming the provider.
  - Contract: fixture keys for a `PicoView` with a verified part on `contract.rs`'s astral `PAPER`, a not-suitable run and `current: null`; `RETURN_TYPES` gains `PicoView`, `PicoRunView`.

- [ ] **Step 1: Write failing tests:**
  - in `view.rs`: `pico_citation_spans_slice_same_text` (astral paper: each citation's UTF-16 span slices to its `text`); `citation_text_is_the_papers` (Review Focus 2: a citation's `text` is `span_text` of its stored span, never anything else); `numbers_run_across_parts` (two parts' citations numbered 1, 2, 3); `earlier_run_uses_its_text` (Review Focus 3, as `earlier_conversation_uses_its_text`: after a new document, `current` is `None`, `canonical_id` the new text's, the old run in `earlier`, and read alone it is `read_only` with the old `canonical_id`); `older_run_on_same_text_is_read_only` (two runs on one text: `current` the newer, the older `read_only`); `condensation_notes_shown` (a run with notes and a `partial` status carries both).
  - in `llm_settings.rs`: `lacks_key_for_keyed_provider` (a keyed provider without a key → true; `ollama` → false; a keyed provider with its key from the environment → false).
  - in `state.rs`: `is_running` between `begin` and `end`.
  - in `error.rs`: a loop over every `PicoRunError` variant and, for `Pico`, every `PicoError` variant (as 2b's `from_ask` loop), `Pico(Stopped)` → `cancelled` among them.
  - the contract tests fail until the fixture is regenerated.
- [ ] **Step 2: Run** `cargo test --workspace`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `UPDATE_CONTRACT=1 cargo test -p studyanalyzer contract`, review the fixture's diff, then every gate. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): commands for running and showing PICO`.

### Task 5: Frontend logic

**Files:**
- Create: `src/lib/pico.ts`, `src/lib/pico.test.ts`

**Interfaces:**
- Consumes: Task 4's `api.ts` types; `chat.ts`'s mark shape (`{ span, current }`).
- Produces:
  - `PART_LABELS: Record<string, string>` — `population` → *Population*, `intervention` → *Intervention*, `comparison` → *Comparison*, `outcome` → *Outcome*.
  - `partNote(part: PicoPartView): string | null` — `verified` → `null`; `unverified` → *Unsure – model could not reproduce {Label} from text provided*; `not_reported` → *Not clearly stated*.
  - `sampleSizeNote(): string` — *As the model gave it; not checked against the paper.* (shown under study type and sample size).
  - `picoMarks(run: PicoRunView, current: number | null)` — every citation of every part as a mark, by its `CitationView.number` (never recounted here), `current` for the clicked number.
  - `runCaption(run: PicoRunView, latestCanonicalId: number): string` — model and date (`parseSqliteUtc`), *condensed from N characters* when condensed, *on an earlier version of the text* when `canonical_id` differs.
  - `runNotes(run: PicoRunView): string[]` — bmlib's notes from the check and the extraction, and *Condensation ended {status}; the digest may be incomplete.* when a status other than `completed` was stored, so nothing about how the run went lives only in the database.
  - `shouldAutoRun(view: PicoView, askable: boolean, modelChosen: boolean, attempted: boolean): boolean` — true only when `current` is `null`, the text may be asked about, a model is chosen, and no run was attempted (started, whatever its end: done, failed or stopped) since the paper was opened. `attempted` lives in the tab host, per paper, so a tab switch does not reset it.

- [ ] **Step 1: Write failing tests** in `pico.test.ts`: `partNote` per state with the exact strings above; `picoMarks` with two parts, the current number in the second part; `runCaption` for a condensed run, an earlier-text run and a plain one; `runNotes` with notes and a `partial` status, and with none; `shouldAutoRun` (Review Focus 5): false with a current run, false when not askable, false with no model, false once attempted (a failed or stopped first run, or one still going), true otherwise.
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm run test:web` and `pnpm run check:web`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): PICO labels, marks and run captions`.

### Task 6: The PICO tab

**Files:**
- Create: `src/components/PicoTab.svelte`, `src/components/PicoCard.svelte`
- Modify: `src/components/Interrogator.svelte` (the tab host: a *PICO* tab between *Interrogator* and *Original*; its doc comment says it hosts the paper's tabs; `busy` covers a PICO run too; it holds the PICO run state, the running flag and `attempted` per paper, as it holds `live`, because its tabs are `{#if}` branches and `PicoTab` remounts on every switch)

Behaviour:
- **Card:** the four parts in order, each with its description, `partNote`, confidence, `[n]` buttons per citation with the paper's wording, and its not-found quotes in a muted list; then study type and sample size with `sampleSizeNote`; `runCaption` and `runNotes`; *Re-run* and the earlier runs (a read-only one shows its text via `paper_view_at`). Not suitable: *This paper does not look like an intervention study, so it may have no PICO*, the study type and rationale, the check's `runNotes`, and *Run anyway* (`run_pico(force: true)`).
- **Running:** *Extracting PICO… A long paper is condensed first and can take several minutes.* with *Stop* (`stop_pico`); the previous run stays visible beneath. Leaving the tab and coming back shows the same run still going (the host's state), never a second one.
- **Opening:** `pico()`, then `run_pico(false)` when `shouldAutoRun` (the model from `llmSettings()`); with no model, *Choose a model in Settings to extract PICO* and a button to Settings.
- **Citation click:** sets the current mark; the paper scrolls to it; on phones the *PICO | Paper* switch flips to *Paper*.
- **Errors:** `needs_model` / `needs_api_key` open Settings, as the Interrogator does; `not_full_text` shows `askBlockedNote`; others go to `ErrorBanner`.

- [ ] **Step 1:** Implement the components (Svelte 5 runes), each documenting itself in a `/** … */` at the top of its script.
- [ ] **Step 2: Run** every gate. Expected: PASS.
- [ ] **Step 3: Check it by hand** with `pnpm dev`, a cloud model and a local Ollama model (owed to the maintainer if no one can click the window): an RCT from PMC, so JATS (four verified parts; click → scroll, current dark, others pale; the check read the abstract); a review (not suitable, rationale, *Run anyway*); a long paper on a small window (condensed caption, quotes still highlight); *Stop* during condensation (nothing stored), then another tab and back (no auto-run); leaving the tab mid-run and back (the same run, no second); Wi-Fi off during condensation (the error after the retries, nothing stored, the previous run kept); *Re-run* with another model (both runs listed); no model (the prompt, no jump to Settings) / no key (Settings, no request); an unconfirmed text (no auto-run); a narrow window; dark mode; quit and reopen (the run shows at once, no new call).
- [ ] **Step 4: Commit** `feat(ui): the PICO tab`.

### Task 7: Hand over

**Files:**
- Modify: `HANDOVER.md`, `ROADMAP.md`, `CLAUDE.md` (*State of the repository*), `README.md` (the PICO tab)

- [ ] **Step 1:** Run every gate. Expected: PASS.
- [ ] **Step 2:** Update the docs: 2c landed; what later analysis tabs build on (`pico_store`'s shape, `has_dependents`, the budget rule, `shouldAutoRun`); the click-through owed; anything found filed as issues.
- [ ] **Step 3: Commit** `docs: phase 2c landed`, push, open the PR.

### Changed while building

(Empty until the build. Record here every name Task 1's PR lands under and
every point the build settles differently, with the commit.)
