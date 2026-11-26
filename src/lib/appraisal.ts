// The reader's own RoB 2 appraisal in the Evidence weight tab (design 2026-11-26, §3–§6): which mode
// the tab opens in, when the paired machine run goes, its neutral line, the answer chips, and the
// draft of steps 1 and 2 checked before it is sent (the core checks it again). Nothing here reads
// what the machine judged before the reveal: a hidden run is counts and neutral failures only.
import type {
  AppraisalEdit,
  AppraisalEvent,
  AppraisalSummary,
  AppraisalView,
  CompareView,
  ConfirmInput,
  DepartureView,
  EvidenceView,
  MachineView,
  ProposalsView,
  ProposedOutcomeView,
  QuestionView,
  ReaderDomainView,
  ReaderOutcomeView,
  ResultView,
  Utf16Span,
  VersionView,
} from "./api";
import { CREATED_AT_FORMAT, parseSqliteUtc } from "./interrogator";
import {
  ANSWER_LABELS,
  DESIGN_LABELS,
  RCT_LAYOUT_LABELS,
  citeKeys,
  designLine,
  evidenceMarks,
  judgementLabel,
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
  /** Its conformance, which today's rules do not check. */
  notChecked: "Not checked (earlier rules).",
  /** Closes the open revision. */
  doneRevising: "Done revising",
  /** Sets the appraisal aside. */
  setAside: "Set this appraisal aside",
  /** What setting it aside asks first. */
  setAsideWarning: "It is kept with every version, and you can start a new appraisal of this paper.",
  /** Setting it aside's yes. */
  setAsideYes: "Set it aside",
  /** Setting it aside's no. */
  setAsideNo: "Keep it",
  /** Arms the paper pane for the sentence an answer rests on. */
  markSentence: "Mark the sentence",
  /** Beside a question while the paper pane waits for its sentence. */
  marking: "Select the sentence in the paper, or cancel.",
  /** Shows the reader's marked sentence in the paper. */
  showMark: "Show your marked sentence",
  /** Clears the mark. */
  removeMark: "Remove the mark",
  /** Loads the latest version, so a change to it opens the next revision. */
  revise: "Revise",
  /** While a set aside waits for the paired run's stop. */
  stoppingForAside: "Stopping the machine's assessment…",
  /** The switch, once revealed: the reader's appraisal. */
  yourAppraisal: "Your appraisal",
  /** The switch, once revealed: phase 2's machine runs. */
  machineRuns: "Machine runs",
} as const;

/** A compare row's note where the two answers differ (text, never colour alone). */
export const DIFFERS = "Differs";

/** A compare side with no answer: the reader left the question open, or the machine has not answered it. */
export const NOT_ANSWERED = "Not answered";

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
 * set aside) appraisal, or one whose row cannot be read (so it can be set
 * aside); else `choice` when no run of the paper was shown (even with an
 * unshown run made before this phase); else `machine`.
 */
