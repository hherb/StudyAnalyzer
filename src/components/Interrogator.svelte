<script lang="ts">
  /**
   * The Interrogator tab: chat on the left, the paper on the right with the
   * conversation's citations highlighted (on phones a *Chat | Paper* switch
   * replaces the split, see app.css). It loads the paper's conversations, asks
   * questions with the answer streaming in, offers the search-model download when
   * an answer needs it, and shows an earlier conversation with the text it was
   * held on. For a paper from a PDF, an *Original* tab shows that PDF.
   * `view`: the paper's latest text. One that neither passed the full-text check
   *   nor was confirmed shows the earlier conversations but takes no question.
   * `question`: the question box's text (bindable, so it outlives a trip to Settings).
   * `busy`: whether an answer, download or load is running (bindable, read by the parent).
   * `onerror`: called with every failure to show, already logged; `needs_embedder`
   *   is handled here and never passed on.
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { paperMarks, type LiveAnswer } from "../lib/chat";
  import { conversationCaption } from "../lib/interrogator";
  import { askBlockedNote } from "../lib/paper";
  import ChatPane from "./ChatPane.svelte";
  import EmbedderDownload from "./EmbedderDownload.svelte";
  import OriginalPdf from "./OriginalPdf.svelte";
  import PaperPane from "./PaperPane.svelte";

  let {
    view,
    question = $bindable(""),
    busy = $bindable(false),
    onerror,
  }: {
    view: api.PaperView;
    question?: string;
    busy?: boolean;
    onerror: (err: api.CommandError) => void;
  } = $props();

  let tab = $state<"interrogator" | "original">("interrogator");
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

  let latest = $derived(iv?.canonical_id ?? view.canonical_id);
  let paper = $derived(otherText ?? view);
  let marks = $derived(shown ? paperMarks(shown.turns, current) : []);
  $effect(() => {
    busy = live !== null || downloading || working;
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
        if (live !== mine) return;
        if (ev.kind === "checking_model") mine.checking = true;
        else if (ev.kind === "indexing") {
          mine.checking = false;
          mine.indexing = { done: ev.done, total: ev.total };
        } else if (ev.kind === "waiting") {
          // Checking and indexing are over: the note falls back to *Waiting for the model…*.
          mine.checking = false;
          mine.indexing = null;
        } else {
          mine.checking = false;
          mine.indexing = null;
          mine.text += ev.text;
        }
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
          {busy}
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
