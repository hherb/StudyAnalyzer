<script lang="ts">
  /**
   * One stored question and its answer: the answer's text with a button for each
   * `[n]` citation, the paper's wording of each citation, the quotes that could not
   * be found (listed, never dropped), the note on an incomplete answer or one that
   * quotes nothing, and which model answered from what.
   * `turn`: the turn to show.
   * `current`: the number of this turn's citation in focus, or null.
   * `oncite`: called with a citation's number when it is clicked.
   */
  import type { TurnView } from "../lib/api";
  import { incompleteNote, noQuotesNote, unlocatedNote } from "../lib/chat";
  import { turnCaption } from "../lib/interrogator";

  let {
    turn,
    current,
    oncite,
  }: { turn: TurnView; current: number | null; oncite: (number: number) => void } = $props();

  /** The paper's wording of citation `n`, for its button's label. */
  const wording = (n: number) => turn.citations.find((c) => c.number === n)?.text;
  /** The accessible name of citation `n`'s buttons. */
  const label = (n: number) => {
    const w = wording(n);
    return w === undefined ? `Citation ${n}` : `Citation ${n}: ${w}`;
  };
  let unlocated = $derived(unlocatedNote(turn.unlocated.length));
  let incomplete = $derived(incompleteNote(turn.status));
  let noQuotes = $derived(noQuotesNote(turn));
</script>

<!-- On one line: whitespace around the button would show inside the pre-wrap answer. -->
{#snippet cite(n: number)}<button class="cite" class:current={current === n} aria-label={label(n)} onclick={() => oncite(n)}>{n}</button>{/snippet}

<div class="q">{turn.question}</div>
<div class="a">
  <p class="answer">{#each turn.pieces as piece}{#if piece.kind === "text"}{piece.text}{:else}{@render cite(piece.number)}{/if}{/each}</p>
  {#if turn.error}<p class="note">{turn.error}</p>{/if}
  {#if incomplete}<p class="note">{incomplete}</p>{/if}
  {#if noQuotes}<p class="note">{noQuotes}</p>{/if}
  {#if turn.citations.length}
    <ul class="citations">
      {#each turn.citations as c}
        <li>{@render cite(c.number)} <span class="wording">{c.text}</span></li>
      {/each}
    </ul>
  {/if}
  {#if unlocated}
    <p class="note">{unlocated}</p>
    <ul class="unlocated muted">
      {#each turn.unlocated as u}<li>{u.quote}</li>{/each}
    </ul>
  {/if}
  <p class="caption muted">{turnCaption(turn)}</p>
</div>

<style>
  .cite {
    font: 700 0.7rem var(--sans);
    padding: 0 0.35rem;
    margin: 0 0.1rem;
    border: none;
    border-radius: 4px;
    background: var(--hl-past);
    color: var(--hl-ink);
    vertical-align: 0.1em;
  }
  .cite.current {
    background: var(--hl-current);
  }
  .citations,
  .unlocated {
    margin: 0.5rem 0 0;
    padding: 0;
    list-style: none;
    font-size: 0.85rem;
  }
  .citations li {
    margin-top: 0.25rem;
  }
  .wording {
    font-family: var(--serif);
  }
  .unlocated li {
    margin-top: 0.2rem;
    padding-left: 0.5rem;
    border-left: 2px solid var(--rule);
  }
  .note {
    margin: 0.4rem 0 0;
    font-size: 0.85rem;
    font-style: italic;
  }
  .caption {
    margin: 0.4rem 0 0;
    font-size: 0.75rem;
  }
</style>
