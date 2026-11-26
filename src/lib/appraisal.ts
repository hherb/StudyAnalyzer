// The reader's own RoB 2 appraisal in the Evidence weight tab (design 2026-11-26, §3–§6): which mode
// the tab opens in, when the paired machine run goes, its neutral line, the answer chips, and the
// draft of steps 1 and 2 checked before it is sent (the core checks it again). Nothing here reads
// what the machine judged before the reveal: a hidden run is counts and neutral failures only.
import type {
  AppraisalEvent,
  AppraisalSummary,
  AppraisalView,
  ConfirmInput,
  EvidenceView,
  MachineView,
  ProposalsView,
  ProposedOutcomeView,
  QuestionView,
  ReaderDomainView,
  ResultView,
  Utf16Span,
} from "./api";
import {
  ANSWER_LABELS,
  DESIGN_LABELS,
  RCT_LAYOUT_LABELS,
  citeKeys,
  designLine,
  evidenceMarks,
  labelOf,
  marksOf,
  type EvidenceCite,
} from "./evidence";

/** The appraisal's fixed words (plan 3's Global Constraints, verbatim). */
export const APPRAISAL_LABELS = {
  /** The choice on a paper never shown: appraise blind first. */
  appraiseFirst: "Appraise it yourself first",
  /** The choice on a paper never shown: phase 2's tab. */
  showMachine: "Show the machine's verdict",
  /** The machine's tab on a shown paper: a not-blind appraisal. */
  appraise: "Appraise it yourself",
  /** The header of a blind appraisal. */
  blind: "Your appraisal (blind)",
  /** The header of one that is not. */
  notBlind: "Your appraisal (not blind)",
  /** The steps across the top. */
  steps: ["1 Design", "2 Outcomes", "3 Your assessment", "4 Compare"],
  /** The reveal before a save. */
  revealNow: "Show the machine's verdict now",
  /** What the reveal asks first, on a blind appraisal. */
  revealWarning: "Your appraisal will no longer count as blind.",
  /** The reveal's yes. */
  revealYes: "Show it",
  /** The reveal's no. */
  revealNo: "Keep answering",
  /** The save, once complete. */
  save: "Save and compare",
  /** A new paired run on the confirmed outcomes. */
  runAgain: "Run the machine again on these outcomes",
  /** The escalation question. */
  escalation:
    "Several domains raise some concerns. Do they together lower confidence enough to judge the risk of bias high?",
  /** Before the sources ticks. */
  sources: "I also used:",
  /** A version made under earlier rules. */
  earlierRules: "Made under earlier rules.",
} as const;

/** Each sources tick's field and label, after `APPRAISAL_LABELS.sources`. */
export const SOURCES = [
  { field: "protocol", label: "protocol" },
  { field: "registry", label: "registry" },
  { field: "supplement", label: "supplement" },
] as const;

/** What a stored answer to a question today's rules no longer have says (it is shown, never offered chips). */
export const UNKNOWN_QUESTION = "An answer to a question this version of the rules no longer has.";

/** The answers in RoB 2's order, as the chips show them. */
const ANSWER_ORDER = ["Y", "PY", "PN", "N", "NI"];

/** Each result scale bmlib knows, with its label. */
export const SCALE_LABELS: Record<string, string> = { ratio: "Ratio", difference: "Difference" };

/** A CI level is stored as a fraction and edited as a percentage. */
const PERCENT = 100;

/** The machine line's opening words, which every form of it starts with. */
const MACHINE = "The machine's assessment";

/** Why a paired run too long for its window cannot go on. */
const TOO_LONG_REASON =
  "the paper does not fit the model's context window beside these outcomes. Raise the context window limit in Settings, or run the machine again on these outcomes with a model with a larger window.";

/** Why a paired run made under earlier rules cannot go on. */
const EARLIER_RULES_REASON = "it was made under earlier rules.";

/** Which side of the Evidence tab shows: the choice, the reader's appraisal, or phase 2's machine runs. */
export type TabMode = "choice" | "appraisal" | "machine";

/**
 * Which mode the tab opens in: `appraisal` while the paper has an open (not
 * set aside) appraisal; else `choice` when no run of the paper was shown
 * (even with an unshown run made before this phase); else `machine`.
 */
export function tabMode(view: EvidenceView): TabMode {
  if (view.appraisal !== null && !view.appraisal.set_aside) return "appraisal";
  return view.any_shown ? "machine" : "choice";
}

/** Appraisal `a` as the tab's opening view names it. */
export function summaryOf(a: AppraisalView): AppraisalSummary {
  return { id: a.id, state: a.state, blind: a.blind, set_aside: a.set_aside_at !== null };
}

/** Whether the paired run has steps not done, or failed ones. */
function pending(m: MachineView): boolean {
  return m.done_steps < m.total_steps || m.failed.length > 0;
}

/**
 * Whether the paired run waits for the reader's confirmation: its design and
 * outcome proposals done, the appraisal not confirmed yet. Steps 1 and 2 can
 * be confirmed only then.
 */
