import { describe, expect, test } from "vitest";
import type { Changed, LibraryStatus } from "./api";
import { SOURCE_KINDS } from "./api";
import { KIND_NAMES, bringLabel, changeNote, libraryCaption, moveLabel, settledNote, unreachableNote } from "./library";

/** A status with no library and nothing kept, changed by `over`. */
function status(over: Partial<LibraryStatus> = {}): LibraryStatus {
  return {
    path: null,
    unreachable: null,
    counts: { app: 0, waiting: 0, library: 0 },
    waiting: [],
    ...over,
  };
}

describe("libraryCaption", () => {
  test("says what the setting is", () => {
    expect(libraryCaption(status())).toBe("No library folder: papers are kept in the app.");
    expect(libraryCaption(status({ path: "/P" }))).toBe("/P");
    expect(libraryCaption(status({ path: "/P", unreachable: "it is unplugged" }))).toBe(
      "/P cannot be reached: it is unplugged",
    );
  });

  test("with no library set, says which files still read from a library folder", () => {
    expect(libraryCaption(status({ counts: { app: 0, waiting: 0, library: 2 } }))).toBe(
      "No library folder: papers are kept in the app. 2 paper files are still in the library folder set before and cannot be read until it is chosen again.",
    );
  });
});

describe("unreachableNote", () => {
  test("is the status line only while the library cannot be reached", () => {
    expect(unreachableNote(status({ path: "/P" }))).toBeNull();
    expect(unreachableNote(status({ path: "/P", unreachable: "it is unplugged" }))).toBe(
      "The paper library cannot be reached: it is unplugged. Papers open from what the app keeps, and new files wait in the app until it is back.",
    );
  });
});

describe("bringLabel", () => {
  test("offers the move with no library, the copy with one, nothing when there is nothing to bring", () => {
    expect(bringLabel(status())).toBeNull();
    expect(bringLabel(status({ counts: { app: 1, waiting: 0, library: 0 } }))).toBe(
      "Move the 1 paper file kept in the app into it",
    );
    expect(bringLabel(status({ counts: { app: 3, waiting: 0, library: 0 } }))).toBe(
      "Move the 3 paper files kept in the app into it",
    );
    expect(bringLabel(status({ path: "/P", counts: { app: 0, waiting: 0, library: 2 } }))).toBe(
      "Copy this app's 2 paper files from the current library",
    );
    expect(bringLabel(status({ path: "/P", counts: { app: 4, waiting: 0, library: 0 } }))).toBeNull();
    expect(
      bringLabel(status({ path: "/P", unreachable: "gone", counts: { app: 0, waiting: 0, library: 2 } })),
    ).toBeNull();
  });
});

describe("changeNote", () => {
  const none: Changed = { copied: 0, moved: 0, failed: [] };
  test("counts what was done, singular and plural, and names each failure", () => {
    expect(changeNote(none)).toBe("Nothing was copied or moved.");
    expect(changeNote({ ...none, moved: 1 })).toBe("1 file moved into the library.");
    expect(changeNote({ copied: 3, moved: 5, failed: [] })).toBe("3 files copied, 5 moved into the library.");
    expect(changeNote({ copied: 1, moved: 0, failed: [] })).toBe("1 file copied.");
    expect(
      changeNote({ ...none, failed: [{ paper_id: "p", title: "A trial", why: "it is gone" }] }),
    ).toBe("1 file could not be copied or moved. A trial: it is gone.");
  });
});

describe("settledNote", () => {
  test("says how many moved and how many still wait, or nothing", () => {
    expect(settledNote({ moved: 0, left: 0, error: null })).toBeNull();
    expect(settledNote({ moved: 1, left: 0, error: null })).toBe("1 file moved into the paper library.");
    expect(settledNote({ moved: 2, left: 1, error: null })).toBe("2 files moved into the paper library. 1 still waits.");
    expect(settledNote({ moved: 0, left: 3, error: null })).toBe("3 files still wait for the paper library.");
    expect(settledNote({ moved: 0, left: 1, error: null })).toBe("1 file still waits for the paper library.");
    expect(settledNote({ moved: 0, left: 0, error: "the database is locked" })).toBe(
      "The files waiting for the paper library could not be moved into it: the database is locked",
    );
  });
});

describe("KIND_NAMES and moveLabel", () => {
  test("every source kind has a name", () => {
    expect(SOURCE_KINDS.map((k) => KIND_NAMES[k])).toEqual(["PDF", "JATS", "HTML", "Markdown", "text"]);
  });
  test("the move button counts the files kept in the app", () => {
    expect(moveLabel(status({ path: "/P", counts: { app: 2, waiting: 0, library: 0 } }))).toBe(
      "Move the 2 paper files kept in the app into the library",
    );
  });
});
