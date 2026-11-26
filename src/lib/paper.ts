// Pure helpers for rendering a paper. No Tauri, no DOM (a selection is read through `PaperNode`).
import type { BlockView, Decoded, FullTextStatus, Identifiers, PaperView, Utf16Span } from "./api";

/** A rendered group: one paragraph or a whole table. */
export type Group =
  | { kind: "paragraph"; span: Utf16Span }
  | { kind: "table"; rows: Utf16Span[] };

/** The text a UTF-16 span names. JS strings index by UTF-16 unit, so `slice` is exact. */
export function sliceSpan(text: string, span: Utf16Span): string {
  return text.slice(span.start, span.end);
}

/** Groups blocks for rendering: consecutive table rows form one table. */
export function groupBlocks(blocks: BlockView[]): Group[] {
  const out: Group[] = [];
  for (const b of blocks) {
    const last = out[out.length - 1];
    if (b.kind === "table_row") {
      if (last?.kind === "table") last.rows.push(b.span);
      else out.push({ kind: "table", rows: [b.span] });
    } else {
      out.push({ kind: "paragraph", span: b.span });
    }
  }
  return out;
}

/** Splits a table row into its cells (the canonical separator is a tab). */
export function splitCells(row: string): string[] {
  return row.split("\t");
}

/** Each cell of a table row as its own span of `text`, so a cell can carry highlights; cells as `splitCells` gives them. */
export function cellSpans(text: string, row: Utf16Span): Utf16Span[] {
  let start = row.start;
  return splitCells(sliceSpan(text, row)).map((cell) => {
    const span = { start, end: start + cell.length };
    start = span.end + 1; // past the tab
    return span;
  });
}

/**
 * The banner sentence naming what the full-text check found missing (Rust's
 * `missing_headings`, too few words); null when it passed or the user confirmed.
 */
export function bannerText(status: FullTextStatus): string | null {
  if (status.passed || status.confirmed) return null;
  const missing = status.missing_headings.map((h) => `no ${h} heading`);
  if (status.body_words < status.min_body_words) {
    missing.push(`fewer than ${status.min_body_words.toLocaleString("en-US")} words (${status.body_words})`);
  }
  const why = missing.length ? missing.join("; ") : "it did not look like a whole paper";
  return `This may not be the whole paper: ${why}.`;
}

/**
 * Why nothing can be asked about the text yet, shown in place of the question
 * box; null when it passed the full-text check or the user confirmed it.
 */
export function askBlockedNote(status: FullTextStatus): string | null {
  if (status.passed || status.confirmed) return null;
  return "Confirm above that this is the whole paper before asking about it.";
}

/** A warning when the file's encoding was guessed; null otherwise. */
export function encodingNote(decoded: Decoded | null): string | null {
  if (!decoded?.guessed) return null;
  return `This file did not say how its text is encoded; it was read as ${decoded.encoding}. If characters look wrong, paste the text instead.`;
}

/** The banner's notes: the encoding warning, if any, then the open's notices (Rust's `OpenOutcome` `notices`). */
export function bannerNotes(decoded: Decoded | null, notices: string[]): string[] {
  const encoding = encodingNote(decoded);
  return encoding ? [encoding, ...notices] : notices;
}

/** The identifier to look a paper up by again: DOI, else PMID, else PMC ID; null if none. */
export function retryIdentifier(ids: Identifiers): string | null {
  return ids.doi ?? ids.pmid ?? ids.pmc_id ?? null;
}

/** The heading of the needs-full-text screen: the cause when the sources were unreachable, else neutral. */
export function needsFullTextHeading(unreachable: boolean): string {
  return unreachable
    ? "The sources could not be reached — check the connection and try again"
    : "No full text yet";
}

/** The attribute the paper pane puts on each block it renders: the block's start in `view.text`, in UTF-16 units. */
export const BLOCK_START_ATTR = "data-start";

/** A DOM text node's `nodeType` (`Node.TEXT_NODE`, which a test without a DOM has no `Node` for). */
export const TEXT_NODE = 3;

/** What the paper pane says while it waits for the sentence a mark rests on. */
export const SELECT_PROMPT = "Select the sentence in the paper.";

/** What the paper pane says when a selection is not one it can mark. */
export const SELECT_ONE_PARAGRAPH = "Select text inside one paragraph.";

/** The part of a DOM node `selectionSpan` reads; a DOM `Node` is one, and a test can build one without a DOM. */
export interface PaperNode {
  /** `TEXT_NODE` for text, anything else for an element. */
  readonly nodeType: number;
  /** The node it sits in; null at the top. */
  readonly parentNode: PaperNode | null;
  /** Its children, in order. */
  readonly childNodes: ArrayLike<PaperNode>;
  /** Its text (a text node's own, an element's children's joined). */
  readonly textContent: string | null;
  /** An element's attribute; text nodes have none. */
  getAttribute?(name: string): string | null;
}

/** The part of a DOM `Selection` `selectionSpan` reads: where it starts (anchor) and ends (focus). */
export interface SelectionPoints {
  readonly anchorNode: PaperNode | null;
  readonly anchorOffset: number;
  readonly focusNode: PaperNode | null;
  readonly focusOffset: number;
}

/** The block `node` sits in (the nearest node carrying `BLOCK_START_ATTR`), if that block lies inside `root`. */
function blockOf(root: PaperNode, node: PaperNode): PaperNode | null {
  let block: PaperNode | null = null;
  for (let n: PaperNode | null = node; n !== null; n = n.parentNode) {
    if (n === root) return block;
    if (block === null && n.getAttribute?.(BLOCK_START_ATTR) != null) block = n;
  }
  return null;
}

/** How many UTF-16 units of `block`'s text come before the DOM boundary point (`node`, `offset`). */
function offsetIn(block: PaperNode, node: PaperNode, offset: number): number {
  let total = 0;
  const visit = (n: PaperNode): boolean => {
    if (n === node) {
      if (n.nodeType === TEXT_NODE) total += offset;
      // An element's boundary point counts its children before `offset`.
      else for (let i = 0; i < offset; i++) total += n.childNodes[i].textContent?.length ?? 0;
      return true;
    }
    if (n.nodeType === TEXT_NODE) {
      total += n.textContent?.length ?? 0;
      return false;
    }
    for (let i = 0; i < n.childNodes.length; i++) if (visit(n.childNodes[i])) return true;
    return false;
  };
  visit(block);
  return total;
}

/**
 * The span of `view.text`, in UTF-16 units, that selection `sel` covers in
 * the paper pane rendered under `root`: from the rendered text nodes back to
 * the canonical text, through each block's `BLOCK_START_ATTR`. Null for a
 * selection that is empty, reaches outside the paper or outside a block, or
 * spans two blocks (a paragraph, heading, title or table cell each is one),
 * or for a block whose rendered text is not the paper's at its offset.
 */
export function selectionSpan(root: PaperNode, view: PaperView, sel: SelectionPoints): Utf16Span | null {
  if (sel.anchorNode === null || sel.focusNode === null) return null;
  const block = blockOf(root, sel.anchorNode);
  if (block === null || blockOf(root, sel.focusNode) !== block) return null;
  const start = Number(block.getAttribute?.(BLOCK_START_ATTR));
  const rendered = block.textContent ?? "";
  if (!Number.isInteger(start) || view.text.slice(start, start + rendered.length) !== rendered) return null;
  const a = start + offsetIn(block, sel.anchorNode, sel.anchorOffset);
  const f = start + offsetIn(block, sel.focusNode, sel.focusOffset);
  return a === f ? null : { start: Math.min(a, f), end: Math.max(a, f) };
}
