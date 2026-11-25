<script lang="ts">
  /**
   * The host of the paper's tabs. *Interrogator*: chat on the left, the paper on
   * the right with the conversation's citations highlighted (on phones a
   * *Chat | Paper* switch replaces the split, see app.css). It loads the paper's
   * conversations, asks questions with the answer streaming in, offers the
   * search-model download when an answer needs it, and shows an earlier
   * conversation with the text it was held on. *PICO*: the paper's PICO card
   * beside the paper (*PICO | Paper* on phones), run once when the tab first
   * opens with no run on the latest text; its run's state lives here, since the
   * tabs remount on every switch, so leaving the tab mid-run loses nothing and
   * starts nothing. *PRISMA*: the PRISMA 2020 checklist beside the paper
   * (*PRISMA | Paper* on phones), with PICO's lifecycle and its own run state
   * here; a stopped or failed run is reloaded, since it keeps the parts it
   * finished. *Evidence*: the evidence weight beside the paper
   * (*Evidence | Paper* on phones), with PRISMA's lifecycle and its own run
   * state here. For a paper with a PDF (its text's source, or one kept beside
   * its JATS), an *Original* tab shows that PDF.
   * `view`: the paper's latest text. One that neither passed the full-text check
   *   nor was confirmed shows the earlier conversations but takes no question.
   * `question`: the question box's text (bindable, so it outlives a trip to Settings).
   * `busy`: whether an answer, download, PICO, PRISMA or evidence run or load
   *   is running (bindable, read by the parent). The chat's own controls wait
   *   only for the chat's work.
   * `older`: whether the text in use shows an earlier version than `view`'s: the
   *   PICO, PRISMA or evidence run's on its tab, otherwise the chat's
   *   (bindable, read by the parent, so the banner never offers to confirm a
   *   text not on screen).
   * `onerror`: called with every failure to show, already logged; `needs_embedder`
   *   is handled here and never passed on, nor is an automatic PICO, PRISMA or
   *   evidence run's missing model or key, nor such a run's `not_full_text`
   *   (both shown in the tab).
   * `onsettings`: opens Settings (the PICO, PRISMA or evidence tab's button
   *   when no model or key is set).
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { applyEvent, paperMarks, type LiveAnswer } from "../lib/chat";
  import { conversationCaption } from "../lib/interrogator";
  import { askBlockedNote } from "../lib/paper";
  import { offerRun, picoMarks, shouldAutoRun } from "../lib/pico";
  import {
    continueOffered as evidenceContinueOffered,
    evidenceMarks,
    shouldAutoRun as evidenceAutoRun,
    type EvidenceCite,
  } from "../lib/evidence";
  import { continueOffered, earlierIncomplete, prismaMarks, type PrismaCite } from "../lib/prisma";
  import { RunTab } from "../lib/runTab.svelte";
  import ChatPane from "./ChatPane.svelte";
  import EmbedderDownload from "./EmbedderDownload.svelte";
  import EvidenceTab from "./EvidenceTab.svelte";
  import OriginalPdf from "./OriginalPdf.svelte";
  import PaperPane from "./PaperPane.svelte";
  import PicoTab from "./PicoTab.svelte";
  import PrismaTab from "./PrismaTab.svelte";

  let {
    view,
    question = $bindable(""),
    busy = $bindable(false),
    older = $bindable(false),
    onerror,
    onsettings,
  }: {
    view: api.PaperView;
    question?: string;
    busy?: boolean;
    older?: boolean;
    onerror: (err: api.CommandError) => void;
    onsettings: () => void;
  } = $props();

  /** Which tab shows: the Interrogator, PICO, PRISMA, Evidence, or (for a paper with a PDF) the original PDF. */
  let tab = $state<"interrogator" | "pico" | "prisma" | "evidence" | "original">("interrogator");
  /** Which half shows on a phone; on an analysis tab, `"chat"` is that tab's side. */
  let pane = $state<"chat" | "paper">("chat");
  let iv = $state<api.InterrogatorView | null>(null);
  /** The conversation shown; null until the first question creates one. */
  let shown = $state<api.ConversationView | null>(null);
  /** The text the shown conversation was held on, when it is not `view`'s. */
  let otherText = $state<api.PaperView | null>(null);
  /** The citation in focus (dark on the paper), named by its turn and number; null for none. */
  let current = $state<{ turnId: number; number: number } | null>(null);
  /** The answer being prepared (question, raw text so far, what it is doing); null when idle. */
  let live = $state<LiveAnswer | null>(null);
  /** Whether the search-model download is offered (an ask was refused for want of it). */
  let offerDownload = $state(false);
  /** Whether the search model is downloading. */
  let downloading = $state(false);
  /** Whether a conversation is loading or being started. */
  let working = $state(false);

  /** Whether a model is chosen in Settings (read when the PICO, PRISMA or evidence tab first opens). */
  let modelChosen = $state(true);
  /** The model chosen in Settings, read with `modelChosen`; null for none. */
  let chosenModel = $state<string | null>(null);

  /** What the tab hosts share: the paper's texts, failures reported here, a text not yet confirmed. */
  const tabHooks = {
    latestCanonicalId: () => view.canonical_id,
    paperAt: (canonicalId: number) => api.paperViewAt(view.paper_id, canonicalId),
    blocked: () => askBlockedNote(view.full_text),
    report,
    onerror: (err: api.CommandError) => onerror(err),
  };
  /** The PICO tab's run state; a stopped or failed run stores nothing, so the view stays. */
  const pico = new RunTab<api.PicoView, api.PicoRunView, boolean, api.PicoEvent, number>({
    ...tabHooks,
    names: { view: "pico", run: "run pico", stop: "stop pico" },
    loadView: () => api.pico(view.paper_id),
    loadRun: api.picoRun,
    start: (force, onEvent) => api.runPico(view.paper_id, force, onEvent),
    stop: api.stopPico,
    reloadOnEnd: false,
  });
  /** What a PRISMA run starts with: a new run (`force` skipping the check), or the run to continue. */
  type PrismaStart = { force: boolean; continueRun: null } | { force: false; continueRun: number };
  /** The PRISMA tab's run state; a stopped or failed run keeps the parts it finished, so the view is reloaded. */
  const prisma = new RunTab<api.PrismaView, api.PrismaRunView, PrismaStart, api.PrismaEvent, PrismaCite>({
    ...tabHooks,
    names: { view: "prisma", run: "run prisma", stop: "stop prisma" },
    loadView: () => api.prisma(view.paper_id),
    loadRun: api.prismaRun,
    start: (s, onEvent) => api.runPrisma(view.paper_id, s.force, s.continueRun, onEvent),
    stop: api.stopPrisma,
    reloadOnEnd: true,
  });
  /** The Evidence tab's run state, started with the run to continue (null for a new one); reloaded as PRISMA's. */
  const evidence = new RunTab<api.EvidenceView, api.EvidenceRunView, number | null, api.EvidenceEvent, EvidenceCite>({
    ...tabHooks,
    names: { view: "evidence", run: "run evidence", stop: "stop evidence" },
    loadView: () => api.evidence(view.paper_id),
    loadRun: api.evidenceRun,
    start: (continueRun, onEvent) => api.runEvidence(view.paper_id, continueRun, onEvent),
    stop: api.stopEvidence,
    reloadOnEnd: true,
  });

  let latest = $derived(iv?.canonical_id ?? view.canonical_id);
  let paper = $derived(otherText ?? view);
  let marks = $derived(shown ? paperMarks(shown.turns, current) : []);
  /** The chat's own work, which its controls wait for; a PICO run does not lock the chat. */
  let chatBusy = $derived(live !== null || downloading || working);
  /** Whether the latest text may be asked about (and a PICO, PRISMA or evidence run started on it). */
  let askable = $derived(askBlockedNote(view.full_text) === null);
  $effect(() => {
    busy =
      chatBusy ||
      pico.running ||
      pico.working ||
      prisma.running ||
      prisma.working ||
      evidence.running ||
      evidence.working;
  });
  $effect(() => {
    const run = tab === "pico" ? pico : tab === "prisma" ? prisma : tab === "evidence" ? evidence : null;
    older = run ? run.text !== null : otherText !== null;
  });

  /** Logs and shows a failure; the user's own cancel shows nothing, a missing search model offers the download. */
  function report(where: string, e: unknown) {
    const err = api.toCommandError(e);
    if (err.kind === "cancelled") return;
    api.logFailure(where, err);
    if (err.kind === "needs_embedder") offerDownload = true;
    else onerror(err);
  }

  /** Runs a load, marking the screen busy and reporting any failure. */
  async function load(where: string, fn: () => Promise<void>) {
    working = true;
    try {
      await fn();
    } catch (e) {
      report(where, e);
    } finally {
      working = false;
    }
  }

  /** Shows a conversation with the text its spans point into, fetching that text if it is an older one. */
  async function show(conv: api.ConversationView | null) {
    const text =
      conv && conv.canonical_id !== view.canonical_id
        ? await api.paperViewAt(view.paper_id, conv.canonical_id)
        : null;
    shown = conv;
    otherText = text;
    current = null;
  }

  /** Reloads the conversations list and shows the current conversation. */
  async function toCurrent() {
    iv = await api.interrogator(view.paper_id);
    await show(iv.current);
  }

  /**
   * Asks the question in the box. The box is cleared once the answer is stored,
   * and only if it still holds that question (a follow-up typed meanwhile stays).
   */
  async function ask() {
    const q = question.trim();
    if (!q || live) return;
    offerDownload = false;
    live = { question: q, text: "", indexing: null, checking: false, stopping: false };
    // `live` read back is the state proxy: compare against that, so a late event of this ask never lands on another's.
    const mine = live;
    try {
      const conv = await api.ask(view.paper_id, q, (ev) => {
        if (live === mine) applyEvent(mine, ev);
      });
      if (question.trim() === q) question = "";
      // `live` stays set until the turn is shown, so the host stays busy and a hand-off cannot unmount it meanwhile (#72).
      try {
        await show(conv);
      } catch (e) {
        // The turn is stored: say so, so the user does not ask it again.
        const err = api.toCommandError(e);
        report("show answer", {
          ...err,
          message: `Your answer was saved but could not be shown: ${err.message} Reopen the conversation to see it.`,
        });
      }
      live = null;
    } catch (e) {
      report("ask", e);
    } finally {
      if (live === mine) live = null;
    }
  }

  /**
   * Asks Rust to stop the answer, showing *Stopping…* until it has; what arrived
   * is kept as a stopped turn, and an answer with no text yet is dropped, its
   * question kept in the box.
   */
  async function stop() {
    const mine = live;
    if (mine) mine.stopping = true;
    try {
      await api.stopWhilePending(api.stopAnswer, () => mine !== null && live === mine);
    } catch (e) {
      if (mine) mine.stopping = false;
      report("stop answer", e);
    }
  }

  /** Puts a citation in focus and shows the paper (the phone's switch flips to it). */
  function cite(turnId: number, number: number) {
    current = { turnId, number };
    pane = "paper";
  }

  /** Reads the model setting, which a tab's auto-run and *Continue*'s label need. */
  async function readModel() {
    const settings = await api.llmSettings();
    modelChosen = settings.model !== null;
    chosenModel = settings.model;
  }

  /** Whether a tab's first open starts a run (`shouldAutoRun` with the host's text and model). */
  const wantsAuto = (v: { current: object | null }, attempted: boolean) =>
    shouldAutoRun(v, askable, modelChosen, attempted);

  /** Opens the PICO tab: the first time, loads its runs and the model setting, then runs once if there is no run yet. */
  function openPico() {
    tab = "pico";
    pico.open(readModel, wantsAuto, false);
  }

  /** Opens the PRISMA tab, as `openPico`. */
  function openPrisma() {
    tab = "prisma";
    // An earlier run left incomplete holds the auto-run back: Continue can finish it.
    prisma.open(readModel, (v, attempted) => wantsAuto(v, attempted) && !earlierIncomplete(v), {
      force: false,
      continueRun: null,
    });
  }

  /** Continues the PRISMA run shown, on its own model and text; it stays shown after. */
  async function continuePrisma() {
    const id = prisma.shown?.id;
    if (id === undefined) return;
    await prisma.continueRun({ force: false, continueRun: id }, id);
  }

  /** Opens the Evidence tab, as `openPrisma`. */
  function openEvidence() {
    tab = "evidence";
    evidence.open(readModel, (v, attempted) => askable && evidenceAutoRun(v, attempted, modelChosen), null);
  }

  /** Continues the evidence run shown, on its own model and text; it stays shown after. */
  async function continueEvidence() {
    const id = evidence.shown?.id;
    if (id !== undefined) await evidence.continueRun(id, id);
  }

  onMount(() => {
    load("interrogator", toCurrent);
    // The mount point (main.ts) becomes a full-height column while this shows; a class, not `:has()`, for older WebKit.
    const app = document.getElementById("app");
    app?.classList.add("fill");
    return () => app?.classList.remove("fill");
  });
