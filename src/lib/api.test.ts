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
  });
});
