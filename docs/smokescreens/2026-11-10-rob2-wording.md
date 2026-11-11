# RoB 2 question wording: bmlib's own against the guidance's

**Date:** 2026-11-10 · **bmlib:** branch `feat/evidence-weight` (prompt version 3 at commit `236d403c`)
· **Models:** `deepseek:deepseek-flash`; `gpt-oss:120B` and `gpt-oss:20b` (Ollama on a DGX
Spark) and Gemma 4 26B-A4B (oMLX on a Mac) on both samples; a two-paper check of Qwen3.8 27B
(oMLX); temperature 0.1
· **Data:** [data/2026-11-10-rob2-wording/](data/2026-11-10-rob2-wording/)
· **How to rerun:** [README.md](README.md)

## Question

bmlib asks RoB 2's 22 signalling questions in its own plain-language wording, because the RoB 2
guidance is CC BY-NC-ND 4.0 and must not be copied or paraphrased. Does that wording lead a model to
the same answers as the guidance's own text, and where it differs, is it faithful to the RoB 2 method?

The yardstick is the method applied to the published evidence, not agreement with Cochrane.
Cochrane reviewers often judge on assumptions ("an experienced team would have randomised
properly"); a wording that is stricter than Cochrane because it insists on documented evidence is
doing what is intended. biasbuster's audit of the same benchmark found Cochrane more lenient than its
own algorithm 8.5 : 1.

## Method

- **Papers:** RCTs from biasbuster's Eisele-Metzger replication benchmark with JATS full text and
  Cochrane's published per-domain judgements, minus the 13 known wrong-paper acquisitions: 24 papers.
  Sample 1 is 10 of them (seed 20261110); sample 2 is 10 of the remaining 14 (seed 20261111).
- **Agent:** bmlib's `Rob2Agent`, one call per domain (D1 once, D2-D5 for the outcome Cochrane
  judged), quotes verified against the paper at 0.99 similarity (StudyAnalyzer's setting), paper sent
  whole (no condensing). The outcome given to the agent is the one Cochrane assessed.
- **Arms**, differing only in the text of the 22 questions:
  - **A:** bmlib's wording, prompt version 2 (before this test).
  - **B:** the guidance's own question and elaboration, verbatim, from the 22 August 2019 PDF.
  - **C:** bmlib's wording, prompt version 3 (after the fixes below).
  - **D:** C with a reworded 5.1 (now bmlib's prompt version 4), D5 only, since 5.1 touches no
    other domain.
- **Two runs per arm and paper,** so the run-to-run noise floor is visible.
- **Token limit:** 8192 for A and B in sample 1; the agent's new default 16384 for C, and for B in
  sample 2.

## Results

### Sample 1 (RCT045, 090, 025, 085, 065, 050, 047, 096, 081, 013)

| | A (v2) | B (guidance) | C (v3) |
|---|---|---|---|
| Same judgement on both runs (noise floor) | 84% | 88% | 87% |
| Agrees with Cochrane, D1-D5 and overall | 41% (κ 0.21) | 51% (κ 0.36) | 40% (κ 0.22) |
| Stricter / more lenient than Cochrane (cells) | 54 / 16 | 47 / 11 | 58 / 14 |
| Domains failed | 1 | 1 | 0 |
| Quotes found in the paper | 601 / 606 | 557 / 559 | 630 / 632 |
| Median seconds per paper | 75 | 79 | 87 |

C and A agree on 87% of cells, C and B on 81%, A and B on 82%.

### Sample 2 (RCT055, 031, 035, 007, 099, 086, 005, 062, 003, 059)

| | B (guidance) | C (v3) |
|---|---|---|
| Same judgement on both runs (noise floor) | 80% | 87% |
| Agrees with Cochrane, D1-D5 and overall | 52% (κ 0.34) | 57% (κ 0.35) |
| Stricter / more lenient than Cochrane (cells) | 49 / 8 | 46 / 6 |
| Domains failed | 0 | 0 |
| Quotes found in the paper | 611 / 611 | 668 / 669 (2 contradicted) |
| Median seconds per paper | 105 | 124 |

C and B agree on 76% of cells.

κ is linear-weighted over low < some concerns < high. Judgements on the same paper are not
independent, and 10 papers per sample is small: read the numbers as directions.

### Second model: gpt-oss 120B, both samples

Same papers, arms C and B, two runs each, Ollama with a 49,152-token context.

| | B, sample 1 | C, sample 1 | B, sample 2 | C, sample 2 |
|---|---|---|---|---|
| Same judgement on both runs (noise floor) | 88% | 80% | 87% | 82% |
| Agrees with Cochrane, D1-D5 and overall | 44% (κ 0.25) | 43% (κ 0.26) | 52% (κ 0.28) | 49% (κ 0.30) |
| Stricter / more lenient than Cochrane (cells) | 60 / 7 | 60 / 8 | 49 / 8 | 56 / 5 |
| Domains failed | 0 | 0 | 0 | 0 |
| Quotes found in the paper | 307 / 386 | 327 / 435 | 332 / 396 | 387 / 493 |
| Median seconds per paper | 319 | 315 | 343 | 345 |

C and B agree on 65% of cells in sample 1, 72% in sample 2.

### Third model: Gemma 4 26B-A4B (4-bit), both samples

Same papers, arms C and B, two runs each, through oMLX's OpenAI-compatible server on a Mac, at
prompt version 3 like the other models.

| | B, sample 1 | C, sample 1 | B, sample 2 | C, sample 2 |
|---|---|---|---|---|
| Same judgement on both runs (noise floor) | 92% | 100% | 98% | 93% |
| Agrees with Cochrane, D1-D5 and overall | 57% (κ 0.41) | 62% (κ 0.46) | 61% (κ 0.29) | 69% (κ 0.40) |
| Stricter / more lenient than Cochrane (cells) | 36 / 8 | 21 / 24 | 25 / 16 | 14 / 23 |
| Domains failed, of 100 | 9 | 1 | 7 | 0 |
| Quotes found in the paper | 200 / 217 | 308 / 324 (4 contradicted) | 196 / 198 | 328 / 344 |
| Median seconds per paper | 169 | 178 | 159 | 191 |

C and B agree on 60% of cells in sample 1, 77% in sample 2. The failures are replies that leave an
applicable question unanswered (mostly 4.4, also 2.6 and 4.3) or give an answer the question does
not allow; bmlib refuses them, so nothing wrong is stored. They are B's problem: 16 against C's 1.

### Fourth model: gpt-oss 20B, both samples

Same papers, arms C and B, two runs each, on the same DGX, Ollama context and bmlib (prompt
version 3 at `236d403c`, before the ellipsis split) as the 120B.

| | B, sample 1 | C, sample 1 | B, sample 2 | C, sample 2 |
|---|---|---|---|---|
| Same judgement on both runs (noise floor) | 72% | 69% | 75% | 72% |
| Agrees with Cochrane, D1-D5 and overall | 46% (κ 0.33) | 47% (κ 0.26) | 63% (κ 0.39) | 55% (κ 0.32) |
| Stricter / more lenient than Cochrane (cells) | 41 / 12 | 51 / 11 | 27 / 13 | 47 / 7 |
| Domains failed, of 100 | 11 | 1 | 6 | 0 |
| Quotes found in the paper | 338 / 430 | 285 / 437 | 375 / 463 | 428 / 527 (1 contradicted) |
| Median seconds per paper | 300 | 270 | 331 | 305 |

C and B agree on 60% of cells in sample 1, 65% in sample 2. B's failures are, as on Gemma,
applicable questions left unanswered (3.4, 4.4) or NI where the question does not allow it (3.2).

### The reworded 5.1 (arm D, D5 only)

5.1 asks whether the result came from an analysis planned before the outcome data by group were
seen; it decides only low against some concerns. Version 3 left it as it was, and C answered Y or
PY on some papers where B said NI. Arm D asks it on the 5.2 / 5.3 evidence ladder: Y needs a
cited, dated plan that fixes the outcome's measure and analysis; PY the same content on the paper's
word alone; a bare "prespecified", a plan cited without its content, or a late registration is NI.

| 5.1 answers, of 20 runs | B | C | D |
|---|---|---|---|
| DeepSeek, sample 1 | PY 2, N 1, NI 17 | NI 20 | PY 1, NI 19 |
| DeepSeek, sample 2 | PY 3, PN 1, NI 16 | PY 6, PN 1, NI 13 | PY 1, NI 19 |
| gpt-oss, sample 1 | NI 20 | Y 1, PY 1, N 1, NI 17 | N 1, NI 19 |
| gpt-oss, sample 2 | Y 1, NI 19 | Y 1, PY 1, N 2, NI 16 | N 2, NI 18 |

D answers NI at least as often as B in every row. Every Y and PY of C became NI under D except one: DeepSeek's PY on RCT096, where the paper shows
what the plan fixes and only its date rests on the paper's word, which is a PY under the new
ladder. D gave the same answer on both runs for 9 of 10 papers in each DeepSeek sample. D was
adopted as bmlib's prompt version 4 (commit `7b55d695`).

### D5 from quoted facts (arm E, D5 only)

The small models answered 5.2 and 5.3 by the question's headline ("was the result chosen?") and
skipped the ladder in the answer definitions. Arm E asks no signalling question. The model reports
nine facts about the plan behind the outcome, each with quotes, in one call per paper
(`scripts/d5_facts.py`): a plan cited; dated before the data by group were seen; dated after; the
measure, the time point and the analysis set in advance; a measurement or an analysis named but not
reported; a switched primary outcome. A fact counts only when it is claimed AND one of its quotes
is found in the paper (the same `verify_quotes` every answer gets). Code applies the ladder of
prompt versions 3 and 4 to give 5.1-5.3, and bmlib's algorithm the judgement. The "measured in only
one way" exception is gone: such an outcome without a plan is NI. Same bmlib (`236d403c`), papers
and two runs as arm C, on all four models.

**E′** re-applies the rules to the stored facts with the two "not reported" facts ignored. Those
claim an absence, and a quote can only show where a measurement is named, never that its result is
missing. Under E they made 5.2 or 5.3 PY, and so D5 high, on DeepSeek in 6 and 4 of 20 runs and on
gpt-oss 120B in 4 (sample 1), mostly wrongly: in RCT085 and RCT090 the "unreported" measurement is
mortality, the outcome assessed, which is reported. A switched primary outcome (a stated fact)
still gives Y.

| D5, of 20 runs (sample 1 / sample 2) | C: low | E′: low | C: same on both runs | E′: same on both runs |
|---|---|---|---|---|
| DeepSeek | 0 / 1 | 2 / 2 | 9 / 9 of 10 | 10 / 10 |
| gpt-oss 120B | 2 / 2 | 3 / 2 | 10 / 8 | 7 / 10 |
| gpt-oss 20B | 5 / 4 | 6 / 4 | 7 / 7 | 8 / 10 |
| Gemma 4 26B | 10 / 7 | 4 / 2 | 10 / 9 | 10 / 10 |

| N on 5.2, of 20 runs (sample 1 / sample 2) | C | E′ |
|---|---|---|
| DeepSeek | 7 / 4 | 3 / 0 |
| gpt-oss 120B | 3 / 1 | 0 / 0 |
| gpt-oss 20B | 9 / 7 | 2 / 0 |
| Gemma 4 26B | 10 / 7 | 4 / 3 |

- **The small models now follow the ladder.** An N needs a cited plan and a quote dating it before
  the data; the rest is PN or NI. gpt-oss 20B's Y on 5.1 (6 and 5 of 20 under C) became PY or
  NI, and its 5.2 / 5.3 Y (4 each in sample 2) are gone. Gemma's lows fell from 10 and 7 to 4
  and 2. The lows that remain mostly come from the paper's-word rung (5.1 PY, 5.2 / 5.3 PN: the
  measure, time point and analysis each quoted as prespecified), which the ladder allows.
- **The models agree with each other.** All four give the same D5 judgement in 12 of 20 paper-runs
  under E′ against 9 under C in sample 1, and in 18 of 20 against 10 in sample 2.
- **The facts are grounded.** 289 of 302 claimed facts had a quote found in the paper (gpt-oss
  20B: 72 of 82; the 10 others were dropped). A found quote need not support its fact: Gemma once
  dated a plan "before the data" with the bare "ClinicalTrials.gov number, NCT04960202."
- **It is cheap.** One call per paper and outcome, a median of 9-15 seconds on DeepSeek and Gemma,
  40-55 on gpt-oss, every reply readable on the first or second try, no domain failed.
- **What it does not yet do:** detect selective reporting by absence. E′ can only reach Y / PY
  through a stated switch, so a missing result goes unnoticed unless the model is asked in a way a
  quote can bear out (for example the measurement named in the methods, and the results' passage
  that should hold it).

### "Not reported", checked (arm F, D5 only)

Arm E′ could reach 5.2 / 5.3 Y or PY only through a stated switch. Arm F asks for the facts again
in one call and makes "not reported" checkable (`scripts/d5_versions.py`). Every flag of every run
was checked by hand against the paper's text (`scripts/review_f.py`).

**Version 2.** The model lists every other measurement and analysis of the outcome, each with the
passage naming it and the passage giving its result. An item is missing only when its naming
passage is found in the paper and neither that first look nor a focused second call finds a
result passage holding a number. The mortality false alarms of arm E are gone, but of 11 flags on
the four models only one (RCT007, in both of DeepSeek's runs: a sensitivity analysis named and
never reported) is a real absence. The rest: a result in a figure, table or supplement the text
does not hold (RCT059's Fig. 3, RCT099's subgroups); an item outside the effect estimate or the
outcome (a within-group test, subgroups, a number needed to treat, a different outcome); and one
result both looks missed.

**Version 3** follows bmlib's own wording. 5.3's PY is "the reported analysis differs from the
planned one", so the model sets the main analysis the methods name against the one whose result is
given, quoting both; it counts only when both are found and it says they differ with no reason
given. Unreported sensitivity or subgroup analyses no longer count: they are incomplete reporting,
not a sign the main result was chosen. For 5.2, a result passage that points to a figure, table,
supplement or appendix counts as reported, since the text cannot show what those hold, and the
list is held to other scales, time points or definitions of the outcome assessed.

| Version 3 flags (both samples, two runs) | DeepSeek | gpt-oss 120B | gpt-oss 20B | Gemma 4 |
|---|---|---|---|---|
| Real | 2 (RCT035) | 0 | 0 | 2 (RCT035) |
| False | 0 | 2 | 0 | 0 |

RCT035's methods analyse the activity-monitor data with "a multivariate repeated measures analysis
of variance"; its table's footnote gives the group P values from "a two-sample t test" on change
scores, with no reason. That is 5.3 PY, and D5 high (Cochrane: low). Gemma's quotes for it are
weaker than DeepSeek's (the paired t tests the methods name for other outcomes). The 120B's two
false flags: a "switched outcome" quoted from a sentence that reports deaths (RCT085), and a 7-week
UPDRS change both looks missed though the results table gives it (RCT065). gpt-oss 20B found no
flag, missing RCT035. All 160 version 3 runs were readable, a median of 20-24 s per paper on
DeepSeek and Gemma and 54-78 s on gpt-oss (the second look costs a call per item without a result).

**What remains is a quote that is found but does not support its fact.** The same weakness moves
the plan facts: under arm F's longer prompt Gemma rates D5 low in 8 of 20 sample-1 runs (E′: 4),
with "dated before the data" backed by a bare registration number (RCT096: "This study is
registered with ClinicalTrials.gov, NCT04546581."), a sentence on when an interim analysis was
planned (RCT081), and the primary outcome's measure quoted for a secondary outcome (RCT096). All
four models agree on D5 in 9 and 15 of 20 paper-runs under version 3 (E′: 12 and 18).

### Checks in code that a quote supports its fact (offline, on arms E and F version 3)

`scripts/fact_guards.py` asks of each found quote the least its fact needs, and
`scripts/guard_report.py` re-derives D5 from the stored runs with them (no new model calls):
"dated before the data" needs a date or "prospectively" / "before enrolment" and the like; a
switch needs the language of a change ("changed", "differs from", "originally", ...); the measure
must name the outcome assessed (a shared word, by five-letter stem); the time point needs a time.
A fact none of whose found quotes passes is absent.

- **The checks catch what they aim at, and nothing else.** They dropped 13 facts on the four
  models, each wrong on reading: a bare registration number or an interim-analysis plan as "dated
  before the data" (10, mostly Gemma), the primary or a safety outcome quoted as the measure of
  another (2), and the RCT085 "switch" (1), whose only change word was "difference" until the
  check asked for "differs from". The 120B's false high on RCT085 goes; nothing true was dropped.
- **They barely move the judgements.** Gemma's N on 5.1-5.3 fall from 19 to 7 and its Y from 8 to
  2, but to PN and PY: the measure, time point and analysis were each still quoted, and RoB 2's
  algorithm judges 5.1 PY with 5.2 and 5.3 PN low, as it does Y and N. So the lows that remain rest
  on what counts as the paper's word on a plan.
- **A planning check settles that** (E′p, F3p). The facts' definitions say "as set in the
  registration, protocol or plan, or described as prespecified"; the check holds them to it: one
  of a measure's, time point's or analysis's quotes must say it was set in advance (prespecified,
  planned, a priori, the protocol or registration, a primary or secondary outcome, end point or
  analysis). It drops 87 facts under F3, nearly all methods text that only describes what was
  done ("Statistical analysis was performed by IBM SPSS", "Assessments were performed at baseline,
  7- and 12-weeks follow-up", "The χ2 test and Fisher's exact test were used").

| All four models give the same D5 (of 20 paper-runs, sample 1 / sample 2) | E′ | E′p | F3 | F3p |
|---|---|---|---|---|
| | 12 / 18 | 15 / 19 | 9 / 15 | 15 / 17 |

Under F3p, Gemma's lows in sample 1 fall from 8 to 4 and gpt-oss 20B's from 5 and 3 to 1 each;
gpt-oss 120B keeps one false high (RCT065, a result both looks missed), and RCT035's real high
stays with DeepSeek and Gemma. D5 low then needs a plan stated as such, which is the strictness
asked for, and makes low rarer still than under C.

### Qwen3.8 27B (8-bit) on two papers (RCT025, RCT045)

One run per arm, through oMLX. 10-46 minutes per run; 14 of 20 domains failed; the 24 quotes it
gave were all found in the paper.

Qwen's failures are oMLX's JSON mode, not the model or bmlib (`scripts/probe_json_mode.py`, raw
replies in `data/.../json-mode-probe/`). The same D1 request with `response_format: json_object`
came back in 7 seconds as the five tokens `[1.1]`, with no reasoning; without it, after 5,778
reasoning tokens and 761 seconds, as a complete, well-founded assessment. The server constrains
decoding to JSON from the first token, which leaves a thinking model no room to think. bmlib
rejected every such reply ("expected a JSON object, got list"), so nothing wrong was stored.

## Findings

1. **Version 2 shifted the method on three questions; version 3 fixes them.**
   - **5.2 / 5.3 (selection of the reported result).** With no pre-specified plan to compare against,
     A answered N ("no analysis appears to be missing") where B answered NI: 9 of 20 runs on 5.3,
     5 of 20 on 5.2. That treats the absence of a visible alternative as evidence that nothing was
     selected. Version 3 makes the answers a ladder of evidence: N needs a plan the paper cites,
     dated before the data by group were seen, that fixes every point the result could have been
     chosen on; PN is that same plan known only from the paper's word; anything less, including a
     late registration, is NI. Under C the drift is gone in both samples. C now answers NI where B
     says N or PN in 2 to 6 of 20 runs per question and sample (5.3: 6 and 2; 5.2: 4 and 6), and
     the reverse is rare (the one N where B said NI is all-cause death, which the wording allows as
     "only one way to measure"). So C is stricter than the guidance's own text.
   - **1.3 (baseline imbalance).** A answered NI when the paper gave only a statement such as "no
     differences at baseline" (RCT047's table is an image, so its numbers never reach the text).
     Version 3 counts a statement as weaker evidence (PN); NI only when nothing is said.
   - **3.3 (could missingness depend on the true value).** A answered NI where B answered Y/PY.
     Version 3 says Y/PY whenever dropout could plausibly relate to the participants' health or the
     outcome. C follows it (PY where B says NI in 5 of 12 runs in sample 1).
2. **D5 is now almost never low.** Under C, 38 of 40 D5 judgements across both samples are some
   concerns (one high, one low); Cochrane rated 11 of 20 papers low. A paper reaches low D5 only by
   citing a dated plan that fixes every choice. This is the intended incentive, and it is the largest
   single source of disagreement with Cochrane.
3. **D3 is often high where Cochrane says low,** in every arm (sample 1: 11-12 of 20 runs high
   against no Cochrane high; sample 2: 10-12 against 2). The wording is not the cause. These papers
   are the obvious first candidates for a human reading.
4. **Agreement with Cochrane is driven by strictness, not by error.** In both samples the cells where
   the model disagrees with Cochrane are stricter far more often than more lenient (C: 58 : 14 and
   46 : 6). In sample 2, C agrees with Cochrane more often than B does (57% against 52%).
5. **Stability depends on the model.** On DeepSeek, C is the most stable wording (87% run-to-run in
   both samples; B fell from 88% to 80%). On gpt-oss it is the reverse (C 80% and 82%, B 88% and
   87%).
6. **Quote verification works, and it is what catches a sloppy quoter.** On DeepSeek, 99% or more of
   the quotes are found in the paper, on Gemma 4 92-99%. On gpt-oss only 75-84% are: about 60% of
   the misses join passages with "..." or "…", the rest are near-copies or table rows rebuilt from
   cells. Each miss loses its citation with a visible note, as intended. C had 2 contradicted quotes
   on DeepSeek sample 2, 2 on gpt-oss sample 1 (B 3) and 4 on Gemma sample 1; they are reported, not
   counted as citations. **Since then** bmlib splits an unlocated quote at its ellipses (commit
   `11719f77`) and locates each piece of three words or more on its own, each a citation of its own;
   a quote with a contradicted piece is contradicted. Replayed over the stored replies, it rescues
   about 96 of gpt-oss's 352 misses (5 come out contradicted) and changes nothing for DeepSeek or
   Gemma, which splice no quotes. The runs above predate it.
7. **The method changes replicate on gpt-oss.** C again answers NI on 5.2 and 5.3 where B answers N
   (N on 5.2: C 3 and 1 of 20 runs, B 6 and 8; on 5.3: C 3 and 1, B 6 and 6), and D5 is some
   concerns in 36 of 40 C judgements (DeepSeek: 38). Agreement with Cochrane is about the same for
   C and B (43% against 44%, 49% against 52%), the disagreements again mostly stricter.
8. **D3 differs between the models.** Under C, gpt-oss rates D3 high in 5 and 7 of 20 runs against
   14 and 11 under B, moving them to some concerns; DeepSeek rates D3 high about equally
   under both. Worth a look at how 3.3 and 3.4 read together.
9. **Reasoning models need room for their thinking.** One D2 reply failed in both A and B in sample 1
   with "response truncated at max_tokens=8192". Replayed (`scripts/probe_d2.py`), it used 8,651
   output tokens: about 36,500 characters of reasoning and a 3,457-character answer. DeepSeek counts
   reasoning against `max_tokens`. bmlib's new full-text agents now default to 16384, and no domain
   failed under C. **For StudyAnalyzer:** where a provider states no output limit (OpenAI, Mistral),
   StudyAnalyzer's reply sizing falls back to 4096, which is too little for such a model; the
   evidence-weight integration must pass the agents' default or a measured limit.
10. **Gemma 4 does not hold the 5.2 / 5.3 ladder.** Under C it answers N on 5.2 and on 5.3 in 10 of
    20 runs in sample 1 and 7 of 20 in sample 2, and D5 is then low; under B it answers NI in 38 of
    40. None of those N answers cites a dated plan. Their reasons are the version 2 failure ("no
    indication that one version was chosen") or the one exception N allows, "this outcome can be
    measured in only one way, for example death from any cause", stretched to an ordinal scale, a
    composite endpoint or any endpoint the methods name. DeepSeek and gpt-oss read the exception
    narrowly. Gemma also answers Y on 5.1 in 10 and 7 of 20 runs under C (v3; D was not run on it).
    So C's lenient cells (24 and 23) outnumber its stricter ones on Gemma, the reverse of the other
    models, and its higher agreement with Cochrane comes from that leniency. Gemma is also the most
    stable model (C: 100% and 93%), which here means consistently lenient.
11. **gpt-oss 20B is no substitute for the 120B.** On the same machine it is barely faster (270-331
    s per paper against 315-345) and much noisier: the same judgement on both runs in 69-75% of
    cells against 80-88%. Like Gemma it does not hold the 5.2 / 5.3 ladder: under C it answers N
    on 5.2 in 9 and 7 of 20 runs and on 5.3 in 8 and 8 (the 120B: 3 and 1 on each), and only 1 of
    those 32 N answers mentions a registration or protocol; the rest say "no evidence of selective
    reporting" or "only one version is reported", the version 2 failure. It also answers Y on 5.1
    in 6 and 5 of 20 runs. Its quoting under C is the weakest seen (65% and 81% found). Under B it
    answers N on 5.2 and 5.3 in 16-17 of 20 runs. So the ladder holds on the two large models and
    on neither small one: the wording asks for more reading than a small model gives it.

12. **Asking for facts instead of answers fixes the small models on D5** (arm E′ above): they stop
    answering N without a plan, the four models agree on D5 far more often, and the large models'
    judgements stay as strict as under C. The ladder is applied by code, so it holds whatever the
    model's reading of the answer definitions.

## Open points

- **Adopting arm E′ in bmlib** for D5 (and asking whether other domains have questions a small
  model answers by the headline). It needs: the facts and rules in Rust and Python with shared
  fixtures, "not reported" as arm F version 3 checks it, and a decision on the "measured in only
  one way" exception, which E′ drops (finding 10).
- **The planning check is adopted** (the maintainer's call, 2026-11-11): the paper's word on a
  plan needs the paper to say the measure, time point or analysis was set in advance; a methods
  description of what was done is not a plan. The design to take to bmlib is F3p: arm F version
  3's facts and "not reported" check, with the per-quote checks and the planning check in code.
- **What no quote check catches:** a result both looks miss (RCT065's 7-week UPDRS in a table).
- **Thinking models behind a JSON-constraining server.** bmlib now names the likely cause when a
  reply of the wrong shape is a few characters long (commit `11719f77`: the reply is quoted, with a
  hint to try another server or model). A per-model setting to send no `response_format` is left for
  later.
- **Human reading of the D3 and D5 disagreements,** which is what the StudyAnalyzer calibration
  workflow is for.

## Data

`data/2026-11-10-rob2-wording/`:

- `sample.json`, `sample2.json`: the papers, their assessed outcome and Cochrane's judgements.
- `results/`, `results2/`: one JSON per paper, arm and run (`RCT045_C1.json`), holding the full
  `Rob2Run` (answers, reasoning, verified quotes) or the error.
- `summary.json`, `summary2.json`: what `scripts/analyze.py` computes from them.
- `results-<model>/`, `results2-<model>/`, `summary-<model>.json`, `summary2-<model>.json`: the same
  for `gptoss120b`, `gptoss20b`, `gemma4-26b-a4b` and `qwen38-27b` (the last: RCT025 and RCT045
  only).
- `results-q51/`, `results2-q51/` (DeepSeek) and `results-gptoss120b-q51/`, `results2-gptoss120b-q51/`:
  arm D, one JSON per paper and run holding the D5 assessment alone (`"d5"`, not `"run"`);
  `analyze.py` does not read them, and the table above was counted from them directly.
- `json-mode-probe/`: Qwen3.8's raw D1 replies with and without JSON mode.
- `intermediate/`: C runs on two wordings between version 2 and the final version 3
  (`c_eb6c50dc_partial`, 11 runs; `c_34f3ce63`, 20 runs), kept for the record.
- `logs/`: the run logs.

The guidance text itself (wording B) is not committed (CC BY-NC-ND 4.0); `scripts/extract_guidance.py`
rebuilds it from your own copy of the PDF.
