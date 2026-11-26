import { describe, expect, test } from "vitest";
import { render } from "svelte/server";
import fixture from "./fixtures/ipc_contract.json";
import type { AppraisalView, EvidenceRunView, EvidenceView, PaperView, QuestionView, ReaderDomainView } from "./api";
import {
  APPRAISAL_LABELS,
  UNKNOWN_QUESTION,
  DIFFERS,
  NOT_ANSWERED,
  answerChoices,
  appraisalMarks,
  compareRows,
  departureLine,
  markKey,
  readerMarks,
  setAsideLine,
  versionEditable,
  versionLabel,
  type QuestionAt,
  confirmDraft,
  confirmInput,
  confirmProblems,
  continueOffered,
  currentStep,
  designChoices,
  domainProgress,
  escalateToggle,
  machineChoice,
  overallToggle,
  overrideToggle,
  readerDomain,
  machineLine,
  needsRun,
  outcomeDraft,
  runAgainOffered,
  summaryOf,
  tabMode,
  waitingForConfirmation,
} from "./appraisal";
import { ANSWER_LABELS, JUDGEMENT_LABELS, citeKeys } from "./evidence";
import AppraisalDomain from "../components/AppraisalDomain.svelte";
import AppraisalPane from "../components/AppraisalPane.svelte";
import PaperPane from "../components/PaperPane.svelte";
import { BLOCK_START_ATTR, SELECT_PROMPT } from "./paper";

/** The fixture's appraisal waiting for confirmation: the proposals stored, its run's two steps done. */
const confirming = fixture.appraisal_confirming as AppraisalView;
/** The fixture's appraisal being answered: its run 3 of 13 steps in, one step failed. */
const answering = fixture.appraisal_answering as AppraisalView;
/** The fixture's saved appraisal, its machine revealed. */
const saved = fixture.appraisal_saved as AppraisalView;

/** A deep copy of `a`, so a test may change it. */
const copy = <T>(a: T): T => structuredClone(a);

/** `answering` with its machine changed by `over`. */
const machineWith = (over: Partial<AppraisalView["machine"]>): AppraisalView => ({
  ...answering,
  machine: { ...answering.machine, ...over },
});

/** The Evidence tab's opening view with `over` laid on an empty one. */
const tabView = (over: Partial<EvidenceView>): EvidenceView => ({
  canonical_id: 1,
  current: null,
  current_error: null,
  earlier: [],
  appraisal: null,
  any_shown: false,
  ...over,
});

/** The part of `html` from the first `from` marker up to the `to` marker after it. */
function between(html: string, from: string, to: string): string {
  const start = html.indexOf(from);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = html.indexOf(to, start + from.length);
  return html.slice(start, end < 0 ? undefined : end);
}

/** The pane's server-rendered HTML for `view`, nothing running. */
function pane(view: AppraisalView): string {
  const noop = () => {};
  return render(AppraisalPane, {
    props: {
      view,
      running: false,
      stopping: false,
      progress: null,
      idle: true,
      notice: null,
      current: null,
      oncite: noop,
      onconfirm: noop,
      onedit: noop,
      onsave: noop,
      onreveal: noop,
      oncontinue: noop,
      onrunagain: noop,
      onstop: noop,
      onsettings: noop,
      marking: null,
      onmark: noop,
      onversion: noop,
      onclose: noop,
      onsetaside: noop,
    },
  }).body;
}

/** The machine's area of the pane: everything before the reader's. */
const machineArea = (html: string) => between(html, 'data-area="machine"', 'data-area="reader"');

/** Domain `domain`'s server-rendered HTML, editable, with `marking` armed (none by default). */
function domainHtml(domain: ReaderDomainView, marking: QuestionAt | null = null): string {
  const noop = () => {};
  const outcome = domain.domain === "D1" ? null : 0;
  return render(AppraisalDomain, {
    props: { domain, outcome, editable: true, idle: true, onedit: noop, marking, onmark: noop, current: null, oncite: noop },
  }).body;
}

