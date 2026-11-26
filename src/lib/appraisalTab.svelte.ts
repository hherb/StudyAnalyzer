// The reader's appraisal as the Evidence tab's host keeps it: the view, the paired run going with its
// counts and its stop, and the reader's writes sent one after another. It lives in the tab host, as
// `RunTab` does, since the tab remounts on every switch: leaving it mid-run loses nothing.
import * as api from "./api";
import {
  confirmDraft,
  needsRun,
  versionEditable,
  waitingForConfirmation,
  type ConfirmDraft,
  type EditFrom,
  type QuestionAt,
} from "./appraisal";
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
  /** Called once a set aside has been sent, whatever came of it, so the host reads its opening view again. */
  onsetaside: () => void;
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
  /** The question whose sentence the paper pane waits for (*Mark the sentence*); null for none. */
  marking = $state<QuestionAt | null>(null);
  /** Whether a set aside is under way (it may be waiting for the paired run's stop), so it is sent once. */
  settingAside = $state(false);
  /** Steps 1 and 2 as the reader edits them, by the appraisal's id: kept here, so a tab switch loses no edit. */
  private draft = $state<{ id: number; draft: ConfirmDraft } | null>(null);
  /** The paired run going, so a set aside can wait for its stop to land; resolved when none goes. */
  private runDone: Promise<void> = Promise.resolve();
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

  /**
   * The draft of steps 1 and 2 of the appraisal shown, made the first time it
   * waits for confirmation (and kept after); null before that or for another
   * appraisal.
   */
  get confirmDraft(): ConfirmDraft | null {
    return this.draft !== null && this.draft.id === this.view?.id ? this.draft.draft : null;
  }

  /**
   * Shows `view`, with the text its spans point into fetched when it is not
   * the latest. The draft of steps 1 and 2 starts from its proposals the
   * first time it waits for confirmation; later views of it keep the draft.
   */
  private async apply(view: api.AppraisalView): Promise<void> {
    const text =
      view.canonical_id === this.opts.latestCanonicalId() ? null : await this.opts.paperAt(view.canonical_id);
    if (this.view?.id !== view.id) this.current = null;
    if (view.proposals && waitingForConfirmation(view) && this.draft?.id !== view.id)
      this.draft = { id: view.id, draft: confirmDraft(view.proposals) };
    // A version the reader cannot change takes no mark: a later selection must write nothing.
    if (!versionEditable(view)) this.marking = null;
    this.view = view;
    this.text = text;
    this.opts.onview(view);
  }

  /**
   * Sends `fn` once every earlier write is done and shows the view it
   * returns; a failure is reported as `where`. Resolves to whether it
   * succeeded.
   */
  private write(where: string, fn: () => Promise<api.AppraisalView>): Promise<boolean> {
    const next = this.queue.then(async () => {
      try {
        await this.apply(await fn());
        return true;
      } catch (e) {
        this.opts.report(where, e);
        return false;
      }
    });
    this.queue = next.then(() => {});
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

  /**
   * Writes one of the reader's edits (an answer chip, an override, a tick, a
   * note). One given as a builder is built when its turn comes, from the view
   * every earlier write left, so quick presses never build on a stale view.
   * Resolves to whether it was stored (false with no appraisal shown).
   */
  edit(edit: EditFrom): Promise<boolean> {
    const view = this.view;
    if (view === null) return Promise.resolve(false);
    this.edits++;
    return this.write("edit appraisal", () =>
      api.editAppraisal(view.id, typeof edit === "function" ? edit(this.view ?? view) : edit),
    );
  }

  /** The paper pane's selection `span` becomes the mark of the question waiting for it, if one is. */
  mark(span: api.Utf16Span): Promise<boolean> {
    const at = this.marking;
    this.marking = null;
    return at === null ? Promise.resolve(false) : this.edit({ kind: "mark", ...at, span });
  }

  /** Shows version `n` (0 the first record) of the appraisal; nothing is written. */
  showVersion(n: number): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return Promise.resolve();
    this.marking = null;
    return this.write("appraisal version", () => api.appraisal(id, n)).then(() => {});
  }

  /** *Done revising*: closes the open revision, so the next change opens another. */
  closeRevision(): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return Promise.resolve();
    this.marking = null;
    return this.write("close revision", () => api.closeRevision(id)).then(() => {});
  }

  /**
   * *Set this appraisal aside* (asked first in the pane): stops the paired run
   * if it goes and waits for it to end, then sets the appraisal aside. Should
   * a run start in between, Rust refuses (*Stop the machine's assessment
   * first.*); that run is stopped and the set aside sent once more, and a
   * second refusal is shown. `appraisalId` names one that could not be
   * opened (its row damaged); by default, the one shown. The host is told
   * after (`onsetaside`), whatever came of it.
   */
  async setAside(appraisalId?: number): Promise<void> {
    const id = appraisalId ?? this.view?.id;
    if (id === undefined || this.settingAside) return;
    this.settingAside = true;
    try {
      await this.sendAside(id);
    } finally {
      this.settingAside = false;
    }
    this.opts.onsetaside();
  }

  /** `setAside`'s steps for appraisal `id`: stop and wait, send, and once more after a race. */
  private async sendAside(id: number): Promise<void> {
    this.marking = null;
    for (let attempt = 0; ; attempt++) {
      if (this.running) {
        await this.stop();
        await this.runDone;
      }
      // In the write queue, so no edit lands between it and the view it returns.
      const sent = this.queue.then(() => api.setAsideAppraisal(id));
      this.queue = sent.then(
        () => {},
        () => {},
      );
      try {
        await this.apply(await sent);
        return;
      } catch (e) {
        if (attempt === 0 && this.running && api.toCommandError(e).kind === "invalid_input") continue;
        this.opts.report("set appraisal aside", e);
        return;
      }
    }
  }

  /** *Save and compare*: freezes the first record and reveals the machine's verdict. */
  async save(): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return;
    this.marking = null;
    await this.write("save appraisal", () => api.saveAppraisal(id));
    await this.goOn();
  }

  /** *Show the machine's verdict now*: the appraisal no longer counts as blind. */
  async reveal(): Promise<void> {
    const id = this.view?.id;
    if (id === undefined) return;
    this.marking = null;
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
    let ended = () => {};
    this.runDone = new Promise((resolve) => (ended = resolve));
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
      ended();
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
