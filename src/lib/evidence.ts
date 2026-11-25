// What the Evidence weight tab shows and when it runs (design 2026-11-24, §7): labels, lines, notes,
// marks and the run's lifecycle checks. The helpers PRISMA shares (Continue's caption, the condensed
// note, *Not assessed*) are PRISMA's own; the condensation wording is PICO's.
import type {
  DesignView,
  DomainView,
  EvidenceEvent,
  EvidenceRunView,
  EvidenceView,
  LadderView,
  NamedErrorView,
  PrismaCitationView,
  ResultView,
  Utf16Span,
} from "./api";
import { runningNote as picoRunningNote } from "./pico";
import { NOT_ASSESSED_LABEL } from "./prisma";

export { CONDENSED_NOTE, continueCaption, continueHint } from "./prisma";

/** Each RoB 2 judgement's label on its chip. */
export const JUDGEMENT_LABELS: Record<string, string> = {
  low: "Low",
  some_concerns: "Some concerns",
  high: "High",
};

/** Each signalling-question answer's label. */
export const ANSWER_LABELS: Record<string, string> = {
  Y: "Yes",
  PY: "Probably yes",
  PN: "Probably no",
  N: "No",
  NI: "No information",
  NA: "Not applicable",
};

/** Each evidence ladder level's label. */
export const LEVEL_LABELS: Record<string, string> = {
  high: "High",
  moderate: "Moderate",
  low: "Low",
  very_low: "Very low",
};

/** Each D5 fact verdict's label: whether the fact counts, and if not, why. */
export const VERDICT_LABELS: Record<string, string> = {
  present: "Present",
  not_claimed: "Not claimed",
  no_quote_found: "Claimed, but no quote was found in the paper",
  quote_does_not_support: "Claimed, but the quotes found do not show it",
  no_plan_stated: "Quoted, but not stated as set in advance",
};

/** Each RoB 2 domain's short label in the outcome card (bmlib's long title is `DomainView.name`). */
export const DOMAIN_LABELS: Record<string, string> = {
  D1: "D1 Randomisation",
  D2: "D2 Deviations",
  D3: "D3 Missing data",
  D4: "D4 Measurement",
  D5: "D5 Selection (facts)",
};

/** Each design's label but a randomised trial's, which names its layout (`RCT_LAYOUT_LABELS`). */
const DESIGN_LABELS: Record<string, string> = {
  systematic_review: "Systematic review",
  meta_analysis: "Meta-analysis",
  cohort_prospective: "Prospective cohort study",
  cohort_retrospective: "Retrospective cohort study",
  case_control: "Case-control study",
  cross_sectional: "Cross-sectional study",
  case_series: "Case series",
  case_report: "Case report",
  guideline: "Guideline",
  editorial: "Editorial",
  letter: "Letter",
  comment: "Comment",
  other: "Other design",
  unknown: "Unknown design",
};

/** A randomised trial's label, before its layout's words. */
const RCT_LABEL = "Randomised trial";
/** Each randomised layout's words after `RCT_LABEL`. */
const RCT_LAYOUT_LABELS: Record<string, string> = {
  parallel: "parallel groups",
  cluster: "cluster",
  crossover: "crossover",
  other: "other layout",
};

/** What a run with no primary outcome says. */
export const NO_PRIMARY_OUTCOME = "No primary outcome was found in the paper.";

/** What a run made under earlier rules says (Rust's `evidence::EARLIER_RULES`). */
const EARLIER_RULES = "Made under earlier rules; Re-run for the current ones.";

/** `wire`'s label in `labels`; a value this version does not know shows as Rust sent it. */
export function labelOf(labels: Record<string, string>, wire: string): string {
  return labels[wire] ?? wire;
}

/** A judgement's label; null (a pending or failed domain, an outcome not judged) is *Not assessed*. */
export function judgementLabel(judgement: string | null): string {
  return judgement === null ? NOT_ASSESSED_LABEL : labelOf(JUDGEMENT_LABELS, judgement);
}

/** The design as the tab's first line reads it: a randomised trial with its layout, any other its own label. */
export function designLine(design: string, layout: string | null): string {
  if (design !== "rct") return labelOf(DESIGN_LABELS, design);
  return layout === null ? RCT_LABEL : `${RCT_LABEL}, ${labelOf(RCT_LAYOUT_LABELS, layout)}`;
}

/**
 * An outcome's reported effect, for example *risk ratio 0.7 (95% CI 0.55 to
 * 0.89), n 1200*. The interval shows only with both bounds; a missing level,
 * interval or count is left out. Numbers show as stored, never rounded.
 */