/** One question's part of a domain's HTML. */
const questionPart = (html: string, id: string) => between(html, `data-question="${id}"`, "data-question=");

describe("which mode the tab opens in", () => {
  test("the choice on a paper never shown, even with an unshown run made before this phase", () => {
    expect(tabMode(tabView({}))).toBe("choice");
    const unshown = { ...(fixture.evidence_run_assessed as EvidenceRunView), first_shown_at: null };
    expect(tabMode(tabView({ current: unshown }))).toBe("choice");
  });

  test("an open appraisal opens on it, shown paper or not; one set aside does not", () => {
    const open = { id: 1, state: "answering" as const, blind: true, set_aside: false };
    expect(tabMode(tabView({ appraisal: open }))).toBe("appraisal");
    expect(tabMode(tabView({ appraisal: open, any_shown: true }))).toBe("appraisal");
    const aside = { ...open, set_aside: true };
    expect(tabMode(tabView({ appraisal: aside }))).toBe("choice");
    expect(tabMode(tabView({ appraisal: aside, any_shown: true }))).toBe("machine");
  });

  test("the machine's tab once a run was shown", () => {
    expect(tabMode(tabView({ any_shown: true }))).toBe("machine");
  });

  test("an appraisal's summary, as the tab's view names it", () => {
    expect(summaryOf(answering)).toEqual({ id: 1, state: "answering", blind: false, set_aside: false });
    expect(summaryOf({ ...answering, set_aside_at: "t" }).set_aside).toBe(true);
  });
});

describe("Show the machine's verdict", () => {
  test("serves the current run through its own view, which marks it shown; with none, runs when wanted", () => {
    const run = fixture.evidence_run_assessed as EvidenceRunView;
    expect(machineChoice(tabView({ current: run }), true)).toEqual({ serve: run.id });
    expect(machineChoice(tabView({}), true)).toBe("run");
    expect(machineChoice(tabView({}), false)).toBeNull();
  });
});

describe("edits built from the view as it stands", () => {
  test("a verdict or escalation chip sets it, or clears it when it is set already", () => {
    expect(readerDomain(saved, "D1", null)?.override).toBe("high");
    expect(overrideToggle("D1", null, "high")(saved)).toEqual({ kind: "override", domain: "D1", outcome: null, judgement: null });
    expect(overrideToggle("D2", 0, "high")(saved)).toEqual({ kind: "override", domain: "D2", outcome: 0, judgement: "high" });
    expect(overallToggle(1, "low")(saved)).toEqual({ kind: "overall_override", outcome: 1, judgement: "low" });
    const asked = copy(saved);
    asked.version.outcomes[1].escalate = true;
    expect(escalateToggle(1, true)(asked)).toEqual({ kind: "escalate", outcome: 1, escalate: null });
    expect(escalateToggle(1, false)(asked)).toEqual({ kind: "escalate", outcome: 1, escalate: false });
  });
});

describe("whether the paired run is started", () => {
  test("an appraisal with no paired run yet starts one, unless a run goes", () => {
    const none = machineWith({ run_id: null, done_steps: 0, total_steps: 0, failed: [] });
    expect(needsRun({ ...none, state: "confirming" }, false)).toBe(true);
    expect(needsRun({ ...none, state: "confirming" }, true)).toBe(false);
  });

  test("not while it waits for confirmation, nor while it runs", () => {
    expect(needsRun(confirming, false)).toBe(false);
    expect(needsRun(answering, true)).toBe(false);
  });

  test("opening_continues_a_pending_paired_run: a confirmed appraisal whose run has pending steps after a quit", () => {
    expect(needsRun(machineWith({ failed: [] }), false)).toBe(true);
    // A failed step is asked again too.
    expect(needsRun(machineWith({ done_steps: 13 }), false)).toBe(true);
  });

  test("not for a run done, under earlier rules, too long, or waiting on another paper's run", () => {
    expect(needsRun(machineWith({ done_steps: 13, failed: [] }), false)).toBe(false);
    expect(needsRun(machineWith({ earlier_rules: true }), false)).toBe(false);
    expect(needsRun(machineWith({ too_long: true }), false)).toBe(false);
    expect(needsRun(machineWith({ running_elsewhere: true }), false)).toBe(false);
  });
});

