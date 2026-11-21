import { describe, expect, test } from "vitest";
import {
  PRISMA_JUDGEMENTS,
  PRISMA_NOT_ASSESSED,
  type KeyGapsView,
  type PrismaItemView,
  type PrismaRunView,
  type PrismaSectionView,
  type PrismaView,
} from "./api";
import {
  JUDGEMENT_LABELS,
  NOT_ASSESSED_LABEL,
  OPEN_SECTIONS,
  SECTION_HEADINGS,
  continueCaption,
  continueHint,
  continueOffered,
  itemNote,
  judgementLabel,
  keyGapsLine,
  keyItems,
  offerRun,
  offerRunAnyway,
  prismaMarks,
  runningNote,
  sectionHeading,
  sectionLine,
  shouldAutoRun,
  variantNote,
} from "./prisma";

/** An item `id` in `section`, judged `judgement`, with one citation per start in `starts`. */
const item = (
  id: string,
  judgement: string | null,
  starts: number[] = [],
  over: Partial<PrismaItemView> = {},
): PrismaItemView => ({
  id,
  section: "methods",
  name: `item ${id}`,
  key: false,
  judgement,
  model_judgement: judgement,
  unbacked: false,
  condensed: false,
  rechecked: false,
  first_judgement: null,
  explanation: "e",
  citations: starts.map((start) => ({ span: { start, end: start + 5 }, text: "t", similarity: 1, exact: true })),
  quotes_not_found: [],
  quotes_contradicted: [],
  quotes_dropped: 0,
  ...over,
});

/** Key gaps of a run, complete unless said otherwise. */
const gaps = (over: Partial<KeyGapsView> = {}): KeyGapsView => ({
  items: ["11", "7"],
  gaps: 4,
  elsewhere: 0,
  total: 10,
  complete: true,
  ...over,
});

/** A run in `status` with `items`, suitable unless `suitable` is false. */
const run = (status = "complete", items: PrismaItemView[] = [], suitable = true, model = "anthropic:a"): PrismaRunView => ({
  id: 1,
  canonical_id: 2,
  model,
  created_at: "2026-11-03 10:00:00",
  forced: false,
  status,
  suitability: {
    is_systematic_review: suitable,
    is_meta_analysis: false,
    is_suitable: suitable,
    document_type: suitable ? "systematic review" : "RCT",
    variant: null,
    rationale: "r",
    confidence: 0.9,
    condensed_from_chars: null,
    condensation_status: null,
  },
  from_abstract: true,
  variant: null,
  items,
  sections: [],
  key_gaps: gaps({ complete: status === "complete" }),
  failed_parts: [],
  notes: [],
});

/** The tab's view with `current` as its run. */
const view = (current: PrismaRunView | null): PrismaView => ({ canonical_id: 2, current, earlier: [] });

/** A section's tally from the counts given, every other key zero. */
const section = (name: string, given: Record<string, number>): PrismaSectionView => ({
  section: name,
  counts: Object.fromEntries([...PRISMA_JUDGEMENTS, PRISMA_NOT_ASSESSED].map((k) => [k, given[k] ?? 0])),
});

