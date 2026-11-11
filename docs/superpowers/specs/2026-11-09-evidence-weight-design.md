# Evidence weight — design

_2026-11-09. Status: draft for approval. Extends
[the StudyAnalyzer design](2026-10-21-studyanalyzer-design.md); its rules
(bmlib first, one canonical text, verified citations, persist everything,
sandbox, trust boundary) all apply and are not repeated here._

## 1. What it is, and why

An analysis tab that answers: **how much weight should this paper's main
findings carry, given how well the study was done?** It is the successor to
bmlibrarian's `agents/paper_weight`, which wrote down the maintainer's own
manual appraisal method but ran on abstracts only and scored every design as
if it were a trial.

It serves two jobs with one method:

1. **A literature-research pipeline** (outside StudyAnalyzer, calling bmlib)
   first drops papers below a quality bar, then weights the quotes extracted
   from the papers that pass. That pipeline needs a number that is
   **comparable across papers**, including across designs as far as that is
   possible: the hard case is a poorly conducted meta-analysis against a
   superbly designed cohort study.
2. **Calibration, inside StudyAnalyzer.** The pipeline cannot be trusted until
   its judgments have been compared with human ones on many papers. The
   maintainer, the maintainer's junior doctors, and later university colleagues and
   students read and appraise papers anyway. The tab makes their appraisal
   easier and more systematic, and as a by-product records their judgments
   next to the machine's. Overlap happens naturally: several people read the
   same papers.

**The incentive is the design.** The tab must be worth using for the reader
first: a guided critical appraisal, with the machine's answers as feedback
afterwards. If it feels like data entry for someone else's study, people stop
using it and the data stops coming.

## 2. Goals, non-goals, anti-goals

**Goals (first build)**

- Randomised controlled trials, end to end: study type, primary outcomes,
  RoB 2 per primary outcome, the evidence ladder, the number.
- The frame every later design reuses: the study-type step, the
  primary-outcome step, the reviewer's own assessment, the reveal and
  comparison, storage of every run.
- Every machine judgment backed by a quote located in the paper and
  highlighted there.
- Every assessment (human or machine) stored with who or what made it.
- Export of one's assessments to a file, and import of others' files.

**Non-goals (first build)**

- Tools for other designs (ROBINS-I, AMSTAR 2, QUADAS-2, …). Each comes later
  as one more tool on the same frame (§9).
- An agreement view inside the app. The export format is documented so the
  first analyses can be done outside (§8).
- Automatic study-type routing without the reader's confirmation. The
  classifier that could do it needs labelled papers, which this tab produces.
- Checking trial registries. Outcome switching is judged from what the paper
  itself says (§4.3); a registry lookup by validated trial ID comes later.
- Body-of-evidence judgments: inconsistency, publication bias, indirectness.
  They depend on the other papers and on the question asked, so they belong to
  the pipeline (§4.4).
- A server, accounts or sync.

**Anti-goals**: failure even if everything technically works.

- **A failure that looks like a score.** bmlibrarian stored 0.0 or a neutral
  5.0 for failed steps, and biasbuster hard-coded "some concerns" when a
  judgment was missing; both looked like real results. Here "not assessed" is
  its own visible state and never a number.
- **A quote that is not in the paper.** Neither earlier project checked its
  evidence quotes. A quote that cannot be located loses its role as evidence,
  with a visible note (§6.3 of the main design).
- **Anchoring the reader.** Showing the machine's (or another reviewer's)
  answers before the reader has saved their own.
- **A score for a design the tool does not cover.** Until a design has its
  tool, it gets "no method for this design yet", never a trial score.
- **Arithmetic or rule application done by the model.**
- **Treating any rater as ground truth,** Cochrane included (§7).

## 3. Who uses it

- **The maintainer**, calibrating the machine against their own judgment.
- **Junior doctors**, appraising papers they have to read anyway. Most
  rewarded by the guidance and the feedback. Approach B (§5.3) suits
  inexperienced reviewers and should raise agreement between reviewers.
