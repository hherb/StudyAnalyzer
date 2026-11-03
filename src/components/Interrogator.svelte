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
   * starts nothing. For a paper from a PDF, an *Original* tab shows that PDF.
   * `view`: the paper's latest text. One that neither passed the full-text check
   *   nor was confirmed shows the earlier conversations but takes no question.
   * `question`: the question box's text (bindable, so it outlives a trip to Settings).
   * `busy`: whether an answer, download, PICO run or load is running (bindable, read
   *   by the parent). The chat's own controls wait only for the chat's work.
   * `older`: whether the paper pane of the tab shown shows an earlier text than
   *   `view`'s (bindable, read by the parent, so the banner never offers to confirm
   *   a text not on screen).
   * `onerror`: called with every failure to show, already logged; `needs_embedder`
   *   is handled here and never passed on, nor is an automatic PICO run's missing key.
   * `onsettings`: opens Settings (the PICO tab's button when no model or key is set).
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { applyEvent, paperMarks, type LiveAnswer } from "../lib/chat";
  import { conversationCaption } from "../lib/interrogator";
  import { askBlockedNote } from "../lib/paper";
  import { offerRun, picoMarks, shouldAutoRun } from "../lib/pico";
  import ChatPane from "./ChatPane.svelte";
  import EmbedderDownload from "./EmbedderDownload.svelte";
  import OriginalPdf from "./OriginalPdf.svelte";
  import PaperPane from "./PaperPane.svelte";
  import PicoTab from "./PicoTab.svelte";

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

  /** Which tab shows: the Interrogator, PICO, or (for a paper from a PDF) the original PDF. */
  let tab = $state<"interrogator" | "pico" | "original">("interrogator");
  /** Which half shows on a phone. */
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

  /** The PICO tab's view; null until the tab is first opened. */
  let pv = $state<api.PicoView | null>(null);
  /** The PICO run shown; null when there is none. */
  let picoShown = $state<api.PicoRunView | null>(null);
  /** The text the shown run was made on, when it is not `view`'s. */
  let picoText = $state<api.PaperView | null>(null);
  /** The PICO citation in focus, by its number; null for none. */
  let picoCurrent = $state<number | null>(null);
  /** Whether a PICO run is going, and whether its stop was pressed. */
  let picoRunning = $state(false);
  let picoStopping = $state(false);
  /** Whether a run was started since this host mounted, whatever its end: so the tab auto-runs at most once. */
  let picoAttempted = $state(false);
  /** Whether a model is chosen in Settings (read when the PICO tab first opens). */
  let modelChosen = $state(true);
  /** Why a PICO run did not start: a missing model or key on an automatic run (with a button to Settings), or a text not yet confirmed. */
  let picoNotice = $state<{ text: string; settings: boolean } | null>(null);
  /** Whether the PICO tab is loading a view or a run. */
  let picoWorking = $state(false);

  let latest = $derived(iv?.canonical_id ?? view.canonical_id);
  let paper = $derived(otherText ?? view);
  let marks = $derived(shown ? paperMarks(shown.turns, current) : []);
  /** The chat's own work, which its controls wait for; a PICO run does not lock the chat. */
  let chatBusy = $derived(live !== null || downloading || working);
  let askable = $derived(askBlockedNote(view.full_text) === null);
  $effect(() => {
    busy = chatBusy || picoRunning || picoWorking;
  });
  $effect(() => {
    older = tab === "pico" ? picoText !== null : otherText !== null;
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
      live = null;
      await show(conv);
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
      await api.stopAnswer();
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

  /** Shows a PICO run with the text its spans point into, fetching that text if it is an older one. */
  async function showRun(run: api.PicoRunView | null) {
    const text =
      run && run.canonical_id !== view.canonical_id ? await api.paperViewAt(view.paper_id, run.canonical_id) : null;
    picoShown = run;
    picoText = text;
    picoCurrent = null;
  }

  /** Runs a PICO load, marking the tab busy and reporting any failure. */
  async function picoLoad(where: string, fn: () => Promise<void>) {
    picoWorking = true;
    try {
      await fn();
    } catch (e) {
      report(where, e);
    } finally {
      picoWorking = false;
    }
  }

  /** Opens the PICO tab: the first time, loads its runs and the model setting, then runs once if there is no run yet. */
  async function openPico() {
    tab = "pico";
    if (pv !== null || picoWorking) return;
    let auto = false;
    await picoLoad("pico", async () => {
      const [loaded, settings] = await Promise.all([api.pico(view.paper_id), api.llmSettings()]);
      modelChosen = settings.model !== null;
      pv = loaded;
      await showRun(loaded.current);
      auto = shouldAutoRun(loaded, askable, modelChosen, picoAttempted);
    });
    if (auto) await runPico(false, true);
  }

  /**
   * Starts a PICO run (`force` skips the suitability check). A run the user
   * started opens Settings on a missing model or key, as asking does; an
   * automatic one shows why with a button instead, never jumping there unasked.
   */
  async function runPico(force: boolean, auto = false) {
    if (picoRunning) return;
    picoAttempted = true;
    picoRunning = true;
    picoStopping = false;
    picoNotice = null;
    try {
      pv = await api.runPico(view.paper_id, force);
      await showRun(pv.current);
    } catch (e) {
      const err = api.toCommandError(e);
      if (err.kind !== "cancelled") {
        api.logFailure("run pico", err);
        if (auto && (err.kind === "needs_model" || err.kind === "needs_api_key"))
          picoNotice = { text: err.message, settings: true };
        else if (err.kind === "not_full_text")
          picoNotice = { text: askBlockedNote(view.full_text) ?? err.message, settings: false };
        else onerror(err);
      }
    } finally {
      picoRunning = false;
      picoStopping = false;
    }
  }

  /** Asks Rust to stop the PICO run before its next model call; nothing of it is stored. */
  async function stopPico() {
    picoStopping = true;
    try {
      await api.stopPico();
    } catch (e) {
      picoStopping = false;
      report("stop pico", e);
    }
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
    {#if view.source_kind === "pdf"}
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
          run={picoShown}
          latestCanonicalId={pv?.canonical_id ?? view.canonical_id}
          current={picoCurrent}
          oncite={(n) => {
            picoCurrent = n;
            pane = "paper";
          }}
          running={picoRunning}
          stopping={picoStopping}
          blocked={askBlockedNote(view.full_text)}
          needsModel={!modelChosen}
          notice={picoNotice}
          offer={pv !== null && offerRun(pv, askable, picoRunning)}
          earlier={(pv?.earlier ?? []).filter((r) => r.id !== picoShown?.id)}
          busy={picoWorking}
          navBusy={busy}
          onrun={(force) => runPico(force)}
          onstop={stopPico}
          onopen={(id) => picoLoad("pico run", async () => showRun(await api.picoRun(id)))}
          oncurrent={() => picoLoad("pico", async () => showRun(pv?.current ?? null))}
          {onsettings}
        />
      </section>
      <section class="paper-side" aria-label="Paper">
        <PaperPane view={picoText ?? view} marks={picoShown ? picoMarks(picoShown, picoCurrent) : []} />
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
