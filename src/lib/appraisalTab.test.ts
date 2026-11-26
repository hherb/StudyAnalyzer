import { beforeEach, describe, expect, test, vi } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import type { AppraisalEvent, AppraisalView, CommandError, PaperView } from "./api";
import { answerToggle, sourcesTick } from "./appraisal";
import { AppraisalTab, type AppraisalTabOptions } from "./appraisalTab.svelte";

vi.mock("@tauri-apps/plugin-log", () => ({ error: vi.fn(async () => {}) }));

/** The api calls the appraisal's state makes, each a fake set per test. */
const api = vi.hoisted(() => ({
  appraisal: vi.fn(),
  startAppraisal: vi.fn(),
  confirmAppraisal: vi.fn(),
  editAppraisal: vi.fn(),
  saveAppraisal: vi.fn(),
  revealAppraisal: vi.fn(),
  runAppraisal: vi.fn(),
  stopEvidence: vi.fn(),
  closeRevision: vi.fn(),
  setAsideAppraisal: vi.fn(),
}));
vi.mock("./api", async (original) => ({ ...(await original<typeof import("./api")>()), ...api }));

/** The fixture's appraisal waiting for confirmation (nothing to run). */
const confirming = fixture.appraisal_confirming as AppraisalView;
/** The fixture's appraisal being answered, its run with pending steps. */
const answering = fixture.appraisal_answering as AppraisalView;
/** `answering` with its run done. */
const done: AppraisalView = { ...answering, machine: { ...answering.machine, done_steps: 13, failed: [] } };

/** A rejection as Rust sends one. */
const failure = (kind: CommandError["kind"]): CommandError => ({ kind, message: `${kind} message` });

/** An appraisal state over fakes; `over` replaces any option. Returns it and what it reported. */
function tab(over: Partial<AppraisalTabOptions> = {}) {
  const reported: string[] = [];
  const errors: CommandError[] = [];
  const views: AppraisalView[] = [];
  const opts: AppraisalTabOptions = {
    latestCanonicalId: () => 1,
    paperAt: async () => ({ title: "older" }) as PaperView,
    blocked: () => null,
    slotBusy: () => false,
    report: (where) => reported.push(where),
    onerror: (err) => errors.push(err),
    onview: (v) => views.push(v),
    ...over,
  };
  return { t: new AppraisalTab(opts), reported, errors, views };
}

beforeEach(() => {
  for (const f of Object.values(api)) f.mockReset();
  api.runAppraisal.mockResolvedValue(done);
});

describe("opening", () => {
  test("continues a pending paired run on its own, its events to the progress", async () => {
    api.appraisal.mockResolvedValue(answering);
    const seen: Array<AppraisalEvent | null> = [];
    const { t, views } = tab();
    api.runAppraisal.mockImplementation(async (_id: number, _again: boolean, on: (e: AppraisalEvent) => void) => {
      on({ kind: "progress", done: 4, total: 13 });
      seen.push(t.progress);
      return done;
    });
    await t.open(1);
    expect(api.appraisal).toHaveBeenCalledWith(1, null);
    expect(api.runAppraisal).toHaveBeenCalledWith(1, false, expect.any(Function));
    expect(seen).toEqual([{ kind: "progress", done: 4, total: 13 }]);
    expect(t.view).toEqual(done);
    expect(t.running).toBe(false);
    expect(t.progress).toBeNull();
    expect(views.at(-1)).toEqual(done);
    // Opened once: a second open loads nothing.
    await t.open(1);
    expect(api.appraisal).toHaveBeenCalledTimes(1);
  });

  test("starts nothing while it waits for confirmation, or while another run holds the slot", async () => {
    api.appraisal.mockResolvedValue(confirming);
    await tab().t.open(1);
    api.appraisal.mockResolvedValue(answering);
    await tab({ slotBusy: () => true }).t.open(1);
    expect(api.runAppraisal).not.toHaveBeenCalled();
  });

  test("an appraisal on an earlier text shows with that text", async () => {
    api.appraisal.mockResolvedValue({ ...done, canonical_id: 0 });
    const { t } = tab();
    await t.open(1);
    expect(t.text?.title).toBe("older");
  });

  test("a failed load is reported and can be retried", async () => {
    api.appraisal.mockRejectedValue(failure("storage"));
    const { t, reported } = tab();
    await t.open(1);
    expect(reported).toEqual(["appraisal"]);
    expect(t.view).toBeNull();
    api.appraisal.mockResolvedValue(done);
    await t.open(1);
    expect(t.view).toEqual(done);
  });
});

