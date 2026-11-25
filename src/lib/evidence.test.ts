import { describe, expect, test } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import type { EvidenceRunView, EvidenceView, LadderView, OutcomeView, ResultView } from "./api";
import {
  ANSWER_LABELS,
  CONDENSED_NOTE,
  DOMAIN_LABELS,
  JUDGEMENT_LABELS,
  LEVEL_LABELS,
  NO_PRIMARY_OUTCOME,
  VERDICT_LABELS,
  citationGroups,
  citeKeys,
  continueCaption,
  continueOffered,
  currentErrorNote,
  designLine,
  earlierRulesNote,
  evidenceMarks,
  judgementLabel,
  labelOf,
  ladderLine,
  notCoveredLine,
  paperWeightLine,
  resultLine,
  runningNote,
  secondLookNote,
  shouldAutoRun,
  tooLongNote,
} from "./evidence";

/** The fixture's assessed run: two outcomes, the second without a number, D3 failed. */
const assessed = fixture.evidence_run_assessed as EvidenceRunView;
/** The fixture's not-covered run (a prospective cohort study). */
const notCovered = fixture.evidence_run_not_covered as EvidenceRunView;

/** `assessed` with `over` laid on it. */
const runWith = (over: Partial<EvidenceRunView>): EvidenceRunView => ({ ...assessed, ...over });

/** The tab's view with `current` (or `currentError`), and `earlier` summaries in the statuses given. */
const view = (
  current: EvidenceRunView | null,
  earlier: Array<"complete" | "incomplete"> = [],
  currentError: string | null = null,
): EvidenceView => ({
  canonical_id: 1,
  current,
  current_error: currentError,
  earlier: earlier.map((status, i) => ({ id: 10 + i, canonical_id: 0, model: "m", created_at: "t", status })),
});

/** Every value of `key` anywhere inside `o`. */
function valuesOf(o: unknown, key: string): unknown[] {
  if (Array.isArray(o)) return o.flatMap((v) => valuesOf(v, key));
  if (o === null || typeof o !== "object") return [];
  return Object.entries(o).flatMap(([k, v]) => [...(k === key && typeof v !== "object" ? [v] : []), ...valuesOf(v, key)]);
}

describe("evidence labels", () => {
  test("each label map is the constraint's wording, exactly", () => {
    expect(JUDGEMENT_LABELS).toEqual({ low: "Low", some_concerns: "Some concerns", high: "High" });
    expect(ANSWER_LABELS).toEqual({
      Y: "Yes",
      PY: "Probably yes",
      PN: "Probably no",
      N: "No",
      NI: "No information",
      NA: "Not applicable",
    });
    expect(LEVEL_LABELS).toEqual({ high: "High", moderate: "Moderate", low: "Low", very_low: "Very low" });
    expect(VERDICT_LABELS).toEqual({
      present: "Present",
      not_claimed: "Not claimed",
      no_quote_found: "Claimed, but no quote was found in the paper",
      quote_does_not_support: "Claimed, but the quotes found do not show it",
      no_plan_stated: "Quoted, but not stated as set in advance",
    });
    expect(DOMAIN_LABELS).toEqual({
      D1: "D1 Randomisation",
      D2: "D2 Deviations",
      D3: "D3 Missing data",
      D4: "D4 Measurement",
      D5: "D5 Selection (facts)",
    });
    expect(CONDENSED_NOTE).toBe("Judged from a condensed paper");
    expect(NO_PRIMARY_OUTCOME).toBe("No primary outcome was found in the paper.");
  });

  test("every judgement, answer, level, verdict, domain and design in the contract fixture has a label", () => {
    const runs = [assessed, notCovered];
    const judgements = valuesOf(runs, "judgement").concat(valuesOf(runs, "overall")).filter((j) => j !== null);
    expect(judgements.length).toBeGreaterThan(0);
    for (const j of judgements) expect(Object.keys(JUDGEMENT_LABELS)).toContain(j);
    for (const a of valuesOf(runs, "answer")) expect(Object.keys(ANSWER_LABELS)).toContain(a);
    const levels = runs.flatMap((r) => r.outcomes.flatMap((o) => (o.ladder ? [o.ladder.start, o.ladder.level] : [])));
    expect(levels.length).toBeGreaterThan(0);
    for (const l of levels) expect(Object.keys(LEVEL_LABELS)).toContain(l);
    for (const v of valuesOf(runs, "verdict")) expect(Object.keys(VERDICT_LABELS)).toContain(v);
    for (const d of valuesOf(runs, "domain")) expect(Object.keys(DOMAIN_LABELS)).toContain(d);
    for (const r of runs) {
      const d = r.design!;
      expect(designLine(d.design, d.rct_layout)).not.toBe(d.design);
    }
  });

  test("null is Not assessed, and a value this version does not know shows as Rust sent it", () => {
    expect(judgementLabel(null)).toBe("Not assessed");
    expect(judgementLabel("some_concerns")).toBe("Some concerns");
    expect(judgementLabel("dire")).toBe("dire");
    expect(labelOf(ANSWER_LABELS, "PN")).toBe("Probably no");
    expect(labelOf(ANSWER_LABELS, "MAYBE")).toBe("MAYBE");
  });

  test("a randomised trial names its layout; every other design has its own label", () => {
    expect(designLine("rct", "parallel")).toBe("Randomised trial, parallel groups");
    expect(designLine("rct", "cluster")).toBe("Randomised trial, cluster");
    expect(designLine("rct", "crossover")).toBe("Randomised trial, crossover");
    expect(designLine("rct", "other")).toBe("Randomised trial, other layout");
    expect(designLine("rct", null)).toBe("Randomised trial");
    expect(designLine("cohort_prospective", null)).toBe("Prospective cohort study");
    expect(designLine("cohort_retrospective", null)).toBe("Retrospective cohort study");
    expect(designLine("case_control", null)).toBe("Case-control study");
    expect(designLine("systematic_review", null)).toBe("Systematic review");
    expect(designLine("something_new", null)).toBe("something_new");
  });
});

