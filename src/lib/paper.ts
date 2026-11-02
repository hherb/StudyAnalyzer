// Pure helpers for rendering a paper. No Tauri, no DOM.
import type { BlockView, Decoded, FullTextStatus, Identifiers, Utf16Span } from "./api";

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
  return "Confirm above that this is the whole paper, or add the full text, before asking about it.";
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