describe("the reader's steps", () => {
  test("starting one runs its paired run", async () => {
    api.startAppraisal.mockResolvedValue({ ...confirming, machine: { ...confirming.machine, run_id: null } });
    const { t } = tab();
    await t.start("p1");
    expect(api.startAppraisal).toHaveBeenCalledWith("p1");
    expect(api.runAppraisal).toHaveBeenCalledTimes(1);
    expect(t.view).toEqual(done);
  });

  test("confirming lets the paired run go on", async () => {
    api.appraisal.mockResolvedValue(confirming);
    api.confirmAppraisal.mockResolvedValue(answering);
    const { t } = tab();
    await t.open(1);
    const input = { design: "rct", rct_layout: "parallel", outcomes: [] };
    await t.confirm(input);
    expect(api.confirmAppraisal).toHaveBeenCalledWith(1, input);
    expect(api.runAppraisal).toHaveBeenCalledTimes(1);
  });

  test("edits are sent one after another, each view shown in turn", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    const order: string[] = [];
    let release = () => {};
    api.editAppraisal.mockImplementationOnce(
      () =>
        new Promise((r) => {
          release = () => r({ ...done, started_at: "first" });
          order.push("first sent");
        }),
    );
    api.editAppraisal.mockImplementationOnce(async () => {
      order.push("second sent");
      return { ...done, started_at: "second" };
    });
    const a = t.edit({ kind: "note", domain: "D1", outcome: null, note: "a" });
    const b = t.edit({ kind: "note", domain: "D1", outcome: null, note: "b" });
    await new Promise((r) => setTimeout(r, 5));
    expect(order).toEqual(["first sent"]);
    release();
    await Promise.all([a, b]);
    expect(order).toEqual(["first sent", "second sent"]);
    expect(t.view?.started_at).toBe("second");
  });

  test("two quick ticks or chip presses while the first edit is pending build on its result, losing nothing", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    const at = { domain: "D1", outcome: null, question_id: "1.2" };
    /** `done` with question 1.2 changed by `over`. */
    const with12 = (over: object): AppraisalView => {
      const v = structuredClone(done);
      Object.assign(v.version.study.questions[1], over);
      return v;
    };
    let release = () => {};
    api.editAppraisal.mockImplementationOnce(
      () => new Promise((r) => (release = () => r(with12({ protocol: true })))),
    );
    api.editAppraisal.mockImplementationOnce(async () => with12({ protocol: true, registry: true }));
    const first = t.edit(sourcesTick(at, "protocol", true));
    const second = t.edit(sourcesTick(at, "registry", true));
    await new Promise((r) => setTimeout(r, 5));
    release();
    await Promise.all([first, second]);
    expect(api.editAppraisal.mock.calls.map((c) => c[1])).toEqual([
      { kind: "sources", ...at, protocol: true, registry: false, supplement: false },
      { kind: "sources", ...at, protocol: true, registry: true, supplement: false },
    ]);
    // Yes pressed twice quickly: set, then cleared, the second built on the first's result.
    api.editAppraisal.mockReset();
    api.editAppraisal.mockImplementationOnce(
      () => new Promise((r) => (release = () => r(with12({ answer: "Y" })))),
    );
    api.editAppraisal.mockImplementationOnce(async () => with12({ answer: null }));
    const a = t.edit(answerToggle(at, "Y"));
    const b = t.edit(answerToggle(at, "Y"));
    await new Promise((r) => setTimeout(r, 5));
    release();
    await Promise.all([a, b]);
    expect(api.editAppraisal.mock.calls.map((c) => c[1].answer)).toEqual(["Y", null]);
  });

  test("a failed edit is reported and the view kept", async () => {
    api.appraisal.mockResolvedValue(done);
    api.editAppraisal.mockRejectedValue(failure("invalid_input"));
    const { t, reported } = tab();
    await t.open(1);
    await t.edit({ kind: "note", domain: "D1", outcome: null, note: "a" });
    expect(reported).toEqual(["edit appraisal"]);
    expect(t.view).toEqual(done);
  });

  test("save and the reveal show the view they return", async () => {
    api.appraisal.mockResolvedValue(done);
    api.saveAppraisal.mockResolvedValue({ ...done, state: "saved" });
    api.revealAppraisal.mockResolvedValue({ ...done, revealed_at: "t" });
    const { t } = tab();
    await t.open(1);
    await t.reveal();
    expect(t.view?.revealed_at).toBe("t");
    await t.save();
    expect(t.view?.state).toBe("saved");
  });
});

