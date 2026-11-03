import { describe, expect, test } from "vitest";
import type { CitationView, TurnView } from "./api";
import {
  applyEvent,
  contradictionNote,
  highlightRuns,
  incompleteNote,
  liveNote,
  liveSegments,
  noQuotesNote,
  paperMarks,
  unlocatedNote,
} from "./chat";

/** A span covering `start`..`end`. */
const sp = (start: number, end: number) => ({ start, end });
/** A turn with id `id` citing the given spans as numbers 1, 2, ... */
const turn = (id: number, spans: Array<[number, number]>): TurnView => ({
  id,
  question: "q",
  pieces: [],
  citations: spans.map(([s, e], i): CitationView => ({ number: i + 1, span: sp(s, e), text: "" })),
  unlocated: [],
  status: "complete",
  error: null,
  model: "m",
  mode: "whole_paper",
});

describe("liveSegments", () => {
  /** A closed quote and an unclosed one both become quote segments. */
  test("closed and open quotes become quote segments", () => {
    expect(liveSegments("A <q>x</q> b <q>y")).toEqual([
      { kind: "text", text: "A " },
      { kind: "quote", text: "x" },
      { kind: "text", text: " b " },
      { kind: "quote", text: "y" },
    ]);
  });
  /** A trailing partial tag (`<`, `<q`, `</`, `</q`) is never shown. */
  test("live_holds_back_partial_tag", () => {
    expect(liveSegments("A <")).toEqual([{ kind: "text", text: "A " }]);
    expect(liveSegments("A <q")).toEqual([{ kind: "text", text: "A " }]);
    expect(liveSegments("A <q>x</")).toEqual([
      { kind: "text", text: "A " },
      { kind: "quote", text: "x" },
    ]);
    expect(liveSegments("A <q>x</q")).toEqual([
      { kind: "text", text: "A " },
      { kind: "quote", text: "x" },
    ]);
  });
  /** Empty input gives no segments; a lone `<` followed by text is shown. */
  test("plain text and empty input", () => {
    expect(liveSegments("")).toEqual([]);
    expect(liveSegments("a < b")).toEqual([{ kind: "text", text: "a < b" }]);
  });
  /** `<qx` is not a tag, so it is shown as text. */
  test("a lookalike is text", () => {
    expect(liveSegments("<qx")).toEqual([{ kind: "text", text: "<qx" }]);
  });
  /** A whole quote and nothing else gives one quote segment. */
  test("a bare closed quote", () => {
    expect(liveSegments("<q>x</q>")).toEqual([{ kind: "quote", text: "x" }]);
  });
  /** A held-back `<` appears once more text shows it was not a tag. */
  test("a held-back < is shown once it is not a tag", () => {
    expect(liveSegments("<q>a <")).toEqual([{ kind: "quote", text: "a " }]);
    expect(liveSegments("<q>a < b")).toEqual([{ kind: "quote", text: "a < b" }]);
  });
});

describe("highlightRuns", () => {
  /** A mark running past the block end is cut at the block end. */
  test("a mark across the block's end is clipped", () => {
    expect(highlightRuns(sp(0, 10), [{ span: sp(6, 15), current: false }])).toEqual([
      { span: sp(0, 6), mark: "none" },
      { span: sp(6, 10), mark: "past" },
    ]);
  });
  /** A mark starting before the block is cut at the block start. */
  test("a mark across the block's start is clipped", () => {
    expect(highlightRuns(sp(10, 20), [{ span: sp(5, 12), current: true }])).toEqual([
      { span: sp(10, 12), mark: "current" },
      { span: sp(12, 20), mark: "none" },
    ]);
  });
  /** Where a past and the current mark overlap, the run is current. */
  test("current wins where marks overlap", () => {
    expect(
      highlightRuns(sp(0, 10), [
        { span: sp(2, 7), current: false },
        { span: sp(5, 9), current: true },
      ]),
    ).toEqual([
      { span: sp(0, 2), mark: "none" },
      { span: sp(2, 5), mark: "past" },
      { span: sp(5, 9), mark: "current" },
      { span: sp(9, 10), mark: "none" },
    ]);
  });
  /** Touching marks of the same kind give one run. */
  test("adjacent runs with the same mark merge", () => {
    expect(
      highlightRuns(sp(0, 6), [
        { span: sp(0, 3), current: false },
        { span: sp(3, 6), current: false },
      ]),
    ).toEqual([{ span: sp(0, 6), mark: "past" }]);
  });
  /** No marks, a mark outside the block, and an empty block. */
  test("no marks, untouching marks and an empty block", () => {
    expect(highlightRuns(sp(0, 4), [])).toEqual([{ span: sp(0, 4), mark: "none" }]);
    expect(highlightRuns(sp(0, 4), [{ span: sp(4, 8), current: true }])).toEqual([{ span: sp(0, 4), mark: "none" }]);
    expect(highlightRuns(sp(3, 3), [{ span: sp(0, 8), current: true }])).toEqual([]);
  });
  /** A zero-length mark changes nothing. */
  test("a zero-length mark is ignored", () => {
    expect(highlightRuns(sp(0, 4), [{ span: sp(2, 2), current: true }])).toEqual([{ span: sp(0, 4), mark: "none" }]);
  });
  /** Marks in any order give the same runs. */
  test("unsorted marks", () => {
    expect(
      highlightRuns(sp(0, 6), [
        { span: sp(4, 6), current: true },
        { span: sp(0, 2), current: false },
      ]),
    ).toEqual([
      { span: sp(0, 2), mark: "past" },
      { span: sp(2, 4), mark: "none" },
      { span: sp(4, 6), mark: "current" },
    ]);
  });
  /** The same span cited twice, once as current, is current. */
  test("duplicate spans, one current", () => {
    expect(
      highlightRuns(sp(0, 4), [
        { span: sp(0, 4), current: false },
        { span: sp(0, 4), current: true },
      ]),
    ).toEqual([{ span: sp(0, 4), mark: "current" }]);
  });
});