describe("what the paired run offers", () => {
  test("it waits for confirmation once its proposals are done, and only then", () => {
    expect(waitingForConfirmation(confirming)).toBe(true);
    expect(waitingForConfirmation({ ...confirming, machine: { ...confirming.machine, done_steps: 1 } })).toBe(false);
    expect(waitingForConfirmation(answering)).toBe(false);
  });

  test("Continue for a run that can go on, or none made yet; never while one runs", () => {
    expect(continueOffered(answering, false)).toBe(true);
    expect(continueOffered(answering, true)).toBe(false);
    expect(continueOffered(machineWith({ run_id: null, failed: [], done_steps: 0, total_steps: 0 }), false)).toBe(true);
    expect(continueOffered(confirming, false)).toBe(false);
    expect(continueOffered(machineWith({ earlier_rules: true }), false)).toBe(false);
  });

  test("a new run on the confirmed outcomes for one that cannot go on", () => {
    expect(runAgainOffered(machineWith({ earlier_rules: true }), false)).toBe(true);
    expect(runAgainOffered(machineWith({ too_long: true }), false)).toBe(true);
    expect(runAgainOffered(machineWith({ too_long: true }), true)).toBe(false);
    expect(runAgainOffered(answering, false)).toBe(false);
    expect(runAgainOffered({ ...confirming, machine: { ...confirming.machine, earlier_rules: true, done_steps: 1 } }, false)).toBe(false);
  });

  test("the step it is at", () => {
    expect([confirming, answering, saved].map(currentStep)).toEqual([1, 3, 4]);
    expect(currentStep({ ...confirming, state: "not_covered" })).toBe(1);
  });
});

