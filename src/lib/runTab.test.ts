import { describe, expect, test, vi } from "vitest";
import type { CommandError, PaperView } from "./api";
import { RunTab, type RunTabOptions } from "./runTab.svelte";

vi.mock("@tauri-apps/plugin-log", () => ({ error: vi.fn(async () => {}) }));

/** A stored run on text `canonical_id`. */
type Run = { id: number; canonical_id: number };
/** A tab view with `current` as its run. */
type View = { current: Run | null };

/** Waits for pending promise callbacks and short timers to run. */
const settle = () => new Promise((r) => setTimeout(r, 5));

/** A run tab over fakes; `over` replaces any option. Returns the tab and what it reported. */
function tab(over: Partial<RunTabOptions<View, Run, boolean, string>> = {}) {
  const reported: string[] = [];
  const errors: CommandError[] = [];
  const opts: RunTabOptions<View, Run, boolean, string> = {
    names: { view: "t", run: "run t", stop: "stop t" },
    loadView: async () => ({ current: null }),
    loadRun: async (id) => ({ id, canonical_id: 2 }),
    latestCanonicalId: () => 2,
    paperAt: async () => ({}) as PaperView,
    start: async () => ({ current: { id: 1, canonical_id: 2 } }),
    stop: async () => true,
    blocked: () => null,
    report: (where) => reported.push(where),
    onerror: (err) => errors.push(err),
    reloadOnEnd: false,
    stopResendMs: 1,
    ...over,
  };
  return { t: new RunTab<View, Run, boolean, string, number>(opts), reported, errors };
}

/** A rejection as Rust sends one. */
const failure = (kind: CommandError["kind"]): CommandError => ({ kind, message: `${kind} message` });

describe("opening the tab", () => {
  test("auto-runs once with no run, never again after an attempt", async () => {
    const start = vi.fn(async () => ({ current: null }) as View);
    const { t } = tab({ start });
    const wants = (v: View, attempted: boolean) => v.current === null && !attempted;
    await t.open(async () => {}, wants, false);
    expect(start).toHaveBeenCalledTimes(1);
    // A second open with the view loaded does nothing; a fresh load would still see the attempt.
    await t.open(async () => {}, wants, false);
    t.view = null;
    await t.open(async () => {}, wants, false);
    expect(start).toHaveBeenCalledTimes(1);
    expect(t.attempted).toBe(true);
  });

  test("a failed load is reported and left to retry; the view is kept only once its run shows", async () => {
    const { t, reported } = tab({ loadView: async () => Promise.reject(failure("storage")) });
    await t.open(async () => {}, () => true, false);
    expect(reported).toEqual(["t"]);
    expect(t.view).toBeNull();
    expect(t.working).toBe(false);
  });

  test("a run on an older text shows with that text; the latest needs none", async () => {
    const older = { title: "older" } as PaperView;
    const { t } = tab({ paperAt: async () => older });
    await t.showRun({ id: 3, canonical_id: 1 });
    expect(t.text).toBe(older);
    await t.showRun({ id: 4, canonical_id: 2 });
    expect(t.text).toBeNull();
  });
});

describe("a run", () => {
  test("a stored run's view replaces the old one; an earlier run shown is read-only", async () => {
    const { t } = tab();
    await t.run(false);
    expect(t.view?.current?.id).toBe(1);
    expect(t.shown?.id).toBe(1);
    expect(t.readOnly).toBe(false);
    await t.openRun(9);
    expect(t.readOnly).toBe(true);
    await t.toCurrent();
    expect(t.shown?.id).toBe(1);
  });

  test("without reload-on-end a failed run leaves the view as it was", async () => {
    const loadView = vi.fn(async () => ({ current: null }) as View);
    const { t, errors } = tab({ loadView, start: async () => Promise.reject(failure("llm")) });
    await t.run(false);
    expect(errors.map((e) => e.kind)).toEqual(["llm"]);
    expect(loadView).not.toHaveBeenCalled();
    expect(t.running).toBe(false);
  });

  test("with reload-on-end a stopped or failed run loads the view again, showing what it kept", async () => {
    const kept = { id: 5, canonical_id: 2 };
    const loadView = vi.fn(async () => ({ current: kept }) as View);
    const { t, errors } = tab({ loadView, reloadOnEnd: true, start: async () => Promise.reject(failure("cancelled")) });
    await t.run(false);
    expect(errors).toEqual([]);
    expect(loadView).toHaveBeenCalledTimes(1);
    expect(t.shown).toEqual(kept);
  });

  test("an automatic run's missing model is a notice, a user's run's an error", async () => {
    const { t, errors } = tab({ start: async () => Promise.reject(failure("needs_model")) });
    await t.run(false, true);
    expect(t.notice).toEqual({ text: "needs_model message", settings: true });
    await t.run(false, false);
    expect(errors.map((e) => e.kind)).toEqual(["needs_model"]);
  });

  test("a second run while one goes is ignored, and a late event of an earlier run is dropped", async () => {
    let send: (e: string) => void = () => {};
    let finish: (v: View) => void = () => {};
    const start = vi.fn(
      (_force: boolean, onEvent: (e: string) => void) =>
        new Promise<View>((resolve) => {
          send = onEvent;
          finish = resolve;
        }),
    );
    const { t } = tab({ start });
    const first = t.run(false);
    send("part 1");
    expect(t.progress).toBe("part 1");
    await t.run(false);
    expect(start).toHaveBeenCalledTimes(1);
    const late = send;
    finish({ current: null });
    await first;
    const second = t.run(false);
    late("from the first run");
    expect(t.progress).toBeNull();
    finish({ current: null });
    await second;
  });
});

describe("a stop", () => {
  test("is sent again until it reaches the run (#61)", async () => {
    let answers = [false, false, true];
    const stop = vi.fn(async () => answers.shift() ?? true);
    let finish: (v: View) => void = () => {};
    const { t } = tab({ stop, start: () => new Promise<View>((r) => (finish = r)) });
    const running = t.run(false);
    await t.stopRun();
    expect(stop).toHaveBeenCalledTimes(3);
    expect(t.stopping).toBe(true);
    finish({ current: null });
    await running;
    expect(t.stopping).toBe(false);
    answers = [];
  });

  test("never reaches a later run than the one it was pressed on", async () => {
    const stop = vi.fn(async () => false);
    let finish: (v: View) => void = () => {};
    const { t } = tab({ stop, start: () => new Promise<View>((r) => (finish = r)) });
    const first = t.run(false);
    const stopping = t.stopRun();
    finish({ current: null });
    await first;
    const second = t.run(false);
    await stopping;
    await settle();
    // The stop gave up when its run ended; the second run was not stopped.
    const calls = stop.mock.calls.length;
    await settle();
    expect(stop.mock.calls.length).toBe(calls);
    expect(t.running).toBe(true);
    finish({ current: null });
    await second;
  });

  test("a stop that fails is reported and the run is no longer marked stopping", async () => {
    let finish: (v: View) => void = () => {};
    const { t, reported } = tab({
      stop: async () => Promise.reject(failure("internal")),
      start: () => new Promise<View>((r) => (finish = r)),
    });
    const running = t.run(false);
    await t.stopRun();
    expect(reported).toEqual(["stop t"]);
    expect(t.stopping).toBe(false);
    finish({ current: null });
    await running;
  });
});