describe("notes", () => {
  /** The unlocated-quotes note: none, singular, plural. */
  test("unlocatedNote", () => {
    expect(unlocatedNote(0)).toBeNull();
    expect(unlocatedNote(1)).toBe("1 quote could not be found in the paper");
    expect(unlocatedNote(2)).toBe("2 quotes could not be found in the paper");
    expect(unlocatedNote(3)).toBe("3 quotes could not be found in the paper");
  });
  /** A refused near match says why; an unknown reason shows as bmlib names it; none, nothing. */
  test("contradictionNote", () => {
    expect(contradictionNote(null)).toBeNull();
    expect(contradictionNote("number")).toBe("changes a number");
    expect(contradictionNote("negation")).toBe("adds or drops a negation");
    expect(contradictionNote("direction")).toBe("changes a direction or comparison");
    expect(contradictionNote("reorder")).toBe("refused: reorder");
  });
  /** Only a stopped or failed turn gets the incomplete note. */
  test("incompleteNote", () => {
    expect(incompleteNote("complete")).toBeNull();
    const note = "Incomplete answer: its quotes were not checked.";
    expect(incompleteNote("stopped")).toBe(note);
    expect(incompleteNote("failed")).toBe(note);
  });
  /** Only a complete turn with no quotes at all (located or not) gets the no-quotes note. */
  test("noQuotesNote", () => {
    const note = "This answer quotes nothing from the paper.";
    expect(noQuotesNote(turn(1, []))).toBe(note);
    expect(noQuotesNote(turn(1, [[0, 1]]))).toBeNull();
    expect(noQuotesNote({ ...turn(1, []), unlocated: [{ quote: "q", contradiction: null }] })).toBeNull();
    expect(noQuotesNote({ ...turn(1, []), status: "stopped" })).toBeNull();
    expect(noQuotesNote({ ...turn(1, []), status: "failed" })).toBeNull();
  });
});

describe("liveNote", () => {
  /** An answer just asked: nothing has arrived yet. */
  const idle = { question: "q", text: "", indexing: null, checking: false, stopping: false };
  /** Each stage says what is happening; streaming text needs no note; a pressed Stop wins over all. */
  test("names the stage", () => {
    expect(liveNote(idle)).toBe("Waiting for the model…");
    expect(liveNote({ ...idle, checking: true })).toBe("Checking the search model…");
    expect(liveNote({ ...idle, indexing: { done: 2, total: 5 } })).toBe("Finding passages… 2 / 5");
    expect(liveNote({ ...idle, text: "A" })).toBeNull();
    expect(liveNote({ ...idle, stopping: true })).toBe("Stopping…");
    expect(liveNote({ ...idle, text: "A", stopping: true })).toBe("Stopping…");
  });
});

describe("applyEvent", () => {
  /** An answer just asked: nothing has arrived yet. */
  const fresh = () => ({ question: "q", text: "", indexing: null, checking: false, stopping: false });
  /** `waiting` ends checking and indexing, so the note says the model is being waited on; text then streams. */
  test("stages, then waiting, then text", () => {
    const live = fresh();
    applyEvent(live, { kind: "checking_model" });
    expect(liveNote(live)).toBe("Checking the search model…");
    applyEvent(live, { kind: "indexing", done: 1, total: 2 });
    expect(liveNote(live)).toBe("Finding passages… 1 / 2");
    applyEvent(live, { kind: "waiting" });
    expect(liveNote(live)).toBe("Waiting for the model…");
    applyEvent(live, { kind: "delta", text: "A" });
    applyEvent(live, { kind: "delta", text: "B" });
    expect(live.text).toBe("AB");
    expect(liveNote(live)).toBeNull();
  });
  /** Waiting right after the search-model check (no indexing) also clears the check. */
  test("waiting after checking", () => {
    const live = fresh();
    applyEvent(live, { kind: "checking_model" });
    applyEvent(live, { kind: "waiting" });
    expect(liveNote(live)).toBe("Waiting for the model…");
  });
});

describe("paperMarks", () => {
  /** Citation numbers restart per turn, so only the matching turn and number is current. */
  test("one current, the rest past; numbers restart per turn", () => {
    const turns = [turn(1, [[0, 2], [4, 6]]), turn(2, [[8, 9]])];
    expect(paperMarks(turns, { turnId: 2, number: 1 })).toEqual([
      { span: sp(0, 2), current: false },
      { span: sp(4, 6), current: false },
      { span: sp(8, 9), current: true },
    ]);
  });
  /** With no current citation, every mark is past. */
  test("null means none current", () => {
    const marks = paperMarks([turn(1, [[0, 2]])], null);
    expect(marks).toEqual([{ span: sp(0, 2), current: false }]);
  });
});
