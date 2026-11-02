import { describe, expect, test } from "vitest";
import type { CitationView, TurnView } from "./api";
import { highlightRuns, incompleteNote, liveSegments, paperMarks, unlocatedNote } from "./chat";

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
  test("closed and open quotes become quote segments", () => {
    expect(liveSegments("A <q>x</q> b <q>y")).toEqual([
      { kind: "text", text: "A " },
      { kind: "quote", text: "x" },
      { kind: "text", text: " b " },
      { kind: "quote", text: "y" },
    ]);
  });
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
  test("plain text and empty input", () => {
    expect(liveSegments("")).toEqual([]);
    expect(liveSegments("a < b")).toEqual([{ kind: "text", text: "a < b" }]);
  });
});

describe("highlightRuns", () => {
  test("a mark across the block's end is clipped", () => {
    expect(highlightRuns(sp(0, 10), [{ span: sp(6, 15), current: false }])).toEqual([
      { span: sp(0, 6), mark: "none" },
      { span: sp(6, 10), mark: "past" },
    ]);
  });
  test("a mark across the block's start is clipped", () => {
    expect(highlightRuns(sp(10, 20), [{ span: sp(5, 12), current: true }])).toEqual([
      { span: sp(10, 12), mark: "current" },
      { span: sp(12, 20), mark: "none" },
    ]);
  });
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
  test("adjacent runs with the same mark merge", () => {
    expect(
      highlightRuns(sp(0, 6), [
        { span: sp(0, 3), current: false },
        { span: sp(3, 6), current: false },
      ]),
    ).toEqual([{ span: sp(0, 6), mark: "past" }]);
  });
  test("no marks, untouching marks and an empty block", () => {
    expect(highlightRuns(sp(0, 4), [])).toEqual([{ span: sp(0, 4), mark: "none" }]);
    expect(highlightRuns(sp(0, 4), [{ span: sp(4, 8), current: true }])).toEqual([{ span: sp(0, 4), mark: "none" }]);
    expect(highlightRuns(sp(3, 3), [{ span: sp(0, 8), current: true }])).toEqual([]);
  });
});

describe("notes", () => {
  test("unlocatedNote", () => {
    expect(unlocatedNote(0)).toBeNull();
    expect(unlocatedNote(1)).toBe("1 quote could not be found in the paper");
    expect(unlocatedNote(3)).toBe("3 quotes could not be found in the paper");
  });
  test("incompleteNote", () => {
    expect(incompleteNote("complete")).toBeNull();
    const note = "Incomplete answer: its quotes were not checked.";
    expect(incompleteNote("stopped")).toBe(note);
    expect(incompleteNote("failed")).toBe(note);
  });
});

describe("paperMarks", () => {
  test("one current, the rest past; numbers restart per turn", () => {
    const turns = [turn(1, [[0, 2], [4, 6]]), turn(2, [[8, 9]])];
    expect(paperMarks(turns, { turnId: 2, number: 1 })).toEqual([
      { span: sp(0, 2), current: false },
      { span: sp(4, 6), current: false },
      { span: sp(8, 9), current: true },
    ]);
  });
  test("null means none current", () => {
    const marks = paperMarks([turn(1, [[0, 2]])], null);
    expect(marks).toEqual([{ span: sp(0, 2), current: false }]);
  });
});