- **Later, colleagues and students**, possibly as coursework. This means scale:
  many files from many people, so the export format must be stable and
  versioned from the start, and carry consent (§8).
- **The pipeline**, a program calling bmlib directly, not this tab.

## 4. The method (decided)

### 4.1 Overview

Built on established methodology, not invented: **GRADE's "start by design,
then move down or up"** for the cross-design scale, and the **formal
risk-of-bias tool for each design** inside it. The maintainer finds GRADE
subjective; the plan contains that subjectivity by keeping the model to narrow
factual questions and putting every rule in code.

For one paper:

1. **Study type.** The machine proposes one from a single bmlib taxonomy, with
   a located quote; the reader confirms or corrects it. The confirmed type is
   stored as a label (it is the training data a future classifier needs).
2. **Primary outcomes and their results.** The machine proposes each primary
   outcome with its reported result (effect estimate, confidence interval,
   number analysed), with quotes (usually from the methods and results); the
   reader confirms or corrects. Stored as labels. *My call:* these are facts,
   not judgments, so they are confirmed openly (not blinded) and every rater's
   ladder uses the same confirmed result for imprecision (§4.4).
3. **The design's tool, per primary outcome.** For an RCT, RoB 2.
4. **The ladder,** from the tool's verdict and the result's precision, in code.
5. **The number,** from the ladder through a configurable table.

*My call:* the machine's proposals in steps 1 and 2 are stored **before** the
reader confirms them, so the data also measures how well the machine would
sort papers and pick outcomes unattended, as the pipeline must. The machine's
RoB 2 run then uses the confirmed type and outcomes. bmlib offers both the
step-by-step functions (for the tab) and an unattended end-to-end entry point
(for the pipeline, which accepts its own proposals).

The unit of assessment is **one paper and one of its primary outcomes**: the
pipeline weights findings, and what matters is how good the evidence is for
that particular answer. A trial with two co-primary outcomes gets two ladders.
*My call:* RoB 2's D1 (randomisation) is assessed once per study and shared by
its outcomes; D2 to D5 are assessed per outcome (D2's analysis questions 2.6
and 2.7 depend on the analysis of each outcome).

Secondary outcomes are not assessed. A finding the pipeline extracts from one
carries "outcome not assessed"; it never inherits a primary outcome's weight.

