# The reader's own appraisal in the Evidence weight tab: design

_2026-11-26. Status: approved by the maintainer 2026-11-26. Phase 3 of the
Evidence weight tab, as the
[tab design](2026-11-24-evidence-weight-tab-design.md) (§1, cited as TAB)
numbers the phases. It builds EW §5.1 to §5.5 of
[the evidence-weight design](2026-11-09-evidence-weight-design.md) (cited as
EW), whose decisions stand: approach B (every signalling question), blinding,
the frozen blind record, compare, conformance. The main design's rules (bmlib
first, one canonical text, verified citations, persist everything, sandbox)
all apply._

## 1. What it is, and why

A reader appraises a trial themselves before they see the machine's
verdict. They confirm the study design and the primary outcomes openly,
answer RoB 2's signalling questions blind, and save. Saving freezes that
blind record. The reveal then shows their answers beside the machine's,
question by question, with both ladders and numbers, and checks each
rater's verdicts against their own answers (conformance).

The why is EW §1. First, calibration: the pipeline cannot be trusted until
its judgements have been set against human ones on many papers. Second,
the reader's own reward: a guided appraisal with feedback, worth doing for
its own sake, so the data keeps coming. The readers are the maintainer
first, then the maintainer's junior doctors, later colleagues and students.

**The conflict this design settles.** Phase 2's tab runs on its own the
first time it opens and shows the verdict at once (TAB §2), and
`first_shown_at` records that it was shown. Left as it is, every paper
opened in the tab would be unblinded before a reader could start. So the
first open of a paper now asks the reader which they want (§3).

## 2. Goals, non-goals, anti-goals

**Goals**

- A per-paper choice between appraising blind and seeing the machine's
  verdict (§3).
- Steps 1 and 2 of EW §5.1: the machine's proposed design and outcomes,
  confirmed or corrected by the reader, then stored as labels beside the
  proposals (§4).
- The machine's RoB 2 run on the **confirmed** design and outcomes, in the
  background and hidden (§4).
- The reader's answers to every applicable signalling question, with the
  derived verdicts, the overrides, the escalation question, marked
  sentences, the sources tick and the notes (§5).
- Save, which freezes the first record; compare; revisions after the
  reveal kept as their own versions (§6).
- Conformance for the reader and the machine (§7).
- Everything stored in SQLite as it is entered (§8).

**Non-goals (this phase)**

- Reviewers (names, pseudonymous IDs, expertise, consent), export and
  import. They are phase 4 (TAB §1; EW §6, §8). This phase has one local
  reader with no name.
- The agreement measures (EW §7). The data they need is stored; computing
  them comes later.
- Other tools (ROBINS-I and the rest) and designs other than
  parallel-group RCTs.
- Changing the ladder settings in the app.
- A bmlib PR. Everything here uses bmlib's existing public API (§10).

**Anti-goals** (EW §2 holds in full; these are the ones this phase can
break)

- **Anchoring the reader.** Nothing the machine judged about RoB 2 (an
  answer, a domain or overall verdict, a ladder, a number) reaches the
  screen during a blind appraisal, not in the run list, the status line,
  a progress label or an error. The design and outcome proposals are
  shown, because EW §4.1 has the reader confirm them openly.
- **A blind record that is not blind.** An appraisal is stored as blind
  only if no run of that paper had been shown when it started and the
  reader saved it before asking to see the machine's verdict.
- **A first record that changes.** After the save, a revision is a new
  version; the first record is never edited.
- **A failure that looks like a score.** A reader's domain with unanswered
  applicable questions has no verdict, its outcome no ladder, and the
  appraisal cannot be saved incomplete (§6).
- **Losing the reader's work.** A quit, a crash or a failed machine step
  never loses an answer already given.

## 3. Entering an appraisal

When the Evidence tab opens on a paper:

- **No run of the paper has been shown** (no `first_shown_at` on any of
  its runs) **and no appraisal exists**: the tab offers two buttons and
  starts nothing.
  - *Show the machine's verdict*: phase 2's tab, unchanged (the run starts,
    its result is shown).
  - *Appraise it yourself first*: an appraisal starts, marked blind, and a
    machine run paired with it starts in the background (§4).
