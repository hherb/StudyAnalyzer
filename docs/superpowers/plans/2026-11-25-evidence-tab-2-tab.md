# Evidence weight tab, plan 2: the tab (machine verdict first)

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** an *Evidence weight* tab beside *PICO* and *PRISMA*. It runs on its
own the first time it opens a trial: study design, primary outcomes, then
RoB 2 a domain at a time and each outcome's ladder. Every step is stored as
it finishes, and *Continue* runs only what is missing. The tab shows the
machine's verdict with highlighted quotes.

**Architecture:** bmlib (plan 1, merged as bmlib PR #815, with #817) asks the
questions, checks the quotes, condenses, judges and builds ladders. This
repository runs bmlib's steps in a stored plan. It keeps what the machine
said in its own tables, never in bmlib's wire form (design §6), and shows
it. The layout is PRISMA's: logic in `crates/studyanalyzer-core` (tested with
a scripted `JsonChat`), thin commands in `src-tauri`, Svelte 5 in `src/`.
Spans are code points in Rust and UTF-16 at the boundary.

**Tech Stack:** Rust 2021, Tauri 2, bmlib at `6819565a`, Svelte 5,
TypeScript, Vite, vitest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-24-evidence-weight-tab-design.md`
§2 and §5–§9, on the method `2026-11-09-evidence-weight-design.md` §4. The
pattern is plan `2026-11-20-prisma-2-tab.md` (read its *Changed while
building*) and the code it built: `prisma.rs`, `prisma_store.rs`,
`prisma_view.rs`, `PrismaTab.svelte`, `runTab.svelte.ts`. Plan 1's *Changed
while building* gives the bmlib names used below.

## Decisions this plan makes (design §8, and what §5 left open)

- **Steps:** a run starts with one pending `design` step. Saving the design
  adds the `outcomes` step, or ends the run as *not covered*. Saving the
  outcomes adds `plan_rob2(n)`'s steps, or ends the run as *no primary
  outcome*. So the driver runs only what is pending, as in PRISMA.
- **A failed step:** a design or outcomes failure ends the call. A failed
  domain or escalation ends the call only when it came from a request (the
  failure is `retryable`, or has an HTTP status): the next step would meet
  the same server. Any other failure (an unreadable reply, a failed
  condensation without a request fault) is stored, and the run goes on.
- **Budget:** one function measures the design prompt, the outcome prompt,
  D1's prompt and the D2–D5 prompts of every outcome known so far. The design
  and outcomes steps use it with no outcomes. The RoB 2 steps use it with the
  run's stored outcomes, so one `Rob2Digests` serves the whole RoB 2 stage.
  The design and outcome digests are not reused by the domains.
- **`first_shown_at`** is set by the `evidence` and `evidence_run` commands
  when they serve a run that has a done domain step, and only if it is still
  empty.
- **Continue on a run made under earlier rules** (any stored version, or the
  ladder settings, differing from the pinned bmlib's) is refused. Such a run
  is shown in full with *Re-run* only: rules must never mix within one run.
- **A step set back to pending loses its stored result** (a D5 with a failed
  second look, an escalation whose domains are re-run). It is replaced by
  the new attempt. A failed design, outcomes or domain step holds no
  result. A failed escalation keeps its outcome's row (overall not assessed,
  with the ladder error) until it is set back.
- **Stored results are read as text for the view** and turned into bmlib
  types only to hand them back to bmlib (an escalation, combining, the
  ladder) on a run with the current versions. So an old run's view never
  depends on today's rules.

## Global Constraints

- Every rule of PRISMA plan 2's Global Constraints that is not
  PRISMA-specific holds here too:
  - the gates (CLAUDE.md § Gates) before every commit
  - constants in `config.rs`, doc comments, and tests on outward-facing items
  - no truncation, and every error logged and shown
  - a citation shows `canonical.span_text`, never the model's copy
  - highlights: the current one `var(--hl-current)`, the others
    `var(--hl-past)`
  - the phone switch below `PHONE_BREAKPOINT_PX`
  - with no model chosen, a run is `needs_model`
  - a new command goes into `handler()`, `commands()`, `RETURN_TYPES`,
    `api.ts` and the vitest call list together
- **Never the abstract:** without full text the tab does not open (the
  Interrogator's rule, `interrogate::full_text`).
- **A failure is never a score:** a failed or pending domain reads *Not
  assessed*, its outcome has no number, and an incomplete run shows no paper
  weight.
- **No *Run anyway*:** a design other than a parallel-group RCT is *not
  covered*, with only *Re-run*.
- Each call goes through `with_backoff` with `RETRIEVAL_ATTEMPTS` (never
  after a 401 or 404, never once stopped). Inside the retried closure, a
  failure value marked `retryable` becomes a retryable error:
  `DomainResult::Failed`, `Rob2Escalation::Failed`, and a D5
  `OtherMeasurement` with `second_look_error` and `second_look_retryable`.
- **Store reads never panic:** a row the shape check refuses is
  `StoreError::Corrupt`, and so is any `Err` bmlib returns on rebuilt rows
  (`combine_outcome`'s `Err(String)`, `Rob2RunError::InvalidDomains`).
- Labels shown to the user, exactly:
  - judgements: `low` → *Low*, `some_concerns` → *Some concerns*, `high` →
    *High*; no judgement → *Not assessed*
  - answers: `Y` *Yes*, `PY` *Probably yes*, `PN` *Probably no*, `N` *No*,
    `NI` *No information*, `NA` *Not applicable*
  - ladder levels: `high` *High*, `moderate` *Moderate*, `low` *Low*,
    `very_low` *Very low*
  - D5 verdicts: `present` *Present*, `not_claimed` *Not claimed*,
    `no_quote_found` *Claimed, but no quote was found in the paper*,
    `quote_does_not_support` *Claimed, but the quotes found do not show it*,
    `no_plan_stated` *Quoted, but not stated as set in advance*
  - domains in the card: *D1 Randomisation*, *D2 Deviations*, *D3 Missing
    data*, *D4 Measurement*, *D5 Selection (facts)*
  - not covered: *No method for this design yet: {design}*
  - no primary outcome: *No primary outcome was found in the paper.*
  - earlier rules: *Made under earlier rules; Re-run for the current ones.*
  - condensed: *Judged from a condensed paper* (PRISMA's constant, imported)
  - a failed second look, on D5 and beside the outcome's number: *The second
    look for “{label}” failed: {error}. D5 may be more lenient than the paper
    warrants.*
  - paper weight: *Paper weight {w}*; incomplete: *Paper weight: not
    complete ({k} of {n} outcomes)*; complete with no ladder: *Paper weight:
    no outcome could be weighed*
- Progress labels (`EvidenceEvent::Step`), exactly: *Study design*,
  *Primary outcomes*, *The trial: D1, the randomisation process*, then
  *Outcome {k} of {n}: D2, deviations from the intended interventions* / *D3,
  missing outcome data* / *D4, measurement of the outcome* / *D5, selection of
  the reported result* / *the overall judgement*.
- Judgement chips carry their text, never colour alone. Style A
  ("Journal").

## Review Focus

1. **A stop or a request failure part-way through outcome 2.** Every step
   finished stays stored and shown, the rest read *Not assessed*, and
   *Continue* runs only those, on the run's own text and model. Tests: Task 3
   `stop_keeps_finished_steps`, `a_request_failure_ends_the_call`,
   `continue_runs_only_what_is_left`, `continue_uses_run_text`; Task 5
   `continueOffered`.
2. **A D5 second look that keeps failing.** D5 is stored with the failed
   look named. The note stands on D5 and beside the number, the run stays
   incomplete, and *Continue* re-runs D5 and that outcome's escalation.
   Tests: Task 3 `a_failed_second_look_is_kept_and_named`,
   `continue_reruns_a_d5_with_a_failed_look`; Task 4
   `a_failed_second_look_is_named_beside_the_number`.
3. **The step driver drifting from bmlib's whole run.** The same scripted
   replies through `assess_unattended` and through this driver must give
   the same overall judgements, ladders and paper weight. Test: Task 3
   `the_steps_match_assess_unattended`.
4. **A damaged or reordered stored row**, for example domains stored out of
   order, an escalation that no longer fits its domains, or an unknown
   question id. It is reported `Corrupt`, never a panic and never a silent
   re-judge. Tests: Task 2 `domains_read_back_in_order`,
   `a_damaged_row_is_corrupt`; Task 3 `a_stale_escalation_is_corrupt`.
5. **A run made under rules bmlib has since changed.** It is shown in full
   with the earlier-rules label, and *Continue* is neither offered nor
   accepted. Tests: Task 3 `continue_refuses_earlier_rules`; Task 4
   `earlier_rules_are_flagged`; Task 5 `continueOffered`.

---

### Task 1: The bmlib pin

**Files:**
- Modify: the root `Cargo.toml` (`rev`)

- [ ] **Step 1:** Set `rev` to `6819565a` (bmlib PR #817's merge). Between
  `406315c3` and it, bmlib changed only `quality/`, so no converter version
  moves and no stored text re-derives (checked 2026-11-25; confirm with `git
  -C ~/src/bmlib diff --stat 406315c3 6819565a -- rust/bmlib/src`).
- [ ] **Step 2:** Run every gate. Expected: PASS.
- [ ] **Step 3: Commit** `chore: bmlib to 6819565a (RoB 2 a step at a time)`.

### Task 2: Storage

**Files:**
- Modify: `crates/studyanalyzer-core/src/schema.rs` (migration 15; check for
  a later one first), `src/lib.rs`, `open.rs` (`has_dependents`)
- Create: `crates/studyanalyzer-core/src/evidence_store.rs` (runs and steps),
  `crates/studyanalyzer-core/src/evidence_rows.rs` (result rows: writing
  bmlib's results, reading them back as text, and the conversion back into
  bmlib types)

**Interfaces:**
- Produces, migration 15 *"Evidence weight runs, steps and results"*
  (foreign keys `ON DELETE CASCADE`, `created_at` as PRISMA's; every
  `*_json` is a JSON array, the contradicted ones as `prisma_items`' are;
  each quote group is `quotes_not_found_json`, `quotes_contradicted_json`,
  `quotes_dropped`, written "Q" below):
  - `evidence_runs(id, paper_id, canonical_id, model, design_prompt_version,
    outcome_prompt_version, rob2_prompt_version, algorithm_version,
    ladder_version, ladder_settings_json, status CHECK IN
    ('incomplete','complete'), kind CHECK (kind IS NULL OR kind IN
    ('not_covered','no_primary_outcome','assessed')), created_at,
    first_shown_at)`
  - `evidence_steps(id, run_id, position, kind CHECK IN
    ('design','outcomes','domain','escalation'), domain CHECK (NULL or
    'D1'..'D5'), outcome, status CHECK IN ('pending','done','failed'),
    condensed, error_message, error_retryable, error_status, UNIQUE (run_id,
    position))`
  - `evidence_designs(step_id PRIMARY KEY, design, rct_layout, reasoning,
    Q)`
  - `evidence_outcomes(id, step_id, position, name, timepoint, measure,
    scale, estimate, ci_low, ci_high, ci_level, n_analysed, result_error, Q,
    UNIQUE (step_id, position))`
  - `evidence_domains(step_id PRIMARY KEY, judgement, ignored_json,
    facts_outcome)`, with `facts_outcome` set exactly for D5
  - `evidence_answers(id, step_id, question_id, answer, reasoning, Q, UNIQUE
    (step_id, question_id))`
  - `evidence_d5_facts(id, step_id, fact, claimed, verdict, Q, UNIQUE
    (step_id, fact))`
  - `evidence_d5_measurements(id, step_id, position, label,
    second_look_asked, second_look_error, second_look_retryable,
    second_look_status, Q)`
  - `evidence_d5_analyses(step_id PRIMARY KEY, differs, Q)`
  - `evidence_outcome_results(step_id PRIMARY KEY` (the escalation step)`,
    escalation CHECK IN ('not_needed','asked','failed'), escalate,
    escalation_reasoning, overall, ladder_start, ladder_level, ladder_weight,
    ladder_error)`
  - `evidence_ladder_steps(id, step_id, position, reason, judgement, n,
    threshold, levels)`
  - `evidence_citations(id, step_id, owner CHECK IN
    ('design','outcome','answer','fact','measurement','analysis'), owner_id,
    role CHECK IN ('quote','named','result','second_look','planned',
    'reported'), position, span_start, span_end, similarity, exact)`.
    `owner_id` is the owning row's id (the step's for `design` and
    `analysis`). Spans only: the text is re-read from the canonical text.
- Produces, `evidence_store.rs`:
  - `pub enum EvidenceStep { Design, Outcomes, Rob2(Rob2Step) }`,
    `StepStatus { Pending, Done, Failed }`, `RunKind { NotCovered,
    NoPrimaryOutcome, Assessed }`; `prisma_store::RunStatus` is reused.
  - `pub struct StepFailure { message: String, retryable: bool, status:
    Option<u16> }`; `pub struct PendingStep { id: i64, step: EvidenceStep }`
  - `pub struct RunVersions { design_prompt: u32, outcome_prompt: u32,
    rob2_prompt: u32, algorithm: String, ladder: String, ladder_settings_json:
    String }` with `RunVersions::current()` (bmlib's constants and
    `LadderSettings::default()` as JSON)
  - `create_run(db, paper: &PaperId, canonical_id: i64, model: &str) ->
    Result<i64, StoreError>`: an `incomplete` run with the current versions
    and one pending `design` step
  - `pending_steps(db, run_id) -> Result<Vec<PendingStep>, StoreError>` (by
    position)
  - `save_design(db, step_id, &DesignProposal)`: in one transaction it
    stores the rows, marks the step done, then either adds the `outcomes`
    step or sets the run `not_covered` and `complete`. Covered is
    `assess_unattended`'s rule: `start_level(design)` is `Some` and
    `rct_layout.is_some_and(RctLayout::is_covered)`.
  - `save_outcomes(db, step_id, &OutcomeProposal)`: none means
    `no_primary_outcome` and `complete`. Otherwise `assessed`, with
    `plan_rob2(n)`'s steps added as pending.
  - `save_domain(db, step_id, &DomainResult)`: `Assessed` stores its rows
    (D5's facts included) and is done. `Failed` marks the step failed with
    its reason, `retryable` and `status`.
  - `save_escalation(db, step_id, &Rob2Escalation, &AssessedOutcome)`: the
    escalation and the outcome's overall judgement and ladder (or
    `ladder_error`). It is done unless the escalation failed (then failed,
    with its failure, the outcome row stored all the same).
  - `fail_step(db, step_id, &StepFailure)`
  - `reset_for_continue(db, run_id) -> Result<usize, StoreError>`: failed
    steps and D5 steps with a failed second look go back to pending, their
    results deleted. A domain set back sets its outcome's escalation back
    (D1 sets back every escalation). Returns how many steps went back.
  - `finish_run(db, run_id) -> Result<bool, StoreError>`: `complete` once
    the kind is set, no step is pending or failed, and no D5 has a failed
    second look
  - `mark_shown(db, run_id) -> Result<bool, StoreError>`: sets
    `first_shown_at` to now only while it is empty and a domain step is
    done
  - `run(db, run_id) -> Result<Option<StoredEvidenceRun>, StoreError>`,
    `latest_run`, `runs`, `run_text`, `has_citations(db, canonical_id)`,
    shaped as their `prisma_store` namesakes. `StoredEvidenceRun` carries
    the run row, its `steps` (each a `StoredStep { id, step: EvidenceStep,
    status, condensed: Option<bool>, failure: Option<StepFailure> }`) and
    `earlier_rules: bool` (its `RunVersions` differ from `current()`).
- Produces, `evidence_rows.rs` (`Stored*` structs mirror the tables above,
  enum columns kept as text; `StoredCitation { start, end, similarity,
  exact }`; `StoredQuotes { citations, not_found, contradicted, dropped }`):
  - `stored_design(db, run_id) -> Result<Option<StoredDesign>, StoreError>`
  - `stored_outcomes(db, run_id) -> Result<Vec<StoredOutcome>, StoreError>`
    (by position)
  - `stored_domain(db, step_id) -> Result<Option<StoredDomain>, StoreError>`
    (answers by question id; D5's `facts`, `measurements` by position,
    `analysis`)
  - `stored_outcome_result(db, step_id) ->
    Result<Option<StoredOutcomeResult>, StoreError>` (ladder steps by
    position)
  - the conversions, each `Result<_, StoreError>` with `Corrupt` naming the
    row: `proposed_outcome(&StoredOutcome, &CanonicalText) ->
    ProposedOutcome`, `domain_result(&StoredStep, Option<&StoredDomain>,
    &CanonicalText) -> DomainResult`, `rob2_inputs(db, run_id, outcome:
    usize, &CanonicalText) -> (ProposedOutcome, DomainResult, Vec<DomainResult>)`
    (D1, then D2–D5 read with an explicit `ORDER BY` on the domain),
    `study_design(&StoredDesign) -> StudyDesign` (round-tripped as
    `DesignProposal::from_json` does), `ladders(db, run_id) ->
    Vec<EvidenceLadder>`
- The shape check (in the readers): a done domain step has its
  `evidence_domains` row, with facts exactly for D5. Every `question_id` is
  in `questions(domain)`, so `question_text` never panics. The steps of an
  `assessed` run equal `[design, outcomes] + plan_rob2(n)` for its `n`
  stored outcomes (an outcome index ≥ `n` is corrupt). Every enum column
  holds a known spelling. A done escalation step has its outcome-result
  row.

- [ ] **Step 1: Write the failing tests** in the two modules:
  - `run_round_trips`: create a run, save a design, outcomes (two), D1, and
    outcome 0's D2–D5 and escalation. The readers return every row,
    citation and quote list.
  - `a_design_that_is_not_covered_completes_the_run`: crossover, then
    cohort.
  - `no_outcome_completes_the_run`
  - `the_outcomes_add_the_plan`: two outcomes, then the pending steps equal
    `plan_rob2(2)`.
  - `a_failed_domain_keeps_its_failure`
  - `reset_sets_back_a_domain_and_its_escalation`, `reset_of_d1_sets_back_every_escalation`,
    `reset_sets_back_a_d5_with_a_failed_look`
  - `finish_needs_every_step_done_and_no_failed_look`
  - `first_shown_is_set_once_and_only_after_a_domain`
  - `domains_read_back_in_order`: rows inserted D5, D3, D2, D4 give D2–D5.
  - `a_damaged_row_is_corrupt`: an unknown question id, a done domain with no
    row, a plan with a missing step. Each is `Corrupt`, none a panic.
  - `the_conversion_gives_bmlib_s_types`: a saved `DomainResult` (D5, with
    facts and a second look) and a `ProposedOutcome` come back equal to
    what was saved.
  - `deleting_paper_cascades`, `has_citations_counts_only_citations`
  - `text_with_evidence_citations_not_rederived`, in `open.rs`, as PRISMA's
    namesake
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** the migration, both modules, and `has_dependents
  = … || evidence_store::has_citations(…)`.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(evidence): store runs step by step`.