describe("PRISMA labels", () => {
  test("every judgement has its label, exactly", () => {
    expect(Object.keys(JUDGEMENT_LABELS).sort()).toEqual([...PRISMA_JUDGEMENTS].sort());
    expect(JUDGEMENT_LABELS).toEqual({
      reported: "Reported",
      partly_reported: "Partly reported",
      not_reported: "Not reported",
      reported_elsewhere: "Reported elsewhere, not checked",
      not_applicable: "Not applicable",
    });
  });

  test("an item no part assessed is Not assessed; an unknown judgement shows as Rust sent it", () => {
    expect(NOT_ASSESSED_LABEL).toBe("Not assessed");
    expect(judgementLabel(null)).toBe("Not assessed");
    expect(judgementLabel("partly_reported")).toBe("Partly reported");
    expect(judgementLabel("vague")).toBe("vague");
  });

  test("every section has its heading; Methods and Results open by default", () => {
    expect(SECTION_HEADINGS).toEqual({
      title: "Title",
      abstract: "Abstract",
      introduction: "Introduction",
      methods: "Methods",
      results: "Results",
      discussion: "Discussion",
      other_information: "Other information",
    });
    expect(sectionHeading("other_information")).toBe("Other information");
    expect(sectionHeading("appendix")).toBe("appendix");
    expect([...OPEN_SECTIONS]).toEqual(["methods", "results"]);
  });

  test("a section line counts what is there, never a zero, with the items not assessed", () => {
    expect(
      sectionLine(
        section("methods", { reported: 9, partly_reported: 4, not_reported: 2, reported_elsewhere: 1, not_applicable: 1 }),
      ),
    ).toBe("Methods: 9 reported · 4 partly · 2 not · 1 elsewhere · 1 n/a");
    expect(sectionLine(section("title", { reported: 1 }))).toBe("Title: 1 reported");
    expect(sectionLine(section("results", { not_reported: 1, not_assessed: 3 }))).toBe(
      "Results: 1 not · 3 not assessed",
    );
  });

  test("a scoping review and a network meta-analysis name their extension; any other variant none", () => {
    for (const v of ["scoping_review", "Scoping Review", "scoping review"])
      expect(variantNote(v)).toBe("A scoping review: PRISMA-ScR fits it better");
    for (const v of ["network_meta_analysis", "NETWORK_META_ANALYSIS", "network meta-analysis"])
      expect(variantNote(v)).toBe("A network meta-analysis: PRISMA-NMA fits it better");
    for (const v of ["umbrella_review", "rapid_review", "living_review", "none", "unknown", "", null])
      expect(variantNote(v)).toBeNull();
  });
});

describe("an item's note", () => {
  test("a backed, whole-paper item has none", () => {
    expect(itemNote(item("7", "reported", [1]))).toBeNull();
  });

  test("each note is the constraint's wording, joined when several apply", () => {
    expect(itemNote(item("7", "not_reported", [], { unbacked: true, model_judgement: "reported" }))).toBe(
      "Not reported: the model's claim could not be checked against the paper",
    );
    expect(itemNote(item("5", "reported", [1], { condensed: true }))).toBe("Judged from a condensed paper");
    // A re-check is on a condensation of its own: it says so in place of the condensed note.
    expect(itemNote(item("7", "not_reported", [], { condensed: true, rechecked: true }))).toBe(
      "Checked again on a condensation for this item",
    );
    expect(itemNote(item("5", "reported", [1], { quotes_dropped: 2 }))).toBe("2 quotes were not usable text");
    expect(
      itemNote(item("7", "not_reported", [], { unbacked: true, condensed: true, quotes_dropped: 1 })),
    ).toBe(
      "Not reported: the model's claim could not be checked against the paper · Judged from a condensed paper · 1 quote was not usable text",
    );
  });
});

describe("key gaps", () => {
  test("a complete run counts its gaps, and the items it cannot read when there are any", () => {
    expect(keyGapsLine(gaps())).toEqual(["4 of 10 key items are not or only partly reported"]);
    expect(keyGapsLine(gaps({ elsewhere: 2 }))).toEqual([
      "4 of 10 key items are not or only partly reported",
      "2 key items rely on material we cannot read (supplements, registries, figures)",
    ]);
    expect(keyGapsLine(gaps({ elsewhere: 1 }))[1]).toBe(
      "1 key item relies on material we cannot read (supplements, registries, figures)",
    );
  });

  test("an incomplete run says Not complete in place of the count", () => {
    expect(keyGapsLine(gaps({ complete: false, elsewhere: 2 }))).toEqual(["Not complete"]);
  });

  test("the key items come in the panel's order; one the run lacks is left out", () => {
    const r = { ...run("complete", [item("7", "reported"), item("11", "not_reported")]), key_gaps: gaps({ items: ["11", "7", "24a"] }) };
    expect(keyItems(r).map((i) => i.id)).toEqual(["11", "7"]);
  });
});