export function tabMode(view: EvidenceView): TabMode {
  if (view.appraisal_error !== null) return "appraisal";
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

/**
 * Whether *Run the machine again on these outcomes* is offered, once
 * confirmed on a covered design and not set aside, with nothing running:
 * for a run under earlier rules, finished or not (design §7: it is shown as
 * made, and only a new run brings today's rules), one stopped as too long,
 * and, so the reader can have it made with another model (design §4), one
 * with failed steps or one complete. A new run takes the model chosen in
 * Settings.
 */
export function runAgainOffered(a: AppraisalView, running: boolean): boolean {
  const m = a.machine;
  const confirmed = a.state === "answering" || a.state === "saved";
  if (running || !confirmed || a.set_aside_at !== null || m.run_id === null) return false;
  return m.earlier_rules || (pending(m) && m.too_long) || m.failed.length > 0 || !pending(m);
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

/**
 * What *Show the machine's verdict* does: serves the current run through its
 * own view, which records it shown (the opening view never marks a run
 * beside the choice); with none, starts the first run when `wantsRun`
 * (`shouldAutoRun` with the choice made); else nothing.
 */
export function machineChoice(view: EvidenceView, wantsRun: boolean): { serve: number } | "run" | null {
  if (view.current !== null) return { serve: view.current.id };
  return wantsRun ? "run" : null;
}

/**
 * An edit as the reader's state sends it: the edit itself, or how to build it
 * from the view as it stands when its turn comes, so a toggle made while an
 * earlier write is in flight builds on that write's result, never on a stale
 * view.
 */
export type EditFrom = AppraisalEdit | ((view: AppraisalView) => AppraisalEdit);

/** Where a question's edit goes: D1 with no outcome, D2–D5 with a confirmed outcome's index. */
export type QuestionAt = { domain: string; outcome: number | null; question_id: string };

/** The key of the reader's mark on the question at `at`, as an `EvidenceCite` key: apart from every machine key. */
export function markKey(at: QuestionAt): string {
  return `mine.${citeKeys.answer(citeKeys.domain(at.outcome, at.domain), at.question_id)}`;
}

/**
 * The reader's marked sentences in `a`'s version, outlined in the paper (never
 * filled, so they are told from the machine's quotes): every one, and the one
 * in focus (`current`, keyed by `markKey`) current.
 */
export function readerMarks(a: AppraisalView, current: EvidenceCite | null): Array<{ span: Utf16Span; current: boolean }> {
  const v = a.version;
  const scopes: Array<{ outcome: number | null; d: ReaderDomainView }> = [
    { outcome: null, d: v.study },
    ...v.outcomes.flatMap((o) => o.domains.map((d) => ({ outcome: o.index, d }))),
  ];
  return scopes.flatMap(({ outcome, d }) =>
    d.questions.flatMap((q) =>
      q.mark === null
        ? []
        : [{ span: q.mark, current: current?.key === markKey({ domain: d.domain, outcome, question_id: q.question_id }) }],
    ),
  );
}

/** A version's name in the picker: *First record*, then *After reveal {n}*. */
export function versionLabel(n: number): string {
  return n === 0 ? "First record" : `After reveal ${n}`;
}

/**
 * Whether the reader may change the version shown: an appraisal being
 * answered or saved, showing its latest version. That is the unsaved first
 * record, or after the save an open revision, or the frozen first record or a
 * closed revision, where an edit opens the next revision (Rust's `edit`) and
 * never writes the version shown. An older version is only read.
 * `VersionView.editable` says only whether an edit writes to it, so it is
 * false on the saved first record.
 */
export function versionEditable(a: AppraisalView): boolean {
  if (a.state !== "answering" && a.state !== "saved") return false;
  const latest = a.versions[a.versions.length - 1];
  return latest !== undefined && a.version.number === latest.number;
}

/**
 * The version *Revise* loads: on a saved appraisal showing a version that is
 * not its latest (as after *Done revising*, which shows the first record
 * again), the latest, where a change writes the open revision or opens the
 * next one. Null when the version shown can be changed already, or nothing can.
 */
export function reviseTarget(a: AppraisalView): number | null {
  const latest = a.versions[a.versions.length - 1];
  return a.state === "saved" && latest !== undefined && !versionEditable(a) ? latest.number : null;
}

/** Where the reader's next change goes, beside *Revise*: the open revision, else the next one it starts. */
export function nextChangeNote(a: AppraisalView): string {
  const latest = a.versions[a.versions.length - 1];
  if (latest === undefined) return "";
  return latest.frozen_at === null && latest.closed_at === null
    ? `Your next change goes to ${versionLabel(latest.number)}.`
    : `Your next change starts ${versionLabel(latest.number + 1)}.`;
}

/** Whether the version shown is an open revision, which *Done revising* closes. */
export function revisionOpen(a: AppraisalView): boolean {
  const v = a.version;
  return v.number > 0 && v.frozen_at === null && v.closed_at === null && versionEditable(a);
}

/** A verdict's departure from RoB 2's rules, for the reader or the machine (whose departure is a defect). */
export function departureLine(d: DepartureView, rater: "reader" | "machine"): string {
  const rules = judgementLabel(d.rules);
  return rater === "reader"
    ? `Your verdict departs from the rules: they give ${rules}.`
    : `The machine's verdict departs from the rules: they give ${rules}. This is a defect; please report it.`;
}

/** *Set aside {date}*, the date as the reader's locale writes it. */
export function setAsideLine(at: string): string {
  return `Set aside ${parseSqliteUtc(at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? at}`;
}

/** One compare row as the pane shows it: both answers in words, whether they differ, and the reader's mark. */
export interface CompareRow {
  /** The domain (`D1` … `D5`). */
  domain: string;
  /** RoB 2's number. */
  question_id: string;
  /** The reader's answer, *Not applicable* under their answers, or *Not answered*. */
  reader: string;
  /** The machine's answer, or *Not answered*. */
  machine: string;
  /** Whether both answered, differently (Rust's `differs`). */
  differs: boolean;
  /** The sentence the reader marked for it, or null. */
  mark: Utf16Span | null;
}

/**
 * Outcome `outcome`'s compare rows (D1's, then its D2–D5's) against version
 * `v`, which says why the reader has no answer (the question does not apply
 * under their answers, or they left it open) and carries their mark. An
 * outcome compare does not have gives none.
 */
export function compareRows(c: CompareView, v: VersionView, outcome: number): CompareRow[] {
  const rows = c.outcomes.find((o) => o.index === outcome)?.questions ?? [];
  const domainOf = (domain: string) =>
    domain === v.study.domain ? v.study : v.outcomes.find((o) => o.index === outcome)?.domains.find((d) => d.domain === domain);
  return rows.map((r) => {
    const q = domainOf(r.domain)?.questions.find((x) => x.question_id === r.question_id);
    const readerNone = q && q.known && !q.applicable ? labelOf(ANSWER_LABELS, "NA") : NOT_ANSWERED;
    return {
      domain: r.domain,
      question_id: r.question_id,
      reader: r.reader === null ? readerNone : labelOf(ANSWER_LABELS, r.reader),
      machine: r.machine === null ? NOT_ANSWERED : labelOf(ANSWER_LABELS, r.machine),
      differs: r.differs,
      mark: q?.mark ?? null,
    };
  });
}

/** Confirmed outcome `outcome` of `v`'s version, if it has one. */
function readerOutcome(v: AppraisalView, outcome: number): ReaderOutcomeView | undefined {
  return v.version.outcomes.find((o) => o.index === outcome);
}

/** Domain `domain` of `v`'s version: D1 with `outcome` null, else that outcome's. */
export function readerDomain(v: AppraisalView, domain: string, outcome: number | null): ReaderDomainView | undefined {
  if (outcome === null) return v.version.study.domain === domain ? v.version.study : undefined;
  return readerOutcome(v, outcome)?.domains.find((d) => d.domain === domain);
}

/** The question at `at` in `v`'s version, if it has it. */
function readerQuestion(v: AppraisalView, at: QuestionAt): QuestionView | undefined {
  return readerDomain(v, at.domain, at.outcome)?.questions.find((q) => q.question_id === at.question_id);
}

/** A chip press: answer `answer` to the question at `at`, or clear it if it is that answer already. */
export function answerToggle(at: QuestionAt, answer: string): (v: AppraisalView) => AppraisalEdit {
  return (v) => ({ kind: "answer", ...at, answer: readerQuestion(v, at)?.answer === answer ? null : answer });
}

/** A sources tick: `field` set to `checked`, the other ticks as stored. */
export function sourcesTick(
  at: QuestionAt,
  field: (typeof SOURCES)[number]["field"],
  checked: boolean,
): (v: AppraisalView) => AppraisalEdit {
  return (v) => {
    const q = readerQuestion(v, at);
    const ticks = { protocol: q?.protocol ?? false, registry: q?.registry ?? false, supplement: q?.supplement ?? false };
    return { kind: "sources", ...at, ...ticks, [field]: checked };
  };
}

/** A domain verdict chip: the reader's own verdict `judgement`, or cleared if it is that already. */
export function overrideToggle(
  domain: string,
  outcome: number | null,
  judgement: string,
): (v: AppraisalView) => AppraisalEdit {
  return (v) => ({
    kind: "override",
    domain,
    outcome,
    judgement: readerDomain(v, domain, outcome)?.override === judgement ? null : judgement,
  });
}

/** An overall verdict chip of outcome `outcome`, as `overrideToggle`. */
export function overallToggle(outcome: number, judgement: string): (v: AppraisalView) => AppraisalEdit {
  return (v) => ({
    kind: "overall_override",
    outcome,
    judgement: readerOutcome(v, outcome)?.override_overall === judgement ? null : judgement,
  });
}

/** An escalation chip of outcome `outcome`: `yes`, or cleared if it is that already. */
export function escalateToggle(outcome: number, yes: boolean): (v: AppraisalView) => AppraisalEdit {
  return (v) => ({ kind: "escalate", outcome, escalate: readerOutcome(v, outcome)?.escalate === yes ? null : yes });
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

/** What the design picker shows, and the problem *Confirm* names, for a randomised trial with no layout. */
export const PICK_LAYOUT = "Pick the trial's layout.";

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
 * before sending: a randomised trial with no layout picked, an outcome with
 * no name, a result without its measure or estimate, a number that is not
 * one, or an interval the wrong way round. Empty when it may be sent; the
 * core checks the rest, among it that a design RoB 2 covers has an outcome
 * (one with no method yet stops at step 1 without any), since which designs
 * are covered is bmlib's to say.
 */
export function confirmProblems(d: ConfirmDraft): string[] {
  const problems: string[] = [];
  if (d.design === "rct" && d.rct_layout === null) problems.push(PICK_LAYOUT);
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

/** How long the reader's typing pauses before their note is stored (design §5: a note is stored as it is entered). */
export const NOTE_SAVE_DEBOUNCE_MS = 800;

/**
 * A note box's sends: its text once the reader pauses for `delayMs`, at once
 * on `flush` (a blur, or the box going away), and never the text last sent
 * (or held when the box took focus) again, so a view that lags a send never
 * sends it twice.
 */
export class NoteSaver {
  /** The text last sent, or held when the box took focus. */
  private sent = "";
  /** The text typed since, not sent yet; null for none. */
  private pending: string | null = null;
  /** The pause's timer while one runs. */
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Called with each text to store. */
  private readonly send: (text: string) => void;
  /** The pause before a send. */
  private readonly delayMs: number;

  /** A saver sending through `send` after `delayMs` of no typing. */
  constructor(send: (text: string) => void, delayMs = NOTE_SAVE_DEBOUNCE_MS) {
    this.send = send;
    this.delayMs = delayMs;
  }

  /** The box took focus holding `text`: what is sent is measured against it. */
  focus(text: string): void {
    this.sent = text;
  }

  /** The reader typed: `text` is sent once they pause. */
  input(text: string): void {
    this.pending = text;
    this.clear();
    this.timer = setTimeout(() => this.flush(), this.delayMs);
  }

  /** Sends the text typed now, if it differs from the one last sent. */
  flush(): void {
    this.clear();
    const text = this.pending;
    this.pending = null;
    if (text === null || text === this.sent) return;
    this.sent = text;
    this.send(text);
  }

  /** Stops the pause's timer, if one runs. */
  private clear(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
}

/** What the tab says for an appraisal whose row cannot be read: why, and that setting it aside is the way on. */
export function damagedNote(error: string): string {
  return `Your appraisal cannot be read: ${error}. It is kept; set it aside to start a new one.`;
}
