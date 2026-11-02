# Phase 2b: the Interrogator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chat with one paper: every answer streams in, every claim carries a
quote that `find_passage` locates in the canonical text, and the paper pane
highlights the paper's own span (current `#e8b931`, earlier `#f6e7b0`).

**Architecture:** As 2a: logic in `crates/studyanalyzer-core` (pure
functions and small traits over bmlib, faked in tests), thin commands in
`src-tauri`, Svelte 5 in `src/`. One answer is one *turn* (question, answer,
status, citations), stored when it ends. Whole-paper mode when the prompt
fits the running window, else retrieval over bmlib's chunk store. Offsets are
code points everywhere in Rust; `view.rs` converts citation spans to UTF-16
as it already does for blocks.

**Tech Stack:** Rust 2021, Tauri 2 (`tauri::ipc::Channel` for streamed
events), bmlib at the pin (features `pdf`, `html`, **`embed`**), `keyring`
(MIT OR Apache-2.0), Svelte 5 + TypeScript + Vite, vitest.

**Spec:** `docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`
(§2 anti-goals, §5.1, §6.1–6.3, §6.5, §6.6, §8). Builds on HANDOVER.md
*Phase 2a* (calls for 2b) and *Landed* items 1, 2, 3, 6, 8, 11.

### Builder's calls this plan settles (spec §11)

- **Quote marking:** the model writes each supporting quote inline, right
  after its claim, as `<q>exact words from the paper</q>`. Inline tags
  survive streaming (no trailing block to lose when a stream breaks), need no
  tools (bmlib refuses tools on a stream) and are parsed only on the final
  `content` (HANDOVER, item 6). The live view shows a quote as *checking…*.
- **What a quote is checked against:** the whole canonical text, in both
  modes. The stored and shown span is `find_passage`'s, the shown wording is
  `canonical.span_text(span)`, never the model's.
- **Threshold:** `MIN_QUOTE_SIMILARITY` starts at bmlibrarian's
  `CITATION_MIN_SIMILARITY` (0.95) and is set by Task 4's measurement.
