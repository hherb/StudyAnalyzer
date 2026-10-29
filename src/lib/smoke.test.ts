import { expect, test } from "vitest";

// Keeps `test:web` non-empty until real suites arrive.
test("smoke", () => {
  expect(1 + 1).toBe(2);
});
