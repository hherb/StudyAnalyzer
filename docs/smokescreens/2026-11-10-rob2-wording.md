# RoB 2 question wording: bmlib's own against the guidance's

**Date:** 2026-11-10 · **bmlib:** branch `feat/evidence-weight` (prompt version 3 at commit `236d403c`)
· **Model:** `deepseek:deepseek-flash`, temperature 0.1 · **Data:** [data/2026-11-10-rob2-wording/](data/2026-11-10-rob2-wording/)
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
5. **C is the most stable wording.** Its run-to-run agreement is 87% in both samples; B's fell from
   88% to 80% in sample 2.
6. **Quote verification works.** 99% or more of the model's quotes are found in the paper in every
   arm. C had 2 quotes the paper contradicts in sample 2; they are reported, not counted as citations.
7. **Reasoning models need room for their thinking.** One D2 reply failed in both A and B in sample 1
   with "response truncated at max_tokens=8192". Replayed (`scripts/probe_d2.py`), it used 8,651
   output tokens: about 36,500 characters of reasoning and a 3,457-character answer. DeepSeek counts
   reasoning against `max_tokens`. bmlib's new full-text agents now default to 16384, and no domain
   failed under C. **For StudyAnalyzer:** where a provider states no output limit (OpenAI, Mistral),
   StudyAnalyzer's reply sizing falls back to 4096, which is too little for such a model; the
   evidence-weight integration must pass the agents' default or a measured limit.

## Open points

- **5.1 leans lenient in sample 2.** C answered PY where B said NI in 3 of 20 runs. 5.1 was not
  reworded in this round; worth a look with the same "evidence, not assumption" eye.
- **Wording against a second model.** Everything here is one model. A run with a local model
  (gemma4:31b or qwen3.8:27b) would show whether the effects are the wording's or this model's.
- **Human reading of the D3 and D5 disagreements,** which is what the StudyAnalyzer calibration
  workflow is for.

## Data

`data/2026-11-10-rob2-wording/`:

- `sample.json`, `sample2.json`: the papers, their assessed outcome and Cochrane's judgements.
- `results/`, `results2/`: one JSON per paper, arm and run (`RCT045_C1.json`), holding the full
  `Rob2Run` (answers, reasoning, verified quotes) or the error.
- `summary.json`, `summary2.json`: what `scripts/analyze.py` computes from them.
- `intermediate/`: C runs on two wordings between version 2 and the final version 3
  (`c_eb6c50dc_partial`, 11 runs; `c_34f3ce63`, 20 runs), kept for the record.
- `logs/`: the run logs.

The guidance text itself (wording B) is not committed (CC BY-NC-ND 4.0); `scripts/extract_guidance.py`
rebuilds it from your own copy of the PDF.
