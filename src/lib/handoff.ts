/** Pure helpers for received hand-offs: which to open next, and how to describe one. */
import type { HandoffSummary } from "./api";
import { CREATED_AT_FORMAT, parseSqliteUtc } from "./interrogator";
import { retryIdentifier } from "./paper";

/** What shows while a paper is being opened. */
export const OPENING = "Opening…";
/** What shows while a lookup runs; it is the only text that offers Cancel. */
export const LOOKING = "Looking for the full text…";

/** The newest pending hand-off not yet tried this launch (`pending` is newest first), or null while busy or while Settings is open (it waits there). */
export function nextHandoff(
  pending: HandoffSummary[],
  tried: ReadonlySet<number>,
  busy: boolean,
  settingsOpen = false,
): HandoffSummary | null {
  if (busy || settingsOpen) return null;
  return pending.find((h) => !tried.has(h.id)) ?? null;
}

/**
 * The line saying a hand-off is held back, or null: the newest untried hand-off that is not a
 * refusal waits while Settings is open or an error is shown, and only the start screen lists it.
 */
export function waitingNote(
  pending: HandoffSummary[],
  tried: ReadonlySet<number>,
  settingsOpen: boolean,
  errorShown: boolean,
): string | null {
  const h = pending.find((p) => !p.refusal && !tried.has(p.id));
  const until = [settingsOpen && "you leave Settings", errorShown && "you dismiss this message"].filter(Boolean);
  if (!h || until.length === 0) return null;
  return `A paper handed over by ${h.sender ?? "another app"} will open once ${until.join(" and ")}.`;
}

/** One line: title (else first identifier, else "untitled"), "from {sender}" when known, then the arrival time. */
export function handoffCaption(h: HandoffSummary): string {
  const when = parseSqliteUtc(h.received_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? h.received_at;
  return [
    h.title ?? retryIdentifier(h.identifiers) ?? "untitled",
    h.sender && `from ${h.sender}`,
    when,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The busy text for opening a hand-off: a lookup (cancellable) without full text, else the plain opening text. */
export function handoffBusyText(h: HandoffSummary): string {
  return h.has_full_text ? OPENING : LOOKING;
}