describe("while a run goes", () => {
  test("the note names the part, a condensation, a re-check, or a stop", () => {
    expect(runningNote(null, false)).toBe(
      "Assessing PRISMA 2020… A review is assessed part by part and can take several minutes.",
    );
    expect(runningNote({ kind: "part", index: 2, of: 5, label: "Methods 5–13c" }, false)).toBe(
      "Part 2 of 5: Methods 5–13c",
    );
    expect(runningNote({ kind: "condensing", batch: 3, batches: 7, level: 0 }, false)).toBe(
      "Condensing the long paper, part 3 of 7…",
    );
    expect(runningNote({ kind: "condensed" }, false)).toBe("Reading the condensed paper…");
    expect(runningNote({ kind: "rechecking", item: "7" }, false)).toBe(
      "Checking key item 7 again on a condensation of its own…",
    );
    expect(runningNote({ kind: "other", stage: "s", message: "m" }, false)).toBe(
      "Condensing… m (stage “s”, not known to this version)",
    );
    expect(runningNote({ kind: "part", index: 1, of: 2, label: "Title 1–2" }, true)).toBe("Stopping…");
  });
});

describe("PRISMA marks", () => {
  const items = [item("5", "reported", [10, 20]), item("7", "partly_reported", [30])];

  test("with a citation in focus, its item's citations are marked, that one current", () => {
    const marks = prismaMarks(run("complete", items), { item: "5", index: 1 });
    expect(marks.map((m) => [m.span.start, m.current])).toEqual([
      [10, false],
      [20, true],
    ]);
  });

  test("with none in focus, every citation of the run is marked, none current", () => {
    const marks = prismaMarks(run("complete", items), null);
    expect(marks.map((m) => [m.span.start, m.current])).toEqual([
      [10, false],
      [20, false],
      [30, false],
    ]);
  });
});

describe("when a run starts, continues or is offered", () => {
  test("auto-run only with no run, an askable text, a model and no attempt yet", () => {
    expect(shouldAutoRun(view(null), true, true, false)).toBe(true);
    // A not-suitable check is a stored run: never auto-run past it.
    expect(shouldAutoRun(view(run("complete", [], false)), true, true, false)).toBe(false);
    expect(shouldAutoRun(view(null), true, true, true)).toBe(false);
    expect(shouldAutoRun(view(null), true, false, false)).toBe(false);
    expect(shouldAutoRun(view(null), false, true, false)).toBe(false);
  });

  test("the first run is offered with no run on an askable text, never while one goes", () => {
    expect(offerRun(view(null), true, false)).toBe(true);
    expect(offerRun(view(null), true, true)).toBe(false);
    expect(offerRun(view(run()), true, false)).toBe(false);
  });

  test("Run anyway only after a check that found the paper not suitable", () => {
    expect(offerRunAnyway(run("complete", [], false))).toBe(true);
    expect(offerRunAnyway(run("complete", [], true))).toBe(false);
    expect(offerRunAnyway({ ...run(), suitability: null, forced: true })).toBe(false);
    expect(offerRunAnyway(null)).toBe(false);
  });

  test("Continue only for a current run that is incomplete, while no run goes", () => {
    expect(continueOffered(view(run("incomplete")), false)).toBe(true);
    expect(continueOffered(view(run("incomplete")), true)).toBe(false);
    expect(continueOffered(view(run("complete")), false)).toBe(false);
    expect(continueOffered(view(run("complete", [], false)), false)).toBe(false);
    expect(continueOffered(view(null), false)).toBe(false);
  });

  test("Continue names the run's model when another is chosen, and Re-run says it uses the chosen one", () => {
    expect(continueCaption(run("incomplete"), "anthropic:a")).toBe("Continue");
    expect(continueHint(run("incomplete"), "anthropic:a")).toBeNull();
    expect(continueCaption(run("incomplete"), "ollama:b")).toBe("Continue with anthropic:a");
    expect(continueHint(run("incomplete"), "ollama:b")).toBe("Re-run uses ollama:b");
    expect(continueCaption(run("incomplete"), null)).toBe("Continue with anthropic:a");
    expect(continueHint(run("incomplete"), null)).toBeNull();
  });
});