describe("the paired run", () => {
  test("a failure is shown and the view read again, which says what failed", async () => {
    api.appraisal.mockResolvedValue(done);
    api.runAppraisal.mockRejectedValue(failure("llm"));
    const { t, errors } = tab();
    await t.open(1);
    api.appraisal.mockResolvedValue(answering);
    await t.run(false);
    expect(errors.map((e) => e.kind)).toEqual(["llm"]);
    expect(t.view).toEqual(answering);
  });

  test("an automatic run's missing model is a notice, not an error; a stop shows nothing", async () => {
    api.appraisal.mockResolvedValue(answering);
    api.runAppraisal.mockRejectedValue(failure("needs_model"));
    const { t, errors } = tab();
    await t.open(1);
    expect(errors).toEqual([]);
    expect(t.notice).toEqual({ text: "needs_model message", settings: true });
    api.runAppraisal.mockRejectedValue(failure("cancelled"));
    await t.run(false);
    expect(errors).toEqual([]);
    expect(t.notice).toBeNull();
  });

  test("its view is read again when an edit was made while it ran", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    let finish = () => {};
    api.runAppraisal.mockImplementation(() => new Promise((r) => (finish = () => r(done))));
    api.editAppraisal.mockResolvedValue({ ...done, started_at: "edited" });
    api.appraisal.mockResolvedValue({ ...done, started_at: "read again" });
    const running = t.run(true);
    expect(api.runAppraisal).toHaveBeenCalledWith(1, true, expect.any(Function));
    await t.edit({ kind: "note", domain: "D1", outcome: null, note: "a" });
    finish();
    await running;
    expect(t.view?.started_at).toBe("read again");
  });

  test("Stop is sent until the run has ended", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    let finish = () => {};
    api.runAppraisal.mockImplementation(
      () => new Promise((_, reject) => (finish = () => reject(failure("cancelled")))),
    );
    api.stopEvidence.mockImplementation(async () => {
      finish();
      return true;
    });
    const running = t.run(false);
    await t.stop();
    await running;
    expect(api.stopEvidence).toHaveBeenCalled();
    expect(t.stopping).toBe(false);
  });
});