describe("an outcome's lines", () => {
  const result: ResultView = {
    measure: "risk ratio",
    scale: "ratio",
    estimate: 0.7,
    ci_low: 0.55,
    ci_high: 0.89,
    ci_level: 0.95,
    n_analysed: 1200,
  };

  test("the result reads in full, its missing parts left out", () => {
    expect(resultLine(result)).toBe("risk ratio 0.7 (95% CI 0.55 to 0.89), n 1200");
    expect(resultLine({ ...result, ci_level: null })).toBe("risk ratio 0.7 (CI 0.55 to 0.89), n 1200");
    expect(resultLine({ ...result, ci_low: null, n_analysed: null })).toBe("risk ratio 0.7");
    expect(resultLine({ ...result, n_analysed: null })).toBe("risk ratio 0.7 (95% CI 0.55 to 0.89)");
    expect(resultLine({ ...result, estimate: -0.4, ci_low: -0.7, ci_high: -0.1, ci_level: 0.9 })).toBe(
      "risk ratio -0.4 (90% CI -0.7 to -0.1), n 1200",
    );
  });

  test("the ladder reads from its start to its level, with every step's reason", () => {
    const ladder = (assessed.outcomes[0] as OutcomeView).ladder!;
    expect(ladderLine(ladder)).toBe("High → Moderate (risk of bias: Some concerns)");
    const long: LadderView = {
      start: "high",
      level: "very_low",
      weight: 0.1,
      steps: [
        { reason: "risk_of_bias", judgement: "high", n: null, threshold: null, levels: 2 },
        { reason: "crosses_no_effect", judgement: null, n: null, threshold: null, levels: 1 },
        { reason: "small_trial", judgement: null, n: 40, threshold: 100, levels: 1 },
        { reason: "no_interval", judgement: null, n: null, threshold: null, levels: 1 },
        { reason: "sample_size_unknown", judgement: null, n: null, threshold: null, levels: 0 },
        { reason: "novel_reason", judgement: null, n: null, threshold: null, levels: 1 },
      ],
    };
    expect(ladderLine(long)).toBe(
      "High → Very low (risk of bias: High; the interval crosses no effect; small trial: 40 analysed, under 100; " +
        "no interval reported; number analysed not reported (no step); novel_reason)",
    );
    expect(ladderLine({ ...long, level: "high", steps: [] })).toBe("High → High");
    const bare = { reason: "small_trial", judgement: null, n: null, threshold: null, levels: 1 };
    expect(ladderLine({ ...long, level: "moderate", steps: [bare] })).toBe("High → Moderate (small trial)");
  });

  test("a failed second look says D5 may be too lenient", () => {
    expect(secondLookNote({ label: "at 6 weeks", error: "busy" })).toBe(
      "The second look for “at 6 weeks” failed: busy. D5 may be more lenient than the paper warrants.",
    );
  });
});

