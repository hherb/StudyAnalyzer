# The PRISMA 2020 tab: design

_Status: approved by the maintainer 2026-11-20 (drafted 2026-11-18). Builds on the main design
(`2026-10-21-studyanalyzer-design.md`, §6.4 "The PICO tab", the pattern
every analysis tab follows)._

## 1. What it is, and why

A tab that appraises a **systematic review** against the PRISMA 2020
reporting checklist, so a reader can judge **how far the review is fit to be
weighed in a decision** (clinical, guideline or policy). It is an appraisal
aid for the reader, not an editor's checklist for authors.

PRISMA measures *reporting*, not conduct. But a review that does not report
its search, its risk-of-bias assessment or its synthesis method cannot be
trusted on those points. The tab therefore leads with those decision-critical
gaps (§5), not with a compliance percentage.

It is the second analysis tab, and is built on the PICO tab's pattern: the
same split (analysis left, paper right), verified citations that highlight the
paper's own spans, every run stored and re-runnable.

## 2. Goals, non-goals, anti-goals

**Goals**

- A suitability check: is this paper a systematic review (with or without a
  meta-analysis)? If not, the tab says so plainly and offers *Run anyway*,
  as PICO does.
- Every PRISMA 2020 checklist item **and sub-item**: 42 in all (§4). Each
  carries a judgement, a one- or two-sentence explanation, and the paper's
  own spans that back it.
- A **key gaps** panel at the top (§5): a fixed set of decision-critical
  items with their judgements. Below it is the checklist by section, with
  counts per section. There is no overall score.
- Runs on any model the user has chosen, whatever its window: the checklist
  is split into parts sized to the model (§6).
- Every run is stored, including a part-finished one, so work done is never
  lost (§7).

**Non-goals**

- The PRISMA 2020 *abstract* checklist (12 items) as separate rows. Item 2 is
  one row, judged against those elements (§4).
- The PRISMA extensions (PRISMA-ScR for scoping reviews, PRISMA-NMA, PRISMA-P
  for protocols, PRISMA-DTA). A scoping review or network meta-analysis may be
  run (§3), and the tab notes that a specialised extension exists. Assessing it
  against that extension is later work.
- Judging conduct: AMSTAR 2 and ROBIS are later tabs, if ever.
- Rendering the review's flow diagram or forest plots (figures are a non-goal
  app-wide).
- Comparing reviews or exporting the assessment (bmlibrarian's JSON and CSV
  export are not ported).

**Anti-goals: these would make it a failure even if it ran**

- **A single number that hides a fatal gap.** "85 %, Good" on a review with no
  search strategy.
- **A judgement the paper does not back.** "Reported" with no located quote.
  This breaks the evidence rule and the house citation rule.
- **A false "not reported" caused by the tool.** For example, the paper was cut
  short or a passage was never shown to the model. The text is never truncated,
  and a call that could not see the paper fails visibly.
