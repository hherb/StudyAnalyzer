# The Evidence weight tab, machine verdict first: design

_2026-11-24. Status: approved by the maintainer 2026-11-25. Amends
[the evidence-weight design](2026-11-09-evidence-weight-design.md) (cited
below as EW), whose method (§4), RoB 2 rules (§4.2) and D5 from facts
(§4.6) stand unchanged. It changes the order of its phases (EW §11) and adds
one bmlib piece. The main design's rules (bmlib first, one canonical text,
verified citations, persist everything, sandbox) all apply._

## 1. What changes, and why

EW's phase 2 was the whole tab, built around the reader's own blinded
appraisal: confirm the study type, confirm the primary outcomes, answer the
22 signalling questions, then compare with the machine. The maintainer's
use now is different (2026-11-24): **open a trial and see the machine's
verdict quickly.** So the phases become:

1. bmlib pieces 1 to 5 (done: bmlib PR #743).
2. **The machine verdict** (this design): the tab runs the pipeline
   unattended and shows its result, as the PICO and PRISMA tabs do.
3. **The reader's appraisal** (EW §5.1 to §5.5, unchanged): confirming
   steps 1 and 2, the blinded answers, the reveal, compare, conformance.
4. Reviewers, export and import (EW's phase 3).
5. Later (EW's phase 4).

The calibration purpose of EW §1 stays. The tab keeps the data it needs
honest by recording when each verdict was first shown (§6), so a later
appraisal of the same paper can say that it was not blind.

## 2. Goals, non-goals, anti-goals

**Goals**

- An *Evidence weight* tab (EW's name) beside *PICO* and *PRISMA*. For a
  trial, it shows:
  - the study design found, with its quotes;
  - each primary outcome with its reported result, with quotes;
  - for each outcome, the five RoB 2 domains with their answers, reasoning
    and highlighted quotes; the overall judgement; the evidence ladder
    with its steps; and the number;
  - the paper's weight (the best outcome's number, EW §4.1).
- It runs on its own the first time the tab opens. Each step is stored as
  it finishes, and *Stop* keeps the finished steps. *Continue* runs only
  what is missing or failed. *Re-run* makes a new run. Every run is kept.
- The run-tab lifecycle PICO and PRISMA share (`runTab.svelte.ts`,
  `stop_slot`, `event_sink`, `stored_view`).

**Non-goals (this phase)**

- Correcting the machine's study type or outcomes. The reader can only
  *Re-run* (§8). Correction is phase 3, where the confirmed values become
  labels (EW §4.1).
- The reader's own answers, blinding, compare and conformance (phase 3).
- Changing the ladder's settings in the app. The bmlib defaults are used and
  stored with each run.
- Designs other than parallel-group RCTs (EW §4.2): they get "no method for
  this design yet".
- Export and import.

**Anti-goals** (EW §2 holds; these are the ones this phase can break)

- **A failure that looks like a score.** A domain that failed is *not
  assessed*, its outcome has no number, and an incomplete run shows no
  paper weight.
- **A quote that is not in the paper.** Every citation is located by
  `find_passage` at `MIN_QUOTE_SIMILARITY`. The paper's span is shown.
- **A trial score for a design without a tool.**
- **A verdict on the abstract.** Without full text the tab does not open
  (the Interrogator's rule).
- **Leniency.** Judge from published evidence only. Stricter than Cochrane
  is intended (the maintainer's standing preference).

## 3. Approaches (decided)

The machine run is about 2 + 4N calls for N outcomes, plus D5 second looks
and an escalation call per outcome where needed. On a slow local model that
is many minutes. bmlib's `Rob2Agent::assess` runs every domain in one call
without progress, and a stop throws the whole RoB 2 run away.

- **A. Use bmlib as it is, storing after each stage** (design, outcomes,
  RoB 2, ladder). No bmlib change. But the RoB 2 stage is most of the calls,
  shows no progress, and is lost whole on a stop or a failed request.
- **B. bmlib exposes RoB 2 a step at a time, as PRISMA does.** StudyAnalyzer
  stores each domain as it lands, shows which one is running, and
  *Continue* reruns only what is missing. This costs a bmlib PR first.
- **C. StudyAnalyzer drives the domains itself** from bmlib's public prompt
  and reply readers. No bmlib PR, but it copies bmlib's per-domain
  condensing and D5 second looks into this app, against *bmlib first*.

**Chosen: B** (maintainer, 2026-11-24). It reuses the pattern the codebase
already has for long runs, and losing a long run to one failed request is
the PRISMA design's reason for storing parts as they finish.

## 4. The bmlib piece: RoB 2 a step at a time

Rust and Python on par, in one bmlib PR with shared contract fixtures, under
bmlib's session rules.

- **A plan of steps** for a paper and its outcomes: D1 once; then for each
  outcome D2, D3, D4 and D5; then that outcome's escalation, which is asked
  only when RoB 2's rule needs it (`needs_escalation_decision`). The plan is
  data the caller can store, so *Continue* runs the same plan.
- **One call per step**:
  - *prepare* the paper for a domain: whole when it fits, else condensed with
    that domain's query. The digest is handed back for the caller to reuse
    for the same domain of later outcomes, as `assess` reuses it today.
  - *assess* a domain step for an outcome (D1 has none): a `DomainResult`,
    a failure being `DomainResult::Failed` as today. D5's step is its facts
    call plus its second looks.
  - *escalate* an outcome from its stored domains, and *combine* them into
    an `OutcomeRob2` with its overall judgement.
- **A size measure per domain prompt** (like PRISMA's `part_prompt_chars`),
  so the caller can size the paper's budget for the model's window.
- **Condensing progress**, as `PrismaCondenser::with_progress` gives.
- **`assess` and `assess_unattended` become loops over these steps.** Their
  results and the existing fixtures (`rob2_run.json`,
  `evidence_weight.json`) must not change. A new fixture pins the plan and
  one step at a time against the whole run.
- bmlib #813 (a stored ladder is not re-derived on read) is not part of this
  PR: the tab stores what the machine said in its own tables (§6) and never
  reads bmlib's wire form back. It stays bmlib's own issue.

The design and outcome steps already run on their own
(`DesignAgent::propose`, `OutcomeAgent::propose`) and need no change, apart
from condensing progress if they lack it.

## 5. Running it in StudyAnalyzer

`evidence.rs` in the core, on the pattern of `prisma.rs`:

1. **The design step.** If the design is not a parallel-group RCT, the run is
   complete as *not covered*, with the proposed design and its quotes. No
   *Run anyway*: RoB 2 on another design means nothing.
2. **The outcomes step.** If no primary outcome is found, the run is
   complete as *no primary outcome*.
3. **The plan** (§4) is stored with the run. Then each step runs in plan
   order and is stored the moment it finishes.
4. **Each outcome's ladder** is built in code once its domains and any
   escalation are done; a failed domain gives a `ladder_error`, never a
   number. The paper weight is the best ladder weight, and is shown only once
   the run is complete.

- **Budget:** the paper's budget is `budget_chars` as for PRISMA, with the
  longest of the design, outcome and domain prompts standing in for the
  longest part prompt. Over it, each step condenses for itself, and the run
  is marked condensed (EW §4.5).
- **Retries:** each call goes through `with_backoff` with
  `RETRIEVAL_ATTEMPTS`, never after a 401 or 404 and never once stopped, as
  in PRISMA.
- **Stop:** read before every call and every condensation. A step whose
  reply arrives as *Stop* is pressed is kept (PRISMA's rule). The run stays
  `incomplete`.
- **A D5 second look that failed** (*my call*). bmlib leaves its
  measurement *undecided* (EW §4.6), so a failed request can make D5 more
  lenient than the paper warrants. The step is stored as done, with the
  failed looks named. The tab shows them in a visible note on D5, and the run
  stays `incomplete` until the look succeeds.
- **Continue** first sets failed steps, and D5 steps with a failed second
  look, back to pending. Then it runs every pending step on the run's own
  text and model, with the stored plan. A digest is not stored: a Continue
  condenses again where a step needs it.
- **Progress events:** "Study design", "Primary outcomes", then "Outcome 1 of
  2: D3, missing outcome data" and the condensing events, through
  `event_sink`.

## 6. Storage

**A run is shown as it was made (decided, maintainer, 2026-11-24).**
bmlib's reader of a stored RoB 2 run refuses any other rules version, and it
re-judges every domain under the current rules. The D5 rules are still being
tuned, so storing bmlib's wire form would make every earlier run unreadable
at the first re-pin that changes a rule. Instead StudyAnalyzer keeps what the
machine said in its own tables, as PICO and PRISMA do: the answers,
judgements, ladders and quotes as they were made. A run made under other
rules or another ladder version than the pinned bmlib's is shown in full,
labelled "made under earlier rules; *Re-run* for the current ones". It is
never re-judged on read. (Rejected: showing an old run only as that label
with *Re-run*, which hides what the machine said; and having bmlib keep every
old rule version readable, which grows with each tuning round.)

One migration, on the pattern of the PRISMA tables:

- `evidence_runs`: paper, canonical text, model, the prompt and algorithm
  versions, the ladder version and settings, `status` (`complete`,
  `incomplete`), `kind` once known (`not_covered`, `no_primary_outcome`,
  `assessed`), whether any step was condensed, `created_at`, and
  **`first_shown_at`**.
- `evidence_steps`: run, position, kind (`design`, `outcomes`, `domain`,
  `escalation`), domain and outcome index where they apply, `status`
  (`pending`, `done`, `failed` with its error), and how its paper was sent.
- The steps' results in tables of their own: the proposed design and
  outcomes (with each outcome's result), each domain's answers with their
  reasoning, judgement, quotes not found, contradicted and dropped, D5's
  facts with their checks and second looks, the escalation and its
  reasoning, and each outcome's overall judgement and ladder (its steps,
  level, number, or its error) as built.
- The citations, spans only, as `pico_citations`. `open::has_dependents`
  counts them, so a text an evidence run points into is never re-derived
  under it.

A row the store's own check contradicts is read back as `Corrupt`, as in
`prisma_store`. That check is about the shape (for example, a done domain
step with no answers), never about whether the current rules agree.

**`first_shown_at`** (*my call*) is set the first time the tab shows the run
with any RoB 2 result in it, a domain's answers or judgement included, not
only an overall judgement. It is never changed afterwards. Phase 3 blinds the
reader at the level of single questions, so a half-finished run counts. The
study design and the outcomes do not count, because EW §4.1 has the reader
confirm those openly. Phase 3 reads it per paper: an appraisal saved after
any run of that paper was shown is not blind.

## 7. The tab

`EvidenceTab.svelte` and `EvidenceCard.svelte` with `src/lib/evidence.ts`,
in `Interrogator.svelte`'s host: *Interrogator | PICO | PRISMA | Evidence |
Original*. On phones, *Evidence | Paper* replaces the split.

```
┌ Evidence weight ─────────────────────────┐┌ Paper ──────────────┐
│ Randomised trial, parallel groups  [1]    ││                     │
│ DeepSeek flash · 24 Nov 2026 · runs ▾     ││ …were randomly      │
│                                           ││ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │
│ PAPER WEIGHT  not complete (1 of 2)       ││ ▓▓▓▓ (current)      │
│                                           ││                     │
│ ▾ HbA1c at 12 months                      ││                     │
│   −0.4 % (95 % CI −0.7 to −0.1), n 312 [2]││                     │
│   RoB 2: some concerns   Ladder: moderate ││                     │
│   high → moderate (risk of bias)          ││                     │
│   Number 0.6                              ││                     │
│   D1 Randomisation        low             ││                     │
│   D2 Deviations           low             ││                     │
│   D3 Missing data         some concerns ▸ ││                     │
│      3.1 PN  "Outcome data were …" [3]    ││                     │
│   D4 Measurement          low             ││                     │
│   D5 Selection (facts)    low ▸           ││                     │
│ ▸ Weight at 12 months    not assessed     ││                     │
│      D4 failed: the model did not answer  ││                     │
│                                           ││                     │
│ [Continue]                                ││                     │
└───────────────────────────────────────────┘└─────────────────────┘
```

- Citations highlight as in PICO and PRISMA: the current one `#e8b931`, the
  others `#f6e7b0`.
- A domain opens to its questions (bmlib's own wording, keyed to RoB 2's
  question numbers), each answer with its reasoning and citations. D5 opens
  to its facts, each with its quotes and the check it passed or failed, and
  to the answers they gave (EW §4.6).
- Judgements carry text as well as colour (accessibility). Style A
  ("Journal").
- A condensed run says so next to its numbers (EW §4.5).
- *Not covered* reads "No method for this design yet: ‹design›", with its
  quotes, and *Re-run*.

## 8. Left to the builder

- The names of the bmlib step functions and types, and of the tables and
  columns.
- The progress events' exact wording.
- How `first_shown_at` is set: by the view command that first serves a run
  with a RoB 2 result, or by a command the tab calls once it renders one.
- Whether a failed domain ends the call (PRISMA's way) or the run goes on to
  the next step (bmlib's `assess`); how a conditional escalation sits in the
  stored plan; whether a stop during D5's second looks keeps its facts call.
- Whether the design and outcome digests are reused by the domains when the
  same condensation would serve (a cost choice, not a behaviour).
- The component split in Svelte.
- The test strategy, following bmlib's shared fixtures and PRISMA's store
  tests.

**Known limit, accepted:** a trial the machine misreads as another design, or
whose outcomes it misses, can only be re-run in this phase. Correcting it is
phase 3.

## 9. Where the code goes

- **bmlib:** §4, one PR, then a re-pin here.
- **StudyAnalyzer:** `evidence.rs`, `evidence_store.rs`, the migration, the
  view functions, the commands (`evidence`, `evidence_run`,
  `run_evidence { continue_run }`, `stop_evidence`, with their own
  `StopSlot`), the IPC contract fixture, the tab and its tests, the
  `has_dependents` update, and HANDOVER, ROADMAP and the main design's
  non-goal line.

Two implementation plans: bmlib first, then the tab, with the bmlib pin
bumped between them.