export function resultLine(r: ResultView): string {
  let line = `${r.measure} ${r.estimate}`;
  if (r.ci_low !== null && r.ci_high !== null) {
    // `toFixed` then `+` drops float noise (0.95 * 100) without rounding a stated level.
    const level = r.ci_level === null ? "" : `${+(r.ci_level * 100).toFixed(6)}% `;
    line += ` (${level}CI ${r.ci_low} to ${r.ci_high})`;
  }
  if (r.n_analysed !== null) line += `, n ${r.n_analysed}`;
  return line;
}

/** One ladder step's reason in words; one this version does not know shows as stored. */
function ladderReason(s: LadderView["steps"][number]): string {
  switch (s.reason) {
    case "risk_of_bias":
      return `risk of bias: ${judgementLabel(s.judgement)}`;
    case "crosses_no_effect":
      return "the interval crosses no effect";
    case "small_trial":
      // Rust refuses a small_trial step without both numbers; defensive, so "null" is never printed.
      return s.n === null || s.threshold === null
        ? "small trial"
        : `small trial: ${s.n} analysed, under ${s.threshold}`;
    case "no_interval":
      return "no interval reported";
    case "sample_size_unknown":
      return "number analysed not reported (no step)";
    default:
      return s.reason;
  }
}

/** The ladder from its start to its level, with every step's reason: *High → Moderate (risk of bias: Some concerns)*. */
export function ladderLine(l: LadderView): string {
  const line = `${labelOf(LEVEL_LABELS, l.start)} → ${labelOf(LEVEL_LABELS, l.level)}`;
  return l.steps.length ? `${line} (${l.steps.map(ladderReason).join("; ")})` : line;
}

/**
 * The paper-weight line of an assessed run: its number once complete, *not
 * complete* with how many outcomes were weighed, or that none could be. Null
 * for a run not assessed (not covered, no primary outcome, design unknown).
 */
export function paperWeightLine(run: EvidenceRunView): string | null {
  if (run.kind !== "assessed") return null;
  if (run.status !== "complete")
    return `Paper weight: not complete (${run.assessed_outcomes} of ${run.outcomes.length} outcomes)`;
  return run.paper_weight === null ? "Paper weight: no outcome could be weighed" : `Paper weight ${run.paper_weight}`;
}

/** What a run whose design has no method says, naming the design. */
export function notCoveredLine(design: DesignView): string {
  return `No method for this design yet: ${designLine(design.design, design.rct_layout)}`;
}

/** The note on a run made under earlier rules (Re-run only); null for one under today's. */
export function earlierRulesNote(run: EvidenceRunView): string | null {
  return run.earlier_rules ? EARLIER_RULES : null;
}

/**
 * What a run stopped because its outcomes leave no room for the paper in its
 * window says (#166). Changing the limit in Settings clears the mark, so
 * Continue is offered again.
 */
export function tooLongNote(run: EvidenceRunView): string | null {
  return run.too_long
    ? `The context window this run was sized for (${run.model}'s, or the limit set in Settings) leaves no room for the paper beside its outcomes. Raise the context window limit in Settings to continue it, or re-run with a model with a larger window.`
    : null;
}

/** What the tab says when its newest run cannot be read (#165): why, and that a new run is the way on. */
export function currentErrorNote(error: string): string {
  return `The latest evidence weight run cannot be read: ${error}. It is kept; a new run can be started.`;
}

/** The note on a D5 measurement whose second look failed, on D5 and beside the outcome's number. */
export function secondLookNote(failure: NamedErrorView): string {
  return `The second look for “${failure.label}” failed: ${failure.error}. D5 may be more lenient than the paper warrants.`;
}

/**
 * What the tab says while a run goes: the step begun (its progress label), how
 * far a condensation has got (PICO's words), or that the stop was pressed.
 * `event` is null before any arrives.
 */
export function runningNote(event: EvidenceEvent | null, stopping: boolean): string {
  if (stopping) return "Stopping…";
  if (event === null) return "Assessing the evidence weight… A trial is assessed step by step and can take several minutes.";
  if (event.kind === "step") return event.label;
  return picoRunningNote(event, false);
}

/** An evidence citation in focus: its group's key (`citeKeys`) and its place in the group. */
export type EvidenceCite = { key: string; index: number };

/**
 * The key of each group of citations, which the components and
 * `citationGroups` both build, so a click and its marks agree. A domain's key
 * prefixes its answers', facts', measurements' and analysis' keys.
 */
