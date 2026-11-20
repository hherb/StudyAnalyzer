# PRISMA 1: bmlib's PRISMA 2020 agent — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** bmlib can appraise a systematic review against PRISMA 2020 in Rust
and Python, on par. It provides:
- the 42-row checklist as data
- a split of the checklist into parts sized to a model's reply budget
- a suitability check
- one call per part, on the whole paper or a condensation aimed at that part
- every item's quotes located in the paper
- the shown judgement under the strict-evidence rule

**Architecture:** this is a new module next to the PICO agent
(`quality/prisma2020_*`). It reuses PICO's machinery rather than copying it:
`JsonChat`, `verify_quotes`, `PicoCitation`, `condense_checked`, and the
context processor's map-reduce. The caller drives the parts, so an app can
store each part as it finishes. No storage, no retry loop over parts, no UI:
those are StudyAnalyzer's (plan PRISMA 2).

**Tech Stack:** bmlib Rust crate (`rust/bmlib`) and Python package
(`bmlib/`); serde_json; pytest; the existing oracle harness
(`rust/oracle/`). No new dependency.

**Spec:** StudyAnalyzer
`docs/superpowers/specs/2026-11-18-prisma2020-tab-design.md` (§3, §4,
§4.1, §5, §6.2–§6.5, §9). Port source: bmlibrarian
`src/bmlibrarian/agents/prisma2020_agent.py` (flow and validation only; its
checklist is the 2009 layout and is **not** ported).

## Global Constraints

- **Where the work happens:** a bmlib **worktree** on its own branch, never the
  shared `~/src/bmlib` checkout. Check `git worktree list` for a live session
  first.
  - Follow bmlib's HANDOVER.md and `docs/SESSION-RULES.md`.
  - Base the branch on bmlib `main` (`e7b7c98a`, release 0.14.0, at the time
    of writing).
