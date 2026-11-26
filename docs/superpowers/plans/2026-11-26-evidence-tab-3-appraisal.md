# Evidence weight tab, plan 3: the reader's own appraisal

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** a reader appraises a trial blind in the Evidence weight tab. They
confirm the machine's proposed design and primary outcomes, answer every
applicable RoB 2 signalling question while the machine's run goes on
hidden on the confirmed outcomes, save, then compare their answers with the
machine's, with conformance shown for both.

**Architecture:** the appraisal lives in tables of its own (design §8,
approach A). Phase 2's tables gain one link: a *paired* run points at its
appraisal, pauses after its proposals, and takes its RoB 2 design and
outcomes from the appraisal's confirmed rows through one reader
(`evidence_rows::rob2_design` / `rob2_outcomes`). The reader's verdicts,
ladders and conformance are pure functions over bmlib's public RoB 2
algorithm (`appraisal.rs`). Logic in `crates/studyanalyzer-core`, thin
commands in `src-tauri`, Svelte 5 in `src/`. Spans are code points in Rust
and UTF-16 at the boundary.

**Tech Stack:** Rust 2021, Tauri 2, bmlib (re-pinned in Task 1), Svelte 5,
TypeScript, Vite, vitest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-26-evidence-appraisal-design.md`
(cited as *design*), on `2026-11-24-evidence-weight-tab-design.md` (TAB) and
`2026-11-09-evidence-weight-design.md` (EW). The pattern is plan
`2026-11-25-evidence-tab-2-tab.md` (read its *Changed while building*) and
the code it built: `evidence.rs`, `evidence_store.rs`, `evidence_rows.rs`,
`evidence_view.rs`, `src-tauri/src/evidence_commands.rs`, `EvidenceTab.svelte`.

## Decisions this plan makes (design §11, and what it left open)

- **One small bmlib PR after all** (Task 1). RoB 2 allows no NI on 3.2, and
  bmlib keeps that rule inside its private `checked`. The reader's screen
  must not offer a refused answer, and copying the rule here would break
  *bmlib first*, so bmlib gains `allowed_answers(question)` in Rust and
  Python with a contract fixture. Everything else uses bmlib as it is.
- **No new run status.** SQLite cannot change `evidence_runs`' `CHECK`
  without rebuilding the table. *Waiting for confirmation* is derived: a
  paired run whose outcomes step is done while its appraisal has no
  `confirmed_at`. It stays `incomplete` with no kind.
- **The paired run's basis.** `evidence_rows::rob2_design(db, run_id)` and
  `rob2_outcomes(db, run_id)` return the run's own rows for an unpaired run
  and the appraisal's confirmed design and outcomes (as `StoredDesign` /
  `StoredOutcome` with empty quotes and reasoning) for a paired one. Every
  RoB 2 reader goes through them: the driver's budget, `rob2_inputs`, the
  escalation's start level, `check_escalations`, the shape check and the
  view's outcome cards. `stored_design` / `stored_outcomes` keep returning
  the run's own proposals.
- **A re-paired run** (*Run the machine again on these outcomes*) has no
  design or outcomes step: it is created `assessed` with `plan_rob2(n)`'s
  steps at `FIRST_ROB2_POSITION`. The shape check accepts that layout for
  a paired run only.
- **Rules versions per version, not per appraisal** (refines design §7).
  Each version row stores `algorithm_version` and `ladder_version`. A
  frozen or closed version keeps its own; an editable one (the unsaved
  first record, or an open *after reveal*) is re-derived under the current
  rules by `refresh_editable` whenever the appraisal is read, and takes the
  current versions.
- **Derived values are written, not computed on read.** Every edit
  re-derives, in its transaction, the touched domain's verdict, every
  outcome's overall (a D1 edit touches all of them) and each reader ladder,
  and stores them. So a version made under earlier rules is shown as made.
  Conformance alone is computed on read (design §7).
- **Not applicable is not stored.** An answer row holds what the reader
  picked. Applicability is `applicable_questions` on read and on derive; an
  answer to an inapplicable question stays stored and is not counted
  (design §5). A row may hold only a mark or a sources tick, with no answer.
- **A mark is a span only**, as citations are: its text is re-read from the
  canonical text, which `has_dependents` keeps.
- **The reader's ladder** uses `LadderSettings::default()`, its
  `start_level(confirmed design)` and the confirmed `ReportedResult`. Its
  steps are stored as `ladder_steps_json`, shaped as `LadderView.steps`.
- **Confirm needs both proposals stored**: the paired run's outcomes step
  done. A design or outcomes step that failed is retried by *Continue*
  first.
- **Blind is decided at start** (`start_appraisal`): blind only if no run of
  the paper has `first_shown_at`. `reveal_now` and `save` set
  `revealed_at`; `reveal_now` before a save also sets `blind = 0`.
- **Hidden runs.** A paired run whose appraisal has no `revealed_at` is
  left out of `evidence_view`, `latest_run`, `runs`, and refused by
  `evidence_run` and phase 2's Continue. This holds for a set-aside
  appraisal too, so the next appraisal can still be blind. `mark_shown` is
  called on a paired run only at its appraisal's reveal.
- **The machine's progress during a blind appraisal** reaches the tab only
  as `AppraisalEvent::Progress { done, total }`. The command counts
  `EvidenceEvent::Step` events from the run's done-step count; condensing
  events are dropped.
- **Set aside is refused while the paired run is running** (*Stop the
  machine's assessment first.*). The tab's button presses Stop first.
- **One edit command.** All the reader's writes go through
  `edit_appraisal(appraisal_id, edit: AppraisalEdit)`, which returns the
  new `AppraisalView`.

## Global Constraints

- Every rule of plan 2's Global Constraints holds: the gates (CLAUDE.md §
  Gates) before every commit; constants in `config.rs`; doc comments and
  tests on outward-facing items; no truncation; every error logged and
  shown; a citation shows `canonical.span_text`; highlights `var(--hl-current)`
  and `var(--hl-past)`; the phone switch below `PHONE_BREAKPOINT_PX`; a new
  command goes into `handler()`, `commands()`, `RETURN_TYPES`, `api.ts` and
  the vitest call list together; store reads never panic (a refused row is
  `StoreError::Corrupt`); plan 2's label maps (judgements, answers, levels,
  verdicts, domains) are reused, not copied.
- **Nothing the machine judged about RoB 2 reaches the screen before the
  reveal** (design §2): no answer, domain or overall verdict, ladder or
  number of a hidden paired run, in any view, list, event, label or error.
  The design and outcome proposals are shown.
- **A frozen version is never written.** Any write to a version with
  `frozen_at` or `closed_at` set is `Corrupt`.
- **The webview is a trust boundary**: every `AppraisalEdit` and
  `ConfirmInput` field is validated in the core (a known domain, question
  id in `questions(domain)`, an outcome index in range, D1 with no outcome
  and D2–D5 with one, an answer in `allowed_answers`, a span inside the
  canonical text, a design and layout bmlib knows, a result that passes
  `ReportedResult::validate`, a note under `APPRAISAL_NOTE_MAX_CHARS`).
  A refusal is `StoreError::Invalid` → `invalid_input`, nothing written.
- Labels, exactly:
  - the choice: *Appraise it yourself first* / *Show the machine's verdict*;
    on a shown paper *Appraise it yourself*
  - the header: *Your appraisal (blind)* / *Your appraisal (not blind)*
  - steps: *1 Design*, *2 Outcomes*, *3 Your assessment*, *4 Compare*
  - *Show the machine's verdict now*, *Save and compare*, *Done revising*,
    *Set this appraisal aside*, *Run the machine again on these outcomes*
  - progress: *The machine's assessment: {done} of {total} steps*; failed:
    *The machine's assessment stopped: {reason}*; waiting on another
    paper's run: *The machine's assessment waits for another paper's run.*
  - a departure: *Your verdict departs from the rules: they give {label}.*;
    the machine's: *The machine's verdict departs from the rules: they give
    {label}. This is a defect; please report it.*
  - earlier rules: *Made under earlier rules.*; its conformance: *Not
    checked (earlier rules).*
  - the escalation question: *Several domains raise some concerns. Do they
    together lower confidence enough to judge the risk of bias high?*
  - sources: *I also used: protocol / registry / supplement*
  - set aside: *Set aside {date}*; not covered: plan 2's *No method for this
    design yet: {design}*
- Answer chips and verdicts carry text, never colour alone; the reader's
  mark is outlined (`outline: 2px solid var(--hl-current)`), not filled.
  Style A ("Journal").

## Review Focus

1. **A hidden verdict leaking before the reveal**: the phase 2 view, the
   run list, `evidence_run(id)` on a hidden run, a progress event, or a
   failed step's message. Tests: Task 3 `hidden_runs_are_left_out`,
   `evidence_run_refuses_a_hidden_run`; Task 5
   `progress_events_carry_counts_only`; Task 6 `noMachineVerdictBeforeReveal`.
2. **The reader changes an early answer that closes later questions**
   (1.1 → a value making 1.x inapplicable; 2.3 → NA-making). The derived
   verdict follows at once, the stale answer is kept but not counted, and it
   reappears if the question applies again. Tests: Task 4
   `an_inapplicable_answer_is_kept_and_not_counted`,
   `it_counts_again_when_it_applies_again`.
3. **NI on 3.2, and an answer to a question bmlib does not know**, sent by
   a tampered webview: refused, nothing written. Tests: Task 4
   `ni_on_3_2_is_refused`, `an_unknown_question_is_refused`; Task 6
   `noNiOn32`.
4. **A quit, a stop, or a request failure in the paired run mid-appraisal.**
   Every answer stays; reopening continues the run on its own; a failed step
   says so neutrally with *Continue*. Tests: Task 3
   `a_paired_run_continues_on_the_confirmed_outcomes`; Task 6
   `opening_continues_a_pending_paired_run`.
5. **A re-pin between a draft and its save.** The draft re-derives under
   the current rules; a frozen first record stays as made with *Not checked
   (earlier rules).*; a paired run under earlier rules offers only *Run the
   machine again on these outcomes*. Tests: Task 4
   `an_editable_version_rederives_under_current_rules`,
   `a_frozen_version_keeps_its_rules`; Task 3 `a_repaired_run_has_only_rob2_steps`.

---

### Task 1: bmlib's `allowed_answers`, and the re-pin

In `~/src/bmlib`, on its own branch, under bmlib's HANDOVER and
`docs/SESSION-RULES.md` (a worktree, never the shared checkout; run `uv run
mypy` before pushing).

**Files (bmlib):**
- Modify: `rust/bmlib/src/quality/rob2_algorithm.rs`,
  `bmlib/quality/rob2_algorithm.py`, `tests/fixtures/rob2_algorithm.json`
  (its generator script if it has one), both CHANGELOGs, both quality
  manuals
- Test: `rust/bmlib/tests/` (the `rob2_algorithm.json` contract test),
  `tests/test_rob2_algorithm.py`

**Interfaces:**
- Produces: Rust `pub fn allowed_answers(question: &str) -> &'static
  [Answer]` (all five in `Answer`'s order; `[Y, PY, PN, N]` for `"3.2"`;
  panics on an unknown id as `question_text` does). Python
  `allowed_answers(question: str) -> list[Answer]`, raising `Rob2Error`
  `UnknownQuestion`. `checked` uses it, so the rule lives once.

- [ ] **Step 1:** Add to the fixture an `allowed_answers` object: every id of
  `questions(d)` for D1–D5 to its list. Write the Rust and Python contract
  tests reading it. Run them: FAIL.
- [ ] **Step 2:** Implement both, with `checked` (Rust) and its Python twin
  refusing through `allowed_answers`. Run bmlib's Rust and Python gates:
  PASS. Commit, push, open the PR (*quality: `allowed_answers`, for a
  reader's RoB 2 form*). After it merges, return here.
- [ ] **Step 3:** In StudyAnalyzer, on branch `feat/evidence-appraisal`, set
  `rev` to the merge commit. Confirm no converter version moved (`git -C
  ~/src/bmlib diff --stat <old> <new> -- rust/bmlib/src`). Run every gate:
  PASS. Commit `chore: bmlib to <rev> (allowed_answers)`.

### Task 2: Storage

**Files:**
- Modify: `crates/studyanalyzer-core/src/schema.rs` (migration 17; check for
  a later one first), `src/lib.rs`, `open.rs` (`has_dependents`),
  `config.rs` (`APPRAISAL_NOTE_MAX_CHARS: usize = 20_000`)
- Create: `crates/studyanalyzer-core/src/appraisal_store.rs`

**Interfaces:**
- Produces, migration 17 *"Evidence weight appraisals"* (foreign keys `ON
  DELETE CASCADE`; times as `evidence_runs.created_at`):
  - `evidence_appraisals(id, paper_id, canonical_id, blind CHECK IN (0,1),
    started_at, confirmed_at, revealed_at, saved_at, set_aside_at, design,
    rct_layout)`, with `CREATE UNIQUE INDEX evidence_appraisals_open ON
    evidence_appraisals(paper_id) WHERE set_aside_at IS NULL`
  - `evidence_appraisal_outcomes(id, appraisal_id, position, from_proposed,
    name, timepoint, measure, scale, estimate, ci_low, ci_high, ci_level,
    n_analysed, UNIQUE (appraisal_id, position))`
  - `evidence_appraisal_versions(id, appraisal_id, number /* 0 = the first
    record */, algorithm_version, ladder_version, created_at, frozen_at,
    closed_at, UNIQUE (appraisal_id, number))`
  - `evidence_appraisal_answers(id, version_id, domain, outcome /* NULL for
    D1 */, question_id, answer CHECK (answer IS NULL OR answer IN
    ('Y','PY','PN','N','NI')), mark_start, mark_end, used_protocol,
    used_registry, used_supplement)`, unique on `(version_id, domain,
    IFNULL(outcome, -1), question_id)` by an expression index
  - `evidence_appraisal_domains(id, version_id, domain, outcome, derived,
    override, note)`, unique as the answers
  - `evidence_appraisal_overalls(id, version_id, outcome, escalate, derived,
    override, ladder_start, ladder_level, ladder_weight, ladder_error,
    ladder_steps_json, UNIQUE (version_id, outcome))`
  - `ALTER TABLE evidence_runs ADD COLUMN appraisal_id INTEGER REFERENCES
    evidence_appraisals(id) ON DELETE CASCADE`
- Produces, `appraisal_store.rs`:
  - `pub enum AppraisalState { Confirming, Answering, Saved, NotCovered }`
    (derived: no `confirmed_at`; confirmed, covered and no `saved_at`;
    saved; confirmed with a design that has no `start_level` or a layout
    that `RctLayout::is_covered` refuses)
  - `pub struct ConfirmInput { design: String, rct_layout: Option<String>,
    outcomes: Vec<ConfirmedOutcomeInput> }`, `pub struct
    ConfirmedOutcomeInput { from_proposed: Option<usize>, name: String,
    timepoint: Option<String>, result: Option<ReportedResult> }` (serde,
    snake_case)
  - `pub enum AppraisalEdit` (serde tag `kind`): `Answer { domain, outcome:
    Option<usize>, question_id, answer: Option<Answer> }`, `Mark { domain,
    outcome, question_id, span: Option<TextSpan> /* code points; the
    command converts the webview's UTF-16 span, Task 5 */ }`,
    `Sources { domain, outcome, question_id, protocol: bool, registry: bool,
    supplement: bool }`, `Override { domain, outcome, judgement:
    Option<Judgement> }`, `Escalate { outcome: usize, escalate: Option<bool>
    }`, `OverallOverride { outcome: usize, judgement: Option<Judgement> }`,
    `Note { domain, outcome, note: Option<String> }`
  - `start_appraisal(db, paper: &PaperId) -> Result<i64, StoreError>`: on
    the paper's latest canonical text; `blind` as the Decisions say; version
    0 under the current rules. An open appraisal already there is `Invalid`.
  - `confirm(db, appraisal_id, &ConfirmInput) -> Result<(), StoreError>`:
    validates, stores the design and outcomes, sets `confirmed_at`, and in
    the same transaction readies the latest paired run
    (`evidence_store::plan_paired`, Task 3). Refused: already confirmed, no
    paired run with its outcomes step done, no outcome, `from_proposed` out
    of range or repeated.
  - `edit(db, appraisal_id, &AppraisalEdit) -> Result<(), StoreError>`:
    writes to the editable version (version 0 unsaved; after a save, the
    open *after reveal* version, made by copying the latest version's rows
    when none is open), then re-derives as the Decisions say. Refused
    before `confirmed_at`, on a set-aside appraisal, and on `NotCovered`.
  - `save(db, appraisal_id)`: refused unless `complete` (Task 4); sets
    `frozen_at` on version 0, `saved_at`, and `revealed_at` if empty.
  - `reveal_now(db, appraisal_id)`: before a save only; `revealed_at`,
    `blind = 0`.
  - `close_revision(db, appraisal_id)`: sets `closed_at` on the open *after
    reveal* version; none open is `Invalid`.
  - `set_aside(db, appraisal_id)`: `set_aside_at`; already set is `Invalid`.
  - `refresh_editable(db, appraisal_id) -> Result<bool, StoreError>`:
    re-derives an editable version whose versions are not current.
  - `open_appraisal(db, paper) -> Result<Option<i64>, StoreError>`,
    `appraisal(db, id) -> Result<Option<StoredAppraisal>, StoreError>`
    (the row, confirmed outcomes by position, versions by number, each
    version's answers, domains and overalls), `has_marks(db, canonical_id)`.
  - The shape check (in the readers): at most one version 0; numbers
    consecutive; only the highest may be open; every `question_id` in
    `questions(domain)` on a version with current rules; D1 rows with no
    outcome and D2–D5 rows with an outcome under the confirmed count; marks
    inside the text; a frozen version complete.

- [ ] **Step 1: Write the failing tests** in `appraisal_store.rs`:
  - `start_is_blind_only_when_nothing_was_shown`: a paper with a run whose
    `first_shown_at` is set starts not blind.
  - `one_open_appraisal_per_paper`, `set_aside_allows_a_new_one`
  - `confirm_needs_the_proposals`, `confirm_validates_its_input` (an
    unknown design, no outcome, a bad CI, `from_proposed` repeated)
  - `edits_go_to_version_0_until_saved`
  - `an_edit_after_save_opens_after_reveal_1_copied_from_0`,
    `close_revision_then_edit_opens_after_reveal_2`
  - `a_frozen_version_is_never_written`
  - `reveal_now_marks_not_blind_and_freezes_nothing`
  - `a_damaged_row_is_corrupt`: two version 0 rows, an unknown question id,
    a D1 row with an outcome, a mark past the text's end
  - `deleting_paper_cascades`, `has_marks_counts_marks_only`,
    `text_with_appraisal_marks_not_rederived` (in `open.rs`)
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** the migration, the module (every write calls
  `fn rederive(tx, version_id) -> Result<(), StoreError>`, a no-op here
  that Task 4 fills in) and
  `has_dependents = … || appraisal_store::has_marks(…)`.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(appraisal): store the reader's appraisal`.

