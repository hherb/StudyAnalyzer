import { describe, expect, test } from "vitest";
import type { BlockView, FullTextStatus, Identifiers } from "./api";
import { bannerText, encodingNote, groupBlocks, sliceSpan, retryIdentifier, splitCells } from "./paper";

const p = (start: number, end: number): BlockView => ({ kind: "paragraph", span: { start, end } });
const row = (start: number, end: number): BlockView => ({ kind: "table_row", span: { start, end } });
const status = (o: Partial<FullTextStatus> = {}): FullTextStatus => ({
  passed: false,
  confirmed: false,
  body_words: 5000,
  min_body_words: 1000,
  section_types: ["methods", "results"],
  ...o,
});

describe("paper", () => {
  test("sliceSpan counts UTF-16 units", () => {
    expect(sliceSpan("a😀b", { start: 1, end: 3 })).toBe("😀");
  });
  test("groupBlocks merges consecutive rows", () => {
    expect(groupBlocks([p(0, 1), row(1, 2), row(2, 3), p(3, 4)])).toEqual([
      { kind: "paragraph", span: { start: 0, end: 1 } },
      { kind: "table", rows: [{ start: 1, end: 2 }, { start: 2, end: 3 }] },
      { kind: "paragraph", span: { start: 3, end: 4 } },
    ]);
  });
  test("splitCells keeps trailing empty cell", () => {
    expect(splitCells("a\tb\t")).toEqual(["a", "b", ""]);
  });
  test("bannerText", () => {
    expect(bannerText(status({ passed: true }))).toBeNull();
    expect(bannerText(status({ confirmed: true }))).toBeNull();
    expect(bannerText(status({ section_types: ["results"] }))).toContain("Methods");
    expect(bannerText(status({ section_types: ["methods"] }))).toContain("Results or Discussion");
    expect(bannerText(status({ body_words: 200 }))).toContain("1,000 words");
  });
  test("encodingNote", () => {
    expect(encodingNote(null)).toBeNull();
    expect(encodingNote({ encoding: "utf-8", guessed: false })).toBeNull();
    expect(encodingNote({ encoding: "windows-1252", guessed: true })).toContain("windows-1252");
  });
  test("retryIdentifier prefers DOI, then PMID, then PMC", () => {
    const ids = (o: Partial<Identifiers>): Identifiers => ({ pmid: null, doi: null, pmc_id: null, ...o });
    expect(retryIdentifier(ids({ doi: "10.1/x", pmid: "1", pmc_id: "PMC2" }))).toBe("10.1/x");
    expect(retryIdentifier(ids({ pmid: "1", pmc_id: "PMC2" }))).toBe("1");
    expect(retryIdentifier(ids({ pmc_id: "PMC2" }))).toBe("PMC2");
    expect(retryIdentifier(ids({}))).toBeNull();
  });
});
