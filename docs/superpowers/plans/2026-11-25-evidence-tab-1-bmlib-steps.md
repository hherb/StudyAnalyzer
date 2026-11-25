# Evidence weight tab, plan 1: RoB 2 a step at a time (bmlib)

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** bmlib can run RoB 2 one stored step at a time (a plan of steps,
one call per step, the outcome combined in code), with prompt-size measures
for every step and condensing progress in Rust, so StudyAnalyzer's tab can
store each domain as it lands.

**Architecture:** `Rob2Agent::assess` today loops privately over D1, then
D2 to D5 per outcome, then an escalation per outcome. This plan makes that
loop's body public (`plan_rob2`, `assess_domain`, `escalate_outcome`,
`combine_outcome`, with a caller-held `Rob2Digests` for the per-domain
condensations) and rewrites `assess` as a loop over it, in Rust and Python.
Nothing a caller of `assess` or `assess_unattended` sees changes.

**Tech Stack:** Rust (`rust/bmlib`), Python (`bmlib/`), shared JSON
fixtures written by `scripts/make_rob2_fixtures.py`.

**Spec:** `docs/superpowers/specs/2026-11-24-evidence-weight-tab-design.md`
§4 (in StudyAnalyzer), on `2026-11-09-evidence-weight-design.md` §4.2, §4.5
and §4.6.

## Global Constraints

- Work in a bmlib worktree, never the shared `~/src/bmlib` checkout:
  `git -C ~/src/bmlib worktree add ~/src/bmlib-rob2-steps -b feat/rob2-steps
  origin/main`. Follow bmlib's `HANDOVER.md` and `docs/SESSION-RULES.md`.
- Rust and Python on par in one PR; every new public name exists in both,
  with the same wire form and the same values in the shared fixture.
- `assess` and `assess_unattended` must give byte-identical results: the
  existing fixtures `rob2_run.json`, `rob2_replies.json` and
  `evidence_weight.json` must not change, and their contract tests pass
  untouched.
- A failure is never a score: a failed call is `DomainResult::Failed` /
  `DomainFailed` or an escalation failure, never a default answer. A stop is
  `Rob2RunError::Stopped` and returns nothing partial from that step.
- Never cut the paper: over the budget it is condensed for the domain's
  query, or the step fails as a condensation failure.
- Prompts stay byte-identical across languages; `ROB2_PROMPT_VERSION` (5)
  does not change, since no prompt changes.
