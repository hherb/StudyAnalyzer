import { describe, expect, test } from "vitest";
import { PICO_PARTS, type CommandError, type PicoPartView, type PicoRunView, type PicoView, type SuitabilityView } from "./api";
import { CREATED_AT_FORMAT } from "./interrogator";
import {
  PART_LABELS,
  checkNote,
  offerRun,
  partNote,
  picoMarks,
  quotesDroppedNote,
  rerunForced,
  runCaption,
  runFailure,
  runNotes,
  sampleSizeNote,
  shouldAutoRun,
} from "./pico";

/** A part in `state` with citations numbered `numbers`. */
const part = (
  name: PicoPartView["part"],
  state: PicoPartView["state"],
  numbers: number[] = [],
  quotes_dropped = 0,
): PicoPartView => ({
  part: name,
  description: "d",
  state,
  confidence: 0.8,
  citations: numbers.map((number) => ({ number, span: { start: number * 10, end: number * 10 + 5 }, text: "t" })),
  quotes_not_found: [],
  quotes_dropped,
});

/** A check, from the abstract or not. */
const check = (from_abstract: boolean, notes: string[] = []): SuitabilityView => ({
  is_suitable: true,
  from_abstract,
  study_type: "RCT",
  confidence: 0.9,
  rationale: "r",
  condensed_from_chars: null,
  condensation_status: null,
  notes,
});

/** An extracted run on text `canonical_id`. */
const run = (canonical_id = 2, over: Partial<NonNullable<PicoRunView["extraction"]>> = {}): PicoRunView => ({
  id: 1,
  canonical_id,
  model: "ollama:m",
  created_at: "2026-11-03 10:00:00",
  read_only: false,
  suitability: check(true),
  extraction: {
    parts: [
      part("population", "verified", [1, 2]),
      part("intervention", "verified", [3]),
      part("comparison", "unverified"),
      part("outcome", "not_reported"),
    ],
    study_type: "RCT",
    sample_size: "N=105",
    overall_confidence: 0.8,
    condensed_from_chars: null,
    condensation_status: null,
    notes: [],
    ...over,
  },
});

/** The tab's view with `current` as its run. */
const view = (current: PicoRunView | null): PicoView => ({ canonical_id: 2, current, earlier: [] });

describe("PICO labels and notes", () => {
  test("the labels cover exactly the four parts", () => {
    expect(Object.keys(PART_LABELS).sort()).toEqual([...PICO_PARTS].sort());
    expect(PART_LABELS).toEqual({
      population: "Population",
      intervention: "Intervention",
      comparison: "Comparison",
      outcome: "Outcome",
    });
  });

  test("a part's note says what backs it", () => {
    expect(partNote(part("population", "verified", [1]))).toBeNull();
    expect(partNote(part("comparison", "unverified"))).toBe(
      "Unsure – model could not reproduce Comparison from text provided",
    );
    expect(partNote(part("outcome", "not_reported"))).toBe("Not clearly stated");
  });

  test("study type and sample size are the model's, unchecked", () => {
    expect(sampleSizeNote()).toBe("As the model gave it; not checked against the paper.");
  });

  test("dropped quotes are counted, none is no note", () => {
    expect(quotesDroppedNote(part("population", "verified", [], 0))).toBeNull();
    expect(quotesDroppedNote(part("population", "verified", [], 1))).toBe("1 quote was not usable text");
    expect(quotesDroppedNote(part("population", "verified", [], 3))).toBe("3 quotes were not usable text");
  });

  test("a check from the abstract says so", () => {
    expect(checkNote(check(true))).toBe("Judged from its abstract");
    expect(checkNote(check(false))).toBeNull();
  });
});

