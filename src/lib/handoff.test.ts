import { describe, expect, test } from "vitest";
import type { HandoffSummary } from "./api";
import { CREATED_AT_FORMAT, parseSqliteUtc } from "./interrogator";
import { LOOKING, OPENING, handoffBusyText, handoffCaption, nextHandoff, waitingNote } from "./handoff";

const h = (id: number, over: Partial<HandoffSummary> = {}): HandoffSummary => ({
  id,
  received_at: "2026-11-02 10:00:00",
  sender: null,
  title: null,
  identifiers: { pmid: null, doi: null, pmc_id: null },
  has_full_text: false,
  refusal: null,
  ...over,
});

describe("hand-offs", () => {
  test("nextHandoff_waits_while_busy", () => {
    expect(nextHandoff([h(1)], new Set(), true)).toBeNull();
  });
  test("nextHandoff_waits_in_settings", () => {
    expect(nextHandoff([h(1)], new Set(), false, true)).toBeNull();
    expect(nextHandoff([h(1)], new Set(), false, false)?.id).toBe(1);
  });
  test("nextHandoff_newest_untried", () => {
    const pending = [h(3), h(2), h(1)]; // newest first
    expect(nextHandoff(pending, new Set([3]), false)?.id).toBe(2);
  });
  test("nextHandoff_none", () => {
    expect(nextHandoff([], new Set(), false)).toBeNull();
    expect(nextHandoff([h(1)], new Set([1]), false)).toBeNull();
  });
  test("nextHandoff_refusal_counts", () => {
    expect(nextHandoff([h(1, { refusal: "no" })], new Set(), false)?.id).toBe(1);
  });
  test("waitingNote_says_what_holds_it", () => {
    const one = [h(1, { sender: "MFC" })];
    expect(waitingNote(one, new Set(), true, false)).toBe("A paper handed over by MFC will open once you leave Settings.");
    expect(waitingNote([h(1)], new Set(), false, true)).toBe(
      "A paper handed over by another app will open once you dismiss this message.",
    );
    expect(waitingNote(one, new Set(), true, true)).toBe(
      "A paper handed over by MFC will open once you leave Settings and you dismiss this message.",
    );
  });
  test("waitingNote_none_when_nothing_waits", () => {
    expect(waitingNote([h(1)], new Set(), false, false)).toBeNull();
    expect(waitingNote([h(1)], new Set([1]), true, true)).toBeNull();
    expect(waitingNote([h(1, { refusal: "no" })], new Set(), true, true)).toBeNull();
    expect(waitingNote([], new Set(), true, true)).toBeNull();
  });
  test("handoffCaption_falls_back", () => {
    const when = parseSqliteUtc("2026-11-02 10:00:00")!.toLocaleString(undefined, CREATED_AT_FORMAT);
    expect(handoffCaption(h(1))).toBe(`untitled · ${when}`);
    expect(handoffCaption(h(1, { identifiers: { pmid: "123", doi: "10.1/x", pmc_id: null } }))).toContain("10.1/x");
    expect(handoffCaption(h(1, { title: "T", sender: "MFC" }))).toBe(`T · from MFC · ${when}`);
    expect(handoffCaption(h(1, { received_at: "garbage" }))).toBe("untitled · garbage");
  });
  test("handoffBusyText_lookup_vs_file", () => {
    expect(handoffBusyText(h(1))).toBe(LOOKING);
    expect(handoffBusyText(h(1, { has_full_text: true }))).toBe(OPENING);
  });
});
