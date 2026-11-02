<script lang="ts">
  /**
   * The Interrogator's chat: *New conversation*, the earlier conversations, the
   * turns of the conversation shown, the answer streaming in, and the question box.
   * `turns`: the shown conversation's turns, oldest first.
   * `caption`: what the shown conversation is when it is not the current one, else null.
   * `readOnly`: nothing more can be asked here, so there is no question box.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   * `live`: the answer being prepared (its question, raw text so far and what it is
   *   doing, see `LiveAnswer`), or null when nothing is being answered.
   * `question`: the question box's text (bindable; kept after a failed ask).
   * `busy`: something is running; every action but *Stop* waits.
   * `earlier` / `latestCanonicalId`: the other conversations and the paper's latest
   *   text, for their labels; `onopen` shows one, `oncurrent` returns to the current one.
   * `onask`, `onstop`, `onnew`: ask the question, stop the answer, start a new conversation.
   */
  import type { ConversationSummary, TurnView as Turn } from "../lib/api";
  import { liveNote, liveSegments, type LiveAnswer } from "../lib/chat";
  import { earlierLabel } from "../lib/interrogator";
  import TurnView from "./TurnView.svelte";

  let {
    turns,
    caption,
    readOnly,
    current,
    oncite,
    live,
    question = $bindable(),
    busy,
    earlier,
    latestCanonicalId,
    onopen,
    oncurrent,
    onask,
    onstop,
    onnew,
  }: {
    turns: Turn[];
    caption: string | null;
    readOnly: boolean;
    current: { turnId: number; number: number } | null;
    oncite: (turnId: number, number: number) => void;
    live: LiveAnswer | null;
    question: string;
    busy: boolean;
    earlier: ConversationSummary[];
    latestCanonicalId: number;
    onopen: (conversationId: number) => void;
    oncurrent: () => void;
    onask: () => void;
    onstop: () => void;
    onnew: () => void;
  } = $props();

  /** How near the end (in CSS px) the log counts as scrolled to the end. */
  const AT_END_SLACK_PX = 24;

  let log: HTMLElement;
  /** Whether the log follows new text: true while the user is at its end. */
  let follow = true;

  /** Notes whether the user left the log at its end (scrolling up stops the following). */
  function onscroll() {
    follow = log.scrollHeight - log.scrollTop - log.clientHeight <= AT_END_SLACK_PX;
  }

  $effect(() => {
    // A question just asked always shows the end of the conversation.
    if (live !== null) {
      void live.question;
      follow = true;
    }
  });

  $effect(() => {
    // A new turn or more streamed text: keep the end in view, unless the user scrolled up.
    void turns.length;
    if (live !== null) void live.text;
    if (follow) log.scrollTop = log.scrollHeight;
  });

  /** Enter asks; Shift+Enter starts a new line. */
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      if (!busy && question.trim()) onask();
    }
  }
</script>

<div class="head">
  {#if caption}
    <span class="caption">{caption}</span>
    <button onclick={oncurrent} disabled={busy}>← Current conversation</button>
  {:else}
    <button onclick={onnew} disabled={busy || turns.length === 0}>New conversation</button>
  {/if}
  {#if earlier.length}
    <details>
      <summary>Earlier conversations ({earlier.length})</summary>
      <ul>
        {#each earlier as s}
          <li><button class="link" onclick={() => onopen(s.id)} disabled={busy}>{earlierLabel(s, latestCanonicalId)}</button></li>
        {/each}
      </ul>
    </details>
  {/if}
</div>

<div class="log" bind:this={log} {onscroll}>
  {#each turns as t (t.id)}
    <TurnView
      turn={t}
      current={current?.turnId === t.id ? current.number : null}
      oncite={(n) => oncite(t.id, n)}
    />
  {/each}
  {#if live}
    <div class="q">{live.question}</div>
    <div class="a" aria-live="polite">
      {#if liveNote(live)}<p class="muted">{liveNote(live)}</p>{/if}
      <!-- On one line: whitespace between segments would show inside the pre-wrap answer. -->
      <p class="answer">{#each liveSegments(live.text) as seg}{#if seg.kind === "text"}{seg.text}{:else}<span class="checking">“{seg.text}” <span class="tag">checking…</span></span>{/if}{/each}</p>
    </div>
  {/if}
</div>

{#if !readOnly}
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (!busy && question.trim()) onask();
    }}
  >
    <label for="question" class="visually-hidden">Question</label>
    <textarea id="question" rows="2" bind:value={question} {onkeydown} placeholder="Ask about this paper"></textarea>
    {#if live}
      <button type="button" onclick={onstop}>Stop</button>
    {:else}
      <button class="primary" disabled={busy || !question.trim()}>Ask</button>
    {/if}
  </form>
{/if}

<style>
  .head {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
  }
  .caption {
    font-size: 0.85rem;
    font-style: italic;
  }
  details {
    font-size: 0.85rem;
  }
  details ul {
    margin: 0.25rem 0 0;
    padding-left: 1rem;
  }
  .link {
    border: none;
    padding: 0.15rem 0;
    text-align: left;
    text-decoration: underline;
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
  .checking {
    opacity: 0.6;
  }
  .tag {
    font-size: 0.75rem;
    font-style: italic;
  }
  form {
    display: flex;
    gap: 0.5rem;
    align-items: flex-end;
    padding: 0.5rem 0.75rem;
    border-top: 1px solid var(--rule);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