export const citeKeys = {
  /** The design's quotes. */
  design: "design",
  /** Outcome `index`'s quotes. */
  outcome: (index: number) => `o${index}`,
  /** Domain `domain` of outcome `outcome`; null for the trial's (D1). */
  domain: (outcome: number | null, domain: string) => `${outcome === null ? "trial" : `o${outcome}`}.${domain}`,
  /** The answer to question `id` of the domain keyed `domain`. */
  answer: (domain: string, id: string) => `${domain}.q${id}`,
  /** D5 fact `fact`'s quotes. */
  fact: (domain: string, fact: string) => `${domain}.f.${fact}`,
  /** D5 measurement `index`'s passages: those naming it, giving its result, or a second look's. */
  measurement: (domain: string, index: number, part: "named" | "result" | "look") => `${domain}.m${index}.${part}`,
  /** D5's analysis check's passages: the planned analysis, or the reported one. */
  analysis: (domain: string, part: "planned" | "reported") => `${domain}.a.${part}`,
};

/** Every group of citations of domain `d` (of outcome `outcome`, null for D1), with its key. */
function domainGroups(d: DomainView, outcome: number | null): Array<{ key: string; citations: PrismaCitationView[] }> {
  const k = citeKeys.domain(outcome, d.domain);
  return [
    ...d.answers.map((a) => ({ key: citeKeys.answer(k, a.question_id), citations: a.quotes.citations })),
    ...(d.facts ?? []).map((f) => ({ key: citeKeys.fact(k, f.fact), citations: f.quotes.citations })),
    ...d.measurements.flatMap((m, j) => [
      { key: citeKeys.measurement(k, j, "named"), citations: m.named },
      { key: citeKeys.measurement(k, j, "result"), citations: m.result },
      { key: citeKeys.measurement(k, j, "look"), citations: m.second_look ?? [] },
    ]),
    ...(d.analysis
      ? [
          { key: citeKeys.analysis(k, "planned"), citations: d.analysis.planned },
          { key: citeKeys.analysis(k, "reported"), citations: d.analysis.reported },
        ]
      : []),
  ];
}

/** Every group of the run's citations that has any, with its key: the design, D1, then each outcome and its domains. */
export function citationGroups(run: EvidenceRunView): Array<{ key: string; citations: PrismaCitationView[] }> {
  const groups = [
    ...(run.design ? [{ key: citeKeys.design, citations: run.design.quotes.citations }] : []),
    ...(run.study ? domainGroups(run.study, null) : []),
    ...run.outcomes.flatMap((o) => [
      { key: citeKeys.outcome(o.index), citations: o.quotes.citations },
      ...o.domains.flatMap((d) => domainGroups(d, o.index)),
    ]),
  ];
  return groups.filter((g) => g.citations.length > 0);
}

/**
 * The run's citations as marks on the paper. With one in focus, its group's
 * citations are marked, that one current (the current one dark, the others
 * pale); with none, every citation of the run, none current.
 */
export function evidenceMarks(
  run: EvidenceRunView,
  current: EvidenceCite | null,
): Array<{ span: Utf16Span; current: boolean }> {
  const groups = citationGroups(run).filter((g) => current === null || g.key === current.key);
  return groups.flatMap((g) =>
    g.citations.map((c, index) => ({ span: c.span, current: current !== null && index === current.index })),
  );
}

/**
 * Whether opening the tab starts a run: none on the current text, a model
 * chosen, and none attempted since the host mounted. An earlier run left
 * incomplete holds it back too, as PRISMA's does: Continue can finish it.
 * It does so even when that run was made under earlier rules or stopped as
 * too long, and Continue is refused; the *Assess* button is still offered
 * then. A newest run that cannot be read holds it back too: the user sees
 * why before a new run (#165).
 */
export function shouldAutoRun(view: EvidenceView, attempted: boolean, hasModel: boolean): boolean {
  return (
    view.current === null &&
    view.current_error === null &&
    hasModel &&
    !attempted &&
    !view.earlier.some((r) => r.status === "incomplete")
  );
}

/**
 * Whether *Continue* is offered for the run shown: it is incomplete, made
 * under today's rules (rules never mix within a run), it is not marked too
 * long for its window (#166), and no run goes. An
 * earlier run counts too, since Continue runs on the run's own text.
 */
export function continueOffered(run: EvidenceRunView | null, running: boolean): boolean {
  return run?.status === "incomplete" && !run.earlier_rules && !run.too_long && !running;
}