- bmlib #813 is **not** part of this PR.
- Gates before every commit (bmlib's): from the worktree root `uv run pytest
  tests/ -q`, `uvx ruff@0.15.20 check .`, `uvx ruff@0.15.20 format --check
  .`, `uv run mypy`; from `rust/`: `cargo test`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo fmt --check`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. Use
  `set -o pipefail` in any chained gate command.
- Doc comments a junior developer can follow on every new item; no magic
  numbers.

## Review Focus

1. **A step run against digests from another call for the same paper** (a
   Continue in StudyAnalyzer): a fresh `Rob2Digests` must condense again,
   not fail. Pinned in Task 2 (`a_fresh_digests_condenses_again`).
2. **A condensation that fails once is reused as a failure for that
   domain's later outcomes**, as `assess` does today, without a second
   request. Pinned in Task 2 (`a_failed_digest_fails_every_outcome_once`).
3. **The escalation asked for an outcome whose D1 failed**: not needed,
   never asked, and the overall is `None`. Pinned in Task 3
   (`escalation_is_not_needed_when_a_domain_failed`).
4. **A stop during D5's second looks** ends that step with `Stopped` and
   keeps nothing of it (today's rule). Pinned in Task 2
   (`a_stop_during_a_second_look_stops_the_step`).
5. **The step driver and `assess` drifting apart later**: the contract test
   drives the steps over `evidence_weight.json`'s assessed case and must equal
   its expected run. Pinned in Task 4.

---

### Task 1: The plan of steps, and the fixture that pins it

**Files:**
- Modify: `rust/bmlib/src/quality/rob2_agent.rs` (new items near
  `Rob2RunError`), `rust/bmlib/src/quality/mod.rs` (re-exports)
- Modify: `bmlib/quality/rob2_agent.py`, `bmlib/quality/__init__.py`
  (`__all__`)
- Modify: `scripts/make_rob2_fixtures.py` (new `build_steps`)
- Create: `tests/fixtures/rob2_steps.json` (written by the script)
- Test: `tests/test_rob2_contract.py`, `rust/bmlib/tests/rob2_contract.rs`

**Interfaces:**
- Produces (Rust): `pub enum Rob2Step { Domain { domain: Domain, outcome:
  Option<usize> }, Escalation { outcome: usize } }` with `pub fn to_json(&self)
  -> Value`; `pub fn plan_rob2(outcome_count: usize) -> Vec<Rob2Step>`.
- Produces (Python): `@dataclass(frozen=True) class Rob2Step: kind:
  Literal["domain", "escalation"]; domain: Domain | None; outcome: int |
  None` with `to_dict() -> dict[str, Any]`; `plan_rob2(outcome_count: int) ->
  list[Rob2Step]`.
- Wire form: `{"kind": "domain", "domain": "D1", "outcome": null}`,
  `{"kind": "domain", "domain": "D3", "outcome": 0}`, `{"kind":
  "escalation", "domain": null, "outcome": 0}`. `outcome` is the index into
  the outcomes list the caller passes.

- [ ] **Step 0: Make the worktree and record the baseline** (Global
  Constraints' command). Run both test suites and note the passed counts.

- [ ] **Step 1: Write the failing tests**

Python, in `tests/test_rob2_agent.py`:

```python
def test_the_plan_is_d1_then_each_outcomes_domains_and_escalation() -> None:
    plan = plan_rob2(2)
    assert [s.to_dict() for s in plan][:6] == [
        {"kind": "domain", "domain": "D1", "outcome": None},
        {"kind": "domain", "domain": "D2", "outcome": 0},
        {"kind": "domain", "domain": "D3", "outcome": 0},
        {"kind": "domain", "domain": "D4", "outcome": 0},
        {"kind": "domain", "domain": "D5", "outcome": 0},
        {"kind": "escalation", "domain": None, "outcome": 0},
    ]
    assert len(plan) == 1 + 2 * 5


def test_no_outcome_plans_nothing() -> None:
    assert plan_rob2(0) == []
```

Rust, in `rob2_agent.rs`'s tests: `the_plan_is_d1_then_each_outcomes_domains
_and_escalation` and `no_outcome_plans_nothing`, with the same values.

Contract: `build_steps()` in `scripts/make_rob2_fixtures.py` writes
`{"plans": [{"outcomes": n, "steps": [...]} for n in (1, 2)]}` (Task 5
adds the prompt sizes). The generator check in
`tests/test_rob2_contract.py` (its loop over `(name, build)` pairs) covers
the new file: add `("rob2_steps.json", make.build_steps)` to its list. Both
contract tests assert `plan_rob2(n)` equals each `steps`.

- [ ] **Step 2: Run them to see them fail**

Run: `uv run pytest tests/test_rob2_agent.py -k plan -q` and
`cargo test -p bmlib plan` (from `rust/`). Expected: fail, `plan_rob2` is
not defined.

- [ ] **Step 3: Implement `Rob2Step` and `plan_rob2` in both languages**,
  then run the script to write `tests/fixtures/rob2_steps.json`. Zero
  outcomes is an empty plan (no D1): `assess` refuses no outcomes before any
  request, and the plan must not promise a request `assess` would refuse.

- [ ] **Step 4: Run the tests and the contract tests**: pass.

- [ ] **Step 5: Commit** — `feat(quality): plan_rob2, the RoB 2 run as a
  plan of steps`.

### Task 2: One domain at a time, with the digests held by the caller

**Files:**
- Modify: `rust/bmlib/src/quality/rob2_agent.rs` (`Rob2Agent`, `Papers`)
- Modify: `bmlib/quality/rob2_agent.py` (`Rob2Agent`)
- Test: the same two modules' unit tests

**Interfaces:**
- Consumes: `Rob2Step` (Task 1).
- Produces (Rust): `#[derive(Default)] pub struct Rob2Digests` (wraps
  today's private `Papers`; fields private); `impl Rob2Agent { pub fn
  assess_domain(&mut self, domain: Domain, title: &str, text: &str, outcome:
  Option<&ProposedOutcome>, min_similarity: f64, digests: &mut Rob2Digests) ->
  Result<DomainResult, Rob2RunError> }`.
- Produces (Python): `class Rob2Digests` (no public attributes);
  `Rob2Agent.assess_domain(self, domain: Domain, title: str | None, text: str
  | None, outcome: ProposedOutcome | None, *, min_similarity: float, digests:
  Rob2Digests) -> DomainResult`.
- Refusals, before any request, as `assess` makes them: an invalid
  similarity (`InvalidSimilarity`), an empty text (`EmptyInput`), D1 given
  an outcome or D2 to D5 given none (a new `Rob2RunError::WrongOutcome` /
  `Rob2RunErrorKind.WRONG_OUTCOME`, message "D1 is assessed for the trial,
  D2-D5 for an outcome").
- Python also runs `self._check_provider()` first, as `assess` does.

- [ ] **Step 1: Write the failing tests** (each language, with the module's
  scripted chat and condenser helpers):

  - `assess_domain_asks_one_domain`: D3 for an outcome over the short test
    text sends exactly one request, whose prompt equals
    `domain_prompt(D3, title, text, outcome)`, and returns the same
    `DomainResult` `assess` gives for that domain on the same reply.
  - `a_digest_is_reused_for_the_same_domain`: over a long text, D2 for
    outcome 0 then D2 for outcome 1 with one `Rob2Digests` condense once.
  - `a_fresh_digests_condenses_again`: the same two calls with a new
    `Rob2Digests` each condense twice.
  - `a_failed_digest_fails_every_outcome_once`: a condenser that fails gives
    `Failed` for D2 of both outcomes with one condensation attempt.
  - `a_stop_during_a_second_look_stops_the_step`: D5 whose facts name a
    measurement without a result, with the stop set when the second look
    is asked, returns `Stopped`.
  - `the_wrong_outcome_is_refused`: D1 with an outcome, and D4 without one,
    are `WrongOutcome` with no request made.

- [ ] **Step 2: Run them to see them fail**: `assess_domain` is not
  defined.

- [ ] **Step 3: Implement `Rob2Digests` and `assess_domain`**: the checks
  above, then today's private `domain` (Rust) / `_domain` (Python) with the
  caller's digests in place of the run-local map. Doc comment: a
  `Rob2Digests` holds one paper's condensations and must not be shared
  between papers; a new one condenses again.

- [ ] **Step 4: Run the new and the existing RoB 2 tests**: pass.

- [ ] **Step 5: Commit** — `feat(quality): Rob2Agent::assess_domain, one
  domain per call`.

### Task 3: The escalation and the outcome, apart; `assess` as a loop

**Files:**
- Modify: `rust/bmlib/src/quality/rob2_agent.rs`, `mod.rs`
- Modify: `bmlib/quality/rob2_agent.py`, `bmlib/quality/__init__.py`
- Test: the same modules' unit tests

**Interfaces:**
- Consumes: `plan_rob2`, `assess_domain`, `Rob2Digests` (Tasks 1, 2).
- Produces (Rust): `pub enum Rob2Escalation { NotNeeded, Asked(Escalation),
  Failed { reason: String, retryable: bool, status: Option<u16> } }`;
  `impl Rob2Agent { pub fn escalate_outcome(&mut self, title: &str, outcome:
  &ProposedOutcome, study: &DomainResult, domains: &[DomainResult]) ->
  Result<Rob2Escalation, Rob2RunError> }`; `pub fn combine_outcome(outcome:
  &ProposedOutcome, study: &DomainResult, domains: Vec<DomainResult>,
  escalation: Rob2Escalation) -> OutcomeRob2`.
- Produces (Python): `@dataclass(frozen=True) class EscalationFailed: reason:
  str; retryable: bool; status: int | None`; `Rob2Agent.escalate_outcome(self,
  title: str | None, outcome: ProposedOutcome, study: DomainResult, domains:
  Sequence[DomainResult]) -> Escalation | EscalationFailed | None` (`None`:
  not needed); `combine_outcome(outcome: ProposedOutcome, study:
  DomainResult, domains: Sequence[DomainResult], escalation: Escalation |
  EscalationFailed | None) -> OutcomeRob2`.
- `domains` is D2, D3, D4, D5 in that order; another length or order is a
  programming error (Rust `assert!`, Python `ValueError`), as `rob2_overall`
  already assumes.

- [ ] **Step 1: Write the failing tests**:

  - `escalation_is_asked_when_the_rule_needs_it`: domains judged so that
    `needs_escalation_decision` holds send one request whose prompt equals
    `escalation_message(...)`, and give `Asked`.
  - `escalation_is_not_needed_when_a_domain_failed`: D1 `Failed` gives
    `NotNeeded` with no request; `combine_outcome` then has `overall` `None`.
  - `a_failed_escalation_is_kept_with_its_reason`: a request failure gives
    `Failed` with its reason, `retryable` and `status`, and
    `combine_outcome` copies them into `escalation_error`,
    `escalation_retryable` and `escalation_status`.
  - `the_steps_give_assess_s_run`: for the module's existing
    two-outcome scenario, the run assembled from `assess_domain`,
    `escalate_outcome` and `combine_outcome` in `plan_rob2` order equals
    `assess`'s `Rob2Run` (same `to_json`).

- [ ] **Step 2: Run them to see them fail.**

- [ ] **Step 3: Implement `escalate_outcome` and `combine_outcome`** from
  today's private `outcome` / `_outcome` split in two; then rewrite
  `assess` as: its input checks, one `Rob2Digests`, and a loop over
  `plan_rob2(outcomes.len())` calling the three. Delete the private
  `domain`/`outcome` helpers it no longer needs.

- [ ] **Step 4: Run every RoB 2 and evidence-weight test, unit and
  contract, in both languages**: pass, with the existing fixtures unchanged
  (`git diff --stat tests/fixtures` shows only `rob2_steps.json`).

- [ ] **Step 5: Commit** — `refactor(quality): Rob2Agent::assess is a loop
  over its steps`.

### Task 4: The steps against the shared run

**Files:**
- Test: `rust/bmlib/tests/evidence_weight_contract.rs`,
  `tests/test_evidence_weight_contract.py`

**Interfaces:**
- Consumes: `DesignAgent::propose`, `OutcomeAgent::propose` (unchanged),
  and Tasks 1 to 3.

- [ ] **Step 1: Write the test** `the_assessed_case_step_by_step` in each
  language: over the fixture's case "assessed, with a ladder, a refused
  result and a failed domain", feed its `replies` in order to the scripted
  chat, run `propose` for the design and the outcomes, then the plan's steps
  one by one with one `Rob2Digests`, combine each outcome, and assert the
  assembled `Rob2Run`'s `to_json` equals `expected["run"]`.

- [ ] **Step 2: Run it**: it should pass at once (Task 3 made `assess` this
  loop). If it fails, the step API diverges from `assess`; fix the API, not
  the fixture.

- [ ] **Step 3: Commit** — `test(quality): the evidence-weight contract
  driven step by step`.

### Task 5: Prompt sizes, and condensing progress in Rust

**Files:**
- Modify: `rust/bmlib/src/quality/rob2_agent.rs`, `design_agent.rs`,
  `outcome_agent.rs`, `full_text_agent.rs`, `mod.rs`
- Modify: `bmlib/quality/rob2_agent.py`, `design_agent.py`,
  `outcome_agent.py`, `__init__.py`
- Modify: `scripts/make_rob2_fixtures.py`, `tests/fixtures/rob2_steps.json`
- Test: unit tests in each module; `rob2_contract.rs`,
  `test_rob2_contract.py`

**Interfaces:**
- Produces (both languages): `domain_prompt_chars(domain, outcome) ->
  usize/int` (outcome `None` for D1), `design_prompt_chars() ->
  usize/int`, `outcome_prompt_chars() -> usize/int`.
- Produces (Rust): `LlmQueryCondenser::with_progress(self, callback: impl
  Fn(&ProgressInfo) + Send + Sync + 'static) -> Self`, as
  `PrismaCondenser::with_progress` (Python's `LlmQueryCondenser` already
  takes `progress`).
- A measure counts what PRISMA's `part_prompt_chars` counts: the step's
  system prompt plus the longest of its question (as sent, with an empty
  title and paper) and `LlmQueryCondenser`'s two PICO condensation prompts
  for the step's query (`condense_query(domain)`, `DESIGN_CONDENSE_QUERY`,
  `OUTCOME_CONDENSE_QUERY`, with empty content), each plus `"\n\n"` and
  `JSON_CORRECTION_NOTE`. For D5 the question is the larger of the facts
  prompt and the second-look prompt for a measurement with an empty label
  and no passages, each with its own system prompt. Doc comment: a second
  look also carries its measurement's passages, which this does not count.

- [ ] **Step 1: Write the failing tests**:
  - `domain_prompt_chars_counts_the_sent_prompt`: for D3 and an outcome,
    equals the system prompt plus `domain_prompt(D3, "", "", outcome)` plus
    `"\n\n"` and the correction note (the PRISMA test's shape), and is at
    least the condensation prompts' measure.
  - `d5_counts_the_larger_of_facts_and_second_look`.
  - `design_prompt_chars_counts_the_sent_prompt`, and the same for outcomes.
  - Rust only, `the_query_condenser_reports_progress`: a condensation over a
    scripted `ContextModel` calls the callback at least once.
  - Contract: `build_steps` adds `"prompt_chars": {"design": …, "outcomes":
    …, "domains": {"D1": …, "D2": …, …}}` for the fixture's first outcome,
    and both contract tests assert each value.

- [ ] **Step 2: Run them to see them fail.**

- [ ] **Step 3: Implement the measures and `with_progress`**; regenerate
  `rob2_steps.json` with the script.

- [ ] **Step 4: Run the tests**: pass, Python and Rust giving the fixture's
  numbers.

- [ ] **Step 5: Commit** — `feat(quality): prompt sizes for every
  evidence-weight step; progress from Rust's query condenser`.

### Task 6: Docs, full gates, PR

**Files:**
- Modify: `docs/manual/quality.md` and `rust/bmlib/docs/manual/quality.md`
  (the evidence-weight sections #814 added: a *RoB 2 a step at a time*
  subsection with a short example driving the plan, doc-tested in Rust),
  `CHANGELOG.md` and `rust/bmlib/CHANGELOG.md` (`[Unreleased]` → *Added*),
  bmlib `HANDOVER.md` (this session's entry; keep it under 500 lines).

- [ ] **Step 1: Write the docs**; reflow edited paragraphs to the file's
  width; never split a backticked span.
- [ ] **Step 2: Run every gate** (Global Constraints). Expected: all pass;
  test counts are the baseline plus the new tests.
- [ ] **Step 3: Commit, push, open the PR** against bmlib `main`, titled
  `quality: RoB 2 a step at a time (StudyAnalyzer's Evidence weight tab)`,
  naming the StudyAnalyzer design and this plan, and saying that #813 is
  left for its own PR.
- [ ] **Step 4: Back in StudyAnalyzer**, once the PR is merged: plan 2 (the
  tab) starts with the re-pin to its merge commit.

## Changed while building

Recorded 2026-11-25 from bmlib PR #815 (merged as `58148480`) and its
follow-up #816 (branch `fix/rob2-steps-816`). Plan 2 uses these names.

- **Names:** `plan_rob2(outcome_count)` and `Rob2Step` (`Domain { domain,
  outcome }`, `Escalation { outcome }`; `from_json` / `from_dict` refuse a
  step no plan holds); `Rob2Agent::assess_domain(domain, title, text,
  outcome, min_similarity, &mut digests)` with `Rob2Digests`;
  `escalate_outcome(title, outcome, study, domains)` returning
  `Rob2Escalation { NotNeeded, Asked, Failed { reason, retryable, status } }`
  (Python `None | Escalation | EscalationFailed`); `combine_outcome(outcome,
  study, domains, escalation)`; `design_prompt_chars`, `outcome_prompt_chars`,
  `domain_prompt_chars(domain, outcome)`.
- **A caller's `Rob2Digests` keeps no failed condensation** (a retry
  condenses again; inside `assess` it is kept for the run), and starts again
  for another text or budget.
- **Errors, not panics, on caller data.** `combine_outcome` returns
  `Result<OutcomeRob2, String>`; after #816 `escalate_outcome` returns
  `Rob2RunError::InvalidDomains(message)` for a study that is not D1 or
  domains not D2–D5 in order. Both refuse a study that is not D1.
- **A stale escalation** is refused only where the need for the question
  changed; where both sets of domains call for it, it cannot be told, so the
  caller re-runs the escalation whenever one of its domains is re-run.
- **The stop is read before every domain**, so a run whose remaining steps
  send no request still ends `Stopped`.
- **From #816:** `evidence_weight::assess_outcome(start, study, outcome,
  rob2, settings)` builds an outcome's `AssessedOutcome` (ladder or
  `ladder_error`, bmlib's wording); `paper_weight` of the built ladders is
  the paper's. A failed D5 second look carries `second_look_retryable` and
  `second_look_status` on its `OtherMeasurement`.