### Task 3: Running it

**Files:**
- Create: `crates/studyanalyzer-core/src/evidence.rs`
- Modify: `crates/studyanalyzer-core/src/config.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: Task 2; bmlib's `DesignAgent`, `OutcomeAgent`, `Rob2Agent`
  (`assess_domain`, `escalate_outcome`), `Rob2Digests`, `combine_outcome`,
  `evidence_weight::assess_outcome`, `start_level`, `LadderSettings`,
  `LlmQueryCondenser::with_progress`, `design_prompt_chars`,
  `outcome_prompt_chars`, `domain_prompt_chars`; ours:
  `interrogate::full_text`, `pico::{paper_budget_chars, is_set, PicoEvent}`,
  `retry::with_backoff`.
- Produces:
  - `config.rs`: `EVIDENCE_TEMPERATURE: f64 = PICO_TEMPERATURE`,
    `EVIDENCE_MIN_BUDGET_CHARS: usize = PICO_MIN_BUDGET_CHARS`.
  - `pub struct EvidenceContext<'a>`: `PrismaContext`'s fields, with
    `on_event: Arc<dyn Fn(EvidenceEvent) + Send + Sync>`.
  - `pub enum EvidenceEvent` (serde `kind`, snake_case): `Step { label:
    String }`, `Condensing { batch, batches, level }`, `Condensed`, `Other {
    stage, message }`, with `From<PicoEvent>` as PRISMA's.
  - `pub fn step_label(step: &EvidenceStep, outcomes: usize) -> String` (the
    Global Constraints' wording; the outcome shown from 1).
  - `pub enum EvidenceRunError`: `NoFullText`, `NotFullText`, `TooLong {
    window }`, `Cancelled`, `EarlierRules`, `Proposal { step: &'static str,
    error: ProposalError }`, `Request { label: String, message: String,
    retryable: bool, status: Option<u16> }`, `Store(StoreError)`,
    `Internal(String)`. Its `Display` gives the refusals PICO shares as PICO
    words them, *The evidence weight assessment was stopped.*, the
    earlier-rules label, `"{step}: {error}"` and `"{label}: {message}"`.
  - `pub fn budget_chars(window: Option<u64>, reply: u64, title: &str,
    outcomes: &[ProposedOutcome]) -> Result<Option<usize>, EvidenceRunError>`:
    `paper_budget_chars` with the largest of `design_prompt_chars()`,
    `outcome_prompt_chars()`, `domain_prompt_chars(D1, None)` and
    `domain_prompt_chars(d, Some(o))` for D2–D5 and every `o`. An `Err` from
    bmlib is `Internal`; `None` means under `EVIDENCE_MIN_BUDGET_CHARS`.
  - `pub fn run_evidence(ctx: &mut EvidenceContext, paper: &PaperId, stop:
    &AtomicBool) -> Result<i64, EvidenceRunError>`: a new run on the latest
    text. The budget with no outcomes is checked before any call or row, and
    is `TooLong` if it fails.
  - `pub fn continue_evidence(ctx: &mut EvidenceContext, run_id: i64, stop:
    &AtomicBool) -> Result<i64, EvidenceRunError>`: a complete run is
    `Internal`, earlier rules are `EarlierRules`, a stop already set is
    `Cancelled` (all with nothing changed). Then `reset_for_continue`, and
    the pending steps run on the run's own text (the command passes the
    run's model).

- [ ] **Step 1: Write the failing tests**, with a scripted `JsonChat` and a
  fake condenser as in `prisma.rs`'s tests; reuse bmlib's fixture texts and
  replies where they fit:
  - The early ends:
    - `not_covered_ends_after_the_design`: a crossover trial; one request; the
      run is complete and `not_covered`.
    - `no_primary_outcome_ends_after_the_outcomes`
    - `too_long_window_is_refused`: nothing is stored.
  - The whole run:
    - `a_trial_runs_every_step_in_plan_order`
    - `the_steps_match_assess_unattended`: the same replies through
      `bmlib::quality::assess_unattended` give equal overall judgements,
      ladder levels and weights, and paper weight.
    - `events_name_each_step`: the labels in plan order, a step's label
      before its condensing events.
  - Failures and retries:
    - `an_unreadable_domain_fails_and_the_run_goes_on`: D3 fails, D4 and D5
      run, the escalation stores the `ladder_error`, the run is
      `incomplete`, and the result is `Ok`.
    - `a_request_failure_ends_the_call`: D3 gets 500 through every attempt.
      D3 is failed, D4 still pending, and the result is `Request`.
    - `a_retryable_failure_is_retried`, `stopped_is_not_retried`
    - `a_failed_second_look_is_kept_and_named`: the look fails retryably on
      every attempt (`RETRIEVAL_ATTEMPTS` facts calls). D5 is done with the
      failed look, and after the last step the run is `incomplete`.
    - `a_stale_escalation_is_corrupt`: an outcome-result row edited to
      escalate where the domains do not call for it is `Store(Corrupt)` on
      Continue.
  - Stop:
    - `stop_keeps_finished_steps`: a stop after outcome 1's D2 leaves D1 and
      D2 done and the rest pending. The run is `incomplete`, the result
      `Cancelled`.
    - `a_reply_during_stop_is_kept`
  - Continue:
    - `continue_runs_only_what_is_left`
    - `continue_reruns_a_failed_domain_and_its_escalation`,
      `continue_reruns_d1_and_every_escalation`,
      `continue_reruns_a_d5_with_a_failed_look`
    - `continue_uses_run_text`: a newer canonical text exists, and the quotes
      are located in the run's own.
    - `continue_refuses_earlier_rules`: `algorithm_version` edited in SQL
      gives `EarlierRules`, with no step changed.
    - `continue_when_stopped_keeps_failures`
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.** The loop, the one algorithm the tests leave
  open:

```text
drive(run, text, stop):
  digests = Rob2Digests::default()                  // one per call (a Continue condenses again)
  while let Some(step) = pending_steps(run).first():
    cancelled(stop)?;  emit Step { step_label }
    match step:
      Design   → budget(&[]); retried DesignAgent::propose → save_design | fail_step + Err(Proposal)
      Outcomes → budget(&[]); retried OutcomeAgent::propose → save_outcomes | fail_step + Err(Proposal)
      Domain   → outcomes = stored outcomes as bmlib's; budget(&outcomes);
                 retried assess_domain(.., &mut digests) → save_domain;
                 Failed from a request → Err(Request)
      Escalation(o) → (outcome, study, domains) = rob2_inputs(o);
                 retried escalate_outcome; rob2 = combine_outcome (Err → Corrupt);
                 assessed = assess_outcome(start_level(stored design), &study, &outcome, &rob2, &LadderSettings::default());
                 save_escalation; Failed from a request → Err(Request)
    if stop is set after a save: finish_run; Err(Cancelled)      // the reply is kept
  finish_run
```

  The agents are built per step from the context, as `with_agent` builds
  PRISMA's: one `LlmQueryCondenser` with progress turned into events,
  `EVIDENCE_TEMPERATURE`, the reply room as `max_tokens`, the step's budget
  as `with_condense_config`, and `with_stop(stop)`. `ProposalError::Stopped`,
  `Rob2RunError::Stopped`, or any failure once the stop is set, is
  `Cancelled`. The other
  `Rob2RunError`s are `Internal`, except `InvalidDomains`, which is
  `Store(Corrupt)`. Log as `run_prisma` does.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(evidence): run the evidence weight a step at
  a time, resumable`.

### Task 4: Views and the IPC contract

**Files:**
- Create: `crates/studyanalyzer-core/src/evidence_view.rs`
- Modify: `src/lib.rs`, `src-tauri/src/contract.rs`,
  `src/lib/fixtures/ipc_contract.json` (regenerated)

**Interfaces:**
- Produces, serde snake_case, citations as `PrismaCitationView`,
  contradicted quotes as `ContradictedQuoteView`:
  - `QuotesView { citations, quotes_not_found, quotes_contradicted,
    quotes_dropped }`
  - `DesignView { design, rct_layout, reasoning, quotes }`
  - `ResultView { measure, scale, estimate, ci_low, ci_high, ci_level,
    n_analysed }`
  - `AnswerView { question_id, question /* bmlib's question_text */, answer,
    reasoning, quotes }`
  - `FactView { fact, meaning /* fact_meaning */, claimed, verdict, quotes
    }`; `MeasurementView { label, named, result, second_look:
    Option<Vec<PrismaCitationView>>, second_look_error, quotes }`;
    `AnalysisView { differs, planned, reported, quotes }`
  - `DomainView { domain /* "D1"… */, name /* bmlib's domain_title */, status
    /* pending, done, failed */, error, judgement, condensed, answers,
    ignored, facts: Option<Vec<FactView>>, measurements, analysis }`
  - `LadderView { start, level, weight, steps: Vec<{reason, judgement, n,
    threshold, levels}> }`
  - `OutcomeView { index, name, timepoint, result, result_error, quotes,
    domains /* always D2–D5 */, escalation: Option<{escalate, reasoning}>,
    escalation_error, overall, ladder, ladder_error, second_look_failures:
    Vec<{label, error}> }`
  - `EvidenceRunView { id, canonical_id, model, created_at, first_shown_at,
    status, kind, earlier_rules, condensed /* any step */, design, outcomes,
    study: Option<DomainView> /* D1 */, assessed_outcomes, paper_weight,
    failed_steps: Vec<{label, error}>, has_rob2_result }`. `paper_weight` is
    bmlib's `paper_weight(&ladders(run))`, and `Some` only when the run is
    complete and `assessed`. `assessed_outcomes` counts outcomes with a
    ladder.
  - `EvidenceRunSummary { id, model, created_at, status, canonical_id }`,
    `EvidenceView { canonical_id, current, earlier }`
  - `pub fn evidence_view(db, paper)`, `pub fn evidence_run_view(db,
    run_id)`, as their PRISMA namesakes. They read rows as text only: an old
    run is never turned into bmlib's types.
  - Fixture keys: `evidence_view`, `evidence_run_assessed`,
    `evidence_run_not_covered`, and one per `EvidenceEvent` kind.

- [ ] **Step 1: Write the failing tests:**
  - `pending_domains_are_not_assessed`: two outcomes, each with D2–D5, the
    pending ones `pending` with no judgement.
  - `a_failed_domain_shows_its_error_and_no_number`
  - `paper_weight_only_when_complete`: incomplete gives `None` with
    `assessed_outcomes` 1 of 2; complete gives bmlib's `paper_weight`.
  - `a_failed_second_look_is_named_beside_the_number`
  - `earlier_rules_are_flagged`: the run still shows every domain.
  - `condensed_run_says_so`
  - `spans_are_utf16`: an astral character in the title before the cited
    span.
  - `earlier_run_uses_its_text`
  - `has_rob2_result_needs_a_done_domain`
  - The contract fixture is regenerated, and the frontend `SameKeys` lines
    are added.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the Rust gates and `pnpm run test:web`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(evidence): views and IPC contract`.

### Task 5: Commands and the tab

**Files:**
- Create: `src-tauri/src/evidence_commands.rs` (`commands.rs` is over 2100
  lines; `event_sink`, `stored_view` and `stop_slot` become `pub(crate)`),
  `src/lib/evidence.ts`, `src/lib/evidence.test.ts`,
  `src/components/EvidenceTab.svelte`, `src/components/EvidenceCard.svelte`
  (one outcome), `src/components/EvidenceDomain.svelte` (one domain, opening
  to its answers or D5's facts)
- Modify: `src-tauri/src/state.rs` (`evidence_stop: StopSlot`),
  `src-tauri/src/lib.rs`, `src-tauri/src/error.rs` (`from_evidence`),
  `src/lib/api.ts`, `src/components/Interrogator.svelte`

**Interfaces:**
- Commands, following PRISMA's four:
  - `evidence(paper_id) -> EvidenceView`
  - `evidence_run(run_id) -> EvidenceRunView`. Both call `mark_shown` for a
    served run with `has_rob2_result`, then serve it with `first_shown_at`
    as now stored.
  - `run_evidence(paper_id, continue_run: Option<i64>, on_event:
    Channel<EvidenceEvent>) -> EvidenceView`: Continue builds the client for
    that run's model. A second concurrent run is `invalid_input`, *An
    evidence weight assessment is already running.*
  - `stop_evidence() -> bool`
- `from_evidence(e, provider)`: a refusal status on `Request` or on
  `Proposal(RequestFailed)` goes through `from_refusal`. Kinds: `NoFullText`
  and `NotFullText` → `NotFullText`; `TooLong` and `EarlierRules` →
  `InvalidInput`; `Cancelled` → `Cancelled`; `Store` → `Storage`;
  `Internal` → `Internal`; `Proposal` and `Request` → `Llm`, with *The model
  could not assess the evidence weight ({provider}): {e}*.
- `evidence.ts`, each label map as the Global Constraints give it:
  - `JUDGEMENT_LABELS`, `ANSWER_LABELS`, `LEVEL_LABELS`, `VERDICT_LABELS`,
    `DOMAIN_LABELS`
  - `designLine(design, layout)`: `rct` with `parallel` gives *Randomised
    trial, parallel groups*; `cluster`, `crossover` and `other` give
    *Randomised trial, cluster* / *crossover* / *other layout*. Each other
    design gets its own label, for example `cohort_prospective` *Prospective
    cohort study*.
  - `resultLine(r)`: `{measure: "risk ratio", estimate: 0.7, ci_low: 0.55,
    ci_high: 0.89, ci_level: 0.95, n_analysed: 1200}` gives *risk ratio 0.7
    (95% CI 0.55 to 0.89), n 1200*. Missing parts are left out.
  - `ladderLine(l)`: *High → Moderate (risk of bias: Some concerns)*, the
    reasons joined by `; `: *the interval crosses no effect*, *small trial:
    {n} analysed, under {threshold}*, *no interval reported*, *number
    analysed not reported (no step)*.
  - `paperWeightLine(run)`, `notCoveredLine(design)`,
    `earlierRulesNote(run)`, `secondLookNote(failure)`
  - `runningNote(event, stopping)`, reusing PRISMA's condensing wording
  - `evidenceMarks(run, current)`, `shouldAutoRun(view, attempted,
    hasModel)`, `continueOffered(run, running)` (never for `earlier_rules`),
    and `continueCaption` imported from `prisma.ts`

- [ ] **Step 1: Write the failing vitest tests** for every `evidence.ts`
  export, among them:
  - every judgement, answer, level, verdict and design wire value in the
    contract fixture has a label
  - `shouldAutoRun` is false after an attempt, with no model, and when a run
    of the current text exists
  - `continueOffered` is true only for an incomplete run while none is
    running, and false for `earlier_rules`
  - `paperWeightLine` reads *not complete (1 of 2 outcomes)* for an
    incomplete run
  - the vitest call-list entries for the four commands
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands (Rust test
  `run_evidence_needs_a_model`, and `evidence_marks_first_shown_once`),
  `evidence.ts`, and the components.
  - The layout is design §7's sketch: the design line with its citation
    and the run caption, *runs ▾*; the paper-weight line; then one card per
    outcome. Each card holds the result, the RoB 2 overall, the ladder line,
    the number and the second-look notes, and the domains D1 (the trial's)
    to D5, collapsed. Then *Continue* or *Re-run*.
  - Not covered and no primary outcome show their line, the design's quotes,
    and *Re-run*.
  - The host adds `"evidence"` to the tab union, a *RunTab* with
    `names: { view: "evidence", run: "run evidence", stop: "stop evidence"
    }` and `reloadOnEnd: true`, and the phone switch *Evidence | Paper*.
- [ ] **Step 4: Run every gate.** Expected: PASS.
- [ ] **Step 5: Hand check** with `pnpm tauri dev` and a cloud model, on an
  open-access parallel RCT, a crossover trial and a cohort study:
  - the RCT runs on first open, showing each step's label
  - *Stop* during outcome 1's D3, then *Continue*, finishes the run
  - a citation click scrolls and highlights (current dark, others pale)
  - the crossover and the cohort study read *No method for this design
    yet*
  - the phone width shows the switch
  The hand check needs a window and a key. If no one can run it, list it in
  HANDOVER as owed.
- [ ] **Step 6: Commit** `feat(evidence): the Evidence weight tab`.

### Task 6: Docs

**Files:**
- Modify: `HANDOVER.md`, `ROADMAP.md`, `docs/DECISIONS.md`, the main design
  `2026-10-21-studyanalyzer-design.md` (its §2 non-goal line: the Evidence
  weight tab is built, see its design)

- [ ] **Step 1:** In DECISIONS, an *Evidence weight* section with this plan's
  *Decisions*, plus any that changed while building.
- [ ] **Step 2:** In ROADMAP, mark phase 2 of the tab as landed, and keep
  phase 3 onwards. In HANDOVER, record the state and the hand check owed.
  Write this plan's *Changed while building*.
- [ ] **Step 3:** Reflow the edited paragraphs, then commit `docs: the
  Evidence weight tab in HANDOVER, ROADMAP, DECISIONS and the design`.

## Changed while building

- **Continue checks the escalations first** (ruling R1). The driver calls
  `escalate_outcome` afresh, so a stale stored escalation would never reach
  `combine_outcome`. `continue_evidence` therefore rebuilds every done
  escalation of a current-rules run and combines it; an error, or an
  overall that differs from the stored one, is `Store(Corrupt)` before any
  reset. Test: `a_stale_escalation_is_corrupt`.
- **The paper weight comes from stored weights** (R2): the largest stored
  `ladder_weight`, only for a complete `assessed` run with every step done.
  The view never calls `ladders()`, so an old run's view is rule-free (and
  `ladders(db, run_id)` was not added, R4). A test checks it equals bmlib's
  `paper_weight` on a current run.
- **`prisma.ts` exports `CONDENSED_NOTE`** (R3), so the tab imports the
  constant instead of copying it.
- **Question ids are checked only on current-rules runs** (R6, after Task
  2's review): readers accept any non-empty id on an earlier-rules run, and
  the view gives bmlib's question text only for an id `questions(domain)`
  knows.
- **`run_evidence` also marks `first_shown_at`** (R7) on the view it
  returns, besides `evidence` and `evidence_run`: without it a run shown
  straight after finishing would stay unmarked.
- **`EvidenceQuotes.svelte` is a fifth component**, so the citation markup
  is written once. `continueCaption`, `continueHint` and `CaptionedRun` were
  loosened to structural types so the evidence run can use them; PRISMA and
  PICO behave as before.
- **The commands live in `src-tauri/src/evidence_commands.rs`**, not in an
  existing file. `prisma.rs`'s `title_of` became `pub(crate)`.
- **The TypeScript view types are in `api.ts`**, and the views check spans
  themselves (a citation without a span is `Corrupt`).
- **`evidence.rs`, `evidence_store.rs` and `evidence_rows.rs` are each over
  1700 lines** (tests included), not split, as `prisma_store.rs` is not.
- **The hand check was not run** (no window or key): it is owed in
  `HANDOVER.md`.
