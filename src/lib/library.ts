// The Library setting's wording (design 2026-11-13, §4.1, §4.6, §4.7): pure,
// so it is tested without a window. Counts are of source files, not papers.
import type { Changed, LibrarySettled, LibraryStatus, SourceKind } from "./api";

/** `n` and `noun`, plural unless `n` is one ("1 file", "3 files"). */
function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? "" : "s"}`;
}

/** The setting's caption: no library, the folder, or the folder and why it cannot be reached. */
export function libraryCaption(s: LibraryStatus): string {
  if (s.path === null) return "No library folder: papers are kept in the app.";
  return s.unreachable === null ? s.path : `${s.path} cannot be reached: ${s.unreachable}`;
}

/** The status line shown while the library set cannot be reached, or null. */
export function unreachableNote(s: LibraryStatus): string | null {
  if (s.unreachable === null) return null;
  return `The paper library cannot be reached: ${s.unreachable}. Papers open from what the app keeps, and new files wait in the app until it is back.`;
}

/**
 * The label of the checkbox beside *Choose…*: with no library, moving the
 * files the app keeps into the folder chosen; with one, copying this app's
 * files from it. Null when there is nothing to bring (no checkbox).
 */
export function bringLabel(s: LibraryStatus): string | null {
  if (s.path === null) {
    return s.counts.app > 0 ? `Move the ${count(s.counts.app, "paper file")} kept in the app into it` : null;
  }
  return s.counts.library > 0
    ? `Copy this app's ${count(s.counts.library, "paper file")} from the current library`
    : null;
}

/** What a change of the setting did: the counts, then each paper that failed and why. */
export function changeNote(c: Changed): string {
  const done: string[] = [];
  if (c.copied > 0) done.push(`${count(c.copied, "file")} copied`);
  if (c.moved > 0) done.push(`${c.copied > 0 ? c.moved : count(c.moved, "file")} moved into the library`);
  const parts = done.length > 0 ? [`${done.join(", ")}.`] : [];
  if (c.failed.length > 0) {
    parts.push(`${count(c.failed.length, "file")} could not be copied or moved.`);
    parts.push(...c.failed.map((f) => `${f.title}: ${f.why}.`));
  }
  return parts.length > 0 ? parts.join(" ") : "Nothing to move.";
}

/** The status line after waiting sources were settled, or null when nothing moved or waits. */
export function settledNote(e: LibrarySettled): string | null {
  if (e.moved === 0 && e.left === 0) return null;
  const wait = e.left === 1 ? "waits" : "wait";
  if (e.moved === 0) return `${count(e.left, "file")} still ${wait} for the paper library.`;
  const moved = `${count(e.moved, "file")} moved into the paper library.`;
  return e.left === 0 ? moved : `${moved} ${e.left} still ${wait}.`;
}

/** Each source kind as the waiting list names it. */
export const KIND_NAMES: Record<SourceKind, string> = {
  pdf: "PDF",
  jats: "JATS",
  html: "HTML",
  markdown: "Markdown",
  plain_text: "text",
};

/** The label of the button that moves the files the app keeps into the library set. */
export function moveLabel(s: LibraryStatus): string {
  return `Move the ${count(s.counts.app, "paper file")} kept in the app into the library`;
}
