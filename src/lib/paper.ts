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

/** What the full-text check found missing; null when it passed or the user confirmed. */
export function bannerText(status: FullTextStatus): string | null {
  if (status.passed || status.confirmed) return null;
  const missing = status.missing_headings.map((h) => `no ${h} heading`);
  if (status.body_words < status.min_body_words) {
    missing.push(`fewer than ${status.min_body_words.toLocaleString("en-US")} words (${status.body_words})`);
  }
  const why = missing.length ? missing.join("; ") : "it did not look like a whole paper";
  return `This may not be the whole paper: ${why}.`;
}

/** A warning when the file's encoding was guessed; null otherwise. */
export function encodingNote(decoded: Decoded | null): string | null {
  if (!decoded?.guessed) return null;
  return `This file did not say how its text is encoded; it was read as ${decoded.encoding}. If characters look wrong, paste the text instead.`;
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