**The paper-level bar (decided).** For the pipeline's first screening, a paper
passes if **any** of its primary outcomes clears the quality bar; each finding
is then weighted by its own outcome's number, so a weak co-primary outcome is
down-weighted rather than dropping the paper. bmlib piece 5 provides this rule
(the best primary outcome's number) next to the per-outcome numbers.

### 4.2 RoB 2 (first tool)

- *My call:* **individually randomised parallel-group trials** only. Cluster
  and crossover trials have their own RoB 2 variants with different questions;
  until those are built they get "no method for this design yet". The
  taxonomy (bmlib piece 1) keeps them as separate types from the start.
- The **signalling questions** of RoB 2 (22 for the effect of assignment:
  D1 3, D2 7, D3 4, D4 5, D5 3), each answered Y / PY / PN / N / NI.
- **Domain judgments** (low / some concerns / high) are derived in code by
  RoB 2's **official** algorithm, implemented from the published tool and its
  guidance, not copied from biasbuster, whose truth tables are simplified
  (its own survey found that D2's "low" checks only 2.1, 2.2 and 2.6, and D3
  ignores the 3.3 branch).
- **The overall judgment** follows RoB 2: low if every domain is low; high if
  any domain is high; otherwise some concerns. RoB 2 also allows "high" when
  several domains have some concerns in a way that substantially lowers
  confidence. *My call:* that step is a reviewer judgment, not a rule. The
  algorithm proposes "some concerns" and the tab asks the reviewer whether the
  combination warrants "high"; the machine's answer to that question is
  stored as its own answer with its reasoning.
- **Wording (decided):** RoB 2's guidance is CC BY-NC-ND 4.0, so its text is
  not copied. bmlib words each question in its own plain language, written
  for a language model to understand, keyed to the official question number;
  the decision rules (facts, not wording) are implemented as published.
- *My call:* the **effect of assignment** variant of D2 (intention to treat)
  first, since it matches most trials' primary analysis. The effect of
  adhering comes later.

### 4.3 Outcome switching

When the paper names a primary outcome that differs from the one it says was
registered or pre-specified (in its protocol or registration statement), that
is evidence for D5 (selection of the reported result) and is shown plainly.

### 4.4 The ladder for a single paper

GRADE's four levels: **high, moderate, low, very low.**

- **Start:** randomised evidence starts **high**. (When ROBINS-I arrives,
  GRADE's guidance lets non-randomised studies assessed with it also start
  high and lose levels through the bias assessment. This is the bridge that
  puts designs on one scale.)
- **Risk of bias:** RoB 2's overall verdict moves the result down.
- **Imprecision:** from the confirmed result (§4.1 step 2): whether the
  confidence interval crosses no effect, and how small the trial is.
- **Up-rating** (large effect, dose-response, confounding that would shrink
  the effect) applies to observational designs and is unused for RCTs.
- **Not here:** inconsistency, publication bias and indirectness (§2).

*My call:* how far each verdict moves the result, the imprecision rule, and the
ladder-to-number table are **settings with defaults**, not constants buried in
code, so they can be tuned once the comparisons show what works. The ladder
and its reasons are always shown next to the number.

### 4.5 How the machine answers

The lessons of biasbuster (`docs/papers/drafts/medrxiv_V5/PRIOR_APPROACHES.md`,
`docs/harness/three_step_approach/`) and bmlibrarian are constraints:

- **One narrow call per domain** (or smaller), never one big call over the
  paper. Large single calls on full text collapsed to a generic "moderate"
  that did not depend on the paper.
- **The model answers signalling questions with quotes and reasoning; code
  does every rule, count and roll-up.** For D5 the model reports facts
  instead, and code answers the questions from them (§4.6).
- **The model reads the paper,** not a summary of it. biasbuster's production
  RoB 2 calls saw only an extraction JSON, so their "verbatim" quotes came from
  an LLM summary.
- **The whole text.** Never truncated. bmlibrarian's 12k-character cut
  silently dropped funding and conflict of interest statements, which sit at
  the end. *My call:* when the paper does not fit the model's window, it is
  condensed (the PICO tab's verbatim-passage condenser) **per domain, with a
  query specific to that domain**, so quotes stay checkable. Such a run is
  **flagged as condensed** in storage, in the tab, next to its number and in
  the export, because a passage the condenser missed turns into "no
  information", which RoB 2 scores as concerns. The pipeline can then choose
  to discount or exclude condensed runs.
- **Model output is validated.** A missing or out-of-range field fails that
  question visibly; it is not clamped, defaulted or guessed.
- **Every run is kept,** with model, prompt version and date. A re-run adds a
  run rather than replacing one. Each machine run is a rater of its own, which
  measures run-to-run variation for free (biasbuster: κ 0.45 to 0.81 between
  runs).

### 4.6 D5 from facts (decided, 2026-11-11)

**Why.** The RoB 2 wording smoke test
(`docs/smokescreens/2026-11-10-rob2-wording.md`, findings 10 to 12 and the
arm E and F sections) found that small models (gpt-oss 20B, Gemma 4 26B)
answer 5.2 and 5.3 by the question's headline ("was the result chosen?") and
skip the evidence ladder in the answer definitions: N without any plan. Asking
the model only for facts, each borne out by a quote, and applying the ladder
in code fixed that. On four models and 20 RCTs, all four give the same D5
judgement in 15 and 17 of 20 paper-runs, against 9 and 10 with the signalling
questions. Large models stay as strict as before. The design below is the
smoke test's arm F version 3 with its checks ("F3p"). The other domains keep
their signalling questions until the same treatment is measured on them (the
maintainer's call: measure first).

**What the model reports**, in one call per paper and outcome (D5 is judged
per outcome):

- **Plan facts**, each `present` (true or false) with 1 to 3 verbatim quotes:
  a registration, protocol or analysis plan cited; that plan dated before the
  outcome data by group were seen; dated after (late); for the outcome
  assessed, the measure, the time point and the analysis set in advance; the
  outcome assessed itself switched against the registration or protocol;
  that switch given a reason that does not depend on its results.
- **Other measurements** of the same outcome (another scale, time point or
  definition of the same thing measured; not other outcomes, subgroups or
  within-group comparisons) that the methods, protocol or registration name,
  each with a label, the passage naming it and the passage giving its result.
- **The main analysis** the methods name for the outcome, set against the
  analysis whose result is given: `differs` with quotes for both.

**What code requires before a fact counts:**

- The model claims it, and at least one of its quotes is found in the paper by
  `find_passage` at the app's threshold (the quote check every answer gets).
- The quote can support the fact. A dated plan needs a date or a phrase such
  as "prospectively" or "before enrolment"; a bare registration number does
  not date anything. A switch needs the language of a change ("changed",
  "differs from", "originally"); "difference" in a results sentence is not
  one. The measure must name the outcome assessed, not the primary outcome of
  a paper reporting a secondary one. The time point needs a time.
- **The planning check (the maintainer's call):** a measure, time point or
  analysis must be stated as set in advance (prespecified, planned, a priori,
  the protocol or registration, a primary or secondary outcome, end point or
  analysis); the statement may sit in another quote of the same fact than the
  one naming the outcome. A methods description of what was done is not a
  plan.

**"Not reported", checked:** an other measurement counts only if its naming
passage is found. It is reported when a found result passage holds a number
or points to a figure, table, supplement or appendix (the text cannot show
what those hold). Otherwise a second, narrow call asks where the paper gives
that item's result; only an item both looks miss is missing. A second look
that fails leaves the item undecided, never missing. An unreported
sensitivity or subgroup analysis is incomplete reporting, not selection, and
does not count.

**The answers, in code:**

- **5.1:** Y when a cited plan dated before the data fixes the measure and the
  analysis; PY when the paper states them as set in advance but nothing cited
  dates the plan; N when the outcome assessed was switched without a reason
  independent of its results; NI otherwise, a late plan included.
- **5.2:** Y for such a switch; PY when an other measurement is missing at
  both looks; N when a cited, dated plan fixes the measure and the time point;
  PN when the paper alone states them as set in advance; NI otherwise. RoB 2's
  "measured in only one way" N is not used: such an outcome without a plan is
  NI.
- **5.3:** PY when the analysis reported differs from the main one, both
  quoted and found, with no reason given; N when a cited, dated plan fixes the
  analysis; PN on the paper's word; NI otherwise.
- The judgement is RoB 2's algorithm on these answers (§4.2).

**Shown and stored:** every fact with its quotes and which check it passed or
failed, every listed measurement with both looks, the analysis check, and the
answers they gave, so a reader sees why each answer came out. The prompt
version records the change.

**Known limits** (from the smoke test): a result both looks miss though the
paper gives it (one in 160 runs, RCT065's 7-week UPDRS in a table); the
checks' vocabularies are English; a quote can pass its check and still not
support the fact, though none was seen once the checks were in place.

## 5. The tab

### 5.1 Layout

The PICO tab's split: the assessment on the left, the paper on the right.
Name: **Evidence weight** (*my call*). Steps across the top:

**1 Study type → 2 Primary outcomes → 3 Your assessment → 4 Compare**

Steps 1 and 2 show the machine's proposal for confirmation; they are factual,
and confirming them is quick. For a design without a tool, the tab stops after
step 2 with "no method for this design yet" (the confirmed labels are still
stored).

### 5.2 Blinding (decided)

The machine's answers to the tool's questions, and any other reviewer's, stay
hidden until the reader saves their own. Every reviewer is always blind; it is
not a setting.

*My call:* saving **freezes the blind record**. After the reveal the reader may
revise their answers (that is much of the teaching value), but each revision is
stored as an **"after reveal"** version next to the frozen one, and exported as
such. Agreement is measured on the blind record; the revisions show what the
feedback changed.

### 5.3 Recording the reader's assessment (decided: approach B)

The reader answers **every signalling question**. The domain verdict follows
from the official algorithm and is shown as it fills in; the reader can
**override** it, and both the derived verdict and the override are stored.
Clicking a question lets the reader mark the sentence they based the answer on
(optional). A free-text note per domain (optional).

**Sources (decided).** The paper is the reference: the machine sees only its
canonical text. Reviewers may also consult the protocol, the trial
registration or the supplement (RoB 2 expects them for D5 and often for D1 and
D2); each answer has an optional tick, *"I also used: protocol / registry /
supplement"*, stored and exported, so disagreements that come from different
inputs can be separated from disagreements in judgment.

Chosen over "one verdict per domain" because every disagreement can then be
traced to a question, conformance can be checked everywhere, and the questions
teach the inexperienced reviewer.

Mockup: [mockups/evidence-weight-assess.html](mockups/evidence-weight-assess.html)
(option B; option A was the rejected alternative).

### 5.4 Compare

After saving, the reader sees their answers next to the machine's (the latest
run), question by question, with the machine's located quotes highlighted in
the paper, and the ladder and number for each. Differences are marked. This is
the feedback that makes the tab worth using.

### 5.5 Conformance (decided)

For every rater (machine, reader, imported reviewer): does each verdict follow
from that rater's own answers under the tool's rules? An override that departs
from the algorithm is shown quietly and stored. Such cases are kept as
**review cases**: they are where either the reviewer or the method is wrong,
and they are how the method gets adjusted.

## 6. Reviewers

- A reviewer is a **name chosen on the device**, not a login.
- Each reviewer also has a **stable pseudonymous ID** (generated once per
  reviewer per install) and an **expertise level** they set once.
  *My call:* the export always carries the pseudonymous ID; the display name
  is included only if the reviewer agrees.
- **Consent** to the use of their assessments for research is recorded with
  the version of the consent text, before their first export.
- *Builder's call:* whether several reviewers can share one install (one
  device in a teaching room) or it is one reviewer per install.

## 7. No rater is gold

Cochrane's own RoB ratings, used as a benchmark in biasbuster, often did not
follow Cochrane's algorithm (biasbuster found Cochrane more lenient than the
algorithm in about 47% of domain cells), and agreement figures measured
against them meant little on their own. So the data supports two separate
measures for every pair of raters:

- **Conformance**: did each rater follow the tool's own rules (§5.5)?
- **Agreement**: do two raters reach the same answer, per signalling question,
  per domain, per overall verdict?

The first build stores everything both measures need; computing them in the
app is a later stage.

## 8. Export and import

- **Export:** one action, "export my assessments", writing a file with every
  assessment made since the last export (*my call*: incremental, so readers
  never have to think about it; a full export is also offered).
- **Contents:** per assessment, the paper's identifiers (DOI / PMID / PMC ID),
  the confirmed study type and primary outcomes, every answer, verdict,
  override, note, marked sentence and sources tick, the rater (reviewer ID,
  or model and prompt version), tool and rule versions, and dates; the frozen
  blind record and any "after reveal" versions, marked as such; whether a
  machine run was condensed. Machine runs made on that device are included
  too, with the machine's unconfirmed proposals for steps 1 and 2.
- **No full text.** Many papers cannot legally be passed around. The importer's
  own StudyAnalyzer fetches the text by identifier as it does today.
- **Quotes as text and span.** A quote is stored as its text, its span, and the
  canonical text's converter version. When the importer's canonical text
  differs, `find_passage` relocates it; when it cannot, the quote is kept as
  text with the usual visible note.
- **Import** keeps imported assessments apart from the reader's own and lines
  overlaps up by paper identifier.
- **Trust boundary:** an imported file is treated like a hand-off file: size
  capped, every identifier's format validated, every field checked, no path or
  URL inside it dereferenced.
- **Format:** stable, versioned and documented from the first release, since
  files from many people and many versions will accumulate. *Builder's call:*
  the container (as for the hand-off file).

## 9. Work in bmlib first

Each its own bmlib PR, Rust and Python on par with shared contract fixtures
(main design §7):

| # | Piece | Source / notes |
|---|---|---|
| 1 | **Study-type taxonomy**, one list for the family | biasbuster `methodologies/base.py:48-60` (11 slugs) and bmlibrarian's four incompatible lists, reconciled. Mind bmlibrarian's `prospective_cohort` vs `cohort_prospective` mismatch (`paper_reviewer/study_detector.py:285` vs `:48-55`) |
| 2 | **Study-type proposal** from the full text, with quotes | LLM, "this paper, not what it discusses" (bmlibrarian `paper_weight/llm_extractors.py:107-111`). Publication types from metadata are a hint only; bmlibrarian_lite's mapping is lossy ("Clinical Trial" → RCT) |
| 3 | **Primary-outcome proposal**, with quotes | New |
| 4 | **RoB 2**: signalling questions, prompts, official algorithm, overall verdict; **D5 from facts** (§4.6): the facts reply, its readers, the quote checks, the second look and the rules | Prompts and schema from biasbuster `methodologies/cochrane_rob2/`; algorithm from the published tool (not biasbuster's `algorithms.py`); `docs/harness/ASSESSING_RISK_OF_BIAS.md` as a reference. Tests: biasbuster `test_cochrane_rob2.py`, `test_rob2_tuple.py` as ideas. D5: the smoke test's `scripts/d5_facts.py`, `d5_versions.py` and `fact_guards.py` with their tests, and its stored runs as fixtures' source |
| 5 | **The ladder and the number**, with their settings | New, pure functions |
| 6 | **Assessment export format**: types, (de)serialisation, validation | New; contract fixtures shared by Rust and Python so the pipeline reads the same files |

Later tools (ROBINS-I, AMSTAR 2, QUADAS-2, ROBIS, …) are each one more bmlib
piece; biasbuster has QUADAS-2 working and the rest as stubs, with
`ASSESSING_RISK_OF_BIAS.md` describing each. bmlibrarian's PRISMA 2020 and
transparency rubrics are separate tabs on the roadmap, not part of this one.

## 10. Storage

In StudyAnalyzer's SQLite, like everything else:

- confirmed study type and primary outcomes per paper, with who confirmed
  them and what the machine had proposed;
- every machine run: model, prompt version, date, each answer with its quote
  (span and text) and reasoning, derived verdicts, ladder, number;
- every human assessment: reviewer, answers, marked sentences, derived
  verdicts, overrides, notes, dates;
- imported assessments, kept apart from the reader's own;
- reviewers (name, pseudonymous ID, expertise, consent and its version);
- what has been exported, so the next export is incremental.

## 11. Phases

Each with its own plan in `docs/superpowers/plans/`:

1. **bmlib pieces 1 to 5** (Rust and Python).
2. **The tab:** steps 1 to 4, approach B, blinding, compare, conformance,
   storage, re-runs.
3. **Reviewers, export and import** (bmlib piece 6 and the app side).
4. **Later:** the agreement view; ROBINS-I and the other tools; registry
   checks; a study-type classifier trained on the collected labels.

## 12. Left to the builder

The exact prompt wording and how the 22 questions are grouped into calls
(narrow, per domain or smaller); how D2's adherence variant is added later;
the default ladder steps and number table (to be tuned); the imprecision
rule's thresholds; the export container; the consent text's wording (the
maintainer may need it to match an ethics application); one or several
reviewers per install; test strategy, using biasbuster's and bmlibrarian's
tests and fixtures as ideas. For D5 (§4.6): the facts prompt's wording, the
checks' exact vocabularies (the smoke test's are a start), whether second
looks run in parallel, and how the facts are shown in the tab.