- **A run has been shown already**: phase 2's tab, as now, with one more
  action, *Appraise it yourself*. The appraisal it starts is stored as not
  blind and says so at its top.
- **An open appraisal exists**: the tab opens on it, in the state it was
  left in: confirming, answering, or saved (compare). *My call:* once the
  appraisal is revealed (saved or *Show the machine's verdict now*), a
  switch *Your appraisal | Machine runs* also reaches phase 2's view (its
  run picker and *Re-run*, the paired runs listed among them). Before the
  reveal there is no such switch.
- **Set aside** (decided, maintainer, 2026-11-26). An appraisal in any
  state can be set aside: a mis-clicked design, a wrongly confirmed
  outcome, a reader who wants to start again. It is kept with all its
  versions, marked *set aside* with the time, and left out of later
  agreement measures; nothing is deleted. A new appraisal can then be
  started as above. It is blind only if no run of the paper has been
  shown, so setting aside an unrevealed appraisal keeps the next one
  blind. A paper has at most one open (not set aside) appraisal.

Each paper asks only once, and the choice is not remembered between papers.
While an appraisal is unsaved, *Show the machine's verdict now* is always
offered. *My call:* using it marks the appraisal not blind, records when
(`revealed_at`) and opens compare against the draft as it stands, but
freezes nothing: the draft goes on until the reader completes it and
saves, and that save freezes its first record as any save does (§6).

## 4. Confirming, then the machine catches up

