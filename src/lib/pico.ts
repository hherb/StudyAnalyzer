// What the PICO tab shows and when it runs: labels, notes, marks and captions.
import type { CommandError, PicoEvent, PicoPartView, PicoRunView, PicoView, SuitabilityView, Utf16Span } from "./api";
import { CREATED_AT_FORMAT, parseSqliteUtc } from "./interrogator";

/** Each PICO part's label on the card. */
export const PART_LABELS: Record<PicoPartView["part"], string> = {
  population: "Population",
  intervention: "Intervention",
  comparison: "Comparison",
  outcome: "Outcome",
};

/** The note under a part that the paper does not back; null for a verified one. */
export function partNote(part: PicoPartView): string | null {
  if (part.state === "unverified") {
    return `Unsure – model could not reproduce ${PART_LABELS[part.part]} from text provided`;
  }
  return part.state === "not_reported" ? "Not clearly stated" : null;
}

/**
 * What the tab says while a run goes: how far a long paper's condensation has
 * got (`event`, null before any arrives or for a paper that needs none), or
 * that the stop was pressed.
 */
export function runningNote(event: PicoEvent | null, stopping: boolean): string {
  if (stopping) return "Stopping…";
  if (event === null) return "Extracting PICO… A long paper is condensed first and can take several minutes.";
  if (event.kind === "condensed") return "Extracting PICO from the condensed paper…";
  const part = `part ${event.batch} of ${event.batches}`;
  return event.level === 0
    ? `Condensing the long paper, ${part}…`
    : `Condensing the condensed parts again (round ${event.level + 1}), ${part}…`;
}

/** The note under study type and sample size: the model's words, never checked. */
export function sampleSizeNote(): string {
  return "As the model gave it; not checked against the paper.";
}

/** How many of a part's quotes were not usable text; null when none. */
export function quotesDroppedNote(part: PicoPartView): string | null {
  const n = part.quotes_dropped;
  if (n === 0) return null;
  return n === 1 ? "1 quote was not usable text" : `${n} quotes were not usable text`;
}

/** Says a verdict came from the abstract, so it never reads as one on the whole paper; null otherwise. */
export function checkNote(s: SuitabilityView): string | null {
  return s.from_abstract ? "Judged from its abstract" : null;
}

/** Every citation of the run as a mark on the paper, by its number; `current` is the clicked one. */
export function picoMarks(run: PicoRunView, current: number | null): Array<{ span: Utf16Span; current: boolean }> {
  return (run.extraction?.parts ?? []).flatMap((p) =>
    p.citations.map((c) => ({ span: c.span, current: c.number === current })),
  );
}

/**
 * The caption under a run: the model, when it ran (local time; a time that
 * cannot be read is shown as stored), whether the paper was condensed, and
 * whether it ran on an older text than `latestCanonicalId`.
 */
export function runCaption(run: PicoRunView, latestCanonicalId: number): string {
  const when = parseSqliteUtc(run.created_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? run.created_at;
  const parts = [run.model, when];
  const condensed = run.extraction?.condensed_from_chars ?? run.suitability?.condensed_from_chars ?? null;
  if (condensed !== null) parts.push(`condensed from ${condensed.toLocaleString("en-US")} characters`);
  if (run.canonical_id !== latestCanonicalId) parts.push("on an earlier version of the text");
  return parts.join(" · ");
}

/** The note on a condensation that did not complete cleanly; null when it did or there was none. */
function statusNote(status: string | null): string | null {
  return status === null ? null : `Condensation ended ${status}; the digest may be incomplete.`;
}

/**
 * Everything bmlib noted on how the run went, check first, so nothing lives
 * only in the database. Any note repeated (a paper condensed once for both
 * calls carries the same notes on each) is shown once.
 */
export function runNotes(run: PicoRunView): string[] {
  const notes: string[] = [];
  for (const step of [run.suitability, run.extraction]) {
    if (step === null) continue;
    notes.push(...step.notes);
    const status = statusNote(step.condensation_status);
    if (status !== null) notes.push(status);
  }
  return [...new Set(notes)];
}

/**
 * Whether opening the tab starts a run: no run on the latest text, a text that
 * may be asked about, a model chosen, and no run attempted since the host
 * mounted (done, failed, stopped or still going), so it never loops.
 */
export function shouldAutoRun(view: PicoView, askable: boolean, modelChosen: boolean, attempted: boolean): boolean {
  return view.current === null && askable && modelChosen && !attempted;
}

/**
 * Whether *Extract PICO* is offered: no run on the latest text, a text that may
 * be asked about, and none going. So it is offered after a failed or stopped
 * run, and after the text is confirmed with the tab open (no auto-run follows).
 */
export function offerRun(view: PicoView, askable: boolean, running: boolean): boolean {
  return view.current === null && askable && !running;
}

/** Why a run did not start, shown in the tab; `settings` when Settings can fix it. */
export type PicoNotice = { text: string; settings: boolean };

/**
 * Where a failed run's error goes: nowhere for the user's own stop
 * (`"nothing"`); a notice in the tab for a missing model or key on an automatic
 * run (with a Settings button, never jumping there unasked), or for a text not
 * yet confirmed (`blocked`, else the error's message); anything else to the
 * error banner (`"error"`), so no failure is swallowed.
 */
export function runFailure(err: CommandError, auto: boolean, blocked: string | null): "nothing" | "error" | PicoNotice {
  if (err.kind === "cancelled") return "nothing";
  if (auto && (err.kind === "needs_model" || err.kind === "needs_api_key")) return { text: err.message, settings: true };
  if (err.kind === "not_full_text") return { text: blocked ?? err.message, settings: false };
  return "error";
}

/** Whether *Re-run* skips the check: a forced run (no check) re-runs forced, so the user's override holds. */
export function rerunForced(run: PicoRunView): boolean {
  return run.suitability === null;
}