describe("PICO marks and captions", () => {
  test("every citation is a mark, the clicked number current", () => {
    const marks = picoMarks(run(), 3);
    expect(marks.map((m) => m.span.start)).toEqual([10, 20, 30]);
    expect(marks.map((m) => m.current)).toEqual([false, false, true]);
    expect(picoMarks(run(), null).some((m) => m.current)).toBe(false);
    const unsuitable = { ...run(), extraction: null };
    expect(picoMarks(unsuitable, 1)).toEqual([]);
  });

  test("the caption names the model, the time, a condensation and an older text", () => {
    const when = new Date(Date.UTC(2026, 10, 3, 10)).toLocaleString(undefined, CREATED_AT_FORMAT);
    expect(runCaption(run(), 2)).toBe(`ollama:m · ${when}`);
    expect(runCaption(run(2, { condensed_from_chars: 50000 }), 2)).toBe(
      `ollama:m · ${when} · condensed from 50,000 characters`,
    );
    expect(runCaption(run(1), 2)).toBe(`ollama:m · ${when} · on an earlier version of the text`);
  });

  test("the run's notes and an unclean condensation are all shown", () => {
    const noted: PicoRunView = {
      ...run(2, { notes: ["e"], condensation_status: "partial" }),
      suitability: check(true, ["c"]),
    };
    expect(runNotes(noted)).toEqual(["c", "e", "Condensation ended partial; the digest may be incomplete."]);
    expect(runNotes(run())).toEqual([]);
    const unsuitable: PicoRunView = {
      ...run(),
      extraction: null,
      suitability: { ...check(false, ["c"]), condensation_status: "truncated" },
    };
    expect(runNotes(unsuitable)).toEqual(["c", "Condensation ended truncated; the digest may be incomplete."]);
  });
});

describe("when a run starts or is offered", () => {
  test("auto-run only with no run, an askable text, a model and no attempt yet", () => {
    expect(shouldAutoRun(view(null), true, true, false)).toBe(true);
    expect(shouldAutoRun(view(run()), true, true, false)).toBe(false);
    expect(shouldAutoRun(view(null), false, true, false)).toBe(false);
    expect(shouldAutoRun(view(null), true, false, false)).toBe(false);
    // A failed, stopped or still running first run counts as attempted.
    expect(shouldAutoRun(view(null), true, true, true)).toBe(false);
  });

  test("Extract PICO is offered whenever no run is on a text that may be asked about, never while one runs", () => {
    // Whether or not a run was attempted (one failed, or the text was confirmed with the tab open).
    expect(offerRun(view(null), true, false)).toBe(true);
    expect(offerRun(view(null), true, true)).toBe(false);
    expect(offerRun(view(run()), true, false)).toBe(false);
    expect(offerRun(view(null), false, false)).toBe(false);
  });

  test("a note the check and the extraction share (one condensation) is shown once", () => {
    const shared: PicoRunView = {
      ...run(2, { notes: ["condensed"], condensation_status: "partial" }),
      suitability: { ...check(false, ["condensed"]), condensation_status: "partial" },
    };
    expect(runNotes(shared)).toEqual(["condensed", "Condensation ended partial; the digest may be incomplete."]);
  });
});

describe("a failed run", () => {
  const err = (kind: CommandError["kind"]): CommandError => ({ kind, message: `${kind} message` });

  test("a stop shows nothing; a missing model or key is a notice only on an automatic run", () => {
    for (const auto of [true, false]) expect(runFailure(err("cancelled"), auto, null)).toBe("nothing");
    for (const kind of ["needs_model", "needs_api_key"] as const) {
      expect(runFailure(err(kind), true, null)).toEqual({ text: `${kind} message`, settings: true });
      expect(runFailure(err(kind), false, null)).toBe("error");
    }
  });

  test("a text not confirmed is a notice, the tab's own words first; anything else is reported", () => {
    for (const auto of [true, false]) {
      expect(runFailure(err("not_full_text"), auto, "Confirm it first.")).toEqual({
        text: "Confirm it first.",
        settings: false,
      });
      expect(runFailure(err("not_full_text"), auto, null)).toEqual({ text: "not_full_text message", settings: false });
      for (const kind of ["llm", "storage", "internal", "invalid_input"] as const)
        expect(runFailure(err(kind), auto, null)).toBe("error");
    }
  });

  test("a forced run re-runs forced; a checked one re-runs with its check", () => {
    expect(rerunForced({ ...run(), suitability: null })).toBe(true);
    expect(rerunForced({ ...run(), suitability: check(true, []) })).toBe(false);
  });
});
