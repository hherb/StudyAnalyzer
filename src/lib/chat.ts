// Pure helpers for the Interrogator: streaming quotes, paper highlights, citation notes. No Tauri, no DOM.
import type { TurnView, Utf16Span } from "./api";

/** Opens a quote in the model's text. Must equal Rust's `config::QUOTE_OPEN`. */
export const QUOTE_OPEN = "<q>";
/** Closes a quote in the model's text. Must equal Rust's `config::QUOTE_CLOSE`. */
export const QUOTE_CLOSE = "</q>";

/** A piece of streaming text: plain, or inside a `<q>` quote. */
export type LiveSegment = { kind: "text"; text: string } | { kind: "quote"; text: string };

/** How a run of the paper is marked: not cited, cited earlier, or the citation in focus. */
export type Mark = "none" | "past" | "current";

/** The length of the longest strict prefix of either quote tag (shorter than the whole tag) that ends `text`; 0 if none. */
function partialTagLength(text: string): number {
  let best = 0;
  for (const tag of [QUOTE_OPEN, QUOTE_CLOSE]) {
    for (let n = Math.min(tag.length - 1, text.length); n > best; n--) {
      if (text.endsWith(tag.slice(0, n))) best = n;
    }
  }
  return best;
}

/**
 * Splits streaming text into plain and quote segments. An unclosed quote still
 * counts as a quote; a trailing partial tag (`<`, `</q`, ...) is held back so
 * it is never shown, and joins the next chunk's tag when it arrives.
 */
export function liveSegments(text: string): LiveSegment[] {
  const rest = text.slice(0, text.length - partialTagLength(text));
  const out: LiveSegment[] = [];
  let pos = 0;
  let inQuote = false;
  while (pos < rest.length) {
    const tag = inQuote ? QUOTE_CLOSE : QUOTE_OPEN;
    const at = rest.indexOf(tag, pos);
    const end = at < 0 ? rest.length : at;
    if (end > pos) out.push({ kind: inQuote ? "quote" : "text", text: rest.slice(pos, end) });
    if (at < 0) break;
    pos = at + tag.length;
    inQuote = !inQuote;
  }
  return out;
}

/**
 * Splits `block` into runs by mark. Marks are clipped to the block and ones
 * that miss it are ignored; where marks overlap `current` wins over `past`.
 * Runs are in order, cover `block` exactly and adjacent equal runs are merged.
 */
export function highlightRuns(
  block: Utf16Span,
  marks: Array<{ span: Utf16Span; current: boolean }>,
): Array<{ span: Utf16Span; mark: Mark }> {
  const inside = marks.filter((m) => m.span.start < block.end && m.span.end > block.start);
  const edges = new Set([block.start, block.end]);
  for (const m of inside) {
    edges.add(Math.max(m.span.start, block.start));
    edges.add(Math.min(m.span.end, block.end));
  }
  const cuts = [...edges].sort((a, b) => a - b);
  const out: Array<{ span: Utf16Span; mark: Mark }> = [];
  for (let i = 0; i + 1 < cuts.length; i++) {
    const [start, end] = [cuts[i], cuts[i + 1]];
    const covering = inside.filter((m) => m.span.start <= start && m.span.end >= end);
    const mark: Mark = covering.some((m) => m.current) ? "current" : covering.length ? "past" : "none";
    const last = out[out.length - 1];
    if (last?.mark === mark) last.span.end = end;
    else out.push({ span: { start, end }, mark });
  }
  return out;
}

/** Names how many quotes of an answer could not be found in the paper; null when none. */
export function unlocatedNote(n: number): string | null {
  if (n === 0) return null;
  return `${n} ${n === 1 ? "quote" : "quotes"} could not be found in the paper`;
}

/** The note on a stopped or failed answer (its quotes were never checked); null when complete. */
export function incompleteNote(status: TurnView["status"]): string | null {
  return status === "complete" ? null : "Incomplete answer: its quotes were not checked.";
}

/** The note on a complete answer without a single `<q>` quote (located or not); null otherwise. */
export function noQuotesNote(turn: TurnView): string | null {
  const quoted = turn.citations.length > 0 || turn.unlocated.length > 0;
  return turn.status === "complete" && !quoted ? "This answer quotes nothing from the paper." : null;
}

/**
 * An answer being prepared: its question, its raw text so far, the indexing
 * progress (null when not indexing), whether the search model is being
 * checked, and whether Stop was pressed.
 */
export type LiveAnswer = {
  question: string;
  text: string;
  indexing: { done: number; total: number } | null;
  checking: boolean;
  stopping: boolean;
};

/**
 * What an answer being prepared is doing, shown above its text: *Stopping…*
 * once Stop is pressed, else checking the search model, finding passages, or
 * waiting for the model's first text; null while text streams.
 */
export function liveNote(live: LiveAnswer): string | null {
  if (live.stopping) return "Stopping…";
  if (live.checking) return "Checking the search model…";
  if (live.indexing) return `Finding passages… ${live.indexing.done} / ${live.indexing.total}`;
  return live.text === "" ? "Waiting for the model…" : null;
}

/**
 * Every citation of every turn as a mark on the paper. Numbers restart per
 * turn, so `current` names a citation by its turn id and number.
 */
export function paperMarks(
  turns: TurnView[],
  current: { turnId: number; number: number } | null,
): Array<{ span: Utf16Span; current: boolean }> {
  return turns.flatMap((t) =>
    t.citations.map((c) => ({
      span: c.span,
      current: current?.turnId === t.id && current.number === c.number,
    })),
  );
}
