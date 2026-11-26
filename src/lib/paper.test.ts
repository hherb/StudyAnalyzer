import { describe, expect, test } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import type { BlockView, FullTextStatus, Identifiers, PaperView } from "./api";
import {
  BLOCK_START_ATTR,
  SELECT_ONE_PARAGRAPH,
  TEXT_NODE,
  selectionSpan,
  type PaperNode,
  needsFullTextHeading, askBlockedNote, bannerNotes, bannerText, encodingNote, groupBlocks, sliceSpan, retryIdentifier, splitCells, cellSpans } from "./paper";

/** A paragraph block covering `start`..`end`. */
const p = (start: number, end: number): BlockView => ({ kind: "paragraph", span: { start, end } });
/** A table-row block covering `start`..`end`. */
const row = (start: number, end: number): BlockView => ({ kind: "table_row", span: { start, end } });
/** A failed, unconfirmed full-text status with ample body words, overridden by `o`. */
const status = (o: Partial<FullTextStatus> = {}): FullTextStatus => ({
  passed: false,
  confirmed: false,
  body_words: 5000,
  min_body_words: 1000,
  missing_headings: [],
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
  test("cellSpans slices to the cells, after astral characters too", () => {
    const text = "𝛼 x\ta\t\tb";
    const spans = cellSpans(text, { start: 2, end: text.length });
    expect(spans.map((s) => sliceSpan(text, s))).toEqual([" x", "a", "", "b"]);
    expect(spans[3]).toEqual({ start: text.length - 1, end: text.length });
  });
  test("bannerText", () => {
    expect(bannerText(status({ passed: true }))).toBeNull();
    expect(bannerText(status({ confirmed: true }))).toBeNull();
    expect(bannerText(status({ missing_headings: ["Methods"] }))).toContain("Methods");
    expect(bannerText(status({ missing_headings: ["Results or Discussion"] }))).toContain("Results or Discussion");
    expect(bannerText(status({ body_words: 200 }))).toContain("1,000 words");
    expect(bannerText(status())).toContain("it did not look like a whole paper");
  });
  test("askBlockedNote", () => {
    expect(askBlockedNote(status({ passed: true }))).toBeNull();
    expect(askBlockedNote(status({ confirmed: true }))).toBeNull();
    expect(askBlockedNote(status())).toContain("Confirm");
  });
  test("encodingNote", () => {
    expect(encodingNote(null)).toBeNull();
    expect(encodingNote({ encoding: "utf-8", guessed: false })).toBeNull();
    expect(encodingNote({ encoding: "windows-1252", guessed: true })).toContain("windows-1252");
  });
  test("bannerNotes puts the encoding warning first, then the notices", () => {
    expect(bannerNotes(null, [])).toEqual([]);
    expect(bannerNotes(null, ["n"])).toEqual(["n"]);
    const guessed = bannerNotes({ encoding: "windows-1252", guessed: true }, ["n"]);
    expect(guessed).toHaveLength(2);
    expect(guessed[0]).toContain("windows-1252");
    expect(guessed[1]).toBe("n");
  });
  test("retryIdentifier prefers DOI, then PMID, then PMC", () => {
    const ids = (o: Partial<Identifiers>): Identifiers => ({ pmid: null, doi: null, pmc_id: null, ...o });
    expect(retryIdentifier(ids({ doi: "10.1/x", pmid: "1", pmc_id: "PMC2" }))).toBe("10.1/x");
    expect(retryIdentifier(ids({ pmid: "1", pmc_id: "PMC2" }))).toBe("1");
    expect(retryIdentifier(ids({ pmc_id: "PMC2" }))).toBe("PMC2");
    expect(retryIdentifier(ids({}))).toBeNull();
  });
});

describe("needsFullTextHeading", () => {
  test("names the connection when unreachable", () => {
    expect(needsFullTextHeading(true)).toContain("could not be reached");
  });
  test("is neutral otherwise", () => {
    expect(needsFullTextHeading(false)).toBe("No full text yet");
  });
});

