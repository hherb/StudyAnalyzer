// The lifecycle every analysis tab shares (PICO, PRISMA): load on first open,
// auto-run at most once, a run going with its progress and stop, the run shown
// with the text it was made on, and the run history. It lives in the tab host,
// since the tabs remount on every switch: leaving a tab mid-run loses nothing.
import * as api from "./api";
import { runFailure, type PicoNotice } from "./pico";

/** What a tab's run state needs from its commands and its host. */
export interface RunTabOptions<V, R, A, E> {
  /** The names failures are logged and reported under: the view's load, a run, a stop. */
  names: { view: string; run: string; stop: string };
  /** The tab's view (its current run and the earlier ones); never runs anything. */
  loadView: () => Promise<V>;
  /** One stored run, from the tab's history. */
  loadRun: (runId: number) => Promise<R>;
  /** The paper's latest text, whose runs need no other text fetched. */
  latestCanonicalId: () => number;
  /** The paper view of text `canonicalId`, for a run on an earlier text. */
  paperAt: (canonicalId: number) => Promise<api.PaperView>;
  /** Starts a run with `args`, its events to `onEvent`; resolves to the view after it. */
  start: (args: A, onEvent: (event: E) => void) => Promise<V>;
  /** Asks Rust to stop the run; resolves to whether one was registered. */
  stop: () => Promise<boolean>;
  /** Why the latest text takes no run (a text not yet confirmed), or null. */
  blocked: () => string | null;
  /** Logs and shows a failure of a load or a stop. */
  report: (where: string, e: unknown) => void;
  /** Shows a run's failure (already logged). */
  onerror: (err: api.CommandError) => void;
  /**
   * Whether the view is loaded again after a run that failed or was stopped:
   * PRISMA keeps the parts such a run finished, PICO stores nothing.
   */
  reloadOnEnd: boolean;
  /** Milliseconds before a stop that found no run is sent again (tests pass a shorter wait). */
  stopResendMs?: number;
}

/**
 * One analysis tab's run state. `V` is the tab's view, `R` a stored run, `A`
 * what a run is started with, `E` a run's event, `C` the citation in focus.
 */
export class RunTab<
  V extends { current: R | null },
  R extends { id: number; canonical_id: number },
  A,
  E,
  C,