describe("the machine's line", () => {
  test("progress, from the event while it runs, else from the view", () => {
    expect(machineLine(answering.machine, { kind: "progress", done: 6, total: 11 })).toBe(
      "The machine's assessment: 6 of 11 steps",
    );
    expect(machineLine(confirming.machine)).toBe("The machine's assessment: 2 of 2 steps");
  });

  test("a failure, in the stored neutral words", () => {
    expect(machineLine(answering.machine)).toBe(
      "The machine's assessment stopped: This step could not be completed; Continue asks it again.",
    );
    const twice = { ...answering.machine, failed: [...answering.machine.failed, ...answering.machine.failed] };
    expect(machineLine(twice)).toBe(machineLine(answering.machine));
  });

  test("another paper's run holding the slot", () => {
    expect(machineLine({ ...answering.machine, running_elsewhere: true })).toBe(
      "The machine's assessment waits for another paper's run.",
    );
  });

  test("a run that cannot go on: too long, or under earlier rules", () => {
    const m = { ...answering.machine, failed: [] };
    expect(machineLine({ ...m, too_long: true })).toMatch(/^The machine's assessment stopped: /);
    expect(machineLine({ ...m, earlier_rules: true })).toMatch(/^The machine's assessment stopped: /);
  });
});

describe("answering", () => {
  test("answer chips in RoB 2's order with plan 2's labels", () => {
    const q = { allowed: ["NI", "N", "Y", "PN", "PY"] } as QuestionView;
    expect(answerChoices(q)).toEqual(
      ["Y", "PY", "PN", "N", "NI"].map((answer) => ({ answer, label: ANSWER_LABELS[answer] })),
    );
  });

  test("noNiOn32: the 3.2 chips lack No information, a normal question offers it", () => {
    const d3 = copy(answering.version.outcomes[0].domains[1]);
    expect(d3.domain).toBe("D3");
    // The fixture's 3.2 does not apply; here it does, so its chips show.
    for (const q of d3.questions) q.applicable = true;
    const html = domainHtml(d3);
    const q32 = questionPart(html, "3.2");
    expect(q32).toContain(">Probably no<");
    expect(q32).not.toContain(ANSWER_LABELS.NI);
    expect(questionPart(html, "3.1")).toContain(`>${ANSWER_LABELS.NI}<`);
  });

  test("a question that does not apply is greyed with no chips, its stored answer struck through", () => {
    const d3 = copy(answering.version.outcomes[0].domains[1]);
    d3.questions[2].answer = "Y";
    const q33 = questionPart(domainHtml(d3), "3.3");
    expect(q33).not.toContain("<button");
    expect(q33).toContain(ANSWER_LABELS.NA);
    expect(q33).toContain(`<s>${ANSWER_LABELS.Y}</s>`);
  });

  test("an answer to a question these rules no longer have is shown, with no chips", () => {
    const d3 = copy(answering.version.outcomes[0].domains[1]);
    d3.questions.push({ ...d3.questions[0], question_id: "3.9", known: false, headline: "", answer: "PN" });
    const q39 = questionPart(domainHtml(d3), "3.9");
    expect(q39).toContain(UNKNOWN_QUESTION);
    expect(q39).toContain(ANSWER_LABELS.PN);
    expect(q39).not.toContain("<button");
  });

  test("a domain counts the questions that apply and those answered", () => {
    expect(domainProgress(answering.version.study)).toEqual({ answered: 1, applicable: 3 });
    expect(domainProgress(answering.version.outcomes[0].domains[0])).toEqual({ answered: 0, applicable: 3 });
  });
});

describe("noMachineVerdictBeforeReveal", () => {
  /** Every judgement label of plan 2. */
  const labels = Object.values(JUDGEMENT_LABELS);

  test("the pane on an unrevealed appraisal shows no judgement of the machine's", () => {
    const area = machineArea(pane(answering));
    expect(area).toContain(machineLine(answering.machine));
    for (const l of labels) expect(area).not.toContain(l);
  });

  test("not even when a view carried the machine's run without a reveal", () => {
    const tampered = { ...answering, machine: { ...answering.machine, revealed: saved.machine.revealed } };
    const area = machineArea(pane(tampered));
    for (const l of labels) expect(area).not.toContain(l);
  });

  test("once revealed, the machine's verdict shows (the control: the check above can fail)", () => {
    const overall = saved.machine.revealed?.outcomes[0].overall;
    expect(overall).toBe("some_concerns");
    expect(machineArea(pane(saved))).toContain(JUDGEMENT_LABELS.some_concerns);
  });

  test("the header says whether it is blind; the steps are named", () => {
    expect(pane(answering)).toContain(APPRAISAL_LABELS.notBlind);
    const html = pane({ ...answering, blind: true });
    expect(html).toContain(APPRAISAL_LABELS.blind);
    for (const s of APPRAISAL_LABELS.steps) expect(html).toContain(s);
  });

  test("an unsaved appraisal offers the reveal; one saved does not", () => {
    expect(pane(answering)).toContain(APPRAISAL_LABELS.revealNow);
    expect(pane(saved)).not.toContain(APPRAISAL_LABELS.revealNow);
  });

  test("Save and compare only once every question that applies is answered", () => {
    expect(pane(answering)).not.toContain(APPRAISAL_LABELS.save);
    const done = copy(answering);
    done.version.complete = true;
    expect(pane(done)).toContain(APPRAISAL_LABELS.save);
  });
});

describe("confirming", () => {
  const proposals = confirming.proposals!;

  test("the draft from the proposals sends back what the reader confirms unchanged", () => {
    const draft = confirmDraft(proposals);
    expect(draft.outcomes[0].result.ci_level).toBe("95");
    expect(confirmProblems(draft)).toEqual([]);
    expect(confirmInput(draft)).toEqual(answering.confirmed);
  });

  test("an added outcome is blank and comes from no proposal", () => {
    const o = outcomeDraft(null, null);
    expect(o.from_proposed).toBeNull();
    expect(o.name).toBe("");
    const draft = { ...confirmDraft(proposals), outcomes: [{ ...o, name: "sleep" }] };
    expect(confirmInput(draft).outcomes).toEqual([
      { from_proposed: null, name: "sleep", timepoint: null, result: null },
    ]);
  });

  test("an edited result is sent as numbers, its level as a fraction", () => {
    const draft = confirmDraft(proposals);
    draft.outcomes[1].result = { ...draft.outcomes[0].result, estimate: " -1.5 ", scale: "difference", ci_level: "90" };
    draft.outcomes[1].result.ci_low = "-2";
    draft.outcomes[1].result.ci_high = "-1";
    draft.outcomes[1].result.n_analysed = "";
    expect(confirmInput(draft).outcomes[1].result).toEqual({
      measure: "risk ratio",
      scale: "difference",
      estimate: -1.5,
      ci_low: -2,
      ci_high: -1,
      ci_level: 0.9,
      n_analysed: null,
    });
  });

  test("what is refused before sending: no outcome, an empty name, a CI the wrong way round, a bad number", () => {
    const draft = confirmDraft(proposals);
    expect(confirmProblems({ ...draft, outcomes: [] })).toEqual(["Confirm at least one primary outcome."]);
    const bad = copy(draft);
    bad.outcomes[0].name = "  ";
    bad.outcomes[0].result.ci_low = "0.9";
    bad.outcomes[0].result.ci_high = "0.5";
    bad.outcomes[1].result.measure = "mean difference";
    bad.outcomes[1].result.estimate = "a lot";
    expect(confirmProblems(bad)).toEqual([
      "Outcome 1 needs a name.",
      "Outcome 1: the interval's lower bound is above its upper bound.",
      "Outcome 2: “a lot” is not a number.",
    ]);
    const half = copy(draft);
    half.outcomes[1].result.measure = "mean difference";
    expect(confirmProblems(half)).toEqual(["Outcome 2: a result needs its measure and its estimate."]);
  });

  test("the designs offered: every randomised layout by name, then the others, none twice", () => {
    const choices = designChoices();
    expect(choices[0]).toEqual({ design: "rct", rct_layout: "parallel", label: "Randomised trial, parallel groups" });
    expect(choices.filter((c) => c.design === "rct").every((c) => c.rct_layout !== null)).toBe(true);
    expect(choices.filter((c) => c.design !== "rct").every((c) => c.rct_layout === null)).toBe(true);
    expect(choices.some((c) => c.design === "cohort_prospective")).toBe(true);
    expect(new Set(choices.map((c) => c.label)).size).toBe(choices.length);
  });

  test("the proposals' citations mark the paper; one in focus marks its group", () => {
    expect(appraisalMarks(confirming, null)).toHaveLength(3);
    const outcome = { key: citeKeys.outcome(1), index: 0 };
    expect(appraisalMarks(confirming, outcome)).toEqual([{ span: { start: 22, end: 35 }, current: true }]);
  });

  test("before the reveal no machine citation beyond the proposals; after it the run's own", () => {
    const tampered = { ...answering, machine: { ...answering.machine, revealed: saved.machine.revealed } };
    expect(appraisalMarks(tampered, null)).toEqual(appraisalMarks(answering, null));
    expect(appraisalMarks(saved, null).length).toBeGreaterThan(0);
  });
});

describe("versions", () => {
  test("versionLabel: the first record, then each revision after the reveal", () => {
    expect(versionLabel(0)).toBe("First record");
    expect(versionLabel(1)).toBe("After reveal 1");
    expect(versionLabel(3)).toBe("After reveal 3");
  });

  /** `saved` showing version `shown` of `versions`. */
  const savedWith = (versions: AppraisalView["versions"], shown: AppraisalView["versions"][number]): AppraisalView => ({
    ...saved,
    versions,
    version: { ...saved.version, number: shown.number, frozen_at: shown.frozen_at, closed_at: shown.closed_at },
  });
  const first = saved.versions[0];
  const open = { number: 1, frozen_at: null, closed_at: null };
  const closed = { ...open, closed_at: "t" };

  test("editable from the screen: the unsaved first record, and once saved the latest version (an edit opens a revision)", () => {
    expect(versionEditable(answering)).toBe(true);
    expect(versionEditable(confirming)).toBe(false);
    expect(versionEditable({ ...answering, state: "not_covered" })).toBe(false);
    // The saved first record, the latest: Rust says not editable, but an edit opens *After reveal 1*.
    expect(saved.version.editable).toBe(false);
    expect(versionEditable(saved)).toBe(true);
    expect(versionEditable(savedWith([first, open], open))).toBe(true);
    expect(versionEditable(savedWith([first, closed], closed))).toBe(true);
  });

  test("a frozen or closed version that is not the latest is never editable from the screen", () => {
    expect(versionEditable(savedWith([first, open], first))).toBe(false);
    expect(versionEditable(savedWith([first, closed, { ...open, number: 2 }], closed))).toBe(false);
  });

  test("Done revising only on an open revision; the picker names every version, the first record shown by default", () => {
    expect(pane(saved)).not.toContain(APPRAISAL_LABELS.doneRevising);
    const revising = pane(savedWith([first, open], open));
    expect(revising).toContain(APPRAISAL_LABELS.doneRevising);
    expect(revising).toContain("First record");
    expect(revising).toContain("After reveal 1");
    expect(pane(savedWith([first, closed], closed))).not.toContain(APPRAISAL_LABELS.doneRevising);
  });
});

describe("compare", () => {
  const rows = compareRows(saved.compare!, saved.version, 0);
  const row = (id: string) => rows.find((r) => r.question_id === id)!;

  test("compareRows puts both answers side by side and marks the differences", () => {
    expect(row("1.1")).toMatchObject({ domain: "D1", reader: "Yes", machine: "Yes", differs: false });
    expect(row("1.3")).toMatchObject({ reader: "No", machine: "Yes", differs: true });
    expect(row("5.1")).toMatchObject({ reader: "Yes", machine: "No information", differs: true });
    expect(rows.map((r) => r.domain)).toEqual([...rows.map((r) => r.domain)].sort());
  });

  test("a side with no answer is said, never a difference", () => {
    expect(row("2.3")).toMatchObject({ reader: "Not applicable", machine: "Yes", differs: false });
    expect(row("4.3")).toMatchObject({ reader: "No", machine: NOT_ANSWERED, differs: false });
    // An applicable question the reader left open (a reveal before the save).
    const draft = copy(saved);
    draft.version.study.questions[2].answer = null;
    draft.compare!.outcomes[0].questions[2] = { ...draft.compare!.outcomes[0].questions[2], reader: null, differs: false };
    expect(compareRows(draft.compare!, draft.version, 0)[2]).toMatchObject({ reader: NOT_ANSWERED });
  });

  test("the reader's marked sentence rides with its row", () => {
    expect(row("1.1").mark).toEqual({ start: 22, end: 35 });
    expect(row("1.2").mark).toBeNull();
  });

  test("an outcome compare does not have has no rows", () => {
    expect(compareRows(saved.compare!, saved.version, 9)).toEqual([]);
  });

  test("departureLine for both raters, the machine's as a defect", () => {
    expect(departureLine(saved.version.study.departure!, "reader")).toBe(
      "Your verdict departs from the rules: they give Low.",
    );
    expect(departureLine(saved.machine.departures[0], "machine")).toBe(
      "The machine's verdict departs from the rules: they give High. This is a defect; please report it.",
    );
  });

  test("once revealed, compare shows the differences, both verdicts and numbers, and each departure once", () => {
    const html = pane(saved);
    expect(html).toContain(DIFFERS);
    expect(html).toContain("Your verdict departs from the rules: they give Low.");
    expect(html.split("The machine's verdict departs from the rules: they give High.").length).toBe(2);
    expect(html).toContain("Your number 0.4");
    expect(html).toContain("The machine's number 0.7");
  });

  test("before the reveal nothing of compare shows, even from a tampered view", () => {
    const tampered = {
      ...answering,
      compare: saved.compare,
      machine: { ...answering.machine, revealed: saved.machine.revealed, departures: saved.machine.departures },
    };
    const html = pane(tampered);
    expect(html).not.toContain(DIFFERS);
    expect(html).not.toContain("The machine's verdict departs");
    expect(html).not.toContain("The machine's number");
  });

  test("a version under earlier rules: its conformance is not checked", () => {
    const earlier = copy(saved);
    earlier.version.earlier_rules = true;
    earlier.version.study.departure = null;
    expect(pane(earlier)).toContain(APPRAISAL_LABELS.notChecked);
    expect(pane(saved)).not.toContain(APPRAISAL_LABELS.notChecked);
  });
});

describe("marks", () => {
  test("the reader's marks are outlined, the one in focus current", () => {
    expect(readerMarks(saved, null)).toEqual([{ span: { start: 22, end: 35 }, current: false }]);
    const at = { domain: "D1", outcome: null, question_id: "1.1" };
    expect(readerMarks(saved, { key: markKey(at), index: 0 })).toEqual([{ span: { start: 22, end: 35 }, current: true }]);
  });

  test("a mark in focus fills nothing of the machine's", () => {
    const at = { domain: "D1", outcome: null, question_id: "1.1" };
    expect(appraisalMarks(saved, { key: markKey(at), index: 0 })).toEqual([]);
  });

  test("a question offers Mark the sentence; armed, it says to select; a mark can be shown or removed", () => {
    const d1 = saved.version.study;
    const html = domainHtml(d1);
    expect(questionPart(html, "1.2")).toContain(APPRAISAL_LABELS.markSentence);
    expect(questionPart(html, "1.1")).toContain(APPRAISAL_LABELS.removeMark);
    expect(questionPart(html, "1.1")).toContain(APPRAISAL_LABELS.showMark);
    const armed = domainHtml(d1, { domain: "D1", outcome: null, question_id: "1.2" });
    expect(questionPart(armed, "1.2")).toContain(APPRAISAL_LABELS.marking);
    expect(questionPart(armed, "1.1")).not.toContain(APPRAISAL_LABELS.marking);
  });

  test("the paper outlines the reader's marks, apart from the filled ones", () => {
    const html = render(PaperPane, {
      props: {
        view: fixture.paper_view as PaperView,
        marks: [{ span: { start: 22, end: 29 }, current: true }],
        outlined: [{ span: { start: 22, end: 35 }, current: false }],
      },
    })
      // Without Svelte's hydration comments and scoping classes.
      .body.replace(/<!--[^>]*-->/g, "")
      .replace(/ svelte-[a-z0-9]+/g, "");
    expect(html).toContain('<mark class="current"><span class="outlined">Body 😀</span></mark>');
    expect(html).toContain('<span class="outlined"> text.</span>');
    expect(html).toContain(`${BLOCK_START_ATTR}="22"`);
  });

  test("armed, the paper asks for a selection", () => {
    const html = render(PaperPane, {
      props: { view: fixture.paper_view as PaperView, onSelect: () => {} },
    }).body;
    expect(html).toContain(SELECT_PROMPT);
  });
});

describe("set aside and the machine's runs", () => {
  test("the pane offers to set the appraisal aside, asking first", () => {
    expect(pane(answering)).toContain(APPRAISAL_LABELS.setAside);
    expect(setAsideLine("2026-11-26 10:30:00")).toMatch(/^Set aside /);
  });

  test("runAgainOffered: a complete paired run under earlier rules offers only the new run", () => {
    const done = { done_steps: 13, total_steps: 13, failed: [] };
    expect(runAgainOffered(machineWith({ ...done, earlier_rules: true }), false)).toBe(true);
    expect(continueOffered(machineWith({ ...done, earlier_rules: true }), false)).toBe(false);
    expect(runAgainOffered(machineWith({ ...done, too_long: true }), false)).toBe(false);
    expect(runAgainOffered(machineWith(done), false)).toBe(false);
  });
});