describe("a run's lines", () => {
  test("the paper weight: the number, not complete, or none weighed; none for a run not assessed", () => {
    expect(paperWeightLine(assessed)).toBe("Paper weight: not complete (1 of 2 outcomes)");
    expect(paperWeightLine(runWith({ status: "complete", paper_weight: 0.7 }))).toBe("Paper weight 0.7");
    expect(paperWeightLine(runWith({ status: "complete", paper_weight: null, assessed_outcomes: 0 }))).toBe(
      "Paper weight: no outcome could be weighed",
    );
    expect(paperWeightLine(notCovered)).toBeNull();
    expect(paperWeightLine(runWith({ kind: "no_primary_outcome", outcomes: [] }))).toBeNull();
  });

  test("not covered names the design; earlier rules say Re-run", () => {
    expect(notCoveredLine(notCovered.design!)).toBe("No method for this design yet: Prospective cohort study");
    expect(notCoveredLine({ ...notCovered.design!, design: "rct", rct_layout: "crossover" })).toBe(
      "No method for this design yet: Randomised trial, crossover",
    );
    expect(earlierRulesNote(assessed)).toBeNull();
    expect(earlierRulesNote(runWith({ earlier_rules: true }))).toBe(
      "Made under earlier rules; Re-run for the current ones.",
    );
  });

  test("while a run goes: the step begun, a condensation in PICO's words, or the stop", () => {
    expect(runningNote(null, false)).toBe(
      "Assessing the evidence weight… A trial is assessed step by step and can take several minutes.",
    );
    expect(runningNote(fixture.evidence_step as never, false)).toBe("Study design");
    expect(runningNote({ kind: "step", label: "Outcome 1 of 2: D3, missing outcome data" }, false)).toBe(
      "Outcome 1 of 2: D3, missing outcome data",
    );
    expect(runningNote({ kind: "condensing", batch: 1, batches: 2, level: 0 }, false)).toBe(
      "Condensing the long paper, part 1 of 2…",
    );
    expect(runningNote({ kind: "condensed" }, false)).toBe("Reading the condensed paper…");
    expect(runningNote({ kind: "other", stage: "s", message: "m" }, false)).toBe(
      "Condensing… m (stage “s”, not known to this version)",
    );
    expect(runningNote({ kind: "step", label: "Study design" }, true)).toBe("Stopping…");
  });
});

describe("evidence marks", () => {
  test("every group of citations has its own key, and the components' keys name them", () => {
    const keys = citationGroups(assessed).map((g) => g.key);
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys).toContain(citeKeys.design);
    expect(keys).toContain(citeKeys.outcome(0));
    const d2 = citeKeys.domain(0, "D2");
    expect(keys).toContain(citeKeys.answer(d2, "2.1"));
    expect(keys).toContain(citeKeys.answer(citeKeys.domain(null, "D1"), "1.1"));
  });

  test("with a citation in focus, its group is marked, that one current", () => {
    const marks = evidenceMarks(assessed, { key: citeKeys.design, index: 0 });
    expect(marks).toEqual([{ span: assessed.design!.quotes.citations[0].span, current: true }]);
  });

  test("with none in focus, every citation of the run is marked, none current", () => {
    const marks = evidenceMarks(assessed, null);
    const total = citationGroups(assessed).reduce((n, g) => n + g.citations.length, 0);
    expect(marks.length).toBe(total);
    expect(marks.every((m) => !m.current)).toBe(true);
    expect(evidenceMarks(notCovered, null).length).toBe(1);
  });
});

describe("when a run starts or continues", () => {
  test("auto-run only with no run on the current text, a model, and no attempt yet", () => {
    expect(shouldAutoRun(view(null), false, true)).toBe(true);
    expect(shouldAutoRun(view(null), true, true)).toBe(false);
    expect(shouldAutoRun(view(null), false, false)).toBe(false);
    expect(shouldAutoRun(view(assessed), false, true)).toBe(false);
    // An earlier run left incomplete holds it back: Continue can finish it.
    expect(shouldAutoRun(view(null, ["complete", "incomplete"]), false, true)).toBe(false);
    expect(shouldAutoRun(view(null, ["complete"]), false, true)).toBe(true);
    // A newest run that cannot be read holds it back: the user sees why first (#165).
    expect(shouldAutoRun(view(null, [], "evidence run 3: x"), false, true)).toBe(false);
  });

  test("a newest run that cannot be read says why and that a new run can start", () => {
    expect(currentErrorNote("evidence run 3: x")).toBe(
      "The latest evidence weight run cannot be read: evidence run 3: x. It is kept; a new run can be started.",
    );
  });

  test("Continue only for an incomplete run while none goes, never under earlier rules", () => {
    expect(continueOffered(assessed, false)).toBe(true);
    expect(continueOffered(assessed, true)).toBe(false);
    expect(continueOffered(runWith({ status: "complete" }), false)).toBe(false);
    expect(continueOffered(runWith({ earlier_rules: true }), false)).toBe(false);
    expect(continueOffered(runWith({ too_long: true }), false)).toBe(false);
    expect(continueOffered(null, false)).toBe(false);
  });

  test("a run stopped as too long for its window says so, Re-run only (#166)", () => {
    expect(tooLongNote(assessed)).toBeNull();
    expect(tooLongNote(runWith({ too_long: true }))).toBe(
      "anthropic:claude's context window leaves no room for the paper beside this run's outcomes, so it cannot be continued. Re-run with a model with a larger window.",
    );
  });

  test("Continue names the run's model when another is chosen", () => {
    expect(continueCaption(assessed, "anthropic:claude")).toBe("Continue");
    expect(continueCaption(assessed, "ollama:b")).toBe("Continue with anthropic:claude");
  });
});