**The paired run.** A run started by an appraisal runs its design and
outcome steps as today and then **waits for confirmation** (a new run
status), even when its own design is not covered or it found no primary
outcome, since the reader may correct either. Its proposals are stored as
they were made (phase 2's tables), so the data still measures how the
machine sorts papers and picks outcomes unattended (EW §4.1).

**Step 1, the design.** The proposal is shown with its quotes, and the
reader confirms it or picks another from bmlib's taxonomy. If the
confirmed design is not a parallel-group RCT, the appraisal stops here
with *no method for this design yet*. The label is stored, and the paired
run completes as *not covered* without RoB 2 steps.

**Step 2, the primary outcomes.** Each proposed outcome is shown with its
result (effect estimate, CI and its level, number analysed) and quotes.
The reader can confirm it, edit its name, time point or result, remove it,
or add one the machine missed. At least one outcome is needed to go on.

**Confirm** stores the confirmed design and outcomes with the appraisal and
lets the paired run continue: its RoB 2 plan is built for the confirmed
outcomes, and each domain step, escalation and ladder uses them (their
names and results), not the run's own proposals. Both raters' ladders use
the same confirmed result for imprecision (EW §4.1). The run stays hidden
(§2) while the reader answers. Progress is shown only as a neutral line
(*The machine's assessment: 6 of 11 steps*). A step that fails says *The
machine's assessment stopped: ‹reason›* with *Continue*, and never names
a judgement.

**Its lifecycle is phase 2's** (*my call*): the same commands, the one
evidence run at a time app-wide (`evidence_stop`'s slot) and the tab
host's run state. When the appraisal is opened and its paired run is
confirmed but has pending or failed steps (a quit, a stop, leaving the
paper), it continues on its own, since a hidden run cannot anchor anyone;
if another paper's run holds the slot, the neutral line says it waits and
offers *Continue*.

**Re-pairing** (*my call*). A paired run that cannot continue (made under
earlier rules after a re-pin, or one the reader wants made with another
model) is replaced by *Run the machine again on these outcomes*: a new
paired run on the same confirmed design and outcomes, which skips its
design and outcome steps. The earlier one is kept; compare uses the
latest. So an appraisal has one or more paired runs.

*My call:* confirmation is final for the appraisal, since the machine's
run and the reader's answers are both made for the confirmed outcomes; a
confirmation the reader later thinks wrong is mended by setting the
appraisal aside and starting a new one (§3). An appraisal on a paper whose run was
already shown pairs with a new run as well, rather than reusing an old
one, so every appraisal is compared with a run made on its own confirmed
outcomes.

## 5. Answering

The layout is the PICO tab's split: the appraisal on the left, the paper
on the right, steps across the top (*1 Design → 2 Outcomes → 3 Your
assessment → 4 Compare*, EW §5.1). On phones, *Evidence | Paper* replaces
the split. Style A ("Journal"). EW's mockup B
(`mockups/evidence-weight-assess.html`) is the starting point.

```
┌ Evidence weight · your appraisal (blind) ┐┌ Paper ─────────┐
│ 1 Design ✓  2 Outcomes ✓  3 You  4 Compare│                 │
│ ▾ D1 Randomisation           → low       ││ …randomly       │
│   1.1 Chance decided groups?  Y PY PN N NI││ ▓▓▓▓▓ (marked)  │
│   1.2 Allocation concealed?   Y PY PN N NI││                 │
│ ▸ D2 Deviations     0 of 7               ││                 │
│ [Show the machine's verdict now] [Save]  ││                 │
└───────────────────────────────────────────┘└─────────────────┘
```

- **D1 once, D2 to D5 per confirmed outcome** (EW §4.1), as the machine
  does.
- **The questions** are bmlib's wording (`rob2_questions`), keyed to RoB
  2's numbers: the headline shown, its *Applies only when*, *Look in* and
  the Y / PY / PN / N / NI definitions one click away. RoB 2's own text is
  never shown (CC BY-NC-ND, EW §4.2).
- **Applicability.** *My call:* questions that do not apply given the
  earlier answers (`applicable_questions`) are greyed and stored as not
  applicable, so the reader never picks NA. Changing an earlier answer
  that makes a later one inapplicable keeps the later answer stored but
  not counted, and shows it again if it applies once more.
- **D5** is answered by its three signalling questions, as for any domain.
  The machine answers D5 from facts (EW §4.6). The two meet at the answers
  (5.1 to 5.3), and the machine's facts explain its answers in compare.
- **Verdicts.** Each domain's verdict follows from the answers by RoB 2's
  algorithm (`domain_judgement`) as soon as every applicable question is
  answered. The reader can override it; both the derived verdict and the
  override are stored (EW §5.3). The overall verdict follows by
  `overall_judgement`. When `needs_escalation_decision` holds, the reader
  is asked whether the combination of concerns warrants *high* (EW §4.2),
  and that answer is stored as such. The overall verdict can be overridden
  like a domain's.
- **An override feeds what follows it** (*my call*). Each domain's
  verdict as the reader leaves it (the override, or else the derived one)
  goes into `needs_escalation_decision` and `overall_judgement`, and the
  overall as the reader leaves it into the ladder. An override is the
  reader's own judgement; carrying it on is what a reader means by it.
  Conformance checks each step against its own inputs (§7), so one
  override is flagged once, not again at every step after it.
- **The reader's ladder and number** come from the reader's overall
  verdict and the confirmed result, by bmlib's `build_ladder` with the
  default settings stored with the paired run.
- **Optional per question:** mark the sentence the answer rests on, by
  selecting text in the paper pane. It is stored as a span of the
  canonical text in code points (converted from UTF-16 only at the
  frontend boundary), with its text. Also optional: the tick *I also used:
  protocol / registry / supplement*. **Optional per domain:** a free-text
  note.
- **Drafts.** Every answer, override, mark, tick and note is stored as it
  is entered (*persist everything*). Quitting and reopening returns to the
  same place.

## 6. Save, reveal, compare

**Save and compare** is offered once every applicable question of every
domain of every confirmed outcome is answered, and the escalation question
where it is asked. *My call:* no partial save. A partly answered appraisal
stays a draft and can be left and resumed, but is not revealed. To see the
machine's verdict anyway, the reader uses *Show the machine's verdict now*
(§3), which marks the appraisal not blind.

**Saving freezes the first record** (EW §5.2's blind record): version
*first record*, with its saved time, complete by the rule above. It is
never edited. The appraisal's blind flag says whether it is blind (§3):
it is blind only for an appraisal started blind and saved before any
*Show the machine's verdict now*. The reveal marks the paired run as shown
(`first_shown_at`), as phase 2's view does.

**Compare** (EW §5.4), per outcome, against the paired run:

- each question with the reader's answer and the machine's side by side,
  the differences marked; the machine's reasoning and quotes on demand,
  each quote highlighting its passage in the paper (`#e8b931` current,
  `#f6e7b0` the others); the reader's marked sentence highlighted too
  (*my call:* outlined rather than filled, so the two are told apart);
- each domain's derived verdict, override and the machine's verdict;
- the overall verdicts, the escalation answers, both ladders with their
  steps, and both numbers;
- for D5, the machine's facts with their checks, under its answers.

If the paired run is not complete at the reveal, compare shows what has
landed, says which steps are still running or failed, and fills in as the
rest lands. A domain the machine failed shows *not assessed* on its side,
never a verdict.

**After the reveal**, the reader may change answers, overrides and notes.
*My call:* the first change after the save starts version *after reveal
1*, copied from the first record; further changes are made in that version
until the reader closes it (*Done revising*), and a later change starts
*after reveal 2*. Compare shows the first record by default and the latest
version on request. Agreement (later) is measured on the first record of
a blind appraisal alone (EW §5.2).

## 7. Conformance

EW §5.5: for each rater, does each verdict follow from that rater's own
answers under RoB 2's rules?

- **The reader:** a domain override that departs from `domain_judgement`
  on that domain's answers, and an overall override that departs from
  `overall_judgement` on the domain verdicts as the reader left them and
  the escalation answer (§5), is a *departure*. In
  compare it is shown quietly beside the verdict (*your verdict departs
  from the rules: they give ‹x›*). It is stored, per version, as a review
  case.
- **The machine:** its verdicts are made by code from its answers, so they
  conform by construction. The check runs on it as well and is shown the
  same way. A machine departure can only be a defect, and is reported as
  one.
- *My call:* conformance is computed on read from the stored answers and
  verdicts, never stored as a result of its own; the review cases are a
  query over departures.
- **Rules versions** (*my call*, after TAB §6's *a run is shown as it was
  made*). An appraisal stores bmlib's `ROB2_ALGORITHM_VERSION` and
  `LADDER_VERSION`. A frozen version made under other rules than the
  pinned bmlib's is shown as made (its stored derived verdicts), labelled
  *made under earlier rules*, and its conformance reads *not checked
  (earlier rules)*, never a departure the current rules invent. The same
  holds for a paired run made under earlier rules. An unsaved draft has
  nothing frozen: on the first open after a re-pin its verdicts are
  derived again under the current rules and it takes their versions.

## 8. Storage

One migration, approach A (decided, maintainer, 2026-11-26): the
appraisal in tables of its own beside the machine's, which stay as phase 2
made them.

- **The appraisal:** paper, canonical text, the paired run, blind or not,
  `started_at`, `revealed_at` (by a save or by *Show the machine's verdict
  now*), `saved_at`, `set_aside_at`, its state (confirming, answering,
  saved), and the confirmed design. At most one appraisal per paper has no
  `set_aside_at`.
- **The confirmed outcomes:** position, name, time point, the result
  fields as in `evidence_outcomes`, and which proposed outcome each came
  from (or none, when the reader added it).
- **The rules versions** the appraisal was made under (§7).
- **Versions:** the first record, then after reveal 1, 2, …, each with
  its created and closed times; a frozen version is complete (§6).
- **Per version:** each answer (domain, outcome, question number, answer
  or not applicable, the marked span and its text, the sources ticks);
  each domain's derived verdict and override; the escalation answer; the
  overall verdict and its override; each domain's note.
- **The paired runs:** phase 2's `evidence_runs` gains the *waiting for
  confirmation* status and a link from each run to its appraisal (one or
  more per appraisal, §4); a re-paired run has no design and outcome
  steps of its own. The run's
  RoB 2 steps read the confirmed outcomes through that link (§4); its own
  outcome step keeps the proposals.
- A paired run is **never listed** in phase 2's run picker (*runs ▾*) and
  never served by `evidence_view` while its appraisal is unrevealed. After
  the reveal it is listed like any run.
- `open::has_dependents` counts an appraisal's marked spans, so a text an
  appraisal points into is never re-derived under it.
- A row the store's own shape check contradicts (for example an answer to
  a question RoB 2 does not have, a frozen version with an unanswered
  applicable question, or two first records) is read back as `Corrupt`,
  as in `evidence_store`.

## 9. Approaches

How to store the appraisal (decided: A, maintainer, 2026-11-26):

- **A. Appraisal tables of its own beside the machine's runs.** Phase 2's
  tables and views stay as they are; compare reads both. The cost: phase
  4's export maps two shapes into one file.
- **B. One rater model:** machine and reader answers in shared tables
  keyed by rater. Agreement, conformance and export would be one query
  each, but it migrates phase 2's just-merged tables and reopens *a run
  is shown as it was made* (TAB §6). Rejected: a large refactor for gains
  phase 4 gets anyway.
- **C. Each version as one JSON document.** The least schema, but it
  breaks with how every analysis tab stores its data (rows read back as
  `Corrupt` when misshapen), and agreement queries would parse JSON.
  Rejected.

Also decided in the conversation (maintainer, 2026-11-26): the per-paper
choice (§3; rejected: a setting, or both); setting an appraisal aside
(§3; rejected: strictly one per paper, and reopening a confirmation until
the first answer, which discards the machine's steps made so far); and *pause, then continue* for
the paired run (§4; rejected: a whole unattended run re-run when the
reader corrects it, which doubles the calls; and comparing only the
outcomes that match, which loses the comparison exactly where the reader
disagreed).

## 10. Where the code goes

- **bmlib:** nothing. `domain_judgement`, `applicable_questions`,
  `overall_judgement`, `needs_escalation_decision`, `build_ladder`, the
  question texts and `ProposedOutcome` (all public fields) are what the
  reader's side needs. The shared export format is phase 4's bmlib piece
  (EW §9, piece 6).
- **StudyAnalyzer core:** the migration; an appraisal store and view on
  the pattern of `evidence_store.rs` / `evidence_view.rs`; the paired
  run's wait and its confirmed outcomes in `evidence.rs`; conformance as
  pure functions; the `has_dependents` update.
- **Commands** (`src-tauri`): start an appraisal, confirm, save an answer
  (and an override, mark, tick, note), save the appraisal, show the
  machine's verdict now, close a revision, set aside, run the machine
  again on these outcomes, and the appraisal's view; the
  IPC contract fixture.
- **Frontend:** the choice, steps 1 and 2, the question list with live
  verdicts, text selection in the paper pane for a marked sentence (new:
  `PaperPane` has no selection handling today), compare, and their tests.
- **Docs:** HANDOVER, ROADMAP, `docs/DECISIONS.md`, and a line in TAB §1
  and EW §11 pointing here.

## 11. Left to the builder

- The names of tables, columns, commands, components and the new run
  status.
- Exactly how the paired run reads the confirmed outcomes (the link, or
  copying them into rows it owns), as long as its proposals stay stored as
  made.
- The design taxonomy picker's form in step 1, and how an outcome's result
  is edited in step 2.
- How a selection in the paper pane is turned into a span (it must land on
  the canonical text's code points, and a selection across the paper's
  section boundaries is either clipped or refused, visibly).
- The neutral progress line's wording, and whether the machine's failed
  steps during a blind appraisal retry on their own.
- The component split in Svelte, and the test strategy (the store's shape
  tests as in `evidence_store`, vitest for the live verdicts and the blind
  guard, a contract fixture for the IPC).
- The plan count: one plan, or two (core and commands, then frontend).

**Known limits, accepted:** one open appraisal per paper per install, by
one unnamed reader, until phase 4 adds reviewers; a confirmation cannot be
undone within an appraisal, only set aside with it (§3, §4).