- **Retrieval parameters:** the current question alone is the query (not the
  conversation; spec §10's warning); `RETRIEVAL_PASSAGES = 8`, sent in
  document order under their section headings; no query rewriting, no keyword
  search in 2b.
- **Fit:** tokens are estimated at `FIT_CHARS_PER_TOKEN = 3` code points each
  (stricter than bmlib's 4, since an Ollama overflow cuts the paper silently),
  plus `ANSWER_TOKENS = 4096` reserved for the answer. Running window =
  `min(setting, maximum)`; maximum unknown and no setting → retrieval mode
  (HANDOVER item 11). A retrieval prompt that still does not fit is refused
  ("start a new conversation"), never cut. `num_ctx` is the running window on
  every request.
- **Converter changes (§5.1):** a canonical text that has a conversation with
  at least one turn is a *dependent*: it is never re-derived, and the same
  file opened again under a newer converter keeps it. A different document
  stores a new text as today; conversations stay bound to the text they were
  held on, are shown with that text and are read-only. Chunk sets are not
  dependents (their label makes them stale, and they are rebuilt); a set is
  keyed by canonical text id.
- **Incomplete answers:** a stopped or broken stream stores the turn as
  `stopped` / `failed` with what arrived and **no citations checked**, shown
  with *Incomplete answer: its quotes were not checked.* A failure before any
  text arrived stores nothing and rejects with a `CommandError`.
- **History:** follow-up prompts carry earlier `complete` turns of the same
  conversation (question and raw answer); stopped and failed ones are left out.
- **Model parameters (§6.6):** 2b sends temperature `ANSWER_TEMPERATURE` (0)
  and no other parameter; per-model parameters are filed as an issue in
  Task 11.
- **Keychain:** in this repository, behind a core trait. Neither bmlib
  language has one today; see *Open questions*.

### Open questions for the maintainer (do not block Tasks 1–5)

1. **Keychain in bmlib?** "Anything another app could use" suggests a
   `bmlib` secret store (Rust `keyring`, Python `keyring`). This plan keeps a
   small trait in core and the `keyring` impl in `src-tauri`, so moving it is
   one file. Say if it should go to bmlib first.
2. **bmlib #528 / #529** (superscript digits; quotes of common words) change
   which quotes Task 4 accepts. Task 4 records the cases that hit them.

## Global Constraints

- Everything in 2a's Global Constraints still holds (gates, `config.rs` for every constant, doc comments, tests on outward-facing items, no truncation, errors logged and shown, sandbox, code points in Rust).
- Never fall back to the abstract or the model's general knowledge: `ask` refuses a paper whose latest text neither passed the full-text check nor was confirmed.
- A citation shows and stores the paper's span; a quote that is not located is listed as not found, never dropped (*"1 quote could not be found in the paper"*, *"N quotes could not be found in the paper"*).
- Highlights: current `var(--hl-current)` (`#e8b931`), earlier `var(--hl-past)` (`#f6e7b0`), already in `app.css`.
- Phone layout (below `PHONE_BREAKPOINT_PX = 700` CSS px): a *Chat | Paper* switch replaces the split; tapping a citation flips to *Paper*.
- API keys only in the OS keychain (service `com.bmlibrarian.studyanalyzer`, account = provider name), env var fallback through bmlib's `ProviderSpec::resolve_api_key`; never in SQLite, a file or a log line, never returned to the frontend.
- No model hard-wired: with no model chosen, `ask` rejects with `needs_model`.
- A new command goes in `handler()`, `commands()`, `RETURN_TYPES` (new return type), `api.ts` and the vitest call list together (HANDOVER, contract notes).

## Review Focus

1. **A quote with a changed number or a dropped "not"** (model: `HR 0.59`, paper: `HR 0.95`): it may pass a near-verbatim threshold, so the user must see the paper's own wording. Test in Task 2 (`located_text_is_the_papers`) and Task 8 (`turn_view_shows_paper_wording`).
2. **Astral characters before a cited passage** (an emoji or `𝛼` in an earlier section): the highlight lands on Rust's span. Test in Task 8 (`citation_spans_slice_same_text`).
3. **A stream stopped or broken inside a quote** (`… <q>Patients over 7`): the live view never shows a raw tag, the stored turn has no citations and says so. Tests in Task 2 (`unclosed_quote_is_marked`), Task 7 (`stopped_turn_has_no_citations`, `broken_stream_keeps_partial`), Task 9 (`live_holds_back_partial_tag`).
4. **A long paper on Ollama with no known window, or a conversation that outgrows the window:** retrieval mode, or a refusal naming the fix, never a cut prompt; `num_ctx` always sent. Tests in Task 3 (`unknown_window_is_retrieval`) and Task 7 (`too_long_refused`, `num_ctx_is_sent`).
5. **A paper whose text changes after a conversation** (a converter bump, or a new file attached): earlier highlights never move. Tests in Task 1 (`text_with_turns_is_not_rederived`, `same_file_after_bump_keeps_text`) and Task 8 (`earlier_conversation_uses_its_text`).

---

### Task 1: Conversations in storage, and dependents

**Files:**
- Create: `crates/studyanalyzer-core/src/conversations.rs`
- Modify: `schema.rs` (migration 5), `store.rs` (`open_store`/`open_memory_store` call `bmlib::embed::store::ensure_schema` after migrations), `open.rs` (the `false` passed to `needs_rederive` in `reopen`, and `persist`'s `StoredCopy::Older` arm), `lib.rs`

**Interfaces:**
- Consumes: `PaperId`, `StoreError`, `bmlib::db::*` as `store.rs` uses them.
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")] pub enum TurnStatus { Complete, Stopped, Failed }` and `pub enum Mode { WholePaper, Retrieval }` (same derives), each with `as_str()` / `parse()`.
  - `pub struct NewQuote { pub number: Option<u32>, pub quote: String, pub span: Option<TextSpan>, pub similarity: f64 }` — `number` and `span` are `Some` together (located).
  - `pub struct NewTurn { pub question: String, pub answer: String, pub status: TurnStatus, pub error: Option<String>, pub model: String, pub mode: Mode, pub passages: Vec<TextSpan>, pub quotes: Vec<NewQuote>, pub input_tokens: i64, pub output_tokens: i64 }`.
  - `pub struct StoredTurn { pub id: i64, /* every NewTurn field */ pub created_at: String }`.
  - `pub struct ConversationSummary { pub id: i64, pub canonical_id: i64, pub turns: usize, pub created_at: String }` (`Serialize`).
  - `pub fn create_conversation(db: &mut dyn Db, paper: &PaperId, canonical_id: i64) -> Result<i64, StoreError>`.
  - `pub fn current_conversation(db: &mut dyn Db, paper: &PaperId, canonical_id: i64) -> Result<Option<i64>, StoreError>` — highest id on that text.
  - `pub fn conversations(db: &mut dyn Db, paper: &PaperId) -> Result<Vec<ConversationSummary>, StoreError>` — every one, newest first.
  - `pub fn conversation_text(db: &mut dyn Db, conversation_id: i64) -> Result<Option<(PaperId, i64)>, StoreError>`.
  - `pub fn save_turn(db: &mut dyn Db, conversation_id: i64, turn: &NewTurn) -> Result<i64, StoreError>` (one transaction with its quotes).
  - `pub fn turns(db: &mut dyn Db, conversation_id: i64) -> Result<Vec<StoredTurn>, StoreError>` — in order.
  - `pub fn has_turns(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError>` — the `has_dependents` answer.

Migration 5:

```sql
CREATE TABLE conversations (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  canonical_id INTEGER NOT NULL REFERENCES canonical_texts(id), created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE turns (id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  question TEXT NOT NULL, answer TEXT NOT NULL, status TEXT NOT NULL, error TEXT, model TEXT NOT NULL, mode TEXT NOT NULL,
  passages_json TEXT NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE quotes (id INTEGER PRIMARY KEY, turn_id INTEGER NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, number INTEGER, quote TEXT NOT NULL, span_start INTEGER, span_end INTEGER, similarity REAL NOT NULL);
```

A stored span outside its text, or `number` without a span, reads back as `StoreError::Corrupt`.

- [ ] **Step 1: Write failing tests** in `conversations.rs` (on `open_memory_store()`, a paper and canonical text made as `store.rs`'s tests make them):
  - `turn_round_trip`: a turn with one located and one unlocated quote reads back equal, quotes in order.
  - `current_is_newest_on_that_text`: two conversations on text 1, one on text 2 → `current_conversation(.., 1)` is the second; `conversations` lists all three newest first.
  - `empty_conversation_is_no_dependent`: `has_turns` false after `create_conversation`, true after `save_turn`.
  - `corrupt_span_reported`: a quote row set by SQL to `span_end` past the text → `Err(Corrupt(_))`.
  - in `open.rs` (Review Focus 5): `text_with_turns_is_not_rederived` (2a's `reopen_rederives_old_version` with a saved turn → no new canonical row, the notice-free old text opens); `same_file_after_bump_keeps_text` (the `StoredCopy::Older` path with a turn on the stored text → no new canonical row); `new_document_keeps_old_conversation` (attach different bytes → new latest text; `conversation_text` of the old conversation still names the old text id).
  - `schema.rs`: `versions_are_sequential` expects `[1, 2, 3, 4, 5]`; `store.rs`: `chunk_schema_exists` (bmlib's chunk table is there after `open_memory_store`).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core conversations open schema store`. Expected: FAIL.
- [ ] **Step 3: Implement** the items above; pass `has_turns(db, stored.id)?` where `reopen` passes `false`, and treat `StoredCopy::Older` as `Same` when `has_turns` is true (logged at `info`).
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): store conversations and keep a text that has one`.

### Task 2: Quote marks and citations

**Files:**
- Create: `crates/studyanalyzer-core/src/answer.rs`
- Modify: `config.rs` (`QUOTE_OPEN: &str = "<q>"`, `QUOTE_CLOSE: &str = "</q>"`, `MIN_QUOTE_SIMILARITY: f64 = 0.95`), `lib.rs`

**Interfaces:**
- Consumes: `bmlib::citations::passage_match::find_passage`, `CanonicalText`, `NewQuote` (Task 1).
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub enum Piece { Text(String), Quote { text: String, closed: bool } }`.
  - `pub fn parse_answer(content: &str) -> Vec<Piece>` — splits on exactly `<q>`/`</q>` (case-sensitive); a `</q>` with no open quote is text; an open quote at the end is `closed: false`; no nesting (a `<q>` inside a quote is part of it). Adjacent text pieces merge.
  - `pub fn clean_quote(quote: &str) -> &str` — trims whitespace, then any run of `"'“”‘’«»` and `…` / `...` at either end.
  - `pub struct Located { pub pieces: Vec<AnswerPiece>, pub quotes: Vec<NewQuote> }`, `#[derive(Serialize)] #[serde(tag = "kind", rename_all = "snake_case")] pub enum AnswerPiece { Text { text: String }, Cite { number: u32 } }`.
  - `pub fn locate(pieces: &[Piece], canonical: &CanonicalText, min_similarity: f64) -> Located` — each closed quote, cleaned, goes to `find_passage(quote, &canonical.text, min_similarity)`; a match gets the next number (from 1) and a `Cite` piece where it stood; a rejected or unclosed quote gets `number: None` and no piece. `quotes` keeps every quote in answer order. A cleaned quote that is empty is unlocated.
  - `pub fn answer_pieces(answer: &str, quotes: &[NewQuote]) -> Vec<AnswerPiece>` — rebuilds the pieces for a stored turn (Task 8) from its raw answer and stored quotes, so storage keeps the raw answer only.

- [ ] **Step 1: Write failing tests** in `answer.rs` (canonical text from `canonical_from_markdown` of a small paper with `Of 412 patients, 61 were over 75 years.` and `The hazard ratio was 0.95.`):
  - `parses_quotes`: `"A <q>x</q> b <q>y</q>."` → `[Text("A "), Quote(x, true), Text(" b "), Quote(y, true), Text(".")]`.
  - `stray_close_is_text`, `unclosed_quote_is_marked` (Review Focus 3): `"A <q>par"` → last piece `Quote { text: "par", closed: false }`.
  - `clean_strips_marks`: `clean_quote(" “…61 were over 75 years.” ")` → `"61 were over 75 years."`.
  - `located_text_is_the_papers` (Review Focus 1): the quote `of 412 patients, 61 were over 75 years` (case, no final stop) is located, and `canonical.span_text(span)` is the paper's `Of 412 patients, 61 were over 75 years.`-span wording, not the quote.
  - `paraphrase_unlocated`: `<q>a quarter of the patients were elderly</q>` → `number: None`, no `Cite` piece, still in `quotes`.
  - `numbers_in_order`: two located and one unlocated between them → numbers 1, 2; the unlocated keeps its position in `quotes`.
  - `pieces_rebuild`: `answer_pieces(raw, &located.quotes) == located.pieces`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core answer`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): parse quote marks and locate them in the paper`.

### Task 3: Fit, mode and the prompt

**Files:**
- Create: `crates/studyanalyzer-core/src/prompt.rs`
- Modify: `config.rs` (`FIT_CHARS_PER_TOKEN: u64 = 3`, `ANSWER_TOKENS: u64 = 4096`, `ANSWER_TEMPERATURE: f64 = 0.0`, `RETRIEVAL_PASSAGES: usize = 8`), `lib.rs`

**Interfaces:**
- Consumes: `CanonicalText`, `StoredTurn`, `Mode`, `TurnStatus` (Task 1), `bmlib::llm::{LLMMessage}`, `bmlib::embed::store::ChunkHit`.
- Produces:
  - `pub const SYSTEM_PROMPT: &str` — the exact text below.
  - `pub fn running_window(setting: Option<u64>, maximum: Option<u64>) -> Option<u64>` — the smaller of the two that are known.
  - `pub fn estimated_tokens(chars: usize) -> u64` — `ceil(chars / FIT_CHARS_PER_TOKEN)`.
  - `pub fn fits(window: Option<u64>, messages: &[LLMMessage]) -> bool` — code points of every message's content, estimated, plus `ANSWER_TOKENS`, at most `window`; `None` never fits.
  - `pub fn paper_context(canonical: &CanonicalText) -> String` — the title, then each section's heading as `## heading` (`#` repeated `level + 1` times) and its blocks separated by blank lines, table rows as they are; every block, never cut.
  - `pub fn passages_context(canonical: &CanonicalText, hits: &[ChunkHit]) -> String` — the hits in document order, each under its section's heading, separated by `\n\n---\n\n`.
  - `pub fn build_messages(context: &str, history: &[StoredTurn], question: &str) -> Vec<LLMMessage>` — system `SYSTEM_PROMPT`; a user message `The paper:\n\n{context}`; then per `Complete` turn of `history` a user (question) and an assistant (raw answer) message; then the question.
  - `pub fn choose_mode(window: Option<u64>, whole: &[LLMMessage]) -> Mode` — `WholePaper` iff `fits(window, whole)`.

```text
You answer questions about one biomedical paper, using only the text of the paper given below.
- Answer only from the paper. If the paper does not say, answer that the paper does not say. Never use outside knowledge.
- Back every claim with a quote copied exactly from the paper, written straight after the claim as <q>exact words from the paper</q>.
- Copy quotes word for word: no ellipses, no paraphrase, no added words. Quote a whole sentence or a clause of one.
- Quote the numbers as the paper gives them.
```

- [ ] **Step 1: Write failing tests** in `prompt.rs`:
  - `window_is_the_smaller`: `(Some(8000), Some(128000))` → `Some(8000)`; `(None, Some(128000))` → `Some(128000)`; `(Some(8000), None)` → `Some(8000)`; `(None, None)` → `None`.
  - `unknown_window_is_retrieval` (Review Focus 4): `choose_mode(None, &msgs)` → `Retrieval` for a one-line paper.
  - `fit_counts_answer_room`: messages of exactly `(window - ANSWER_TOKENS) * FIT_CHARS_PER_TOKEN` code points fit; one more does not; astral characters count one each.
  - `paper_context_is_whole`: every block's text of a 3-section canonical text appears in `paper_context`, in order, with its heading.
  - `passages_in_document_order`: hits given best-first `[c3, c1]` come out `c1` then `c3`, each after its heading.
  - `history_skips_incomplete`: one `Complete` and one `Stopped` turn → 2 + 2 + 1 messages, the stopped one absent.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core prompt`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): decide whole-paper or retrieval mode and build the prompt`.

### Task 4: Measure the acceptance threshold

**Files:**
- Create: `crates/studyanalyzer-core/tests/fixtures/quotes.json`, `crates/studyanalyzer-core/tests/quote_threshold.rs`
- Modify: `config.rs` (`MIN_QUOTE_SIMILARITY`, with a comment naming the measurement)

Fixture shape: `{ "paper": "article.xml", "cases": [ { "quote": "…", "accept": true, "why": "…" } ] }`, the paper being the existing JATS fixture's canonical text. At least 40 cases, half each way:
- **accept**: exact sentences; changed case; curly vs straight quotes and dashes; collapsed or doubled whitespace; accents dropped; one dropped or doubled short word in a 20+ word sentence; a quote across a line break or two sentences; a table row's cells space-joined.
- **reject**: paraphrases; a sentence from another paper; two sentences from far apart joined; a quote that keeps the words but reverses the finding (`increased` for `decreased`) in a short sentence; an invented sentence on the paper's topic; a quote with an ellipsis in the middle.

- [ ] **Step 1: Write the test** `threshold_separates_labelled_quotes`: for every case, `find_passage(clean_quote(q), &text, MIN_QUOTE_SIMILARITY)` matches iff `accept`; on failure print each case's similarity. And `print_margins` (`#[ignore]`): prints the lowest accepted and highest rejected similarity.
- [ ] **Step 2: Collect real quotes** with an `#[ignore]`d `collect_model_quotes` in the same file: given `STUDYANALYZER_TEST_MODEL` (e.g. `anthropic:<model>`) and its key in the environment, ask it 10 questions about the fixture paper with Task 3's `SYSTEM_PROMPT` and `build_messages`, and print every `<q>` quote with its similarity. Label each by hand (is it the paper's words?) and add every one scoring between 0.80 and 1.0 to the fixture with its true label.
- [ ] **Step 3: Run** `cargo test -p studyanalyzer-core --test quote_threshold -- --include-ignored`. Set `MIN_QUOTE_SIMILARITY` midway between the margins (rounded to 0.01), unless an accept case falls below a reject case: then keep the value that rejects every reject case, mark those accept cases `"accept": false` with a `why` naming the overlap, and file a bmlib issue if one is #528/#529's kind.
- [ ] **Step 4: Run** the gates. Expected: PASS.
- [ ] **Step 5: Commit** `test(core): set the quote threshold by measurement`, the commit body naming the margins and the model used.

### Task 5: Passages by embedding

**Files:**
- Create: `crates/studyanalyzer-core/src/passages.rs`
- Modify: root `Cargo.toml` (bmlib features `["pdf", "html", "embed"]`), `config.rs` (`EMBEDDING_MODEL: bmlib::embed::ModelFile = bmlib::embed::ARCTIC_EMBED_L_V2_F16`, `EMBEDDER_SUBDIR: &str = "models"`), `lib.rs`

**Interfaces:**
- Consumes: `bmlib::embed::store::{index_paper, search, ChunkEmbedder, ChunkHit, StoreError as ChunkStoreError}`.
- Produces:
  - `pub fn index_key(canonical_id: i64) -> String` — `"canonical:{id}"`.
  - `pub fn find_passages(db: &mut dyn Db, canonical_id: i64, canonical: &CanonicalText, question: &str, embedder: &mut dyn ChunkEmbedder, on_progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Vec<ChunkHit>, PassageError>` — `search(.., RETRIEVAL_PASSAGES)`; on `NotIndexed`, `Stale(_)` or `Corrupt(_)` (logged at `info`/`warn`), `index_paper` with `on_progress`, then search once more.
  - `pub enum PassageError { Cancelled, Store(ChunkStoreError) }` with `Display`, `Error`.

- [ ] **Step 1: Write failing tests** with a `FakeEmbedder` (`model()` = `Some(&EMBEDDING_MODEL)`; a document vector from word hashes, L2-normalised to 256 dims, so a question sharing words with a chunk scores highest):
  - `indexes_then_finds`: first call indexes (progress seen) and returns the chunk holding the question's words first; a second call indexes nothing.
  - `stale_reindexes`: a set indexed from another text under the same key is re-indexed, then found.
  - `cancel_keeps_nothing_new`: `on_progress` returning false → `Err(Cancelled)`.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core passages`. Expected: FAIL.
- [ ] **Step 3: Implement.** Enabling `embed` builds llama.cpp (C++ and CMake; present on `macos-latest`). If CI needs more, change `ci.yml` and CLAUDE.md § Gates together.
- [ ] **Step 4: Run** again, then every gate. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): find a paper's passages by embedding`.

### Task 6: Model settings and the keychain

**Files:**
- Create: `crates/studyanalyzer-core/src/llm_settings.rs`, `src-tauri/src/secrets.rs`
- Modify: `config.rs` (`LLM_MODEL_KEY: &str = "llm_model"`, `CONTEXT_TOKENS_KEY: &str = "context_tokens"`, `KEYCHAIN_SERVICE: &str = "com.bmlibrarian.studyanalyzer"`), `src-tauri/Cargo.toml` (`keyring`: read its docs for the platform-store features first; bottie pins `4.1` with feature `v1`)

**Interfaces:**
- Consumes: `bmlib::llm::{provider_specs, parse_model_string, ProviderSpec}`, `store::{get_setting, set_setting}`.
- Produces (core):
  - `pub trait SecretStore: Send + Sync { fn get(&self, provider: &str) -> Result<Option<String>, SecretError>; fn set(&self, provider: &str, key: &str) -> Result<(), SecretError>; fn delete(&self, provider: &str) -> Result<(), SecretError>; }`, `pub struct SecretError(pub String)` (never holds the key), and `pub struct MemorySecrets` for tests.
  - `#[derive(Serialize)] pub struct ProviderInfo { pub name: String, pub default_model: String, pub needs_key: bool, pub has_key: bool, pub key_env: Option<String> }`; `#[derive(Serialize)] pub struct LlmSettings { pub model: Option<String>, pub context_tokens: Option<u64>, pub providers: Vec<ProviderInfo> }`.
  - `pub fn llm_settings(db: &mut dyn Db, secrets: &dyn SecretStore) -> Result<LlmSettings, SettingsError>` — `has_key` is keychain-or-env.
  - `pub fn set_model(db, model: &str) -> Result<(), SettingsError>` — refuses an empty model or an unknown provider (`parse_model_string`, then `provider_specs()`).
  - `pub fn set_context_tokens(db, tokens: u64)` (refuses 0) and `pub fn clear_context_tokens(db)`.
  - `pub fn api_key(spec: &ProviderSpec, secrets: &dyn SecretStore) -> Result<Option<String>, SecretError>` — the keychain's, else `spec.resolve_api_key(None)`.
  - `pub enum SettingsError { Invalid(String), Store(StoreError), Secret(SecretError) }`.
- Produces (`src-tauri/src/secrets.rs`): `pub struct KeyringSecrets` implementing `SecretStore` over `keyring`, caching each provider's answer for the launch (one keychain prompt per provider, as bottie found necessary on macOS). A keyring error becomes `SecretError` with the platform's message but no key material.

- [ ] **Step 1: Write failing tests** in `llm_settings.rs`: `model_remembered` (set then read back); `unknown_provider_refused` (`"nosuch:model"` → `Invalid`); `zero_window_refused`; `keychain_wins_over_env` (with `MemorySecrets` holding `k1` for a provider whose `api_key_env` is set to `k2` → `k1`; deleted → `k2`); `key_never_in_settings` (after `set` through `MemorySecrets`, no `settings` row contains the key).
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core llm_settings`. Expected: FAIL.
- [ ] **Step 3: Implement** both files.
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat: model settings, API keys in the keychain`.

### Task 7: Asking a question

**Files:**
- Create: `crates/studyanalyzer-core/src/interrogate.rs`
- Modify: `lib.rs`

**Interfaces:**
- Consumes: Tasks 1, 2, 3, 5; `bmlib::llm::{LlmClient, ChatRequest, ChatError, StreamDelta, LLMResponse}`.
- Produces:
  - `pub trait ChatStreamer { fn chat_stream(&self, request: &ChatRequest, on_delta: &mut dyn FnMut(&StreamDelta) -> ControlFlow<()>) -> Result<LLMResponse, ChatError>; }`, implemented for `LlmClient`.
  - `#[derive(Serialize)] #[serde(tag = "kind", rename_all = "snake_case")] pub enum AnswerEvent { Indexing { done: usize, total: usize }, Delta { text: String } }`.
  - `pub struct AskContext<'a> { pub db: &'a mut dyn Db, pub llm: &'a dyn ChatStreamer, pub embedder: Option<&'a mut dyn ChunkEmbedder>, pub model: String, pub window: Option<u64> }`.
  - `pub fn ask(ctx: &mut AskContext, paper: &PaperId, question: &str, stop: &AtomicBool, on_event: &mut dyn FnMut(AnswerEvent)) -> Result<i64, AskError>` — returns the saved conversation id.
  - `pub enum AskError { EmptyQuestion, NoFullText, NotFullText, NeedsEmbedder, TooLong { window: u64 }, Cancelled, Chat(ChatError), Passages(PassageError), Store(StoreError) }` with `Display` (user-readable; `TooLong` says *This conversation no longer fits the model's context window ({window} tokens). Start a new conversation.*).

Flow: refuse a blank question; the paper's latest canonical text (none → `NoFullText`; neither passed nor confirmed → `NotFullText`); the current conversation on it and its turns; whole-paper messages; `choose_mode`. In retrieval mode: no embedder → `NeedsEmbedder`; `find_passages` (forwarding `Indexing`; `stop` cancels → `Cancelled`); the retrieval messages must `fits` → else `TooLong`. The request: `ChatRequest::new(messages).with_model(model)`, `max_tokens = ANSWER_TOKENS`, `temperature = ANSWER_TEMPERATURE`, `num_ctx = window` (as `NonZeroU32`, saturating). `on_delta` forwards `Text` as `Delta` (drops `Thinking`) and breaks when `stop` is set. Then:
- `Ok` with `stop_reason == Some("cancelled")` → `Stopped`, no quotes.
- `Ok` otherwise → `parse_answer` + `locate` with `MIN_QUOTE_SIMILARITY` → `Complete`.
- `Err(ChatError::Stream(_))` with text already received → `Failed` with that text and the error; no quotes.
- any other `Err` → `AskError::Chat`, nothing stored.

The conversation is created (if none) and the turn saved in one transaction, after the model's answer.

- [ ] **Step 1: Write failing tests** with a `FakeChat` (scripted deltas, then a result; records the request) and `FakeEmbedder` (Task 5):
  - `whole_paper_answer_cited`: a passing text, window 100 000, the fake streams `Over 75: <q>61 were over 75 years</q>.` in three deltas → three `Delta` events; one turn, `Complete`, `WholePaper`, one located quote.
  - `unconfirmed_text_refused`: a failing, unconfirmed text → `Err(NotFullText)`, fake not called.
  - `num_ctx_is_sent` (Review Focus 4): the recorded request has `num_ctx == Some(window)` and `max_tokens == ANSWER_TOKENS`.
  - `retrieval_when_too_long`: window 2 000 and a long paper → `Retrieval`, `Indexing` events seen, the request's paper message holds passages, not the whole text.
  - `too_long_refused` (Review Focus 4): a window too small even for the passages and a long history → `Err(TooLong { window })`, fake not called, nothing stored.
  - `no_embedder_refused`: the same with `embedder: None` → `Err(NeedsEmbedder)`.
  - `stopped_turn_has_no_citations` (Review Focus 3): `stop` set after the first delta → `Stopped`, answer holds the deltas so far, no quotes.
  - `broken_stream_keeps_partial`: deltas then `Err(Stream)` → `Failed`, error kept, no quotes.
  - `failure_before_text_stores_nothing`: `Err(Transport)` → `Err(Chat(_))`, no conversation row.
  - `follow_up_sees_history`: the second ask's request holds the first turn's question and answer.
- [ ] **Step 2: Run** `cargo test -p studyanalyzer-core interrogate`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): ask a question of the paper`.

### Task 8: Views and commands

**Files:**
- Modify: `crates/studyanalyzer-core/src/view.rs`, `src-tauri/src/{commands.rs,state.rs,error.rs,lib.rs,contract.rs}`, `src/lib/fixtures/ipc_contract.json` (regenerated), `src/lib/api.ts`, `src/lib/contract.test.ts`

**Interfaces:**
- Consumes: Tasks 1–7; `tauri::ipc::Channel`; `bmlib::embed::{download_model, Embedder}`.
- Produces (core, `view.rs`):
  - `#[derive(Serialize)] pub struct CitationView { pub number: u32, pub span: Utf16Span, pub text: String }` (`text` = `canonical.span_text`); `pub struct UnlocatedView { pub quote: String }`.
  - `pub struct TurnView { pub id: i64, pub question: String, pub pieces: Vec<AnswerPiece>, pub citations: Vec<CitationView>, pub unlocated: Vec<UnlocatedView>, pub status: TurnStatus, pub error: Option<String>, pub model: String, pub mode: Mode }`.
  - `pub struct ConversationView { pub id: i64, pub canonical_id: i64, pub read_only: bool, pub turns: Vec<TurnView> }` (`read_only` when the paper's latest text is another).
  - `pub struct InterrogatorView { pub canonical_id: i64, pub current: Option<ConversationView>, pub earlier: Vec<ConversationSummary> }`.
  - `pub fn conversation_view(db: &mut dyn Db, conversation_id: i64) -> Result<ConversationView, StoreError>` and `pub fn interrogator_view(db, paper: &PaperId) -> Result<InterrogatorView, StoreError>`.
  - `paper_view` gains a sibling `pub fn paper_view_at(db, paper, canonical_id) -> Result<PaperView, StoreError>` for an earlier conversation's text (refuses a text of another paper).
- Produces (commands, each in `handler()`, `commands()`, `api.ts`, the vitest call list):
  - `ask(paperId: string, question: string, onEvent: Channel<AnswerEvent>) -> ConversationView` — no model setting → `needs_model` before anything else; `AppState.answer_stop` is set fresh per ask; window from `model_window` cached per model in `AppState` plus the setting.
  - `stop_answer() -> ()`; `new_conversation(paperId) -> ConversationView`; `interrogator(paperId) -> InterrogatorView`; `conversation(conversationId: number) -> ConversationView`; `paper_view_at(paperId, canonicalId: number) -> PaperView`.
  - `llm_settings() -> LlmSettings`, `set_model(model)`, `set_context_tokens(tokens: number)`, `clear_context_tokens()`, `set_api_key(provider, key)`, `delete_api_key(provider)`, `model_window(model) -> number | null` (bmlib `context_window`; `NoSuchModel` → `invalid_input` naming the model).
  - `embedder_status() -> EmbedderStatus { downloaded: bool, size_bytes: number }`; `download_embedder(onProgress: Channel<DownloadProgress { done: number, total: number }>) -> ()`, which also loads it into `AppState.embedder`; `cancel_download() -> ()`.
  - `ErrorKind` gains `NeedsModel`, `NeedsApiKey` (`ChatError::MissingApiKey`, `ContextWindowError::MissingApiKey`), `NeedsEmbedder`, `NotFullText`, `Llm` (every other chat or window failure, message naming the provider); `EVERY_KIND`, `every_kind_is_listed` and `ERROR_KINDS` with them.
  - Contract: `"channel"` joins the JSON types in `sample` (`"__CHANNEL__:1"`, Tauri's `IPC_PAYLOAD_PREFIX`) and `wrong_sample` (`1`); vitest's recorder maps a `Channel` instance to `"channel"`. Each new shape gets a fixture key (a `TurnView` with one located, one unlocated quote, on `contract.rs`'s astral `PAPER`; a `Stopped` turn; an `InterrogatorView` with `current: null`), `keysMatch` and `valuesMatch` rows; `RETURN_TYPES` gains `ConversationView`, `InterrogatorView`, `PaperView`, `LlmSettings`, `Option<u64>` → `number | null`, `EmbedderStatus`.

- [ ] **Step 1: Write failing tests:** in `view.rs` `citation_spans_slice_same_text` (Review Focus 2: on the astral `PAPER`, the citation's UTF-16 span slices to `text`) and `turn_view_shows_paper_wording` (Review Focus 1: `text` is the paper's span, not the stored quote); `earlier_conversation_uses_its_text` (Review Focus 5: after a new document, `conversation_view` of the old one is `read_only` with the old `canonical_id`, and `paper_view_at` returns the old text); in `error.rs` one mapping assertion per new `AskError`, `SettingsError` and `ChatError` variant; the contract tests fail until the fixture is regenerated.
- [ ] **Step 2: Run** `cargo test --workspace`. Expected: FAIL.
- [ ] **Step 3: Implement.** Read Tauri's `ipc::Channel` docs before wiring it; blocking work goes through `blocking()`. `download_model` goes through `retry::with_backoff` for its network and file-system faults (not a mismatch or a cancel, rule 7); its `progress` returns false once `cancel_download` sets `AppState.download_stop`. Load the embedder with `Embedder::load_model(&EMBEDDING_MODEL, &path)`, never `load`.
- [ ] **Step 4: Run** `UPDATE_CONTRACT=1 cargo test -p studyanalyzer contract`, review the fixture's diff, then every gate. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): commands for asking, conversations, settings and the embedder`.

### Task 9: Frontend logic

**Files:**
- Create: `src/lib/chat.ts`, `src/lib/chat.test.ts`

**Interfaces:**
- Consumes: the `api.ts` types of Task 8.
- Produces:
  - `liveSegments(text: string): Array<{ kind: "text"; text: string } | { kind: "quote"; text: string }>` — for the streaming text: closed and open `<q>` quotes become `quote` segments; a trailing prefix of `<q>` or `</q>` (`<`, `<q`, `</`, `</q`) is held back, never shown.
  - `highlightRuns(block: Utf16Span, marks: Array<{ span: Utf16Span; current: boolean }>): Array<{ span: Utf16Span; mark: "none" | "past" | "current" }>` — splits the block at every mark edge inside it; `current` wins where marks overlap; runs cover the block exactly.
  - `unlocatedNote(n: number): string | null` — `null` for 0; `"1 quote could not be found in the paper"`; `"N quotes could not be found in the paper"`.
  - `incompleteNote(status: TurnView["status"]): string | null` — `null` for `complete`; else *Incomplete answer: its quotes were not checked.*
  - `paperMarks(turns: TurnView[], current: number | null)` — every citation of every turn as a mark, `current` for the one clicked.

- [ ] **Step 1: Write failing tests** in `chat.test.ts`: `liveSegments("A <q>x</q> b <q>y")` → text, quote `x`, text, quote `y`; `live_holds_back_partial_tag` (Review Focus 3): `liveSegments("A <")` and `("A <q>x</")` show no `<`; `highlightRuns` with a mark across the block's end, two overlapping marks (one current), and none; `unlocatedNote` for 0, 1, 3; `incompleteNote` per status.
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm run test:web` and `pnpm run check:web`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): streaming segments, highlight runs and citation notes`.

### Task 10: The Interrogator screen

**Files:**
- Create: `src/components/Interrogator.svelte` (tab bar with *Interrogator*, split or switch), `src/components/ChatPane.svelte`, `src/components/TurnView.svelte`, `src/components/EmbedderDownload.svelte`
- Modify: `src/components/PaperPane.svelte` (optional `marks` prop rendered through `highlightRuns`; scrolls the current mark into view), `src/components/Settings.svelte` (provider, model with its maximum from `model_window`, context-window field, per-provider *Save key* / *Remove key* showing only *saved* / *from ANTHROPIC_API_KEY* / *none*), `src/App.svelte` (the paper screen shows `Interrogator` once the text passed or was confirmed, else 2a's paper pane and banner with *Confirm this is the whole paper to ask questions*), `src/app.css` (`PHONE_BREAKPOINT_PX` as a CSS variable comment and media query)

Behaviour:
- **Chat pane:** earlier turns (question, then answer pieces with `[n]` buttons, the citation list showing each paper wording, `unlocatedNote` with the model's unlocated quotes in a muted list, `incompleteNote`, the model and *whole paper* / *passages* as a small caption). While streaming: `liveSegments` with quotes shown muted as *checking…*, a *Stop* button (`stop_answer`), *Finding passages… n / total* during `Indexing`. *New conversation* and a list of earlier conversations (read-only ones say *on an earlier version of the text*, and show that text via `paper_view_at`).
- **Citation click:** sets the current mark; the paper scrolls to it; on phones the switch flips to *Paper*.
- **Errors:** `needs_model` / `needs_api_key` open Settings; `needs_embedder` shows `EmbedderDownload` (*Answering from this paper needs the search model (1.1 GB, downloaded once). Download it?*, progress bar, *Cancel*) and asks again when it finishes; others go to `ErrorBanner`. The typed question stays in the box after a rejection.

- [ ] **Step 1:** Implement the components (Svelte 5 runes), each documenting itself in a `/** … */` at the top of its script.
- [ ] **Step 2: Run** `pnpm run check:web`, `pnpm run test:web`, `pnpm exec vite build`. Expected: PASS.
- [ ] **Step 3: Check it by hand** with `pnpm dev`, a cloud model and a local Ollama model: a whole-paper question with citations (click → scroll, current dark, earlier pale); a follow-up; a question the paper does not answer; *Stop* mid-answer; Wi-Fi off mid-stream; a small context-window setting forcing retrieval (the download, then *Finding passages*); a quote that fails to locate (ask for a paraphrase); *New conversation*; a narrow window for the phone switch; dark mode; quit and reopen (everything still there); the key in Keychain Access, not in the database (`sqlite3 … "select * from settings"`).
- [ ] **Step 4: Commit** `feat(ui): the Interrogator`.

### Task 11: Hand over

**Files:**
- Modify: `HANDOVER.md`, `ROADMAP.md`, `CLAUDE.md` (*State of the repository*), `README.md` (the model download and keys)

- [ ] **Step 1:** Run every gate. Expected: PASS.
- [ ] **Step 2:** Update the docs: 2b landed; what 2c builds on (`find_passage` threshold, `locate`, `Channel` events, `paper_view_at`, the dependents rule: a PICO run with citations is a dependent too); anything found on the way filed as issues, and one for per-model parameters (§6.6).
- [ ] **Step 3: Commit** `docs: phase 2b landed`, push, open the PR.