describe("marks, versions and set aside", () => {
  /** `done` as Rust returns it set aside. */
  const asideView: AppraisalView = { ...done, set_aside_at: "2026-11-26 11:00:00" };

  test("an armed question takes the next selection as its mark, once", async () => {
    api.appraisal.mockResolvedValue(done);
    api.editAppraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    await t.mark({ start: 1, end: 2 });
    expect(api.editAppraisal).not.toHaveBeenCalled();
    t.marking = { domain: "D1", outcome: null, question_id: "1.1" };
    await t.mark({ start: 22, end: 35 });
    expect(api.editAppraisal).toHaveBeenCalledWith(done.id, {
      kind: "mark",
      domain: "D1",
      outcome: null,
      question_id: "1.1",
      span: { start: 22, end: 35 },
    });
    expect(t.marking).toBeNull();
  });

  test("a version is read by its number; Done revising closes the revision", async () => {
    api.appraisal.mockResolvedValue(done);
    api.closeRevision.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    await t.showVersion(0);
    expect(api.appraisal).toHaveBeenLastCalledWith(done.id, 0);
    await t.closeRevision();
    expect(api.closeRevision).toHaveBeenCalledWith(done.id);
  });

  test("set aside with nothing running sets it aside and shows the view", async () => {
    api.appraisal.mockResolvedValue(done);
    api.setAsideAppraisal.mockResolvedValue(asideView);
    const { t, views } = tab();
    await t.open(1);
    await t.setAside();
    expect(api.stopEvidence).not.toHaveBeenCalled();
    expect(t.view?.set_aside_at).toBe(asideView.set_aside_at);
    expect(views.at(-1)).toBe(t.view);
  });

  test("set aside presses Stop first while the paired run goes, and waits for it to end", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    let finish = () => {};
    api.runAppraisal.mockImplementation(
      () => new Promise((_, reject) => (finish = () => reject(failure("cancelled")))),
    );
    api.stopEvidence.mockImplementation(async () => {
      finish();
      return true;
    });
    api.setAsideAppraisal.mockImplementation(async () => {
      expect(t.running).toBe(false);
      return asideView;
    });
    const running = t.run(false);
    await t.setAside();
    await running;
    expect(api.stopEvidence).toHaveBeenCalled();
    expect(t.view?.set_aside_at).toBe(asideView.set_aside_at);
  });

  test("a run that started in between is stopped and the set aside sent again; a second refusal is shown", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t, reported } = tab();
    await t.open(1);
    let finish = () => {};
    api.runAppraisal.mockImplementation(
      () => new Promise((_, reject) => (finish = () => reject(failure("cancelled")))),
    );
    api.stopEvidence.mockImplementation(async () => {
      finish();
      return true;
    });
    let running: Promise<void> = Promise.resolve();
    api.setAsideAppraisal.mockImplementationOnce(async () => {
      running = t.run(false); // the race: a run starts just before Rust looks
      throw failure("invalid_input");
    });
    api.setAsideAppraisal.mockResolvedValueOnce(asideView);
    await t.setAside();
    await running;
    expect(api.setAsideAppraisal).toHaveBeenCalledTimes(2);
    expect(t.view?.set_aside_at).toBe(asideView.set_aside_at);
    expect(reported).toEqual([]);

    api.setAsideAppraisal.mockRejectedValue(failure("invalid_input"));
    await t.setAside();
    expect(reported).toEqual(["set appraisal aside"]);
  });
});

describe("fix round 1", () => {
  const at = { domain: "D1", outcome: null, question_id: "1.1" };

  test("a second set aside while the first waits for Stop sends nothing", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    let finish = () => {};
    api.runAppraisal.mockImplementation(
      () => new Promise((_, reject) => (finish = () => reject(failure("cancelled")))),
    );
    let stopped = () => {};
    api.stopEvidence.mockImplementation(() => new Promise<boolean>((r) => (stopped = () => r(true))));
    api.setAsideAppraisal.mockResolvedValue({ ...done, set_aside_at: "t" });
    const running = t.run(false);
    const firstPress = t.setAside();
    expect(t.settingAside).toBe(true);
    await t.setAside();
    expect(api.setAsideAppraisal).not.toHaveBeenCalled();
    stopped();
    finish();
    await firstPress;
    await running;
    expect(api.setAsideAppraisal).toHaveBeenCalledTimes(1);
    expect(t.settingAside).toBe(false);
  });

  test("an armed mark is dropped by Done revising, the save and the reveal", async () => {
    api.appraisal.mockResolvedValue(done);
    api.closeRevision.mockResolvedValue(done);
    api.saveAppraisal.mockResolvedValue(done);
    api.revealAppraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    for (const step of [() => t.closeRevision(), () => t.save(), () => t.reveal()]) {
      t.marking = at;
      await step();
      expect(t.marking).toBeNull();
    }
    await t.mark({ start: 22, end: 35 });
    expect(api.editAppraisal).not.toHaveBeenCalled();
  });

  test("a view the reader cannot change drops an armed mark", async () => {
    api.appraisal.mockResolvedValue(done);
    const { t } = tab();
    await t.open(1);
    t.marking = at;
    const older: AppraisalView = {
      ...done,
      state: "saved",
      versions: [
        { number: 0, frozen_at: "t", closed_at: null },
        { number: 1, frozen_at: null, closed_at: "t" },
      ],
    };
    // An edit's view, which leaves `marking` alone itself, showing a version that is not the latest.
    api.editAppraisal.mockResolvedValue(older);
    await t.edit({ kind: "note", domain: "D1", outcome: null, note: "n" });
    expect(t.marking).toBeNull();
  });
});
