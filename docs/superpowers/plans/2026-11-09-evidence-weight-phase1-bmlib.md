# Evidence weight, phase 1 (bmlib) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** bmlib can assess a parallel-group RCT's primary outcomes with RoB 2
and turn each into an evidence ladder and a number, step by step (for
StudyAnalyzer's tab) or unattended (for the pipeline), identically in Rust and
Python.

**Architecture:** Pure functions first: RoB 2's official decision rules and
the GRADE-style ladder, held to shared JSON contract fixtures. Then three
LLM agents on the `PicoAgent` pattern (`JsonChat` seam, verified quotes via
`verify_quotes`/`find_passage`, condensing when a paper does not fit, a stop
flag): design proposal, primary outcomes with results, RoB 2 per domain. Last,
one unattended entry point chains them. Everything lives in `quality/` beside
`pico_agent`, in both languages.

**Tech Stack:** bmlib Rust (`rust/bmlib`, serde, the crate's `JsonChat`,
`Condenser`, `LlmCondenser`, `find_passage`) and Python (`bmlib/quality`,
`BaseAgent`, `condense_text`), pytest, cargo test, JSON contract fixtures in
`tests/fixtures/`.

**Spec:** [docs/superpowers/specs/2026-11-09-evidence-weight-design.md](../specs/2026-11-09-evidence-weight-design.md)
(in the StudyAnalyzer repo; §4 and §9 are the parts this plan builds). The
official RoB 2 source is *Revised Cochrane risk-of-bias tool for randomized
trials (RoB 2)*, Higgins, Savović, Page, Sterne (eds.), 22 August 2019,
https://www.cochrane.de/sites/cochrane.de/files/uploads/RoB_2.0_guidance_2019.pdf
(Table 1 overall; Tables 3/4 D1, 6 D2, 10 D3, 12 D4, 14 D5).

## Global Constraints

- Work in a bmlib **worktree**, never the shared `~/src/bmlib` checkout; check `git worktree list` for a live session first.
- Read bmlib's `docs/DECISIONS.md` and `docs/SESSION-RULES.md` before the first task.
- Every piece lands in **Rust and Python in the same PR**, held together by a shared fixture under `tests/fixtures/` that a Python `tests/test_*_contract.py` and a Rust `rust/bmlib/tests/*_contract.rs` both assert.
- One PR per task (Task 5 and 6 may share one). Stacked PRs **target main** and say "merge #N first".
- Gates before each commit, Python from the worktree root: `uv run pytest tests/ -v`, `uvx ruff@0.15.20 check .`, `uvx ruff@0.15.20 format --check .`, `uv run mypy bmlib`. Rust from `rust/`: `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. Chain with `set -o pipefail`.
- New files carry bmlib's AGPL header. Every public item has a doc comment a junior developer can follow.
- No magic numbers: thresholds and steps are named constants or fields of a settings struct with `Default`.
- **A failure is never a score.** A missing, unreadable or out-of-range model answer fails that domain/outcome visibly, naming the field; it is never defaulted, clamped or guessed.
- **Never truncate** the paper; when it does not fit, condense (verbatim passages) and flag the result `condensed`.
- `StudyDesign`'s existing wire values do not change.
- Free-form strings, not enums, for data-derived kinds (an effect measure's name).
- Each PR adds a CHANGELOG `[Unreleased]` entry and its new files to CLAUDE.md's directory list.

## Review Focus

1. A model answers "NA" for, or omits, a question that **is** applicable on its branch → that domain fails, naming the question (Task 5, `read_domain_reply`).
2. A model answers outside the allowed set: 3.2 "NI", "Unclear", " py " → case and surrounding space are tolerated (`" py "` → PY); a value the question does not allow is refused (Task 1 parse, Task 5).
3. A reported result is incoherent: `ci_low > ci_high`, estimate outside its interval, a ratio interval ≤ 0 → that outcome's result is refused with the reason; no ladder is built from it (Task 4).
4. A quote is not in the paper → the answer stands, the quote is listed in `quotes_not_found`, never silently dropped; an answer with no quote at all (typical of NI) is kept (Task 5).
5. Stop is requested between domain calls → the run ends `Stopped`; no partial run is returned as complete (Task 5).

---

### Task 1: RoB 2 decision rules (pure)

The official mapping tables as code. No LLM, no wording.

**Files:**
- Create: `bmlib/quality/rob2_algorithm.py`, `rust/bmlib/src/quality/rob2_algorithm.rs`
- Modify: `bmlib/quality/__init__.py`, `rust/bmlib/src/quality/mod.rs` (exports)
- Create: `scripts/make_rob2_algorithm.py`, `tests/fixtures/rob2_algorithm.json`
- Test: `tests/test_rob2_algorithm.py`, `tests/test_rob2_algorithm_contract.py`, `rust/bmlib/tests/rob2_algorithm_contract.rs`

**Interfaces:**
- Produces (Rust; Python mirrors in snake_case, enums as `Enum` with these wire values):
  - `enum Answer { Y, Py, Pn, N, Ni }`, wire `"Y" "PY" "PN" "N" "NI"`; `Answer::parse(&str) -> Result<Answer, Rob2Error>` (trims, case-insensitive; `"NA"` is not an answer → `Err`).
  - `enum Judgement { Low, SomeConcerns, High }`, wire `"low" "some_concerns" "high"`.
  - `enum Domain { D1, D2, D3, D4, D5 }`, wire `"D1"`…`"D5"`; `Domain::ALL`; `Domain::scope(self) -> Scope` where `enum Scope { Study, Outcome }` (D1 Study, rest Outcome; spec §4.1).
  - `type Answers = BTreeMap<String, Answer>` (question id like `"2.3"` → answer; a question not asked is absent).
  - `fn questions(domain: Domain) -> &'static [&'static str]`: D1 `1.1–1.3`, D2 `2.1–2.7`, D3 `3.1–3.4`, D4 `4.1–4.5`, D5 `5.1–5.3`.
  - `fn applicable_questions(domain: Domain, answers: &Answers) -> Vec<&'static str>`: the questions to ask given the answers so far, in order.
  - `fn domain_judgement(domain: Domain, answers: &Answers) -> Result<Judgement, Rob2Error>`.
  - `fn needs_escalation_decision(domains: &[Judgement; 5]) -> bool`.
  - `fn overall_judgement(domains: &[Judgement; 5], escalate: Option<bool>) -> Result<Judgement, Rob2Error>`.
  - `enum Rob2Error { MissingAnswer { question }, NotAllowed { question, answer }, UnknownQuestion { question }, EscalationUndecided, UnreadableAnswer { value } }` (Python: one `Rob2Error(ValueError)` with a `kind` attribute of these names).
  - `const ROB2_ALGORITHM_VERSION: &str = "rob2-2019-08-22/assignment/parallel/1"`.

