// The reader's appraisal as the Evidence tab's host keeps it: the view, the paired run going with its
// counts and its stop, and the reader's writes sent one after another. It lives in the tab host, as
// `RunTab` does, since the tab remounts on every switch: leaving it mid-run loses nothing.
import * as api from "./api";
import { needsRun } from "./appraisal";
import type { EvidenceCite } from "./evidence";
import { runFailure, type PicoNotice } from "./pico";

/** What the appraisal's state needs from its host. */
export interface AppraisalTabOptions {
  /** The paper's latest text, whose appraisal needs no other text fetched. */
  latestCanonicalId: () => number;
  /** The paper view of text `canonicalId`, for an appraisal on an earlier text. */
  paperAt: (canonicalId: number) => Promise<api.PaperView>;
  /** Why the latest text takes no run (a text not yet confirmed), or null. */
  blocked: () => string | null;
  /** Whether another evidence run of this window holds the one slot, so the paired run waits. */
  slotBusy: () => boolean;
  /** Logs and shows a failure of a load or a write. */
  report: (where: string, e: unknown) => void;
  /** Shows a run's failure (already logged). */
  onerror: (err: api.CommandError) => void;
  /** Called with every view shown, so the host's opening view names the appraisal as it stands. */
  onview: (view: api.AppraisalView) => void;
}

/** The reader's appraisal in the Evidence tab, and its paired run. */
export class AppraisalTab {
  /** The appraisal shown; null until one is opened or started. */
  view = $state<api.AppraisalView | null>(null);
  /** The text its spans point into, when it is not the latest. */
  text = $state<api.PaperView | null>(null);
  /** The citation in focus; null for none. */
  current = $state<EvidenceCite | null>(null);
  /** Whether the paired run goes. */
  running = $state(false);
  /** Whether its stop was pressed. */
  stopping = $state(false);
  /** Its latest counts; null before one arrives. */
  progress = $state<api.AppraisalEvent | null>(null);
  /** Why an automatic run did not start: a missing model or key, or a text not yet confirmed. */
  notice = $state<PicoNotice | null>(null);
  /** Whether the appraisal is loading or being started. */
  working = $state(false);
  /** The writes in order: each waits for the one before, so views are shown in the order they were made. */
  private queue: Promise<void> = Promise.resolve();
  /** Counts the reader's edits, so a run's view older than one is read again. */
  private edits = 0;

  /** The host hooks this state works with. */
  private readonly opts: AppraisalTabOptions;

  /** Builds the state over `opts`. */
  constructor(opts: AppraisalTabOptions) {
    this.opts = opts;
  }

  /** Shows `view`, with the text its spans point into fetched when it is not the latest. */
  private async apply(view: api.AppraisalView): Promise<void> {
    const text =
      view.canonical_id === this.opts.latestCanonicalId() ? null : await this.opts.paperAt(view.canonical_id);
    if (this.view?.id !== view.id) this.current = null;
    this.view = view;
    this.text = text;
    this.opts.onview(view);
  }

  /** Sends `fn` once every earlier write is done and shows the view it returns; a failure is reported as `where`. */
  private write(where: string, fn: () => Promise<api.AppraisalView>): Promise<void> {
    const next = this.queue.then(async () => {
      try {
        await this.apply(await fn());
      } catch (e) {
        this.opts.report(where, e);
      }
    });
    this.queue = next;
    return next;
  }

  /** Starts the paired run when `needsRun` says so, as an automatic run. */
  private async goOn(): Promise<void> {
    if (this.view && needsRun(this.view, this.running || this.opts.slotBusy())) await this.run(false, true);
  }

  /**
   * Opens appraisal `appraisalId` (its editable version, else its first
   * record), once: then continues its paired run on its own if it has to
   * (`needsRun`). A failed load is reported and tried again on the next open.
   */
  async open(appraisalId: number): Promise<void> {
    if (this.view?.id === appraisalId || this.working) return;
    this.working = true;
    try {
      await this.write("appraisal", () => api.appraisal(appraisalId, null));
    } finally {
      this.working = false;
    }
    await this.goOn();
  }

  /** *Appraise it yourself (first)*: starts an appraisal of paper `paperId`, then its paired run. */
  async start(paperId: string): Promise<void> {
    this.working = true;
    try {
      await this.write("start appraisal", () => api.startAppraisal(paperId));
    } finally {
      this.working = false;
    }
    await this.goOn();
  }

  /** Confirms steps 1 and 2 with `input`, then lets the paired run go on to its RoB 2 steps. */
  async confirm(input: api.ConfirmInput): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return;
    await this.write("confirm appraisal", () => api.confirmAppraisal(id, input));
    await this.goOn();
  }

  /** Writes one of the reader's edits (an answer chip, an override, a tick, a note). */
  edit(edit: api.AppraisalEdit): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return Promise.resolve();
    this.edits++;
    return this.write("edit appraisal", () => api.editAppraisal(id, edit));
  }

  /** *Save and compare*: freezes the first record and reveals the machine's verdict. */
  async save(): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return;
    await this.write("save appraisal", () => api.saveAppraisal(id));
    await this.goOn();
  }

  /** *Show the machine's verdict now*: the appraisal no longer counts as blind. */
  async reveal(): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return;
    await this.write("reveal appraisal", () => api.revealAppraisal(id));
    await this.goOn();
  }

  /**
   * Runs the paired run: going on with the latest (`again` false) or a new one
   * on the confirmed outcomes (`again`). An automatic run (`auto`) shows a
   * missing model or key as a notice, as `RunTab` does; a stop shows nothing;
   * any other failure is shown. After it, the view it returned is shown, or
   * read again when it failed (its failed steps are stored) or when an edit
   * was made while it ran (its view may be older).
   */
  async run(again: boolean, auto = false): Promise<void> {
    const id = this.view?.id;
    if (id === undefined || this.running) return;
    this.running = true;
    this.stopping = false;
    this.progress = null;
    this.notice = null;
    const edits = this.edits;
    let next: api.AppraisalView | null = null;
    try {
      next = await api.runAppraisal(id, again, (event) => {
        if (this.running) this.progress = event;
      });
    } catch (e) {
      const err = api.toCommandError(e);
      if (err.kind !== "cancelled") api.logFailure("run appraisal", err);
      const goes = runFailure(err, auto, this.opts.blocked());
      if (goes === "error") this.opts.onerror(err);
      else if (goes !== "nothing") this.notice = goes;
    } finally {
      this.running = false;
      this.stopping = false;
      this.progress = null;
    }
    const known = this.edits === edits ? next : null;
    await this.write("appraisal", async () => known ?? api.appraisal(id, null));
  }

  /** Asks Rust to stop the paired run before its next model call, sent again until it lands. */
  async stop(): Promise<void> {
    this.stopping = true;
    try {
      await api.stopWhilePending(api.stopEvidence, () => this.running);
    } catch (e) {
      this.stopping = false;
      this.opts.report("stop appraisal", e);
    }
  }
}