### Task 3: The paired run

**Files:**
- Modify: `crates/studyanalyzer-core/src/evidence_store.rs`,
  `evidence_rows.rs`, `evidence.rs`, `evidence_view.rs`

**Interfaces:**
- Consumes: Task 2's tables and `appraisal_store::appraisal`.
- Produces:
  - `create_run(db, paper, canonical_id, model, appraisal: Option<i64>)`
    (every caller updated); `create_repaired_run(db, appraisal_id, model)
    -> Result<i64, StoreError>`: confirmed and covered only.
  - `save_design` on a paired run always adds the outcomes step;
    `save_outcomes` on a paired run stores and stops (no kind, no plan).
  - `pub(crate) fn plan_paired(tx, run_id, design_covered: bool, n: usize)
    -> Result<(), StoreError>`: covered → `assessed` and `plan_rob2(n)`;
    else `not_covered` and complete.
  - `StoredEvidenceRun.appraisal: Option<i64>`; `pub fn is_hidden(db,
    run_id) -> Result<bool, StoreError>` (paired, appraisal not revealed).
  - `evidence_rows::rob2_design(db, run_id) -> Result<Option<StoredDesign>,
    StoreError>`, `rob2_outcomes(db, run_id) -> Result<Vec<StoredOutcome>,
    StoreError>`, as the Decisions say.
  - `evidence.rs`: `pub fn run_paired(ctx: &mut EvidenceContext,
    appraisal_id: i64, again: bool, stop: &AtomicBool) -> Result<i64,
    EvidenceRunError>`: with no paired run, creates one and drives it;
    `again` makes a re-paired run; otherwise continues the latest paired run
    as `continue_evidence` does (waiting or complete → `Ok` with nothing
    run; earlier rules → `EarlierRules`). `continue_evidence` refuses a
    hidden run (`Internal`).
  - `latest_run`, `runs` and `evidence_view` leave hidden runs out.

