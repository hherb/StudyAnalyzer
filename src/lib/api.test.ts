import { describe, expect, test } from "vitest";
import { toCommandError } from "./api";

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