**The rules** (transcribed from the guidance; the fixture's `official` section cites the table per row). "Y" stands for Y/PY, "N" for N/PN.

- Conditions for asking: 2.3 if 2.1 or 2.2 ∈ {Y,PY,NI}; 2.4 if 2.3 ∈ {Y,PY}; 2.5 if 2.4 ∈ {Y,PY,NI}; 2.7 if 2.6 ∈ {N,PN,NI}; 3.2 if 3.1 ∈ {N,PN,NI}; 3.3 if 3.2 ∈ {N,PN}; 3.4 if 3.3 ∈ {Y,PY,NI}; 4.3 if 4.1 and 4.2 both ∈ {N,PN,NI}; 4.4 if 4.3 ∈ {Y,PY,NI}; 4.5 if 4.4 ∈ {Y,PY,NI}. All others always.
- Allowed answers: all five, except **3.2: no NI**.
- Every applicable question must be answered (`MissingAnswer`); answers to questions that are not applicable are ignored; an id outside the domain is `UnknownQuestion`.
- D1: Low iff 1.2 Y and 1.1 ∈ {Y,NI} and 1.3 ∈ {N,NI}; High iff 1.2 N, or 1.2 NI and 1.3 Y; else Some concerns (covers Table 3's "1.2 Y and 1.1 N" case missing from Table 4).
- D2 part 1: Low if 2.1 and 2.2 both N, or 2.3 N; Some concerns if 2.3 NI, or 2.3 Y and 2.4 N, or 2.4 ∈ {Y,NI} and 2.5 Y; High if 2.4 ∈ {Y,NI} and 2.5 ∈ {N,NI}. Part 2: Low if 2.6 Y; Some concerns if 2.7 N; High if 2.7 ∈ {Y,NI}. Domain: High if either part High, else Some concerns if either is, else Low.
- D3: Low if 3.1 Y, or 3.2 Y, or 3.3 N; Some concerns if 3.4 N; High if 3.4 ∈ {Y,NI}.
- D4: High if 4.1 Y or 4.2 Y. Otherwise with 4.2 N: Low if 4.3 N or 4.4 N; Some concerns if 4.5 N; High if 4.5 ∈ {Y,NI}. With 4.2 NI: High if 4.5 ∈ {Y,NI}, else Some concerns.
- D5: High if 5.2 Y or 5.3 Y; Low if 5.1 Y and 5.2 N and 5.3 N; else Some concerns.
- Overall (Table 1): High if any domain High; Low if all Low; else Some concerns, **except** when `needs_escalation_decision` (no High and ≥ 2 Some concerns): then `escalate` is required (`None` → `EscalationUndecided`); `Some(true)` → High, `Some(false)` → Some concerns.

- [ ] **Step 1: Write the failing Python unit tests** in `tests/test_rob2_algorithm.py`, one parametrised test per domain plus refusals and overall:

```python
@pytest.mark.parametrize("answers, expected", [
    ({"1.1": "Y", "1.2": "Y", "1.3": "N"}, "low"),
    ({"1.1": "NI", "1.2": "PY", "1.3": "NI"}, "low"),
    ({"1.1": "Y", "1.2": "Y", "1.3": "Y"}, "some_concerns"),
    ({"1.1": "N", "1.2": "Y", "1.3": "N"}, "some_concerns"),   # Table 3, absent from Table 4
    ({"1.1": "Y", "1.2": "NI", "1.3": "N"}, "some_concerns"),
    ({"1.1": "Y", "1.2": "NI", "1.3": "PY"}, "high"),
    ({"1.1": "Y", "1.2": "N", "1.3": "N"}, "high"),
])
def test_d1(answers, expected): assert judge("D1", answers) == expected
```

(`judge(domain, answers)` is a test helper: parse the strings with `Answer.parse` and return `domain_judgement(...).value`.)

D2: `{2.1 N, 2.2 PN, 2.6 Y}` low; `{2.1 Y, 2.2 N, 2.3 N, 2.6 Y}` low; `{2.1 Y, 2.2 N, 2.3 NI, 2.6 Y}` some_concerns; `{2.1 Y, 2.2 N, 2.3 Y, 2.4 N, 2.6 Y}` some_concerns; `{… 2.3 Y, 2.4 Y, 2.5 Y, 2.6 Y}` some_concerns; `{… 2.3 Y, 2.4 NI, 2.5 N, 2.6 Y}` high; `{2.1 N, 2.2 N, 2.6 N, 2.7 N}` some_concerns; `{2.1 N, 2.2 N, 2.6 NI, 2.7 NI}` high.
D3: `{3.1 Y}` low; `{3.1 N, 3.2 Y}` low; `{3.1 N, 3.2 N, 3.3 N}` low; `{3.1 NI, 3.2 PN, 3.3 NI, 3.4 N}` some_concerns; `{… 3.4 NI}` high.
D4: `{4.1 N, 4.2 N, 4.3 N}` low; `{4.1 NI, 4.2 N, 4.3 Y, 4.4 N}` low; `{4.1 N, 4.2 N, 4.3 Y, 4.4 Y, 4.5 N}` some_concerns; `{… 4.5 PY}` high; `{4.1 N, 4.2 NI, 4.3 N}` some_concerns; `{4.1 N, 4.2 NI, 4.3 Y, 4.4 Y, 4.5 Y}` high; `{4.1 Y, 4.2 N}` high; `{4.1 N, 4.2 PY}` high.
D5: `{Y,N,N}` low; `{NI,N,N}` some_concerns; `{Y,N,NI}` some_concerns; `{Y,NI,NI}` some_concerns; `{Y,PY,N}` high; `{N,N,Y}` high.
Refusals: D3 `{3.1 N, 3.2 NI}` → `NotAllowed(3.2)`; D2 `{2.1 Y, 2.2 N, 2.6 Y}` → `MissingAnswer(2.3)`; D4 `{4.1 Y}` → `MissingAnswer(4.2)`; D1 with `"9.9"` → `UnknownQuestion`; `parse(" py ")` → PY; `parse("NA")`, `parse("Unclear")` → `UnreadableAnswer`.
Applicable: D2 `{2.1 N, 2.2 N}` → `["2.1","2.2","2.6"]`; `{2.1 N, 2.2 N, 2.6 NI}` → adds `"2.7"`.
Overall: five low → low; one some_concerns → some_concerns, `needs_escalation_decision` false; two some_concerns + `None` → `EscalationUndecided`, `+False` → some_concerns, `+True` → high; any high → high (escalate ignored).

- [ ] **Step 2: Run, expect failure.** `uv run pytest tests/test_rob2_algorithm.py -v` → FAIL (`ModuleNotFoundError: bmlib.quality.rob2_algorithm`).

- [ ] **Step 3: Implement `bmlib/quality/rob2_algorithm.py`** with the interfaces above. One function per domain behind `domain_judgement`; the conditions table in one place, used by both `applicable_questions` and the missing-answer check.

- [ ] **Step 4: Run, expect pass.** Same command → PASS.

- [ ] **Step 5: Write `scripts/make_rob2_algorithm.py`** producing `tests/fixtures/rob2_algorithm.json` with: `algorithm_version`; `official` (the Step 1 cases, each with `"source": "Table 4"` etc., hand-listed in the script, **not** computed); `exhaustive` (every combination of answers over each domain's applicable branch, enumerated by walking `applicable_questions`, with Python's judgement); `refusals` (Step 1's, with the error `kind`); `overall` cases. Commit the generated file.

- [ ] **Step 6: Write the contract tests** `tests/test_rob2_algorithm_contract.py` (Python reproduces every section, and re-running the script reproduces the file byte for byte) and `rust/bmlib/tests/rob2_algorithm_contract.rs` (Rust reproduces every case and refusal). Run `cargo test --test rob2_algorithm_contract` → FAIL (unresolved module).

- [ ] **Step 7: Implement `rust/bmlib/src/quality/rob2_algorithm.rs`** with the same interfaces; serde `rename` to the wire values. Run both contract tests → PASS. Run all gates.

- [ ] **Step 8: Commit and open PR** "RoB 2 decision rules, Rust and Python (StudyAnalyzer evidence weight piece 4a)".

```bash
git add bmlib/quality/rob2_algorithm.py bmlib/quality/__init__.py scripts/make_rob2_algorithm.py tests/fixtures/rob2_algorithm.json tests/test_rob2_algorithm*.py rust/bmlib/src/quality/rob2_algorithm.rs rust/bmlib/src/quality/mod.rs rust/bmlib/tests/rob2_algorithm_contract.rs CHANGELOG.md CLAUDE.md
git commit -m "feat(quality): RoB 2 decision rules from the 2019 guidance, Rust and Python"
```

---

### Task 2: The evidence ladder and the number (pure)

**Files:**
- Create: `bmlib/quality/evidence_ladder.py`, `rust/bmlib/src/quality/evidence_ladder.rs`
- Create: `scripts/make_evidence_ladder.py`, `tests/fixtures/evidence_ladder.json`
- Test: `tests/test_evidence_ladder.py`, `tests/test_evidence_ladder_contract.py`, `rust/bmlib/tests/evidence_ladder_contract.rs`

**Interfaces:**
- Consumes: `Judgement` (Task 1); `StudyDesign` (`quality::data_models`).
- Produces:
  - `enum Level { High, Moderate, Low, VeryLow }`, wire `"high" "moderate" "low" "very_low"`.
  - `enum EffectScale { Ratio, Difference }`, wire `"ratio" "difference"` (no effect: 1.0 and 0.0).
  - `struct ReportedResult { measure: String, scale: EffectScale, estimate: f64, ci_low: Option<f64>, ci_high: Option<f64>, ci_level: Option<f64>, n_analysed: Option<u32> }` with `fn validate(&self) -> Result<(), String>` (both bounds or neither; `ci_low ≤ estimate ≤ ci_high`; ratio values > 0; all finite; `0 < ci_level < 1`).
  - `struct LadderSettings { some_concerns_steps: u8 = 1, high_steps: u8 = 2, crosses_no_effect_steps: u8 = 1, small_trial_n: u32 = 400, small_trial_steps: u8 = 1, no_interval_steps: u8 = 1, max_imprecision_steps: u8 = 2, weight_high: f64 = 1.0, weight_moderate: f64 = 0.7, weight_low: f64 = 0.4, weight_very_low: f64 = 0.1 }` with `Default` (Python dataclass with these defaults) and serde.
  - `enum LadderReason { RiskOfBias(Judgement), CrossesNoEffect, SmallTrial { n: u32, threshold: u32 }, NoInterval, SampleSizeUnknown }`.
  - `struct LadderStep { reason: LadderReason, levels: u8 }`; `struct EvidenceLadder { start: Level, steps: Vec<LadderStep>, level: Level, weight: f64 }`.
  - `fn start_level(design: StudyDesign) -> Option<Level>`: `Rct` → `High`; every other design → `None` (not covered).
  - `fn build_ladder(start: Level, rob_overall: Judgement, result: Option<&ReportedResult>, settings: &LadderSettings) -> Result<EvidenceLadder, String>` (`Err` only for a result failing `validate`).
  - `fn paper_weight(ladders: &[EvidenceLadder]) -> Option<f64>`: the largest `weight`; `None` for an empty slice (spec §4.1, decided: a paper passes if any primary outcome passes).
  - `const LADDER_VERSION: &str = "ladder/1"`.

Rules: a risk-of-bias step always appears (0 levels for Low). Imprecision: no result, or no interval → `NoInterval`; interval includes the no-effect value (inclusive) → `CrossesNoEffect`; `n_analysed` below `small_trial_n` → `SmallTrial`, absent → `SampleSizeUnknown` with 0 levels (shown, not hidden). Imprecision levels summed then capped at `max_imprecision_steps`; the cap is recorded by trimming the last step's `levels`. The level never drops below `VeryLow`. `small_trial_n = 400` is GRADE's rule-of-thumb optimal information size for a continuous outcome; it is a default to tune, said so in its doc comment.

- [ ] **Step 1: Write failing tests** in `tests/test_evidence_ladder.py`:

```python
def test_clean_trial_stays_high():
    r = ReportedResult("risk ratio", EffectScale.RATIO, 0.7, 0.55, 0.88, 0.95, 1200)
    lad = build_ladder(Level.HIGH, Judgement.LOW, r, LadderSettings())
    assert (lad.level, lad.weight) == (Level.HIGH, 1.0)
    assert [s.levels for s in lad.steps] == [0]
```

Also: some_concerns + interval 0.6–1.02 (ratio) → moderate then low (`[1, 1]`, level LOW, weight 0.4); high + crossing + n=80 → steps `[2, 1, 1]` capped imprecision 2 → VERY_LOW (floor), weight 0.1; difference scale −2.0 (−3.1, −0.9) not crossing; `ci_low` only → `validate` error; ratio `ci_low = 0` → error; `result=None` → `NoInterval`; `n_analysed=None` → `SampleSizeUnknown` with 0 levels; `start_level(StudyDesign.COHORT_PROSPECTIVE) is None`; `paper_weight([lad_low, lad_high]) == 1.0`, `paper_weight([]) is None`; a custom `LadderSettings(weight_low=0.5)` is honoured.

- [ ] **Step 2: Run, expect failure.** `uv run pytest tests/test_evidence_ladder.py -v` → FAIL (module missing).
- [ ] **Step 3: Implement `bmlib/quality/evidence_ladder.py`.**
- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Fixture and contract**: `scripts/make_evidence_ladder.py` writes `tests/fixtures/evidence_ladder.json` (`ladder_version`, `default_settings`, `cases` = Step 1's inputs with expected ladders as dicts, `refusals`); Python contract test checks the file and the script's reproduction; `rust/bmlib/tests/evidence_ladder_contract.rs` → FAIL.
- [ ] **Step 6: Implement `rust/bmlib/src/quality/evidence_ladder.rs`.** Floats compare exactly (the weights are table values, not computed). Contracts → PASS; all gates.
- [ ] **Step 7: Commit and open PR** "Evidence ladder and number (piece 5)", stacked on Task 1's.

---

### Task 3: Trial layout and the design proposal (pieces 1 and 2)

**Files:**
- Modify: `bmlib/quality/data_models.py`, `rust/bmlib/src/quality/data_models.rs` (add `RctLayout`)
- Create: `bmlib/quality/design_agent.py`, `rust/bmlib/src/quality/design_agent.rs`
- Modify: `bmlib/quality/_condense.py` and/or the Rust `Condenser` seam (see Step 3)
- Create: `tests/fixtures/design_replies.json`
- Test: `tests/test_design_agent.py`, `tests/test_design_replies_contract.py`, `rust/bmlib/tests/design_replies_contract.rs`, Rust unit tests in `design_agent.rs`

**Interfaces:**
- Consumes: `StudyDesign` and its tolerant label lookup (`STUDY_DESIGN_MAPPING` / Rust equivalent); `verify_quotes`, `PicoCitation` (`quality::pico_agent` / `pico_models`); `JsonChat`, `LlmCondenser::with_prompts`, `ProcessingConfig`.
- Produces:
  - `enum RctLayout { Parallel, Cluster, Crossover, Other }`, wire `"parallel" "cluster" "crossover" "other"`; `fn is_covered(self) -> bool` (only `Parallel`). Separate from `StudyDesign`, so its wire values and every existing tier stay unchanged.
  - `trait QueryCondenser { fn condense(&mut self, query: &str, text: &str, label: &str, config: &ProcessingConfig) -> ProcessingResult; }` with production `LlmQueryCondenser::new(model: &dyn ContextModel, temperature, max_tokens)` building an `LlmCondenser` per call with the given query and the existing verbatim-passage prompts. Python: `condense_text` gains a keyword `query` if it lacks one. Tasks 4 and 5 use the same seam.
  - `struct DesignProposal { design: StudyDesign, rct_layout: Option<RctLayout>, citations: Vec<PicoCitation>, quotes_not_found: Vec<String>, reasoning: String, condensed: bool }` (serde / `to_dict`/`from_dict`).
  - `fn read_design_reply(data: &Value, text: &str, min_similarity: f64) -> Result<DesignProposal, String>` (pure; `condensed` false).
  - `struct DesignAgent<'a>` (`new(chat: &'a mut dyn JsonChat)`, `with_condenser(&'a mut dyn QueryCondenser)`, `with_stop(&'a AtomicBool)`, sampling `with_` setters as `PicoAgent`) with `fn propose(&mut self, title: &str, text: &str, min_similarity: f64) -> Result<DesignProposal, ProposalError>`.
  - `enum ProposalError { EmptyInput, CondensationFailed(String), RequestFailed(String), Unreadable(String), Stopped, InvalidSimilarity }` (shared by Task 4).
  - Constants: `DESIGN_SYSTEM_PROMPT`, `DESIGN_TASK_PROMPT`, `DESIGN_CONDENSE_QUERY`, `DESIGN_PROMPT_VERSION: u32 = 1`.

Reply shape asked for: `{"study_design": "<wire value>", "rct_layout": "parallel|cluster|crossover|other|null", "quotes": ["…"], "reasoning": "…"}`. The prompt says *this paper's own design, not designs it cites* (bmlibrarian `paper_weight/llm_extractors.py:107-111`). A label the tolerant lookup does not know → `Err`, never `Unknown`. `rct_layout` is required when the design is `rct` (`Err("rct_layout missing")`) and refused otherwise (`Err("rct_layout given for a non-RCT")`).

- [ ] **Step 1: Failing tests for `read_design_reply`** (Python): parallel RCT with a quote present in the text → citation located; `"rct"` without layout → error; `"cohort_prospective"` with layout → error; `"randomised trial"` (not a wire value but in the tolerant mapping) → RCT; `"banana"` → error; a quote absent from the text → `quotes_not_found == [quote]`.
- [ ] **Step 2: Run, expect failure.**
- [ ] **Step 3: Implement** `RctLayout`, `read_design_reply`, the `QueryCondenser` seam, and `DesignAgent` (Python `DesignAgent(BaseAgent)` on `PicoAgent`'s model: full text when it fits the window, else one condensation with `DESIGN_CONDENSE_QUERY`, `condensed = True`).
- [ ] **Step 4: Agent tests** with a scripted chat: a reply is read; an empty text → `EmptyInput` with no request; a stop set before the call → `Stopped`; a paper over the window with a scripted condenser → `condensed` true and the quote verified against the **original** text.
- [ ] **Step 5: Run, expect pass.**
- [ ] **Step 6: Contract** `tests/fixtures/design_replies.json` (reply, text, expected proposal or error), Python and Rust contract tests; implement the Rust side; all gates.
- [ ] **Step 7: Commit and open PR** "Trial layout and the full-text design proposal (pieces 1, 2)".

---

### Task 4: Primary outcomes and their results (piece 3)

**Files:**
- Create: `bmlib/quality/outcome_agent.py`, `rust/bmlib/src/quality/outcome_agent.rs`
- Create: `tests/fixtures/outcome_replies.json`
- Test: `tests/test_outcome_agent.py`, `tests/test_outcome_replies_contract.py`, `rust/bmlib/tests/outcome_replies_contract.rs`

**Interfaces:**
- Consumes: `ReportedResult`, `EffectScale` (Task 2); `QueryCondenser`, `ProposalError` (Task 3); `verify_quotes`, `PicoCitation`.
- Produces:
  - `struct ProposedOutcome { name: String, timepoint: Option<String>, result: Option<ReportedResult>, result_error: Option<String>, citations: Vec<PicoCitation>, quotes_not_found: Vec<String> }`. A result failing `ReportedResult::validate` is kept out of `result` and its reason put in `result_error` (Review Focus 3).
  - `struct OutcomeProposal { outcomes: Vec<ProposedOutcome>, condensed: bool }`.
  - `fn read_outcome_reply(data: &Value, text: &str, min_similarity: f64) -> Result<OutcomeProposal, String>`.
  - `struct OutcomeAgent<'a>` with the same constructor/`with_` set as `DesignAgent` and `fn propose(&mut self, title, text, min_similarity) -> Result<OutcomeProposal, ProposalError>`.
  - Constants: `OUTCOME_SYSTEM_PROMPT`, `OUTCOME_TASK_PROMPT`, `OUTCOME_CONDENSE_QUERY`, `OUTCOME_PROMPT_VERSION: u32 = 1`.

Reply shape: `{"primary_outcomes": [{"name", "timepoint", "quotes", "result": {"measure", "scale": "ratio|difference", "estimate", "ci_low", "ci_high", "ci_level", "n_analysed"} | null}]}`. The prompt asks only for outcomes **the paper calls primary** (co-primary allowed) and the result for the primary analysis. An empty list is a valid answer (the tab asks the reader; the unattended path reports "no primary outcome found"). `measure` stays a free-form string.

- [ ] **Step 1: Failing tests for `read_outcome_reply`**: one outcome with a ratio result → read, quote located; two co-primary outcomes; `ci_low > ci_high` → `result is None`, `result_error` names it; ratio `ci_low = 0` → same; `"scale": "percent"` → error for the reply; `[]` → empty proposal; a non-finite estimate (`1e999` after strict parsing) → refused.
- [ ] **Step 2: Run, expect failure.**
- [ ] **Step 3: Implement** `read_outcome_reply` and `OutcomeAgent` (Python and Rust, as Task 3).
- [ ] **Step 4: Agent tests** as Task 3 Step 4.
- [ ] **Step 5: Contract** `tests/fixtures/outcome_replies.json`, both languages; all gates.
- [ ] **Step 6: Commit and open PR** "Primary outcomes and results proposal (piece 3)".

---

### The questions' wording (decided)

RoB 2's guidance is **CC BY-NC-ND 4.0**, so its wording is not copied. bmlib
words each of the 22 questions **in its own plain language, written for a
language model to understand**: one self-contained question per id, saying
what counts as each answer (Y/PY/PN/N/NI) in this trial, with no reference to
"the previous question". Write it from the meaning of each signalling question,
not by editing the official sentences. The method is credited (RoB 2, Higgins
et al., 2019) in the module's doc comment. The text lives only in
`rob2_questions.{py,rs}`, one constant per id; Python and Rust hold the same
strings (pinned by `rob2_replies.json`'s `question_texts` section).

---

### Task 5: The RoB 2 assessor (piece 4)

**Files:**
- Create: `bmlib/quality/rob2_questions.py`, `rust/bmlib/src/quality/rob2_questions.rs` (wording per the gate, a domain condense query per domain, the escalation question)
- Create: `bmlib/quality/rob2_agent.py`, `rust/bmlib/src/quality/rob2_agent.rs`
- Create: `tests/fixtures/rob2_replies.json`, `tests/fixtures/rob2_run.json`
- Test: `tests/test_rob2_agent.py`, `tests/test_rob2_contract.py`, `rust/bmlib/tests/rob2_contract.rs`

**Interfaces:**
- Consumes: Task 1 (all), `QueryCondenser`, `ProposedOutcome` (Task 4), `verify_quotes`, `PicoCitation`, `JsonChat`.
- Produces:
  - `fn question_text(id: &str) -> &'static str`; `const ROB2_CONDENSE_QUERIES: [(Domain, &str); 5]`; `const ESCALATION_PROMPT`; `const ROB2_PROMPT_VERSION: u32 = 1`.
  - `struct MachineAnswer { answer: Answer, citations: Vec<PicoCitation>, quotes_not_found: Vec<String>, reasoning: String }`.
  - `struct DomainAssessment { domain: Domain, answers: BTreeMap<String, MachineAnswer>, judgement: Judgement, condensed: bool, ignored: Vec<String> }` (`ignored`: ids answered but not applicable, listed rather than dropped silently).
  - `enum DomainResult { Assessed(DomainAssessment), Failed { domain: Domain, reason: String } }`.
  - `struct Escalation { escalate: bool, reasoning: String }`.
  - `struct OutcomeRob2 { outcome: String, domains: Vec<DomainResult> /* D2–D5 */, escalation: Option<Escalation>, overall: Option<Judgement> }` (`overall` is `None` when any of D1–D5 failed: "not assessed", never a judgement).
  - `struct Rob2Run { algorithm_version: String, prompt_version: u32, study: DomainResult /* D1 */, outcomes: Vec<OutcomeRob2> }`, serde and `to_dict`/`from_dict`, pinned by `rob2_run.json`.
  - `fn read_domain_reply(domain: Domain, data: &Value, text: &str, min_similarity: f64) -> Result<DomainAssessment, String>` (pure).
  - `struct Rob2Agent<'a>` (`new(chat)`, `with_condenser(&mut dyn QueryCondenser)`, `with_stop`, sampling setters) with `fn assess(&mut self, title: &str, text: &str, outcomes: &[ProposedOutcome], min_similarity: f64) -> Result<Rob2Run, Rob2RunError>`; `enum Rob2RunError { EmptyInput, NoOutcomes, Stopped, InvalidSimilarity }` (per-domain failures live in `DomainResult::Failed`, not here).

Calls: one per domain, D1 once, D2–D5 once per outcome (the outcome's name, timepoint and result in the prompt), never one call for the whole tool. Each asks for `{"answers": {"<id>": {"answer": "Y|PY|PN|N|NI|NA", "quotes": [...], "reasoning": "..."}}}` over all the domain's questions, NA allowed for conditional ones. `read_domain_reply` parses with `Answer::parse` (NA → absent), checks applicability with Task 1, derives the judgement with `domain_judgement`, verifies quotes against the **original** text. The D5 prompt tells the model to compare the paper's stated registered or pre-specified primary outcome with the reported one (spec §4.3). When `needs_escalation_decision`, one more call with `ESCALATION_PROMPT` → `Escalation`; `overall_judgement` then decides. A paper over the window is condensed **per domain** with that domain's query; `condensed` set on that domain.

- [ ] **Step 1: Failing tests for `read_domain_reply`** (Python): a complete D1 reply → judgement low, quotes located; D2 reply giving `"NA"` for 2.3 when 2.1 is `"Y"` → `Err` naming `2.3` (Review Focus 1); D3 `3.2: "NI"` → `Err` naming 3.2; `" py "` accepted as PY; `"Unclear"` → `Err`; an answer for 2.7 when 2.6 is Y → judgement computed, `ignored == ["2.7"]`; NI answer with no quotes → kept; a quote not in the text → in `quotes_not_found` (Review Focus 4); an unknown id `"2.9"` → `Err`.
- [ ] **Step 2: Run, expect failure.**
- [ ] **Step 3: Implement** `rob2_questions` (per the gate), `read_domain_reply`, the models, `Rob2Agent` (Python on `PicoAgent`'s model).
- [ ] **Step 4: Agent tests** with a scripted chat: one outcome → 5 calls in order D1, D2…D5; two outcomes → 9; scripted replies giving two some_concerns → a sixth (escalation) call, and `{"escalate": true}` → overall high; one domain's reply unreadable → that `DomainResult::Failed`, other domains assessed, `overall None`; stop set after the second call → `Stopped` (Review Focus 5); `outcomes=[]` → `NoOutcomes` with no request; over-window paper with a scripted `QueryCondenser` → condenser called with each domain's query, every `condensed` true.
- [ ] **Step 5: Run, expect pass.**
- [ ] **Step 6: Contracts**: `rob2_replies.json` (domain, reply, text, expected assessment or error) and `rob2_run.json` (a full run's dict; both languages round-trip it unchanged). Implement the Rust side; all gates.
- [ ] **Step 7: Commit and open PR** "RoB 2 assessor, per domain with verified quotes (piece 4)".

---

### Task 6: The unattended entry point

**Files:**
- Create: `bmlib/quality/evidence_weight.py`, `rust/bmlib/src/quality/evidence_weight.rs`
- Test: `tests/test_evidence_weight.py`, Rust unit tests in `evidence_weight.rs`

**Interfaces:**
- Consumes: Tasks 2–5.
- Produces:
  - `struct AssessedOutcome { outcome: ProposedOutcome, rob2: OutcomeRob2, ladder: Option<EvidenceLadder>, ladder_error: Option<String> }` (`ladder` `None` when `rob2.overall` is `None` or the result is refused; the reason in `ladder_error`).
  - `enum EvidenceWeight { NotCovered { design: DesignProposal }, NoPrimaryOutcome { design: DesignProposal, outcomes: OutcomeProposal }, Assessed { design: DesignProposal, outcomes: OutcomeProposal, run: Rob2Run, assessed: Vec<AssessedOutcome>, paper_weight: Option<f64>, condensed: bool } }`, serde/`to_dict`.
  - `fn assess_unattended(chat: &mut dyn JsonChat, condenser: Option<&mut dyn QueryCondenser>, stop: Option<&AtomicBool>, settings: &LadderSettings, title: &str, text: &str, min_similarity: f64) -> Result<EvidenceWeight, EvidenceWeightError>` (Python: `assess_unattended(llm_client, model, *, settings, title, text, min_similarity, stop=None)`), where `EvidenceWeightError` wraps `ProposalError` and `Rob2RunError`.

Flow: design proposal; `start_level(design)` is `None` or layout not covered → `NotCovered`; outcome proposal; empty → `NoPrimaryOutcome`; RoB 2 over **all** proposed outcomes; per outcome `build_ladder(start, overall, result)`; `paper_weight` over the ladders built; `condensed` true if any step condensed. Machine proposals are accepted as they come: that is what "unattended" means (spec §4.1).

- [ ] **Step 1: Failing tests** with a scripted chat: cohort design → `NotCovered`, one request only; crossover RCT → `NotCovered`; parallel RCT, no primary outcome → `NoPrimaryOutcome`; parallel RCT, two outcomes, one with a refused result → that outcome's `ladder None` with `ladder_error`, `paper_weight` from the other; a D3 failure for one outcome → its `ladder None`, `ladder_error` says not assessed; any condensed step → top-level `condensed` true.
- [ ] **Step 2: Run, expect failure.** `uv run pytest tests/test_evidence_weight.py -v` → FAIL (module missing).
- [ ] **Step 3: Implement both languages.**
- [ ] **Step 4: Run, expect pass; all gates.**
- [ ] **Step 5: Commit**, into Task 5's PR or its own stacked PR.

---

## D5 from facts (spec §4.6, added 2026-11-11)

Tasks 7 to 9 replace D5's signalling-question call with the facts design the
RoB 2 wording smoke test settled on (StudyAnalyzer
`docs/smokescreens/2026-11-10-rob2-wording.md`, arm F version 3 with the
checks, "F3p"). Its scripts are the **reference implementation** to port:
`docs/smokescreens/scripts/fact_guards.py` (checks), `d5_facts.py` (facts,
rules), `d5_versions.py` (measurements, analysis check, second look), with
their tests. Work continues on `feat/evidence-weight` in
`~/src/bmlib-evidence-weight`. Everything above still applies; in addition:

- **Quote checks are code, never model output.** The planning check is always
  on (the maintainer's call).
- **A digit is ASCII `0-9`** in both languages (Python's `str.isdigit` is
  wider than Rust's `is_ascii_digit`); regexes are case-insensitive and use no
  lookaround, so `re` and the `regex` crate agree; the fixture pins it.
- The D5 signalling-question texts stay in `rob2_questions` (the tab's human
  reviewer answers them); the agent no longer sends them.
- **Review Focus for these tasks:**
  1. A facts reply missing a fact key, with `present` not a boolean, a
     measurement without a label, or `main_analysis.differs` not a boolean →
     unreadable, asked again once, then D5 `Failed` naming the field (Task 8,
     Task 9).
  2. A claimed fact whose found quotes all fail its check → absent, but stored
     with `claimed: true` and which check refused it, never dropped (Task 7,
     Task 8).
  3. A second look whose call fails → that measurement is undecided (not
     missing), its error stored; the D5 answer is computed without it (Task 9).
  4. Stop requested between second looks → the run ends `Stopped` (Task 9).
  5. Non-ASCII quotes and outcome names ("≤", "–", "é", "χ2") → checks give the
     same verdict in both languages (Task 7 fixture cases).

---

### Task 7: D5 quote checks (pure)

**Files:**
- Create: `bmlib/quality/rob2_d5_checks.py`, `rust/bmlib/src/quality/rob2_d5_checks.rs`
- Create: `tests/fixtures/rob2_d5_checks.json`, `scripts/make_rob2_d5_fixtures.py` (writes this fixture and Task 8's)
- Test: `tests/test_rob2_d5_checks.py`, `tests/test_rob2_d5_contract.py`, `rust/bmlib/tests/rob2_d5_contract.rs`

**Interfaces:**
- Produces:
  - `enum D5Fact` with wire names `plan_cited`, `plan_dated_early`, `plan_late`, `measure_fixed`, `timepoint_fixed`, `analysis_fixed`, `switched`, `change_explained`, in that order (`D5_FACTS` lists them).
  - `fn supports(fact: D5Fact, quote: &str, outcome: &str) -> bool`: the per-quote check (`plan_dated_early`: a date or a before-phrase; `switched`: change words; `measure_fixed`: names the outcome; `timepoint_fixed`: a time; every other fact: true).
  - `fn states_a_plan(quotes: &[&str]) -> bool`.
  - `fn fact_verdict(fact: D5Fact, claimed: bool, found: &[&str], outcome: &str) -> FactVerdict` where `enum FactVerdict { Present, NotClaimed, NoQuoteFound, QuoteDoesNotSupport, NoPlanStated }` (wire names snake_case); `Present` only when claimed, `found` non-empty, some quote `supports`, and, for `measure_fixed`/`timepoint_fixed`/`analysis_fixed`, `states_a_plan(found)`.
  - `fn shows_result(passages: &[&str]) -> bool`: some passage holds an ASCII digit or points to a figure, table, supplement or appendix.

The vocabularies are `fact_guards.py`'s `_DATE`, `_BEFORE`, `_CHANGE` (with `differ(s|ed)? from|different from`, not bare `differ`), `_TIME`, `_GENERIC`, `_PLANNING`, and `d5_versions.py`'s `POINTER`, as named module constants.

- [ ] **Step 1: Failing Python tests**, ported from `test_fact_guards.py` and `test_d5_versions.py` (pointer cases), plus: `fact_verdict(PLAN_DATED_EARLY, True, ["registered with ClinicalTrials.gov, NCT04546581."], "x") == QuoteDoesNotSupport`; `fact_verdict(MEASURE_FIXED, True, ["Pain was rated on a VAS."], "pain") == NoPlanStated`; `fact_verdict(SWITCHED, True, [], "x") == NoQuoteFound`; `fact_verdict(PLAN_CITED, False, ["…"], "x") == NotClaimed`; `shows_result(["p = 0.04"])`, `shows_result(["shown in Figure 2"])`, `not shows_result(["Function improved in both groups."])`.
- [ ] **Step 2: Run, expect failure** (`uv run pytest tests/test_rob2_d5_checks.py -v`: module missing).
- [ ] **Step 3: Implement the Python module.**
- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Contract.** `make_rob2_d5_fixtures.py` writes `rob2_d5_checks.json`: `{"supports": [{fact, quote, outcome, expected}], "states_a_plan": [...], "shows_result": [...], "fact_verdict": [...]}`, at least every case of Step 1, the smoke test's 13 dropped facts (guard_report output) and five non-ASCII cases (Review Focus 5). Both contract tests assert every case; Python also asserts the script rebuilds the committed file.
- [ ] **Step 6: Implement Rust** (`regex::Regex` in `LazyLock`s); all gates.
- [ ] **Step 7: Commit** "feat(quality): D5 quote checks, Rust and Python (§4.6)".

### Task 8: The D5 facts reply and its rules (pure)

**Files:**
- Create: `bmlib/quality/rob2_d5_facts.py`, `rust/bmlib/src/quality/rob2_d5_facts.rs`
- Create: `tests/fixtures/rob2_d5_facts.json` (by `make_rob2_d5_fixtures.py`)
- Test: `tests/test_rob2_d5_facts.py`, the Task 7 contract tests (extended)

**Interfaces:**
- Consumes: Task 7; `verify_quotes`, `PicoCitation`, `PicoContradictedQuote`, `ProposedOutcome`, `Answer`, `Domain`, `domain_judgement`, `MachineAnswer`, `DomainAssessment`.
- Produces:
  - `struct FactReport { claimed: bool, citations: Vec<PicoCitation>, quotes_not_found: Vec<String>, contradicted: Vec<PicoContradictedQuote>, verdict: FactVerdict }`.
  - `struct OtherMeasurement { label: String, named: Vec<PicoCitation>, result: Vec<PicoCitation>, second_look: Option<Vec<PicoCitation>>, second_look_error: Option<String> }` with `fn counts(&self) -> bool` (named non-empty), `fn reported(&self) -> bool` (`shows_result` on `result` or `second_look`), `fn needs_second_look(&self) -> bool` (counts, not reported, no look yet), `fn missing(&self) -> bool` (counts, not reported, `second_look` is `Some`).
  - `struct AnalysisCheck { differs: bool, planned: Vec<PicoCitation>, reported: Vec<PicoCitation> }`, `fn counts(&self) -> bool` (differs, both non-empty).
  - `struct D5Facts { facts: BTreeMap<D5Fact, FactReport>, measurements: Vec<OtherMeasurement>, analysis: AnalysisCheck }`, `to_json`/`from_json` (Python `to_dict`/`from_dict`); `from_json` recomputes each `verdict` and refuses a stored one that disagrees.
  - `fn facts_prompt(title: &str, paper: &str, outcome: &ProposedOutcome) -> String`; `fn second_look_prompt(title: &str, paper: &str, outcome: &ProposedOutcome, item: &OtherMeasurement) -> String`; `const D5_FACTS_SYSTEM_PROMPT`, `const D5_SECOND_LOOK_SYSTEM_PROMPT`; the fact and list meanings as constants (wording from `d5_facts.py` `FACTS` with `d5_versions.py`'s `switched`/`change_explained` overrides, `LISTS["other_measurements"]`, `MAIN_ANALYSIS`).
  - `fn read_facts_reply(data: &Value, text: &str, outcome: &str, min_similarity: f64) -> Result<D5Facts, String>`; `fn read_second_look(data: &Value, text: &str, min_similarity: f64) -> Result<Vec<PicoCitation>, String>`.
  - `fn d5_answers(facts: &D5Facts) -> BTreeMap<String, (Answer, Vec<D5Fact>)>` (the rules of spec §4.6: `switched` counts only when `change_explained` is not present; 5.2 PY from any `missing()` measurement, 5.3 PY from `analysis.counts()`); `fn d5_assessment(facts: D5Facts) -> DomainAssessment` (each `MachineAnswer`'s citations are the deciding facts' citations, its `reasoning` lists every fact's verdict; judgement by `domain_judgement`; `facts` attached, Task 9).

- [ ] **Step 1: Failing Python tests**, ported from `test_d5_facts.py` and `test_d5_versions.py` (rules, reader, refusals, the RCT035 analysis case, the switch-with-reason case), plus: a fact claimed with a found quote that fails its check → `verdict == QuoteDoesNotSupport`, absent from the rules, present in `to_dict()` (Review Focus 2); `read_facts_reply` with `"present": "yes"` → error naming `facts.plan_cited.present`; without `main_analysis` → error naming it (Review Focus 1); `D5Facts.from_dict(x.to_dict()) == x`.
- [ ] **Step 2: Run, expect failure.**
- [ ] **Step 3: Implement the Python module.**
- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Contract** `rob2_d5_facts.json`: `prompts` (both prompts rendered for one fixed paper, outcome and item, so the wording is pinned across languages), `replies` (reply, text, outcome → expected `D5Facts` dict, answers and judgement, or the error), `second_looks` (reply, text → expected passages or error), built from the smoke test's patterns (RCT035-like differing analysis, an RCT059-like figure pointer, an RCT085-like "difference" non-switch, an RCT096-like undated registration).
- [ ] **Step 6: Implement Rust**; all gates.
- [ ] **Step 7: Commit** "feat(quality): the D5 facts reply and its rules, Rust and Python (§4.6)".

### Task 9: The assessor asks D5 for facts

**Files:**
- Modify: `bmlib/quality/rob2_agent.py` (`_domain` for D5, `DomainAssessment`), `rust/bmlib/src/quality/rob2_agent.rs` (`domain()` for D5, `DomainAssessment`)
- Modify: `bmlib/quality/rob2_questions.py`, `rust/bmlib/src/quality/rob2_questions.rs` (`ROB2_PROMPT_VERSION = 5`)
- Modify: `scripts/make_rob2_fixtures.py`, `tests/fixtures/rob2_run.json`, `rob2_replies.json`, `evidence_weight.json` (regenerated), `tests/test_rob2_agent.py`, `rust/bmlib/src/quality/rob2_agent.rs` tests, `CHANGELOG.md`, `CLAUDE.md` (new files)

**Interfaces:**
- Consumes: Task 8.
- Produces: `DomainAssessment.facts: Option<D5Facts>` (`Some` only for D5; in `to_json` as `"facts"` when present; `from_json` reads it when present and, for D5, re-derives the answers and judgement from it and refuses a mismatch).

Flow for D5, per outcome: the paper (or D5's digest, condensed with D5's query as now) goes into `facts_prompt`; `ask_until_readable` with `D5_FACTS_SYSTEM_PROMPT` and `read_facts_reply` (unreadable → asked once more, then `DomainResult::Failed`, as every domain). Then, in list order, each measurement with `needs_second_look()` gets one `ask_until_readable` with `second_look_prompt` over the same text sent and `read_second_look`; a failed call stores `second_look_error` and leaves `second_look` `None` (undecided); a stop between looks ends the run `Stopped`. Then `d5_assessment(facts)` with `condensed` set.

- [ ] **Step 1: Failing agent tests** (scripted chat, Python): one outcome, a facts reply with no measurement needing a look → 5 calls (D1, D2, D3, D4, D5 facts), D5's answers from the rules; a facts reply with two measurements without a result → 7 calls, the second looks in list order, one `found: false` → 5.2 PY, judgement high; a second look whose call fails → that measurement undecided, `second_look_error` set, 5.2 not PY (Review Focus 3); stop set after the first second look → `Stopped` (Review Focus 4); an unreadable facts reply twice → D5 `Failed` naming the field, other domains assessed, `overall None`; an over-window paper → the facts prompt and the second looks carry D5's digest, `condensed` true; `Rob2Run.from_dict(run.to_dict()) == run` with facts.
- [ ] **Step 2: Run, expect failure.**
- [ ] **Step 3: Implement Python;** bump the prompt version; regenerate the fixtures with `scripts/make_rob2_fixtures.py` and `scripts/make_evidence_weight.py`.
- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Implement Rust** to the regenerated fixtures and the ported agent tests; all gates.
- [ ] **Step 6: CHANGELOG** `[Unreleased]` entry (D5 from facts, the checks, prompt version 5, the reference: the smoke test), CLAUDE.md file list.
- [ ] **Step 7: Commit** "feat(quality): RoB 2 D5 from facts with checked quotes and a second look (§4.6)".

---

## After phase 1

StudyAnalyzer's phase 2 (the tab) pins bmlib at the merge of the last PR,
as the repo's `chore: pin bmlib` PRs do. Piece 6 (the export format) belongs
to phase 3's plan.