- **Leniency.** As with RoB 2 (maintainer's standing preference), judge only
  from what is published. Stricter than a human rater is intended.

## 3. Suitability

- It runs on the abstract when the paper has one, else on the paper prepared
  as for the assessment. This is PICO's `run_pico` flow exactly.
- bmlibrarian's check sends `full_text[:2000]` and `text[:3000]`. That is not
  ported: it breaks the no-truncation rule.
- It answers whether this is a systematic review, whether it has a
  meta-analysis, the document type (for example "systematic review with
  meta-analysis", "scoping review", "narrative review", "RCT"), a rationale and
  a confidence.
- A suitable paper is a systematic review, with or without a meta-analysis.
- **Variants.** For a scoping review, network meta-analysis, umbrella review,
  rapid review or living review, the paper is suitable. The variant is stored,
  and the tab shows one line naming the specialised extension (where there is
  one) that would fit it better.
- **Not suitable** (narrative review, primary study, protocol, …): the tab shows
  the type and the rationale, and offers *Run anyway* (`force`, as PICO does).

## 4. The checklist

The source is the PRISMA 2020 statement and checklist (Page MJ et al.,
*BMJ* 2021;372:n71), published under CC BY 4.0. Item wording is the
checklist's own, with attribution in the module and in the tab's footer.
Before shipping, the builder confirms the licence on the PRISMA site.

42 rows in 7 sections:

| Section | Items |
|---|---|
| Title | 1 |
| Abstract | 2 |
| Introduction | 3 rationale, 4 objectives |
| Methods | 5 eligibility, 6 information sources, 7 search strategy, 8 selection process, 9 data collection, 10a data items (outcomes), 10b data items (other variables), 11 study risk of bias assessment, 12 effect measures, 13a–13f synthesis methods (eligibility for each synthesis, data preparation, tabulation and display, synthesis method, heterogeneity, sensitivity), 14 reporting bias assessment, 15 certainty assessment |
| Results | 16a study selection, 16b excluded studies, 17 study characteristics, 18 risk of bias in studies, 19 results of individual studies, 20a–20d results of syntheses (characteristics and risk of bias of contributing studies, results of each synthesis, heterogeneity, sensitivity), 21 reporting biases, 22 certainty of evidence |
| Discussion | 23a interpretation, 23b limitations of the evidence, 23c limitations of the review process, 23d implications |
| Other information | 24a registration, 24b protocol, 24c amendments, 25 support, 26 competing interests, 27 availability of data, code and other materials |

The catalogue lives in bmlib as data. It is a constant table in each language,
held equal by a shared fixture `prisma2020_checklist.json`. Each row has:

- id (`"13d"`), section, short name, the checklist wording
- guidance for the model: what counts as fully or partly reported. This is
  ported and corrected from bmlibrarian's prompt, which uses the 2009 item
  layout and must not be copied for items 23–27.
- `key` (§5)
- `may_be_not_applicable`: true only for items that can be inapplicable. The
  builder fixes the exact list from the PRISMA explanation and elaboration
  paper (*my call*, settled by the builder check):
  - **13a–13f and 20a–20d apply to every synthesis**, statistical or not (vote
    counting, SWiM, structured narrative). They are not applicable only when
    the review has **no synthesis at all**. A review without a meta-analysis
    still owes 13d and 20b, so they stay key gaps when unreported.
  - Only the statistical parts of an item (for example 13d's model and
    heterogeneity method) may be judged not applicable when there is no
    meta-analysis. The item as a whole may not.
- `reply_chars`: an estimate of one item's reply, used by the split (§6)

### 4.1 Judgements

Each row gets one of:

- **reported**: the paper reports everything the item asks.
- **partly reported**: some of it, with what is missing named.
- **not reported**.
- **reported elsewhere, not checked** (*decided*): the paper says where the
  item is reported, outside the text we hold, and the pointer is the backing
  quote. Examples: "full search strategy in Supplementary File 1", "protocol
  registered as PROSPERO CRD…", "excluded studies in Appendix 3". We cannot read
  supplements, registry records or protocols (the canonical text lists a
  supplement but does not hold its content), so the claim is neither accepted
  nor refused. Fetching supplements is on the roadmap (§11).
  - Allowed on any item. The guidance names it for the items where it is
    common: 7, 16b, 19, 24a–24c and 27.
  - A pointer that is vague ("available on request") is *not reported*, not
    this state. Item 27's guidance spells out that "on request" is a
    statement of availability, which is the item's own subject.
- **not applicable**: only where `may_be_not_applicable`, and only with a
  located quote that shows why (for example "a meta-analysis was not possible
  because of heterogeneity").

**Strict evidence:**

- *Reported* or *partly reported* needs at least one located quote.
- A model judgement above *not reported* with no located quote is stored as the
  model said, but shown as **"not reported: the model's claim could not be
  checked against the paper"**. It also counts as not reported in the key gaps
  and the section counts.
- Its unlocated quotes show as notes, as in PICO: not found, or contradicted by
  the paper.
- The same applies to an unbacked *not applicable* or *reported elsewhere*.
- Storing the model's word and the shown judgement separately keeps the
  downgrade visible and auditable. It is never silent.

**A judgement is never upgraded** (*my call*). PICO's `read_part` makes a
part reported when a quote beside `reported: false` is located (bmlib #577).
Here a *not reported* stays *not reported* even with a located quote, because
an upgrade would be leniency.

There is no per-item confidence. bmlibrarian has only an overall one, and a
backing quote does that job better here.

## 5. Key gaps (the verdict panel)

A fixed list in bmlib's catalogue (`key: true`), shown first, in this order:

| Concern | Items |
|---|---|
| Search | 7 search strategy |
| Risk of bias | 11 methods, 18 results |
| Synthesis | 13d method, 20b results |
| Reporting bias | 14 methods, 21 results |
| Certainty | 15 methods, 22 results |
| Registration | 24a |

- Each row shows the item, its judgement chip, and the explanation on
  expanding. Clicking it opens the item in the checklist below and highlights
  its first citation.
- Under the panel: one line counting the key items **not reported or only
  partly reported**, for example "4 of 10 key items are not or only partly
  reported". This is a count, not a score. When any key item is *reported
  elsewhere, not checked*, a second line says so: "2 key items rely on
  material outside the paper". Those items are neither gaps nor counted as
  reported.
- A key item judged *not reported* from a condensed paper is checked again
  first (§6.4), so a gap in this panel is never only the condenser's.
- A not-applicable key item (for example 14 and 21 in a review with no
  synthesis at all, §4) shows *not applicable* and does not count as a gap.

Below the panel is the full checklist by section. Each section heading shows
counts ("Methods: 9 reported · 4 partly · 2 not · 1 elsewhere · 1 n/a").
There is no percentage or category anywhere.

## 6. Running it: the checklist split to fit the model (approach A)

### 6.1 Approaches considered

- **A. Parts sized to the budget (chosen).** Items stay in checklist order and
  are packed into parts that fit the model's reply budget and window. Each part
  is one call that sees the whole paper, or a condensation aimed at that part's
  items. Cost: a splitter, and per-item reply estimates.
- **B. One call per sub-item, fed passages found by embedding.** This fits tiny
  windows but takes about 42 calls, needs the embedder (phones, §9 of the main
  design), and a passage retrieval misses becomes a false *not reported*: the
  worst failure for this tab. Rejected.
- **C. One call for the whole checklist.** 42 items with quotes overflow many
  models' reply limits, which is the split the maintainer asked for. Rejected.

### 6.2 The split

The split is a pure function in bmlib, in both languages, with a shared
fixture of cases.

- **Inputs:** the catalogue, the reply budget (StudyAnalyzer's
  `prompt::reply_tokens`, converted with `FIT_CHARS_PER_TOKEN`) and a per-part
  overhead.
- **Packing:** greedy, in checklist order. A part takes items while the sum of
  their `reply_chars` plus the overhead fits the reply budget. Each part is
  labelled by its first and last item, for example "Methods 5–13c".
- **A single item over the budget:** the run is refused before any call, with
  "the model's reply room is too small for PRISMA; choose another model". This
  is the PICO `TooLong` pattern.
- **A large-window model** (for example 200k tokens with a 32k reply) packs into
  very few parts. That is fine: the parts exist for the reply limit, not for
  their own sake.

### 6.3 What each part sees

- **The paper fits:** the paper's budget is as in PICO's `budget_chars`, with
  the longest part's prompt standing in for the longest PICO prompt. If the
  whole paper fits, every part gets the whole paper, identical across parts.
- **The paper does not fit:** each part condenses the paper for itself. The
  query is built from the guidance of that part's items, through bmlib's
  `LlmCondenser` with PRISMA condensation prompts, as PICO's does.
  - A failed batch fails the part (`continue_on_error(false)`). A part is
    never judged from a partial digest.
  - Quotes are always located in the paper, never in the digest.
  - Cost: a long paper with P parts is condensed P times. That is accepted,
    because one condensation for all 42 items would keep most of the paper and
    lose focus. *Builder's option:* if measurement shows that one shared
    condensation fits and judges as well, use it. That changes cost, not
    behaviour.

### 6.4 Calls and failures

- **Retries:** each part is one `chat_json` call, retried with backoff on
  retryable failures (`with_backoff`, `RETRIEVAL_ATTEMPTS`). Never after a 401
  or 404, and never once stopped.
- **A reply cut off at its length limit:** the part is split in half and each
  half retried, down to single items. This keeps a wrong `reply_chars` estimate
  from failing a run. Only a single item cut off fails the part, with PICO's
  "reply reached its length limit" message.
- **A reply that omits an item it was asked for:** retried once as a part of
  just the missing items. If it is still missing, the part fails, naming the
  item. Never a silent gap.
- **A reply with an item it was not asked for:** ignored, and logged as a
  diagnostic (house rule: surface mismatches while developing).
- **Stop:** honoured before each call and each condensation, as in PICO.

### 6.5 Re-checking a key gap judged from a condensation (*decided*)

A condensation can leave out the one sentence an item needs, for example a
PROSPERO number. A *not reported* from a condensed part may therefore be the
tool's fault. So:

- **Key items** (§5) judged *not reported* from a condensed part are checked
  again, one item at a time:
  - The paper is condensed once more, with a query built from that item's
    guidance alone.
  - The item is asked again on that digest.
  - The second answer is the judgement. If the item is still not found, it
    stands as *not reported*, marked "checked again on a condensation for this
    item".
  - This costs at most 10 extra condensations plus calls, and only on papers
    too long for the window.
- **Other items** judged from a condensed part keep their judgement, with a
  visible note: "judged from a condensed paper".
- A paper sent whole needs neither.
- The re-check is part of the run. It is stored with its item (§7) and resumed
  by Continue like any part.

## 7. Storage

The PICO tables are the pattern. One migration adds:

- `prisma_runs`: paper, canonical text, model, the suitability as JSON and
  whether it came from the abstract, `forced`, the variant, `status`
  (`complete`, `incomplete`) and `created_at`.
- `prisma_parts`: run, position, first and last item, how its paper was sent
  (`whole`, or condensed from N characters with the condensation status and
  notes) and `status` (`done`, `failed` with the error kind and message).
- `prisma_items`: part, item id, the model's judgement, the **shown**
  judgement (§4.1), the explanation, quotes not found, quotes contradicted
  (JSON, as `pico_parts`) and quotes dropped. It also records whether the
  item was judged from a condensation, and, for a re-checked key item (§6.5),
  the first judgement and how the re-check's paper was condensed.
- `prisma_citations`: item, position, span, similarity and exact match, as
  `pico_citations`.

Behaviour:

- **A part is stored as soon as it finishes.** A run that is stopped, or whose
  part fails after retries, stays `incomplete` with its finished parts. This
  follows the persist-everything rule: a 6-part run is several minutes of model
  time.
  - The tab shows an incomplete run's finished items, marks the missing ones
    "not assessed", and offers **Continue**. Continue runs only the missing
    parts, with the run's own model and the same split.
  - If the user has since chosen another model, Continue says it will use the
    run's model, and *Re-run* starts fresh.
  - An incomplete run's key-gaps line says "not complete" instead of a count.
- **Calls settled by the builder check** (*my calls*):
  - The part plan is stored when the run starts, so Continue uses the same
    split even if the model's limits have changed since.
  - A part whose reply arrives as Stop is pressed is kept. This differs from
    PICO, which drops it, and follows the persist-everything rule.
  - Continue runs against the run's own canonical text, even if a newer one
    exists.
- **Re-run** always makes a new run. Every run is kept and listed with model
  and date, as PICO.
- `open::has_dependents` must count PRISMA citations, as it counts PICO's, so a
  text a PRISMA run points into is not re-derived under it.

## 8. The tab

It sits in `Interrogator.svelte`'s host beside *PICO*: *Interrogator | PICO |
PRISMA | Original*. It follows PICO's lifecycle exactly: load on first open,
auto-run once if there is no run, a stop button, and run history. On phones,
*PRISMA | Paper* replaces the split.

```
┌ PRISMA 2020 ─────────────────────────────┐┌ Paper ──────────────┐
│ Systematic review with meta-analysis      ││                     │
│ Claude Sonnet · 18 Nov 2026 · runs ▾      ││ …we searched MED-   │
│                                           ││ LINE, Embase and    │
│ KEY GAPS   4 of 10 not or partly reported ││ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │
│  7  Search strategy        ● not reported ││ ▓▓▓▓▓▓▓▓ (current)  │
│ 11  Risk of bias methods   ● reported     ││                     │
│ 18  Risk of bias results   ◐ partly       ││                     │
│ 13d Synthesis method       ● reported     ││                     │
│ 20b Synthesis results      ● reported     ││                     │
│ 14  Reporting bias methods ○ not reported ││                     │
│ …                                         ││                     │
│                                           ││                     │
│ ▸ Title & abstract       2 ✓              ││                     │
│ ▾ Methods  9 ✓ · 4 ◐ · 2 ✗ · 1 n/a        ││                     │
│    7  Search strategy  ✗ not reported     ││                     │
│       "Only databases are named; no …"    ││                     │
│       ⚠ the model's claim could not be    ││                     │
│         checked against the paper         ││                     │
│ …                                         ││                     │
│ PRISMA 2020 © Page et al. 2021, CC BY 4.0 ││                     │
└───────────────────────────────────────────┘└─────────────────────┘
```

- **Progress while running:** "Part 2 of 5: Methods 5–13c", and PICO's
  condensing events ("condensing part 3 of 7").
- Citations work as in PICO: the current one is `#e8b931`, the item's others
  `#f6e7b0`.
- Styling is style A ("Journal"). Judgement chips need text as well as colour
  (accessibility).
- **Without full text** the tab does not open. This is the Interrogator's rule:
  never the abstract alone.

## 9. Where the code goes

**bmlib** (Rust `rust/bmlib/src/quality/`, Python `bmlib/quality/`, on par in
one PR, held together by shared fixtures):

- the catalogue (§4), the key list (§5), the prompts: system, suitability,
  part assessment, condensation
- the split (§6.2)
- the reply reader: an item's judgement, explanation and quotes, verified with
  the same quote location as PICO, and the shown judgement (§4.1)
- a `Prisma2020Agent` with `PicoAgent`'s builder shape (`with_stop`,
  `with_condenser`, `with_max_tokens`, …): `check_suitability`, `plan_parts`
  and `assess_part`. The caller drives the parts, so StudyAnalyzer can store
  each one as it finishes.
- **Port oracle:** bmlibrarian's tests pin the flow (the suitability shape and
  the validation of a reply's fields), not the checklist, which is new.

**StudyAnalyzer:**

- `prisma.rs`, on the pattern of `pico.rs`: budget, retries, stop, events, the
  truncation split, continue
- `prisma_store.rs`, on the pattern of `pico_store.rs`
- the migration
- the `view` functions
- the Tauri commands `prisma`, `prisma_run`, `run_prisma { force, continue_run }`
  and `stop_prisma`, with their own `StopSlot`
- the IPC contract fixture
- `PrismaTab.svelte`, `PrismaCard.svelte` and `src/lib/prisma.ts`, with tests
- updates to `has_dependents`, HANDOVER, ROADMAP and the main design's
  non-goal line

Two implementation plans: bmlib first, then StudyAnalyzer, with the bmlib pin
bumped between them.

## 10. Approaches decided in conversation

- **Purpose:** appraisal for decision-making (maintainer, 2026-11-18).
- **Verdict:** key gaps first, no overall score (option B over bmlibrarian's
  percentage).
- **Split:** approach A (§6.1).
- **Supplements:** the *reported elsewhere, not checked* state, with fetching
  supplements on the roadmap (option A over counting the pointer as reported,
  or fetching supplements now).
- **Condensed gaps:** re-check key items only, and note the rest (option A over
  a note only, or re-checking all 42).

## 11. Later

- **Fetching a review's supplements** (and, where open, its registry record),
  so *reported elsewhere, not checked* can be checked. This goes in ROADMAP
  beside the further analysis tabs.

## 12. Left to the builder

- each item's guidance text, `reply_chars` estimates, and the exact
  `may_be_not_applicable` list (from the PRISMA explanation and elaboration
  paper)
- the reply JSON shape
- whether `verify_quotes` and `PicoCitation` are generalised in bmlib or reused
  as they are
- the per-part prompt overhead constant
- the one-shared-condensation option (§6.3)
- table and column names
- the component split in Svelte
- the test strategy, following bmlib's habit of checking ports against shared
  fixtures