- **Gates:** each commit passes bmlib's gates:
  - Python: `uv run pytest tests/ -v`, `uvx ruff@0.15.20 check .`,
    `uvx ruff@0.15.20 format --check .` and `uv run mypy` (CI runs mypy even
    though CLAUDE.md's list omits it).
  - Rust, from `rust/`: `cargo test`,
    `cargo clippy --all-targets --all-features -- -D warnings`,
    `cargo fmt --check`, and
    `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
  - Use `set -o pipefail` in gate chains.
- **Parity:** Rust and Python land in **one PR**, on par, held together by:
  - the shared fixture `tests/fixtures/prisma2020_checklist.json`
  - `tests/fixtures/prisma2020_parts.json` (split cases)
  - the oracle `rust/oracle/dump_prisma2020_agent.py` with
    `rust/bmlib/tests/data/prisma2020_agent_{cases,expected}.json`
- **No truncation anywhere.**
  - bmlibrarian's `full_text[:2000]` and `text[:3000]` in the suitability check
    are not ported.
  - Text over the budget is condensed or refused, never cut.
- **Judgement vocabulary**, the exact strings in both languages and in the
  reply: `reported`, `partly_reported`, `not_reported`, `reported_elsewhere`,
  `not_applicable`.
- **The suitability variant** is the model's string, stored as given. It is
  data-derived, so it is not an enum while developing (maintainer's rule).
- **Checklist source:** item wording is copied from the PRISMA 2020 checklist
  (Page MJ et al., *BMJ* 2021;372:n71, CC BY 4.0).
  - The attribution goes in the module doc and in the checklist fixture's
    `"source"` field.
  - Before the PR, confirm the licence on prisma-statement.org and name the
    page in the PR body.
- **Errors:** every error is reported with a kind string, never swallowed.
  Mismatches the reader tolerates (an extra item id) are logged at WARN, since
  the rule while developing is to surface diagnostics.

## Review Focus

1. **A model that claims "reported" with a paraphrase.** No quote is located.
   The shown judgement is `not_reported` with `unbacked = true`, and the
   model's judgement is kept. Test: Task 3 `unbacked_claim_is_not_reported`
   (both languages, oracle case `unbacked`).
2. **A reply that leaves out an item or adds one.** The missing item is
   listed in `missing`, never invented. An extra one is dropped and logged.
   Tests: Task 3 `missing_item_is_listed`, `extra_item_is_ignored`.
3. **A tiny reply budget.** The smallest item does not fit, so `plan_parts`
   refuses before any call. A huge budget gives one part. Tests: Task 2
   `budget_below_one_item_is_refused`, `large_budget_is_one_part`, and the
   fixture rows `tiny`, `huge`.
4. **A long paper.** Each part condenses with its own query, and quotes are
   located in the whole paper, not the digest. A failed batch fails the part.
   Tests: Task 4 `part_condenses_with_its_query`,
   `quotes_located_in_whole_text`, `partial_digest_is_refused` (and the oracle
   case `condensed`).
5. **A `not_reported` with a located quote.** It is never upgraded. This
   differs from PICO's `read_part`, #577. Test: Task 3
   `not_reported_is_never_upgraded`.

---

### Task 1: The checklist as data

**Files:**
- Create: `tests/fixtures/prisma2020_checklist.json`
- Create: `bmlib/quality/prisma2020_checklist.py`
- Create: `rust/bmlib/src/quality/prisma2020_checklist.rs`
- Modify: `rust/bmlib/src/quality/mod.rs` and `bmlib/quality/__init__.py`
  (exports)
- Test: `tests/test_prisma2020_checklist.py` and
  `rust/bmlib/tests/prisma2020_checklist_contract.rs`

**Interfaces:**
- Produces, in Rust:
  `pub struct ChecklistItem { pub id: &'static str, pub section: Section, pub name: &'static str, pub wording: &'static str, pub guidance: &'static str, pub condense_hint: &'static str, pub key: bool, pub may_be_not_applicable: bool, pub reply_chars: usize }`
  - `pub enum Section { Title, Abstract, Introduction, Methods, Results, Discussion, OtherInformation }`
    with `as_str()` giving `title`, `abstract`, `introduction`, `methods`,
    `results`, `discussion` and `other_information`.
  - `pub const CHECKLIST: [ChecklistItem; 42]`
  - `pub fn item(id: &str) -> Option<&'static ChecklistItem>`
  - `pub const KEY_ITEMS: [&str; 10] = ["7", "11", "18", "13d", "20b", "14", "21", "15", "22", "24a"];`
    This is display order (spec §5), and it must equal the `key: true` rows.
- Produces, in Python: the same as a frozen dataclass `ChecklistItem`, a
  `Section` `StrEnum`, a `CHECKLIST` tuple, `item()` and `KEY_ITEMS`.

- [ ] **Step 1: Write the fixture.**
  - It holds 42 objects in checklist order (spec §4's table), each with every
    field above, plus a top-level `"source"`.
  - `wording` is the checklist's own text.
  - `guidance` is the model's rubric: what counts as reported, partly reported
    or not reported. Write it from the PRISMA 2020 explanation and elaboration
    paper (Page et al., *BMJ* 2021;372:n160). Where it is common, it names
    `reported_elsewhere` (7, 16b, 19, 24a–24c, 27). Item 2's guidance lists the
    PRISMA 2020 for Abstracts elements, and item 27's says that "available on
    request" is a statement of availability.
  - `may_be_not_applicable` is true for the synthesis items 13a–13f and 20a–20d
    (only when the review has **no synthesis at all**, spec §4), and for 12,
    14 and 21. Settle the rest from the explanation and elaboration paper, and
    name each choice in the PR body.
  - `reply_chars` is an estimate of one item's reply (explanation plus up to
    three quotes). Start from 900 and raise it for items whose quotes run long
    (7, 13d, 20b).
  - `condense_hint` is one sentence naming what to keep for that item, for
    example "every database, register and website searched, with dates".
- [ ] **Step 2: Write the failing tests** in both languages:
  - `checklist_matches_fixture`: every field of every row equals the fixture's.
  - `ids_are_unique_and_ordered`.
  - `key_items_equal_key_rows`: the set of `KEY_ITEMS` equals the rows with
    `key`.
  - `forty_two_rows`.
- [ ] **Step 3: Run them.** Expected: FAIL (the modules don't exist).
- [ ] **Step 4: Implement** both modules as constant tables. Do not load the
  fixture at runtime. The fixture pins the tables.
- [ ] **Step 5: Run the tests and both gate sets.** Expected: PASS.
- [ ] **Step 6: Commit** `feat(quality): PRISMA 2020 checklist as data, both languages`.

### Task 2: The split

**Files:**
- Create: `rust/bmlib/src/quality/prisma2020_parts.rs` and
  `bmlib/quality/prisma2020_parts.py`
- Create: `tests/fixtures/prisma2020_parts.json`
- Test: `tests/test_prisma2020_parts.py` and
  `rust/bmlib/tests/prisma2020_parts_contract.rs`

**Interfaces:**
- Consumes: `CHECKLIST` and `item()` (Task 1).
- Produces, in Rust:
  - `pub struct PartPlan { pub item_ids: Vec<&'static str> }`, with
    `pub fn label(&self) -> String`, for example `"Methods 5–13c"`, or
    `"Methods 13d – Results 17"` across sections, using an en dash.
  - `pub fn plan_parts(items: &[&'static ChecklistItem], reply_budget_chars: usize) -> Result<Vec<PartPlan>, PartsError>`
  - `pub enum PartsError { ItemTooLarge { id: &'static str, needed: usize, budget: usize } }`
    with `kind()` returning `"item_too_large"`.
  - `pub const PART_OVERHEAD_CHARS: usize = 400;`: the reply's JSON frame,
    counted once per part.
  - `pub fn part_query(items: &[&'static ChecklistItem]) -> String`: the
    condensation query, which is the items' `condense_hint`s joined with
    `"; "`, prefixed by
    `"Everything needed to judge these PRISMA 2020 items: "`.
- Produces, in Python: the same names, with `plan_parts` raising
  `PartsError(kind="item_too_large", ...)`.
- The callers pass `&CHECKLIST` for a run, and a subset for a re-check or a
  split part.

- [ ] **Step 1: Write the fixture** of cases `{name, item_ids, reply_budget_chars, expected: [[ids…], …] | {"error": "item_too_large", "id": …}}`, including:
  - `huge` (1 000 000 gives one part)
  - `tiny` (100 gives `item_too_large` on item `"1"`)
  - `exact_fit` (a budget equal to two items plus the overhead gives exactly
    those two)
  - `realistic_16k` (a 16 000-character budget, expected computed by hand from
    the Task 1 estimates)
  - `subset` (only `["13d"]`)
- [ ] **Step 2: Write the failing tests** in both languages:
  - `fixture_cases` runs every case.
  - `large_budget_is_one_part`
  - `budget_below_one_item_is_refused`
  - `order_is_checklist_order`
  - `label_spans_sections`
  - `part_query_names_every_hint`
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement** `plan_parts` as a greedy packing in the given
  order. A part takes the next item while `PART_OVERHEAD_CHARS + Σ reply_chars`
  is at most `reply_budget_chars`. An item that does not fit even alone is
  `ItemTooLarge`, raised before any part is returned. Use saturating
  arithmetic.
- [ ] **Step 5: Run the tests and both gate sets.** Expected: PASS.
- [ ] **Step 6: Commit** `feat(quality): split the PRISMA checklist to a reply budget`.

### Task 3: Reading a part's reply and the strict-evidence rule

**Files:**
- Create: `rust/bmlib/src/quality/prisma2020_models.rs` and
  `bmlib/quality/prisma2020_models.py`
- Test: `tests/test_prisma2020_models.py`, plus Rust unit tests in the module

**Interfaces:**
- Consumes: `verify_quotes(quotes, text, min_similarity) -> QuoteCheck` and
  `PicoCitation` and `PicoContradictedQuote` from `pico_agent`/`pico_models`,
  reused as they are (spec §12 leaves this open; reuse is the smaller change).
- Produces, in Rust:
  - `pub enum Judgement { Reported, PartlyReported, NotReported, ReportedElsewhere, NotApplicable }`
    with `as_str()` and `parse(&str) -> Option<Self>` over the five strings.
  - `pub struct ItemAssessment { pub id: &'static str, pub model_judgement: Judgement, pub judgement: Judgement, pub unbacked: bool, pub explanation: String, pub citations: Vec<PicoCitation>, pub quotes_not_found: Vec<String>, pub quotes_contradicted: Vec<PicoContradictedQuote>, pub quotes_dropped: usize }`
    with `to_json()`.
  - `pub struct PartAssessment { pub items: Vec<ItemAssessment>, pub missing: Vec<&'static str>, pub condensed_from_chars: Option<usize>, pub condensation_status: Option<String>, pub notes: Vec<String> }`
    with `to_json()`.
  - `pub fn shown_judgement(model: Judgement, located: usize, may_be_not_applicable: bool) -> (Judgement, bool)`
  - `pub fn read_part_reply(data: &Value, asked: &[&'static str], text: &str, min_similarity: f64) -> Result<PartAssessment, String>`
  - `pub struct Suitability { pub is_systematic_review: bool, pub is_meta_analysis: bool, pub is_suitable: bool, pub document_type: String, pub variant: Option<String>, pub rationale: String, pub confidence: f64, pub condensed_from_chars: Option<usize>, pub condensation_status: Option<String>, pub notes: Vec<String> }`
  - `pub fn read_suitability(data: &Value) -> Result<Suitability, String>`
- Produces, in Python: the same names. `Judgement` is a `StrEnum`, and the
  models are dataclasses with `to_dict()`.
- **Reply shape** (the plan's call, spec §12):
  `{"items": [{"id": "7", "judgement": "…", "explanation": "…", "quotes": ["…"]}]}`
  and
  `{"is_systematic_review": b, "is_meta_analysis": b, "document_type": "…", "variant": "…"|null, "rationale": "…", "confidence": 0–1}`.

- [ ] **Step 1: Write the failing tests** in both languages:
  - `unbacked_claim_is_not_reported`:
    `shown_judgement(Reported, 0, false) == (NotReported, true)`. The same
    holds for `PartlyReported`, `ReportedElsewhere` and `NotApplicable` with no
    located quote.
  - `backed_claim_stands`: `shown_judgement(PartlyReported, 1, false) == (PartlyReported, false)`.
  - `not_applicable_needs_the_flag`:
    `shown_judgement(NotApplicable, 1, false) == (NotReported, true)`, and with
    the flag set it gives `(NotApplicable, false)`.
  - `not_reported_is_never_upgraded`: a `not_reported` item whose quote the
    text holds stays `NotReported`, its citation kept and `unbacked == false`.
  - `missing_item_is_listed`: asked `["7", "8"]`, the reply has only 7, so
    `missing == ["8"]`.
  - `extra_item_is_ignored`: a reply item `"99"` is dropped, a WARN is logged
    (Rust: `tests/common/log_capture.rs`), and nothing fails.
  - `unknown_judgement_is_unreadable`: `"judgement": "maybe"` is `Err`, so
    `chat_json`'s re-ask applies.
  - `duplicate_id_is_unreadable`.
  - `suitable_is_sr_or_ma`: `is_suitable == is_systematic_review || is_meta_analysis`.
  - `variant_kept_as_given`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `shown_judgement`:
    - `NotReported` is returned as it is.
    - Any other judgement with `located == 0`, or `NotApplicable` without the
      flag, becomes `(NotReported, true)`.
    - Otherwise the judgement stands with `false`.
  - `read_part_reply` verifies each item's quotes with `verify_quotes` against
    `text`, then applies `shown_judgement` with that item's flag.
- [ ] **Step 4: Run the tests and both gate sets.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(quality): read PRISMA replies under the strict-evidence rule`.

### Task 4: The agent: suitability, prepare per part, assess a part

**Files:**
- Create: `rust/bmlib/src/quality/prisma2020_agent.rs` and
  `bmlib/quality/prisma2020_agent.py`
- Create: `rust/oracle/dump_prisma2020_agent.py`,
  `rust/oracle/prisma2020_agent_cases.json`, and
  `rust/bmlib/tests/data/prisma2020_agent_{cases,expected}.json` (as PICO's:
  see `rust/oracle/dump_pico_agent.py`)
- Test: `rust/bmlib/tests/prisma2020_oracle.rs`, `tests/test_prisma2020_agent.py`
- Modify: the CHANGELOG, ROADMAP (a row under *Quality*) and HANDOVER, per
  bmlib's rules

**Interfaces:**
- Consumes: Tasks 1–3; `JsonChat`, `ContextModel`, `LlmCondenser::with_prompts`,
  `PromptTemplates`, `condense_checked` and `ProcessingConfig`; Python's
  `BaseAgent` and `condense_text(..., query=...)`.
- Produces, in Rust:
  - `pub trait QueryCondenser { fn condense(&mut self, text: &str, query: &str, label: &str, config: &ProcessingConfig) -> ProcessingResult; }`
    This is the seam a per-part query needs: `Condenser` fixes the query at
    construction.
  - `pub struct PrismaCondenser<'a>`, built by
    `PrismaCondenser::new(model: &'a dyn ContextModel, temperature: f64, max_tokens: i64)`
    and `.with_progress(...)`. It implements `QueryCondenser` by building an
    `LlmCondenser::with_prompts(model, PRISMA prompts, query, …)` for each call.
  - `pub struct Prisma2020Agent<'a>`, with `PicoAgent`'s builder shape:
    `new(chat: &'a mut dyn JsonChat)`, `with_condenser(&'a mut dyn QueryCondenser)`,
    `with_stop`, `with_temperature`, `with_max_tokens`, `with_max_retries` and
    `with_condense_config`. It has:
    - `check_suitability(&mut self, title: &str, text: &str) -> Result<Suitability, PrismaError>`:
      `text` is the abstract or the whole paper. A paper over the budget is
      condensed with `SUITABILITY_CONDENSE_QUERY`.
    - `prepare_part(&mut self, title: &str, text: &str, item_ids: &[&'static str]) -> Result<PrismaPrepared, PrismaError>`:
      the whole text when it fits `max_context_chars`, else condensed with
      `part_query` of those items. It is never cut, and a partial digest is
      refused by `condense_checked` under `continue_on_error(false)`.
    - `assess_part(&mut self, prepared: &PrismaPrepared, item_ids: &[&'static str], min_similarity: f64) -> Result<PartAssessment, PrismaError>`:
      the quotes are verified against `prepared.text()`, which is the paper,
      never the digest.
  - `pub struct PrismaPrepared`, shaped as `PicoPrepared`: `title()`,
    `text()` (the paper as passed), `sent()` (the text or digest sent),
    `condensed_from_chars() -> Option<usize>`,
    `condensation_status() -> Option<&str>` and `notes() -> &[String]`.
  - `pub enum PrismaError`: the same variants and kind strings as `PicoError`
    (`empty_input`, `request_failed { retryable }`, `truncated`,
    `unusable_reply`, `condensation_failed`, `stopped`, `invalid_similarity`,
    `prepared_over_budget` and `prepared_under_budget`), plus `parts` (from
    `PartsError`).
  - Prompt constants: `PRISMA_SYSTEM_PROMPT`, `SUITABILITY_TASK_PROMPT`,
    `SUITABILITY_RESPONSE_FORMAT`, `PART_TASK_PROMPT` (with `{items}`, filled
    with each item's id, name, wording and guidance) and `PART_RESPONSE_FORMAT`,
    plus `SUITABILITY_CONDENSE_QUERY`, `PRISMA_CONDENSE_EXTRACTION_PROMPT` and
    `PRISMA_CONDENSE_CONSOLIDATION_PROMPT`. Derive these from PICO's (verbatim
    extraction) and bmlibrarian's ("score only what is actually present").
  - `pub fn part_prompt_chars(item_ids: &[&'static str]) -> usize`: the
    longest prompt a part sends without the paper and title, for the caller's
    budget.
- Produces, in Python: the same names. Python's agent calls `condense_text`
  with the part's query, and has `num_ctx`, `stop` and `condense_progress`, as
  `PicoAgent`.

- [ ] **Step 1: Write the oracle cases** (prompts included, as PICO's oracle
  dumps its eight constants):
  - `whole`: a short paper, one part of three items, all backed.
  - `unbacked`: a paraphrased quote.
  - `missing`: a reply that leaves out one of the items asked.
  - `condensed`: `max_context_chars` under the paper's length, with a digest
    and `digest_status: "complete"`.
  - `partial_digest`: `digest_status: "partial"`, refused.
  - `not_suitable`: a narrative review.
  - `scoping_variant`: suitable, with `variant: "scoping_review"`.
  - `truncated`: the reply hits its length limit, giving `truncated`.
  - `stopped`: the stop is set before the call, so no messages are sent.
- [ ] **Step 2: Run the dumper** to write `prisma2020_agent_expected.json`,
  and write the failing Rust oracle test: each case gives the same result or
  error kind, **and the same messages sent**. Also write the Python unit tests:
  - `part_condenses_with_its_query`: the query the patched map-reduce receives
    equals `part_query` of the part's items.
  - `quotes_located_in_whole_text`: a citation's offsets index the paper, not
    the digest.
  - `partial_digest_is_refused`
  - `suitability_never_cut`: a 200 000-character text with no abstract is sent
    whole when the budget allows, or condensed, never sliced. Assert that no
    sent message is a prefix of the input that is shorter than it.
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement** both agents. Mirror `PicoAgent::ask` for the call
  and `prepare_for` for the condensation, including stop handling (a stop set
  during a failing call is `Stopped`).
- [ ] **Step 5: Run the dumper again** (expected unchanged), the tests, and both
  gate sets. Expected: PASS.
- [ ] **Step 6: Commit** `feat(quality): PRISMA 2020 agent, Rust and Python on par`,
  then update the bmlib CHANGELOG, ROADMAP and HANDOVER, and commit those.
- [ ] **Step 7: Open the PR** on hherb/bmlib. The body names StudyAnalyzer's
  spec and this plan, the licence page checked, and the
  `may_be_not_applicable` choices. Wait for review and merge, and note the
  merge commit for plan PRISMA 2, Task 1.

## Changed while building

_(Record renames and decisions here as they land, so plan PRISMA 2 uses the
names that merged.)_
