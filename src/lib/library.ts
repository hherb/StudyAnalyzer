// The Library setting's wording (design 2026-11-13, §4.1, §4.6, §4.7, and the
// manual clear, §4.8): pure, so it is tested without a window. Counts are of
// source files, not papers.
import type { Changed, LibrarySettled, LibraryStatus, ListedPaper, SourceKind } from "./api";

/** `n` and `noun`, plural unless `n` is one ("1 file", "3 files"). */
function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? "" : "s"}`;
}

/** The setting's caption: no library, the folder, or the folder and why it cannot be reached. */
export function libraryCaption(s: LibraryStatus): string {
  if (s.path === null) {
    const none = "No library folder: papers are kept in the app.";
    const n = s.counts.library;
    if (n === 0) return none;
    return `${none} ${count(n, "paper file")} ${n === 1 ? "is" : "are"} still in the library folder set before and cannot be read until it is chosen again.`;
  }
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
 * files from it. Null when there is nothing to bring, or the library set
 * cannot be reached to copy from (no checkbox).
 */
export function bringLabel(s: LibraryStatus): string | null {
  if (s.unreachable !== null) return null;
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
  return parts.length > 0 ? parts.join(" ") : "Nothing was copied or moved.";
}

/** The status line after waiting sources were settled (or settling failed), or null when nothing moved or waits. */
export function settledNote(e: LibrarySettled): string | null {
  if (e.error !== null) return `The files waiting for the paper library could not be moved into it: ${e.error}`;
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

/** Bytes in each unit a size is shown in, largest first: decimal, as Finder counts. */
const SIZE_UNITS: [number, string][] = [
  [1e9, "GB"],
  [1e6, "MB"],
  [1e3, "KB"],
];

/** A size in bytes as the clear shows it ("1.2 MB", "512 bytes"), or "size unknown" for null. */
export function sizeLabel(bytes: number | null): string {
  if (bytes === null) return "size unknown";
  const unit = SIZE_UNITS.find(([n]) => bytes >= n);
  return unit ? `${(bytes / unit[0]).toFixed(1)} ${unit[1]}` : count(bytes, "byte");
}

/** A paper's size: the sum of its files', saying so when some could not be read. */
export function paperSize(p: ListedPaper): string {
  const known = p.files.flatMap((f) => (f.size === null ? [] : [f.size]));
  if (known.length === 0) return sizeLabel(null);
  const sum = sizeLabel(known.reduce((a, b) => a + b, 0));
  return known.length < p.files.length ? `${sum}, some sizes unknown` : sum;
}

/** Who uses a paper: the other installations using any of its files (each once), and whether this one does. */
export function paperUsers(p: ListedPaper): { usedBy: string[]; usedHere: boolean } {
  return {
    usedBy: [...new Set(p.files.flatMap((f) => f.used_by))],
    usedHere: p.files.some((f) => f.used_here),
  };
}

/** Who uses a file or paper, as its line in the list says it. */
export function usersNote(usedBy: string[], usedHere: boolean): string {
  const who = [...(usedHere ? ["this app"] : []), ...usedBy];
  return who.length === 0 ? "used by no app" : `used by ${who.join(", ")}`;
}

/**
 * The confirmation before `what` (a paper's title or a file's name) is removed from the library:
 * it names the other apps still using it, and says what this app keeps when it uses it too.
 */
export function removeQuestion(what: string, usedBy: string[], usedHere: boolean): string {
  const parts = [`Remove \u201c${what}\u201d from the paper library? It is deleted from the folder.`];
  if (usedBy.length > 0) {
    parts.push(
      `Still used by ${usedBy.join(", ")}: ${usedBy.length === 1 ? "it reports" : "they report"} it missing and will not download it again.`,
    );
  } else if (!usedHere) {
    parts.push("No app uses it.");
  }
  if (usedHere) {
    parts.push(
      "This app's paper then says its file is gone; its text, conversations and PICO runs stay, and opening the same file again puts it back.",
    );
  }
  return parts.join(" ");
}

/** What the scan found odd, as one line under the list, or null when nothing was. */
export function problemsNote(problems: string[]): string | null {
  if (problems.length === 0) return null;
  return `The library scan also found ${count(problems.length, "thing")} it did not use: ${problems.join("; ")}.`;
}