</script>

<div class="interrogator" data-pane={pane}>
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={tab === "interrogator"} onclick={() => (tab = "interrogator")}>Interrogator</button>
    <button role="tab" aria-selected={tab === "pico"} onclick={openPico}>PICO</button>
    <button role="tab" aria-selected={tab === "prisma"} onclick={openPrisma}>PRISMA</button>
    <button role="tab" aria-selected={tab === "evidence"} onclick={openEvidence}>Evidence</button>
    {#if view.has_original_pdf}
      <button role="tab" aria-selected={tab === "original"} onclick={() => (tab = "original")}>Original</button>
    {/if}
  </div>
  {#if offerDownload}
    <EmbedderDownload
      bind:downloading
      ondone={() => {
        offerDownload = false;
        ask();
      }}
      oncancel={() => (offerDownload = false)}
      {onerror}
    />
  {/if}
  {#if tab === "original"}
    <div class="original">
      {#key view.paper_id}
        <OriginalPdf paperId={view.paper_id} {onerror} />
      {/key}
    </div>
  {:else if tab === "pico"}
    <div class="pane-switch" role="group" aria-label="Show">
      <button aria-pressed={pane === "chat"} onclick={() => (pane = "chat")}>PICO</button>
      <button aria-pressed={pane === "paper"} onclick={() => (pane = "paper")}>Paper</button>
    </div>
    <div class="split">
      <section class="chat-side" aria-label="PICO">
        <PicoTab
          run={pico.shown}
          latestCanonicalId={pico.view?.canonical_id ?? view.canonical_id}
          current={pico.current}
          oncite={(n) => {
            pico.current = n;
            pane = "paper";
          }}
          running={pico.running}
          stopping={pico.stopping}
          progress={pico.progress}
          blocked={askBlockedNote(view.full_text)}
          needsModel={!modelChosen}
          notice={pico.notice}
          offer={pico.view !== null && offerRun(pico.view, askable, pico.running)}
          earlier={(pico.view?.earlier ?? []).filter((r) => r.id !== pico.shown?.id)}
          busy={pico.working}
          navBusy={busy}
          onrun={(force) => pico.run(force)}
          onstop={() => pico.stopRun()}
          onopen={(id) => pico.openRun(id)}
          oncurrent={() => pico.toCurrent()}
          {onsettings}
        />
      </section>
      <section class="paper-side" aria-label="Paper">
        <PaperPane view={pico.text ?? view} marks={pico.shown ? picoMarks(pico.shown, pico.current) : []} />
      </section>
    </div>
  {:else if tab === "prisma"}
    <div class="pane-switch" role="group" aria-label="Show">
      <button aria-pressed={pane === "chat"} onclick={() => (pane = "chat")}>PRISMA</button>
      <button aria-pressed={pane === "paper"} onclick={() => (pane = "paper")}>Paper</button>
    </div>
    <div class="split">
      <section class="chat-side" aria-label="PRISMA">
        <PrismaTab
          run={prisma.shown}
          readOnly={prisma.readOnly}
          latestCanonicalId={prisma.view?.canonical_id ?? view.canonical_id}
          current={prisma.current}
          oncite={(cite, toPaper = true) => {
            prisma.current = cite;
            if (toPaper) pane = "paper";
          }}
          running={prisma.running}
          stopping={prisma.stopping}
          progress={prisma.progress}
          blocked={askBlockedNote(view.full_text)}
          needsModel={!modelChosen}
          {chosenModel}
          notice={prisma.notice}
          offer={prisma.view !== null && offerRun(prisma.view, askable, prisma.running)}
          offerContinue={continueOffered(prisma.shown, prisma.running)}
          earlier={(prisma.view?.earlier ?? []).filter((r) => r.id !== prisma.shown?.id)}
          busy={prisma.working}
          navBusy={busy}
          onrun={(force) => prisma.run({ force, continueRun: null })}
          oncontinue={continuePrisma}
          onstop={() => prisma.stopRun()}
          onopen={(id) => prisma.openRun(id)}
          oncurrent={() => prisma.toCurrent()}
          {onsettings}
        />
      </section>
      <section class="paper-side" aria-label="Paper">
        <PaperPane view={prisma.text ?? view} marks={prisma.shown ? prismaMarks(prisma.shown, prisma.current) : []} />
      </section>
    </div>
  {:else if tab === "evidence"}
    <div class="pane-switch" role="group" aria-label="Show">
      <button aria-pressed={pane === "chat"} onclick={() => (pane = "chat")}>Evidence</button>
      <button aria-pressed={pane === "paper"} onclick={() => (pane = "paper")}>Paper</button>
    </div>
    <div class="split">
      <section class="chat-side" aria-label="Evidence weight">
        <EvidenceTab
          run={evidence.shown}
          readOnly={evidence.readOnly}
          latestCanonicalId={evidence.view?.canonical_id ?? view.canonical_id}
          current={evidence.current}
          oncite={(cite) => {
            evidence.current = cite;
            pane = "paper";
          }}
          running={evidence.running}
          stopping={evidence.stopping}
          progress={evidence.progress}
          blocked={askBlockedNote(view.full_text)}
          needsModel={!modelChosen}
          {chosenModel}
          notice={evidence.notice}
          offer={evidence.view !== null && offerRun(evidence.view, askable, evidence.running)}
          offerContinue={evidenceContinueOffered(evidence.shown, evidence.running)}
          earlier={(evidence.view?.earlier ?? []).filter((r) => r.id !== evidence.shown?.id)}
          busy={evidence.working}
          navBusy={busy}
          onrun={() => evidence.run(null)}
          oncontinue={continueEvidence}
          onstop={() => evidence.stopRun()}
          onopen={(id) => evidence.openRun(id)}
          oncurrent={() => evidence.toCurrent()}
          {onsettings}
        />
      </section>
      <section class="paper-side" aria-label="Paper">
        <PaperPane
          view={evidence.text ?? view}
          marks={evidence.shown ? evidenceMarks(evidence.shown, evidence.current) : []}
        />
      </section>
    </div>
  {:else}
    <div class="pane-switch" role="group" aria-label="Show">
      <button aria-pressed={pane === "chat"} onclick={() => (pane = "chat")}>Chat</button>
      <button aria-pressed={pane === "paper"} onclick={() => (pane = "paper")}>Paper</button>
    </div>
    <div class="split">
      <section class="chat-side" aria-label="Chat">
        <ChatPane
          turns={shown?.turns ?? []}
          caption={shown ? conversationCaption(shown, latest) : null}
          readOnly={shown?.read_only ?? false}
          blocked={askBlockedNote(view.full_text)}
          {current}
          oncite={cite}
          {live}
          bind:question
          busy={chatBusy}
          earlier={iv?.earlier ?? []}
          latestCanonicalId={latest}
          onopen={(id) => load("conversation", async () => show(await api.conversation(id)))}
          oncurrent={() => load("interrogator", toCurrent)}
          onask={ask}
          onstop={stop}
          onnew={() =>
            load("new conversation", async () => {
              await show(await api.newConversation(view.paper_id));
              iv = await api.interrogator(view.paper_id);
            })}
        />
      </section>
      <section class="paper-side" aria-label="Paper">
        <PaperPane view={paper} {marks} />
      </section>
    </div>
  {/if}
</div>

<style>
  .tabs,
  .pane-switch {
    justify-content: center;
    gap: 0.25rem;
    padding: 0.5rem;
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
  }
  .tabs {
    display: flex;
  }
  .tabs button[aria-selected="true"],
  .pane-switch button[aria-pressed="true"] {
    background: var(--accent);
    color: var(--paper-bg);
  }
  .original {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