> {
  /** The tab's view; null until the tab is first opened. */
  view = $state<V | null>(null);
  /** The run shown; null when there is none. */
  shown = $state<R | null>(null);
  /** The text the shown run was made on, when it is not the latest. */
  text = $state<api.PaperView | null>(null);
  /** The citation in focus; null for none. */
  current = $state<C | null>(null);
  /** Whether a run is going. */
  running = $state(false);
  /** Whether the running run's stop was pressed. */
  stopping = $state(false);
  /** The running run's latest event; null before one arrives. */
  progress = $state<E | null>(null);
  /** Whether a run was started since the host mounted, whatever its end: so the tab auto-runs at most once. */
  attempted = $state(false);
  /** Why a run did not start: a missing model or key on an automatic run, or a text not yet confirmed. */
  notice = $state<PicoNotice | null>(null);
  /** Whether a view or a run is loading. */
  working = $state(false);
  /** Counts runs started, so a re-sent stop or a late event never reaches a later run than its own. */
  private runCount = 0;
  /** Counts runs asked to show, so a slow fetch of a run or its older text never replaces a later choice. */
  private showCount = 0;

  /** The commands and host hooks this state works with. */
  private readonly opts: RunTabOptions<V, R, A, E>;

  /** Builds the state over `opts`. */
  constructor(opts: RunTabOptions<V, R, A, E>) {
    this.opts = opts;
  }

  /** Whether the run shown is an earlier one, not the view's current run. */
  get readOnly(): boolean {
    return this.shown !== null && this.shown.id !== this.view?.current?.id;
  }

  /**
   * Shows `run` (or the run it resolves to) with the text its spans point
   * into, fetching that text if it is an older one. A later call wins: if
   * another run was asked to show while either was fetched, this one is
   * dropped.
   */
  async showRun(pending: R | null | Promise<R | null>): Promise<void> {
    const mine = ++this.showCount;
    const run = await pending;
    const text =
      run && run.canonical_id !== this.opts.latestCanonicalId() ? await this.opts.paperAt(run.canonical_id) : null;
    if (mine !== this.showCount) return;
    this.shown = run;
    this.text = text;
    this.current = null;
  }

  /** Runs a load, marking the tab busy and reporting any failure as `where`. */
  async load(where: string, fn: () => Promise<void>): Promise<void> {
    this.working = true;
    try {
      await fn();
    } catch (e) {
      this.opts.report(where, e);
    } finally {
      this.working = false;
    }
  }

  /**
   * Opens the tab: the first time, loads the view while `prepare` reads what
   * the auto-run decision needs (the model setting), then runs once with
   * `autoArgs` if `wantsAuto` says so (it is told whether a run was attempted).
   */
  async open(prepare: () => Promise<void>, wantsAuto: (view: V, attempted: boolean) => boolean, autoArgs: A) {
    if (this.view !== null || this.working) return;
    let auto = false;
    await this.load(this.opts.names.view, async () => {
      const [loaded] = await Promise.all([this.opts.loadView(), prepare()]);
      // Kept only once its run shows, so a failed load can be retried by opening the tab again.
      await this.showRun(loaded.current);
      this.view = loaded;
      auto = wantsAuto(loaded, this.attempted);
    });
    if (auto) await this.run(autoArgs, true);
  }

  /**
   * Starts a run with `args`. A run the user started reports a missing model
   * or key as an error; an automatic one (`auto`) shows a notice instead
   * (`runFailure`). A late event from an earlier run is dropped. After a run
   * that stored its result the view is the one it returned; after one that
   * failed or was stopped, it is loaded again when `reloadOnEnd`.
   */
  async run(args: A, auto = false): Promise<void> {
    if (this.running) return;
    this.attempted = true;
    this.running = true;
    this.runCount++;
    this.stopping = false;
    this.progress = null;
    this.notice = null;
    const mine = this.runCount;
    let next: V | null = null;
    try {
      next = await this.opts.start(args, (event) => {
        if (this.running && this.runCount === mine) this.progress = event;
      });
    } catch (e) {
      const err = api.toCommandError(e);
      if (err.kind !== "cancelled") api.logFailure(this.opts.names.run, err);
      const goes = runFailure(err, auto, this.opts.blocked());
      if (goes === "error") this.opts.onerror(err);
      else if (goes !== "nothing") this.notice = goes;
      if (!this.opts.reloadOnEnd) return;
    } finally {
      this.running = false;
      this.stopping = false;
      this.progress = null;
    }
    // A stored run: never offer it again, nor leave the old card standing in for it.
    if (next !== null) {
      this.view = next;
      this.shown = null;
    }
    const known = next;
    await this.load(this.opts.names.view, async () => {
      const loaded = known ?? (await this.opts.loadView());
      await this.showRun(loaded.current);
      this.view = loaded;
    });
  }

  /**
   * Continues stored run `runId` with `args` and keeps it shown after, even
   * when it is an earlier run, not the view's current one (it stays
   * read-only then).
   */
  async continueRun(args: A, runId: number): Promise<void> {
    await this.run(args);
    if (this.view !== null && this.view.current?.id !== runId) await this.openRun(runId);
  }

  /** Asks Rust to stop the run before its next model call, sent again until it lands (#61). */
  async stopRun(): Promise<void> {
    this.stopping = true;
    try {
      const mine = this.runCount;
      await api.stopWhilePending(
        this.opts.stop,
        () => this.running && this.runCount === mine,
        this.opts.stopResendMs,
      );
    } catch (e) {
      this.stopping = false;
      this.opts.report(this.opts.names.stop, e);
    }
  }

  /** Shows stored run `runId` from the history. */
  openRun(runId: number): Promise<void> {
    return this.load(`${this.opts.names.view} run`, async () => this.showRun(this.opts.loadRun(runId)));
  }

  /** Returns to the view's current run. */
  toCurrent(): Promise<void> {
    return this.load(this.opts.names.view, async () => this.showRun(this.view?.current ?? null));
  }
}