- [ ] **Step 1: Write the failing tests** (scripted `JsonChat` as plan 2's):
  - `a_paired_run_waits_after_its_proposals`: two requests, outcomes done,
    no pending step, no kind, `incomplete`.
  - `a_paired_run_waits_even_when_not_covered`: a crossover proposal still
    gets its outcomes step.
  - `confirm_plans_the_confirmed_outcomes`: the machine proposed two, the
    reader confirmed one edited and one added; the pending steps are
    `plan_rob2(2)`.
  - `confirming_a_design_not_covered_completes_the_run`
  - `a_paired_run_continues_on_the_confirmed_outcomes`: the D2 prompt names
    the confirmed outcome, and its ladder uses the confirmed CI.
  - `the_proposals_stay_as_made`: `stored_outcomes` still returns the
    machine's.
  - `a_repaired_run_has_only_rob2_steps`, and its shape reads back.
  - `hidden_runs_are_left_out`: `evidence_view`, `runs`, `latest_run`;
    after the reveal they are listed.
  - `evidence_run_refuses_a_hidden_run` (in `evidence_view.rs`'s reader)
  - `an_unpaired_run_is_unchanged`: plan 2's driver tests pass untouched.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement**, routing every RoB 2 reader through
  `rob2_design` / `rob2_outcomes`.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(appraisal): a paired run on the confirmed
  outcomes`.

### Task 4: The reader's verdicts and conformance

**Files:**
- Create: `crates/studyanalyzer-core/src/appraisal.rs` (pure functions)
- Modify: `appraisal_store.rs` (the derive Task 2 stubbed),
  `evidence_rows.rs` (`rater_record`)

**Interfaces:**
- Consumes: bmlib's `questions`, `applicable_questions`, `allowed_answers`,
  `domain_judgement`, `needs_escalation_decision`, `overall_judgement`,
  `start_level`, `build_ladder`, `LadderSettings::default`.
- Produces:
  - `pub fn derived(domain: Domain, answers: &Answers) -> Option<Judgement>`
    (`None` while an applicable question is unanswered; any other
    `Rob2Error` is the caller's `Invalid`)
  - `pub fn effective(derived: Option<Judgement>, override_:
    Option<Judgement>) -> Option<Judgement>` (the override, else the derived)
  - `pub struct OutcomeVerdict { needs_escalation: bool, derived_overall:
    Option<Judgement>, overall: Option<Judgement>, ladder:
    Option<EvidenceLadder>, ladder_error: Option<String> }` and `pub fn
    outcome_verdict(domains: [Option<Judgement>; 5] /* effective */,
    escalate: Option<bool>, overall_override: Option<Judgement>, start:
    Level, result: Option<&ReportedResult>) -> OutcomeVerdict`
  - `pub struct RaterRecord { domains: Vec<RaterDomain>, overalls:
    Vec<RaterOverall> }`, `RaterDomain { domain: Domain, outcome:
    Option<usize>, answers: Answers, verdict: Option<Judgement> /* effective
    */ }`, `RaterOverall { outcome: usize, escalate: Option<bool>, overall:
    Option<Judgement> }`: one rater's record, built from a reader's stored
    version (`appraisal_store::rater_record(&StoredVersion)`) or a machine
    run's stored rows (`evidence_rows::rater_record(db, run_id)`)
  - `pub fn complete(r: &RaterRecord, outcomes: usize) -> bool`: every
    applicable question answered in D1 and each outcome's D2–D5, and every
    needed escalation answered
  - `pub struct Departure { scope: DepartureScope /* Domain(Domain,
    Option<usize>) | Overall(usize) */, given: Judgement, rules: Judgement
    }`; `pub fn departures(r: &RaterRecord) -> Vec<Departure>`: a domain
    whose verdict differs from `domain_judgement` of its answers; an overall
    whose verdict differs from `overall_judgement` of that outcome's domain
    verdicts (D1 and its D2–D5, as recorded) and its escalation.

- [ ] **Step 1: Write the failing tests:**
  - `derived_follows_bmlib`: every case of bmlib's `rob2_algorithm.json`
    fixture.
  - `an_inapplicable_answer_is_kept_and_not_counted`,
    `it_counts_again_when_it_applies_again`
  - `ni_on_3_2_is_refused`, `an_unknown_question_is_refused` (through
    `appraisal_store::edit`, nothing written)
  - `an_override_feeds_the_overall_and_the_ladder`: D2 overridden to high
    gives overall high and the ladder from high.
  - `one_override_is_one_departure`: that override gives exactly one
    departure (D2), not a second at the overall.
  - `the_escalation_is_asked_only_when_needed`, and an overall with it
    undecided is `None`.
  - `d1_edits_rederive_every_outcome`
  - `the_machine_conforms_by_construction`: a stored current run gives no
    departure.
  - `an_editable_version_rederives_under_current_rules`,
    `a_frozen_version_keeps_its_rules` (versions edited in SQL)
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(appraisal): the reader's verdicts, ladders
  and conformance`.

### Task 5: The appraisal's view, its commands and the IPC contract

**Files:**
- Create: `crates/studyanalyzer-core/src/appraisal_view.rs`,
  `src-tauri/src/appraisal_commands.rs`
- Modify: `crates/studyanalyzer-core/src/evidence_view.rs` (`EvidenceView`
  gains `appraisal: Option<AppraisalSummary>` and `any_shown: bool`),
  `src-tauri/src/lib.rs`, `src-tauri/src/contract.rs`,
  `src/lib/fixtures/ipc_contract.json` (regenerated), `src/lib/api.ts`

**Interfaces:**
- Produces, serde snake_case, reusing plan 2's `DesignView`, `ResultView`,
  `QuotesView`, `LadderView`, `NamedErrorView`, `EvidenceRunView`:
  - `AppraisalSummary { id, state, blind, set_aside: bool }`
  - `QuestionView { question_id, headline /* question_text's first line
    */, details /* the rest */, applicable, allowed: Vec<String>, answer,
    mark: Option<Utf16Span-shaped span>, protocol, registry, supplement }`
  - `ReaderDomainView { domain, name, questions, derived, override_,
    verdict, note, departure: Option<DepartureView> }`
  - `ReaderOutcomeView { index, name, timepoint, result: Option<ResultView>,
    domains /* D2–D5 */, needs_escalation, escalate, derived_overall,
    override_overall, overall, ladder: Option<LadderView>, ladder_error,
    overall_departure }`
  - `VersionView { number, editable, frozen_at, closed_at, earlier_rules,
    complete, study /* D1 */, outcomes }`
  - `MachineView { run_id, done_steps, total_steps, running_elsewhere:
    bool, failed: Vec<NamedErrorView>, earlier_rules, too_long, revealed:
    Option<EvidenceRunView> /* Some only once revealed_at is set */,
    departures: Vec<DepartureView> /* empty unless revealed and current */ }`
  - `CompareView { outcomes: Vec<{ index, questions: Vec<{ domain,
    question_id, reader, machine, differs }> }> }`, `Some` only once
    revealed
  - `AppraisalView { id, canonical_id, state, blind, started_at,
    revealed_at, saved_at, set_aside_at, proposals: Option<{ design:
    DesignView, outcomes: Vec<{ name, timepoint, result, result_error,
    quotes }> }>, confirmed: Option<{ design, rct_layout, outcomes }>,
    versions: Vec<{ number, frozen_at, closed_at }>, version: VersionView,
    machine: MachineView, compare: Option<CompareView> }`
  - `pub fn appraisal_view(db, appraisal_id, version: Option<u32>,
    running: Option<i64> /* the run holding the stop slot */) ->
    Result<AppraisalView, StoreError>`; `version` `None` shows the editable
    version if there is one, else the first record; `Some(n)` shows version
    `n`.
  - `view.rs`: `pub fn span_from_utf16(offsets: &[usize], span: Utf16Span)
    -> Option<TextSpan>`, the inverse of `span_to_utf16` (`None` when an end
    falls inside a character or past the text); `edit_appraisal` converts a
    `Mark`'s span with it, `None` being `invalid_input`.
  - `pub enum AppraisalEvent { Progress { done: usize, total: usize } }`
    (serde `kind`)
- Commands, in `appraisal_commands.rs`, each returning `AppraisalView`:
  `appraisal(appraisal_id, version)`, `start_appraisal(paper_id)`,
  `confirm_appraisal(appraisal_id, input)`, `edit_appraisal(appraisal_id,
  edit)`, `save_appraisal`, `reveal_appraisal`, `close_revision`,
  `set_aside_appraisal` (refused while its run holds `evidence_stop`),
  `run_appraisal(appraisal_id, again: bool, on_event:
  Channel<AppraisalEvent>)` (claims `evidence_stop`; a busy slot is
  `invalid_input`, *An evidence weight assessment is already running.*;
  the run's model for a continue, the chosen model for a new or re-paired
  run). `appraisal` calls `refresh_editable`. `save_appraisal` and
  `reveal_appraisal` call `mark_shown` on the latest paired run. Stopping
  is plan 2's `stop_evidence`. `evidence_run` refuses a hidden run.

- [ ] **Step 1: Write the failing tests:**
  - `no_machine_verdict_before_reveal`: a hidden run with done domains
    gives a `MachineView` with `revealed` `None`, no departures, no
    `compare`, and its serialised JSON holds none of `low`,
    `some_concerns`, `high`, `"Y"`… (a check over the JSON text).
  - `progress_events_carry_counts_only`
  - `compare_marks_differences`: reader 2.1 Y, machine PN → `differs`.
  - `a_departure_is_shown_once`
  - `spans_are_utf16`: an astral character before a mark, both ways
    (`span_from_utf16` round-trips `span_to_utf16`; a span ending inside a
    surrogate pair is `None`).
  - `run_appraisal_needs_a_model`, `set_aside_is_refused_while_running`
  - the contract fixture regenerated with keys `appraisal_confirming`,
    `appraisal_answering`, `appraisal_saved`, `appraisal_event_progress`;
    the frontend `SameKeys` lines added.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement**, with `from_evidence` for `run_appraisal`'s
  errors and `from_store` for the others.
- [ ] **Step 4: Run** the Rust gates and `pnpm run test:web`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(appraisal): views, commands and IPC
  contract`.

### Task 6: The tab: choice, confirming, answering

**Files:**
- Create: `src/lib/appraisal.ts`, `src/lib/appraisal.test.ts`,
  `src/components/AppraisalPane.svelte` (the steps and the machine line),
  `src/components/AppraisalConfirm.svelte` (steps 1–2),
  `src/components/AppraisalDomain.svelte` (one domain's questions)
- Modify: `src/components/EvidenceTab.svelte`,
  `src/components/Interrogator.svelte`, `src/lib/evidence.ts` (`shouldAutoRun`
  now false while the choice is offered), `src/lib/api.ts`

**Interfaces:**
- `appraisal.ts`:
  - `tabMode(view: EvidenceView): "choice" | "appraisal" | "machine"`:
    `choice` when `!any_shown` and no open appraisal; `appraisal` when an
    open one exists; else `machine`.
  - `needsRun(a: AppraisalView, running: boolean): boolean`: no paired run;
    or the machine has pending or failed steps, current rules, not too
    long, not running, and not waiting for confirmation.
  - `machineLine(m: MachineView, event?: AppraisalEvent): string`, the
    Global Constraints' wording.
  - `answerChoices(q: QuestionView)`: `q.allowed` in Y, PY, PN, N, NI order,
    with plan 2's `ANSWER_LABELS`.
  - `outcomeDraft(p)`, `confirmInput(draft): ConfirmInput`, and
    `confirmProblems(draft): string[]` (no outcome, an empty name, a CI the
    wrong way round), checked before sending; the core checks again.

- [ ] **Step 1: Write the failing vitest tests:**
  - `tabMode` for each case, including a paper with an unshown run made
    before this phase (`choice`).
  - `needsRun`: false while waiting for confirmation and while running;
    `opening_continues_a_pending_paired_run`: true for a confirmed
    appraisal whose machine has pending steps after a quit.
  - `noMachineVerdictBeforeReveal`: rendering `AppraisalPane` on the
    `appraisal_answering` fixture shows no judgement label from the machine.
  - `noNiOn32`: the 3.2 chips lack *No information*.
  - `machineLine` for progress, a failure and another paper's run.
  - the vitest call-list entries for the nine commands.
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement.** `EvidenceTab` routes by `tabMode`. *Show the
  machine's verdict* is plan 2's auto-run path. In `machine` mode with no
  open appraisal, the tab also offers *Appraise it yourself* (a not-blind
  appraisal, design §3). *Appraise it yourself
  first* calls `start_appraisal`, then `run_appraisal`; the pane calls
  `run_appraisal` again after `confirm_appraisal` and whenever `needsRun`
  holds on open. Each answer chip, override, escalation answer, tick and
  note (on blur) is one `edit_appraisal`. Questions not applicable are
  greyed with their stored answer struck through. *Show the machine's
  verdict now* asks first, in the pane: *Your appraisal will no longer
  count as blind.* with *Show it* / *Keep answering*.
- [ ] **Step 4: Run every gate.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(appraisal): the choice, confirming and
  answering`.

### Task 7: Marking a sentence, compare, revisions, set aside

**Files:**
- Create: `src/components/AppraisalCompare.svelte`
- Modify: `src/components/PaperPane.svelte` (an optional `onSelect` prop
  and `outlined` marks), `src/lib/paper.ts` (selection → span),
  `src/lib/appraisal.ts`, `src/components/AppraisalPane.svelte`,
  `src/components/AppraisalDomain.svelte`

**Interfaces:**
- `paper.ts`: `selectionSpan(root: HTMLElement, view: PaperView, sel:
  Selection): Utf16Span | null`, from the rendered text nodes back to
  `view.text` offsets; a selection outside the paper, empty, or across two
  blocks is `null`, and the pane says *Select text inside one paragraph.*
- `PaperPane`: `onSelect?: (span: Utf16Span) => void`, called on mouseup
  and touchend with a non-null span; marks gain `kind: "past" | "current" |
  "outlined"`.
- `appraisal.ts`: `compareRows(c: CompareView, outcome)`, `departureLine(d,
  rater)`, `versionLabel(n)` (*First record*, *After reveal {n}*).

- [ ] **Step 1: Write the failing vitest tests:**
  - `selectionSpan` on a fixture `PaperView`: inside a paragraph, across a
    `<mark>`, with an astral character before it, across two blocks (null)
  - `compareRows` marks differences; `departureLine` for both raters
  - `versionLabel`
- [ ] **Step 2: Run** `pnpm run test:web`. Expected: FAIL.
- [ ] **Step 3: Implement.** A question's *Mark the sentence* arms the pane;
  the next selection becomes an `edit_appraisal` `Mark`. Compare shows,
  per outcome, the rows with differences marked, the machine's reasoning
  and quotes on demand (a click highlights in the paper), both verdicts,
  ladders and numbers, and the departures; the machine's D5 facts come
  from `machine.revealed` through plan 2's `EvidenceDomain`. *Done revising* closes the
  revision; the version picker shows the first record by default. *Set this
  appraisal aside* asks first in the pane, and presses Stop if the paired
  run is running. *Your appraisal | Machine runs* appears once revealed.
- [ ] **Step 4: Run every gate.** Expected: PASS.
- [ ] **Step 5: Hand check** with `pnpm tauri dev` and a cloud model, on an
  open-access parallel RCT:
  - first open offers the choice; *Appraise it yourself first* shows the
    proposals while the machine line counts
  - correct one outcome's CI, confirm; answer D1–D5 for each outcome with
    one override; nothing of the machine's verdict appears
  - quit mid-way, reopen: answers kept, the machine continues
  - *Save and compare*: differences marked, quotes highlight, the override's
    departure shown once
  - change an answer: *After reveal 1*; *Done revising*
  - set it aside, start again: still blind
  - mark a sentence; it shows outlined in compare
  - phone width shows the switch
  If no one can run it, list it in HANDOVER as owed.
- [ ] **Step 6: Commit** `feat(appraisal): marks, compare, revisions and
  set aside`.

### Task 8: Docs

**Files:**
- Modify: `HANDOVER.md`, `ROADMAP.md`, `docs/DECISIONS.md`, the main design
  `2026-10-21-studyanalyzer-design.md` if its non-goal line names the
  reader's appraisal

- [ ] **Step 1:** In DECISIONS, an *Evidence weight appraisal* section with
  this plan's *Decisions*, plus any that changed while building.
- [ ] **Step 2:** In ROADMAP, mark phase 3 of the tab as landed, keep phase 4
  (reviewers, export, import). In HANDOVER, record the state and the hand
  check owed. Write this plan's *Changed while building*.
- [ ] **Step 3:** Reflow the edited paragraphs, then commit `docs: the
  reader's appraisal in HANDOVER, ROADMAP and DECISIONS`.

## Changed while building

- **Pre-flight order rulings.** Task 2 implemented `plan_paired` (Task 3's
  consumer), testing a paired run by `create_run` plus an SQL
  `UPDATE evidence_runs SET appraisal_id`. Task 2 had private stubs
  `rederive` and `version_complete` (now `appraisal::complete`), filled in
  by Task 4. Task 5 added the
  `api.ts` wrappers and the vitest call-list entries together.
- **bmlib pin ruling.** The bmlib PR #821 is not merged by the builder:
  StudyAnalyzer pins its head `4f2a097f` meanwhile. OWED before merge:
  re-pin to #821's merge commit and recopy bmlib's
  `rob2_algorithm.json` fixture.
- `plan_paired` and `covered` live in `evidence_store`; readers split into
  `appraisal_rows.rs`, writers in `appraisal_store.rs`. `derived` returns
  `Result<Option<Judgement>, Rob2Error>`; `appraisal_view` takes
  `running_elsewhere: bool`; `AppraisalState`, `appraisal` and `has_marks`
  live in `appraisal_rows`.
- The finalcheck review: an unpaired run marked shown while a blind
  appraisal is open and not revealed clears `blind`; an appraisal whose
  summary reads but whose full read fails can be set aside; an answer the
  current rules no longer allow is shown struck through and not counted.
- Hidden-run neutrality needed three layers (errors, stored step failures,
  failed-step labels); the progress counter's hints are a known limit.
- Marking a run shown: the final review found a stopped or failed run left
  unmarked, so a later appraisal was falsely blind; fixed, rules in
  DECISIONS.
- A damaged appraisal row no longer locks the tab (`appraisal_error`);
  answers to unknown question ids are kept and flagged; compare's
  `differs` needs both answers; *Revise* and *Run the machine again*
  were added; notes save on a debounce.
- Open: #170 (two `cargo doc` failures), #171 (the review's follow-ups).
