// Reads the JSON contract Rust pins (`src-tauri/src/contract.rs`) through the TS types,
// and checks the argument keys `api.ts` sends against it.
import { describe, expect, test, vi } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import * as api from "./api";
import apiSource from "./api.ts?raw";
import {
  ERROR_KINDS,
  SOURCE_KINDS,
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
/** T with its string literals widened to `string`, as a JSON import types them. */
type Widen<T> = T extends string ? string : T extends object ? { [K in keyof T]: Widen<T[K]> } : T;
type Section = (typeof fixture.paper_view.sections)[number];
type BareSection = (typeof fixture.paper_view_bare.sections)[number];
// Checked by `check:web`, not by vitest (which strips types): a field added,
// renamed or dropped on one side fails to compile.
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
  SameKeys<typeof fixture.paper_view_bare, PaperView>,
  SameKeys<BareSection, SectionView>,
] = [true, true, true, true, true, true, true, true, true, true, true, true, true];
// Also `check:web` only: a field whose type changed fails to compile. The tag
// values the JSON widens to `string` are pinned at run time below.
const valuesMatch: [
  Widen<OpenOutcome>,
  Widen<OpenOutcome>,
  Widen<PaperSummary>,
  Widen<PaperView>,
  Widen<PaperView>,
  Widen<CommandError>[],
] = [
  fixture.open_ready,
  fixture.open_needs_full_text,
  fixture.paper_summary,
  fixture.paper_view,
  fixture.paper_view_bare,
  fixture.command_errors,
];

const view = fixture.paper_view as PaperView;

/** What `invoke` was called with: each command's argument keys and their JSON types. */
const sent: Record<string, Record<string, string>> = {};
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
    sent[cmd] = Object.fromEntries(Object.entries(args).map(([k, v]) => [k, typeof v]));
  },
}));

describe("the Rust↔TS contract", () => {
  test("the open outcomes carry the tags the frontend branches on", () => {
    const tags: OpenOutcome["status"][] = ["ready", "needs_full_text"];
    expect([fixture.open_ready.status, fixture.open_needs_full_text.status]).toEqual(tags);
  });

  test("the source kinds are Rust's names", () => {
    const kinds: string[] = [fixture.paper_view.source_kind, fixture.paper_view_bare.source_kind];
    expect(kinds).toEqual(["plain_text", "pdf"]);
    expect(kinds.every((k) => (SOURCE_KINDS as readonly string[]).includes(k))).toBe(true);
  });

  test("an absent title, heading or decoding is null, not missing", () => {
    const bare = fixture.paper_view_bare as PaperView;
    expect(bare.title_span).toBeNull();
    expect(bare.sections[0].heading).toBeNull();
    expect(bare.decoded).toBeNull();
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

  test("every command is invoked with the argument keys and types Rust decodes", async () => {
    await Promise.all([
      api.openFile("p"),
      api.openPaste("t"),
      api.openIdentifier("i"),
      api.cancelLookup(),
      api.attachFile("p1", "p"),
      api.attachPaste("p1", "t"),
      api.reopenPaper("p1"),
      api.recentPapers(),
      api.paperView("p1"),
      api.originalPdf("p1"),
      api.confirmFullText("p1", 1),
      api.getContactEmail(),
      api.setContactEmail("e"),
      api.getKeepMismatched(),
      api.setKeepMismatched(true),
    ]);
    expect(sent).toEqual(fixture.commands);
  });

  test("the fixture lists every command api.ts invokes, so the call list above misses none", () => {
    const names = [...apiSource.matchAll(/invoke(?:<[^>]*>)?\("(\w+)"/g)].map((m) => m[1]);
    expect(names.sort()).toEqual(Object.keys(fixture.commands).sort());
  });
});
