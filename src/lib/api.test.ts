import { describe, expect, test, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";
import { FILES_DROPPED_EVENT, onFilesDropped, stopWhilePending, toCommandError } from "./api";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

describe("onFilesDropped", () => {
  /** It listens for Rust's drop event and hands the grants to the callback (#67). */
  test("passes the dropped grants on", async () => {
    const got: unknown[] = [];
    await onFilesDropped((files) => got.push(files));
    const [event, handler] = vi.mocked(listen).mock.calls[0];
    expect(event).toBe(FILES_DROPPED_EVENT);
    const files = [{ id: 1, name: "a.pdf" }];
    (handler as (ev: { payload: unknown }) => void)({ payload: files });
    expect(got).toEqual([files]);
  });
});

describe("stopWhilePending", () => {
  /** A stop that reaches the run is sent once. */
  test("sent once when it stops a run", async () => {
    let calls = 0;
    await stopWhilePending(async () => (++calls, true), () => true, 0);
    expect(calls).toBe(1);
  });

  /** A stop handled before the run began is sent again until it reaches it. */
  test("sent again while the command is pending and found no run", async () => {
    let calls = 0;
    await stopWhilePending(async () => ++calls === 3, () => true, 0);
    expect(calls).toBe(3);
  });

  /** Once the command is over, a stop that found nothing is not sent again. */
  test("not sent again once the command is over", async () => {
    let calls = 0;
    let pending = true;
    await stopWhilePending(
      async () => {
        calls++;
        pending = false;
        return false;
      },
      () => pending,
      0,
    );
    expect(calls).toBe(1);
  });

  /** A command that ends during the wait gets no further stop: a later run may have begun. */
  test("not sent again when the command ends during the wait", async () => {
    let calls = 0;
    let pending = true;
    const sent = stopWhilePending(async () => (calls++, false), () => pending, 20);
    setTimeout(() => (pending = false), 5);
    await sent;
    expect(calls).toBe(1);
  });

  /** A failed stop rejects for the caller to report. */
  test("a failed stop rejects", async () => {
    await expect(stopWhilePending(() => Promise.reject(new Error("x")), () => true, 0)).rejects.toThrow("x");
  });
});

describe("toCommandError", () => {
  test("a CommandError from Rust passes through unchanged", () => {
    const e = { kind: "too_large", message: "This PDF file is 200.0 MB" };
    expect(toCommandError(e)).toBe(e);
  });

  test("anything else becomes an internal error with its text", () => {
    expect(toCommandError("invalid args `path`")).toEqual({
      kind: "internal",
      message: "invalid args `path`",
    });
    expect(toCommandError(new Error("boom")).kind).toBe("internal");
    expect(toCommandError(undefined)).toEqual({ kind: "internal", message: "undefined" });
    expect(toCommandError({ code: 7 })).toEqual({ kind: "internal", message: '{"code":7}' });
  });

  test("an unknown kind or a non-string message is not taken as Rust's", () => {
    expect(toCommandError({ kind: "network", message: "down" })).toEqual({
      kind: "internal",
      message: 'down (unknown error kind "network")',
    });
    expect(toCommandError({ kind: "storage", message: null })).toEqual({
      kind: "internal",
      message: '{"kind":"storage","message":null}',
    });
  });
});