export function waitingForConfirmation(a: AppraisalView): boolean {
  return a.state === "confirming" && a.machine.run_id !== null && !pending(a.machine);
}

/**
 * Whether the paired run is started on its own (on opening, after starting or
 * confirming): when none runs, either there is no paired run yet, or it has
 * pending or failed steps under the current rules, not stopped as too long,
 * with the slot free. A run waiting for confirmation has none pending.
 */
export function needsRun(a: AppraisalView, running: boolean): boolean {
  if (running) return false;
  const m = a.machine;
  if (m.run_id === null) return true;
  return pending(m) && !m.earlier_rules && !m.too_long && !m.running_elsewhere;
}

/** Whether *Continue* is offered for the paired run: none runs, and it can go on as it is. */
export function continueOffered(a: AppraisalView, running: boolean): boolean {
  const m = a.machine;
  return !running && (m.run_id === null || pending(m)) && !m.earlier_rules && !m.too_long;
}

/** Whether *Run the machine again on these outcomes* is offered: a confirmed run that cannot go on. */
export function runAgainOffered(a: AppraisalView, running: boolean): boolean {
  const m = a.machine;
  return !running && a.state !== "confirming" && pending(m) && (m.earlier_rules || m.too_long);
}

/**
 * The paired run's one neutral line, never naming a step: its progress (from
 * `event` while it runs, else the view's counts), that it waits for another
 * paper's run, or why it stopped (the stored neutral messages, each once).
 */
export function machineLine(m: MachineView, event?: AppraisalEvent): string {
  if (event) return `${MACHINE}: ${event.done} of ${event.total} steps`;
  if (m.running_elsewhere) return `${MACHINE} waits for another paper's run.`;
  const stopped = (reason: string) => `${MACHINE} stopped: ${reason}`;
  if (m.failed.length) return stopped([...new Set(m.failed.map((f) => f.error))].join(" "));
  if (pending(m) && m.too_long) return stopped(TOO_LONG_REASON);
  if (pending(m) && m.earlier_rules) return stopped(EARLIER_RULES_REASON);
  return `${MACHINE}: ${m.done_steps} of ${m.total_steps} steps`;
}

/** The step the appraisal is at, from 1: confirming (or a design with no method) 1, answering 3, saved 4. */
export function currentStep(a: AppraisalView): number {
  if (a.state === "saved") return 4;
  return a.state === "answering" ? 3 : 1;
}

/** The chips question `q` offers: the answers it allows, in RoB 2's order, with plan 2's labels. */
export function answerChoices(q: QuestionView): Array<{ answer: string; label: string }> {
  return ANSWER_ORDER.filter((a) => q.allowed.includes(a)).map((answer) => ({
    answer,
    label: labelOf(ANSWER_LABELS, answer),
  }));
}

/** How many of domain `d`'s questions apply (today's rules asking them), and how many of those are answered. */
export function domainProgress(d: ReaderDomainView): { answered: number; applicable: number } {
  const applicable = d.questions.filter((q) => q.known && q.applicable);
  return { answered: applicable.filter((q) => q.answer !== null).length, applicable: applicable.length };
}

/**
 * The paper's marks in the appraisal: before the reveal the proposals'
 * citations (design, then each proposed outcome), as the run's are marked;
 * once revealed, the machine's run's own (`evidenceMarks`).
 */
export function appraisalMarks(
  a: AppraisalView,
  current: EvidenceCite | null,
): Array<{ span: Utf16Span; current: boolean }> {
  if (a.revealed_at !== null && a.machine.revealed) return evidenceMarks(a.machine.revealed, current);
  const p = a.proposals;
  if (!p) return [];
  return marksOf(
    [
      { key: citeKeys.design, citations: p.design.quotes.citations },
      ...p.outcomes.map((o, i) => ({ key: citeKeys.outcome(i), citations: o.quotes.citations })),
    ],
    current,
  );
}

/** Every design the reader may confirm: each randomised layout by name, then every other design. */
export function designChoices(): Array<{ design: string; rct_layout: string | null; label: string }> {
  return [
    ...Object.keys(RCT_LAYOUT_LABELS).map((l) => ({ design: "rct", rct_layout: l, label: designLine("rct", l) })),
    ...Object.keys(DESIGN_LABELS).map((d) => ({ design: d, rct_layout: null, label: designLine(d, null) })),
  ];
}

/** A result as the reader edits it: every field as typed, the CI level as a percentage. */
export interface ResultDraft {
  /** The measure's name. */
  measure: string;
  /** `ratio` or `difference`. */
  scale: string;
  /** The point estimate. */
  estimate: string;
  /** The interval's lower bound. */
  ci_low: string;
  /** The interval's upper bound. */
  ci_high: string;
  /** The interval's level, in percent (`95`). */
  ci_level: string;
  /** How many were analysed. */
  n_analysed: string;
}

