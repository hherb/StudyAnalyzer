// Reads the JSON contract Rust pins (`src-tauri/src/contract.rs`) through the TS types.
import { describe, expect, test } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import {
  ERROR_KINDS,
  type BlockView,
  type CommandError,
  type Decoded,
  type FullTextStatus,
  type Identifiers,
  type OpenOutcome,
  type PaperSummary,
  type PaperView,
  type SectionView,
  type Utf16Span,
} from "./api";
import { bannerText, groupBlocks, sliceSpan, splitCells } from "./paper";

/** `true` only when A and B have exactly the same keys. */
type SameKeys<A, B> = [keyof A] extends [keyof B] ? ([keyof B] extends [keyof A] ? true : false) : false;
type Section = (typeof fixture.paper_view.sections)[number];
// Checked by `check:web`: a field added, renamed or dropped on one side fails to compile.
const keysMatch: [
  SameKeys<typeof fixture.open_ready, Extract<OpenOutcome, { status: "ready" }>>,
  SameKeys<typeof fixture.open_needs_full_text, Extract<OpenOutcome, { status: "needs_full_text" }>>,
  SameKeys<typeof fixture.paper_summary, PaperSummary>,
  SameKeys<typeof fixture.paper_summary.identifiers, Identifiers>,
  SameKeys<typeof fixture.paper_view, PaperView>,
  SameKeys<typeof fixture.paper_view.title_span, Utf16Span>,
  SameKeys<typeof fixture.paper_view.full_text, FullTextStatus>,
  SameKeys<typeof fixture.paper_view.decoded, Decoded>,
  SameKeys<Section, SectionView>,
  SameKeys<Section["blocks"][number], BlockView>,
  SameKeys<(typeof fixture.command_errors)[number], CommandError>,
] = [true, true, true, true, true, true, true, true, true, true, true];

const view = fixture.paper_view as PaperView;

describe("the Rust↔TS contract", () => {
  test("every field is mirrored", () => {
    expect(keysMatch.every(Boolean)).toBe(true);
  });

  test("the error kinds are Rust's, all of them", () => {
    expect(fixture.command_errors.map((e) => e.kind)).toEqual([...ERROR_KINDS]);
  });

  test("every span slices to the text Rust cut by code point", () => {
    const spans = [view.title_span!, ...view.sections.flatMap((s) => [s.heading!, ...s.blocks.map((b) => b.span)])];
    expect(spans.map((s) => sliceSpan(view.text, s))).toEqual(fixture.span_texts);
  });

  test("table rows group into one table", () => {
    const groups = groupBlocks(view.sections[0].blocks);
    expect(groups.map((g) => g.kind)).toEqual(["paragraph", "table", "paragraph"]);
    const table = groups[1];
    if (table.kind !== "table") throw new Error("not a table");
    expect(table.rows.map((r) => splitCells(sliceSpan(view.text, r)))).toEqual([
      ["a", "b😀"],
      ["1", "2"],
    ]);
  });

  test("the banner names what Rust found missing", () => {
    const text = bannerText(view.full_text);
    expect(text).toContain("no Methods heading");
    expect(text).toContain("no Results or Discussion heading");
    expect(text).toContain("fewer than 1,000 words (8)");
  });
});