describe("selectionSpan", () => {
  /** The fixture paper: its text has astral characters in the title, a heading and the first paragraph. */
  const paper = fixture.paper_view as PaperView;

  /** A fake DOM node: just what `selectionSpan` reads, so the test needs no DOM. */
  interface Fake extends PaperNode {
    parentNode: Fake | null;
    childNodes: Fake[];
  }
  /** A text node holding `data`. */
  const txt = (data: string): Fake => ({ nodeType: TEXT_NODE, parentNode: null, childNodes: [], textContent: data });
  /** An element with attributes `attrs` around `kids`. */
  function el(attrs: Record<string, string>, ...kids: Fake[]): Fake {
    const e: Fake = {
      nodeType: 1,
      parentNode: null,
      childNodes: kids,
      get textContent() {
        return kids.map((k) => k.textContent).join("");
      },
      getAttribute: (name: string) => attrs[name] ?? null,
    };
    for (const k of kids) k.parentNode = e;
    return e;
  }
  /** A block the pane renders from `span` of the paper's text, its runs split at `cuts` (offsets into the block). */
  function block(span: { start: number; end: number }, cuts: number[] = []): Fake {
    const text = sliceSpan(paper.text, span);
    const edges = [0, ...cuts, text.length];
    const runs = edges.slice(1).map((end, i) => txt(text.slice(edges[i], end)));
    // Every other run sits in a <mark>, as a highlighted run does.
    const kids = runs.map((r, i) => (i % 2 === 1 ? el({}, r) : r));
    return el({ [BLOCK_START_ATTR]: String(span.start) }, ...kids);
  }
  const heading = block({ start: 10, end: 20 });
  /** "Body 😀 text." as three runs: "Body ", the marked "😀", " text.". */
  const body = block({ start: 22, end: 35 }, [5, 7]);
  const root = el({}, heading, el({}, body));
  /** A selection from (`an`, `ao`) to (`fn`, `fo`). */
  const sel = (an: Fake | null, ao: number, fn: Fake | null, fo: number) => ({
    anchorNode: an,
    anchorOffset: ao,
    focusNode: fn,
    focusOffset: fo,
  });
  const [before, marked, after] = body.childNodes;
  const markedText = marked.childNodes[0];

  test("inside a paragraph, after an astral character: offsets into the canonical text in UTF-16 units", () => {
    const span = selectionSpan(root, paper, sel(after, 1, after, 5));
    expect(span).toEqual({ start: 30, end: 34 });
    expect(sliceSpan(paper.text, span!)).toBe("text");
  });

  test("across a <mark>, either way round", () => {
    const span = selectionSpan(root, paper, sel(before, 1, after, 5));
    expect(sliceSpan(paper.text, span!)).toBe("ody 😀 text");
    expect(selectionSpan(root, paper, sel(after, 5, before, 1))).toEqual(span);
    expect(sliceSpan(paper.text, selectionSpan(root, paper, sel(markedText, 0, after, 1))!)).toBe("😀 ");
  });

  test("an element's boundary counts its children before it: the whole paragraph", () => {
    expect(selectionSpan(root, paper, sel(body, 0, body, 3))).toEqual({ start: 22, end: 35 });
  });

  test("across two blocks, empty, or outside the paper: null", () => {
    expect(selectionSpan(root, paper, sel(heading.childNodes[0], 2, after, 2))).toBeNull();
    expect(selectionSpan(root, paper, sel(after, 2, after, 2))).toBeNull();
    expect(selectionSpan(root, paper, sel(null, 0, null, 0))).toBeNull();
    const elsewhere = block({ start: 22, end: 35 });
    el({}, elsewhere);
    const out = elsewhere.childNodes[0];
    expect(selectionSpan(root, paper, sel(out, 0, out, 4))).toBeNull();
    // Inside the paper but in no block (the title shown without a span).
    const bare = txt("Trial");
    el({}, root, bare);
    expect(selectionSpan(root, paper, sel(bare, 0, bare, 3))).toBeNull();
  });

  test("a block whose text is not the paper's at its offset (a stale render): null", () => {
    const stale = el({ [BLOCK_START_ATTR]: "23" }, txt("Body"));
    const r = el({}, stale);
    expect(selectionSpan(r, paper, sel(stale.childNodes[0], 0, stale.childNodes[0], 2))).toBeNull();
  });

  test("the pane's note for a selection that is not one", () => {
    expect(SELECT_ONE_PARAGRAPH).toBe("Select text inside one paragraph.");
  });
});