/** One outcome as the reader edits it in step 2. */
export interface OutcomeDraft {
  /** The proposed outcome it came from; null for one the reader added. */
  from_proposed: number | null;
  /** Its name. */
  name: string;
  /** When it was measured; empty for none. */
  timepoint: string;
  /** Its result; every field empty for none. */
  result: ResultDraft;
}

/** Steps 1 and 2 as the reader edits them. */
export interface ConfirmDraft {
  /** The design's bmlib name. */
  design: string;
  /** A randomised trial's layout; null for any other design. */
  rct_layout: string | null;
  /** The outcomes, in the order they are appraised. */
  outcomes: OutcomeDraft[];
}

/** The number fields of a result draft, checked as numbers. */
const NUMBER_FIELDS = ["estimate", "ci_low", "ci_high", "ci_level", "n_analysed"] as const;

/** `n` as a field shows it; null is empty. */
const numText = (n: number | null | undefined) => (n === null || n === undefined ? "" : String(n));

/** Field text `s` as a number; empty is null, anything else not a number is NaN. */
const num = (s: string): number | null => (s.trim() === "" ? null : Number(s.trim()));

/**
 * Proposed outcome `p` (number `from` among the proposals) as a draft, or
 * with `p` null a blank one the reader adds. A level shows as a percentage,
 * without float noise and never rounded.
 */
export function outcomeDraft(p: ProposedOutcomeView | null, from: number | null): OutcomeDraft {
  const r = p?.result ?? null;
  return {
    from_proposed: from,
    name: p?.name ?? "",
    timepoint: p?.timepoint ?? "",
    result: {
      measure: r?.measure ?? "",
      scale: r?.scale ?? Object.keys(SCALE_LABELS)[0],
      estimate: numText(r?.estimate),
      ci_low: numText(r?.ci_low),
      ci_high: numText(r?.ci_high),
      // `toFixed` then `+` drops float noise (0.95 * 100), as `resultLine` does.
      ci_level: r?.ci_level === null || r?.ci_level === undefined ? "" : String(+(r.ci_level * PERCENT).toFixed(6)),
      n_analysed: numText(r?.n_analysed),
    },
  };
}

/** The draft steps 1 and 2 start from: the machine's proposed design and outcomes. */
export function confirmDraft(p: ProposalsView): ConfirmDraft {
  return {
    design: p.design.design,
    rct_layout: p.design.rct_layout,
    outcomes: p.outcomes.map((o, i) => outcomeDraft(o, i)),
  };
}

/** Whether the reader gave a result: any field but the scale (which always has one) filled. */
function resultGiven(r: ResultDraft): boolean {
  return [r.measure, ...NUMBER_FIELDS.map((f) => r[f])].some((s) => s.trim() !== "");
}

/**
 * What stops draft `d` from being confirmed, in the reader's words, checked
 * before sending: no outcome, an outcome with no name, a result without its
 * measure or estimate, a number that is not one, or an interval the wrong way
 * round. Empty when it may be sent; the core checks the rest.
 */
export function confirmProblems(d: ConfirmDraft): string[] {
  if (d.outcomes.length === 0) return ["Confirm at least one primary outcome."];
  const problems: string[] = [];
  d.outcomes.forEach((o, i) => {
    const n = i + 1;
    if (o.name.trim() === "") problems.push(`Outcome ${n} needs a name.`);
    const r = o.result;
    if (!resultGiven(r)) return;
    if (r.measure.trim() === "" || r.estimate.trim() === "")
      problems.push(`Outcome ${n}: a result needs its measure and its estimate.`);
    for (const f of NUMBER_FIELDS) {
      const value = num(r[f]);
      if (value !== null && !Number.isFinite(value)) problems.push(`Outcome ${n}: “${r[f].trim()}” is not a number.`);
    }
    const low = num(r.ci_low);
    const high = num(r.ci_high);
    if (low !== null && high !== null && low > high)
      problems.push(`Outcome ${n}: the interval's lower bound is above its upper bound.`);
  });
  return problems;
}

/** Draft result `r` as sent, or null when the reader gave none. Assumes `confirmProblems` found nothing. */
function resultInput(r: ResultDraft): ResultView | null {
  if (!resultGiven(r)) return null;
  const level = num(r.ci_level);
  return {
    measure: r.measure.trim(),
    scale: r.scale,
    estimate: num(r.estimate) ?? Number.NaN,
    ci_low: num(r.ci_low),
    ci_high: num(r.ci_high),
    ci_level: level === null ? null : level / PERCENT,
    n_analysed: num(r.n_analysed),
  };
}

/** Draft `d` as `confirm_appraisal` takes it; send it only once `confirmProblems(d)` is empty. */
export function confirmInput(d: ConfirmDraft): ConfirmInput {
  return {
    design: d.design,
    rct_layout: d.rct_layout,
    outcomes: d.outcomes.map((o) => ({
      from_proposed: o.from_proposed,
      name: o.name.trim(),
      timepoint: o.timepoint.trim() === "" ? null : o.timepoint.trim(),
      result: resultInput(o.result),
    })),
  };
}
