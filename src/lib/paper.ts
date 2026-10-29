// Pure helpers for rendering a paper. No Tauri, no DOM.
import type { BlockView, Decoded, FullTextStatus, Utf16Span } from "./api";

/** The full-text check's minimum body length; mirrors the Rust check. */
export const MIN_BODY_WORDS = 1000;

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
  const missing: string[] = [];
  const has = (t: string) => status.section_types.includes(t);
  if (!has("methods")) missing.push("no Methods heading");
  if (!has("results") && !has("discussion")) missing.push("no Results or Discussion heading");
  if (status.body_words < MIN_BODY_WORDS) {
    missing.push(`fewer than ${MIN_BODY_WORDS.toLocaleString("en-US")} words (${status.body_words})`);
  }
  const why = missing.length ? missing.join("; ") : "it did not look like a whole paper";
  return `This may not be the whole paper: ${why}.`;
}

/** A warning when the file's encoding was guessed; null otherwise. */
export function encodingNote(decoded: Decoded | null): string | null {
  if (!decoded?.guessed) return null;
  return `This file did not say how its text is encoded; it was read as ${decoded.encoding}. If characters look wrong, paste the text instead.`;
}
