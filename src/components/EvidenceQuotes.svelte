<script lang="ts">
  /**
   * One claim's backing in the Evidence weight tab: `[n]` buttons with the
   * paper's wording (never the model's copy), then what was lost: the quotes
   * not found (listed, never dropped), those the paper contradicts with why,
   * and how many were not usable text.
   * `citations`: the paper's spans backing the claim, in order.
   * `lost`: the claim's quotes with what was lost, or null when it has none
   *   of its own (a D5 measurement's passages, whose losses show once).
   * `citeKey`: the group's key (`citeKeys`), so a click and its marks agree.
   * `current`: the citation in focus, or null; `oncite` is called with a
   *   citation's key and place when it is clicked.
   */
  import type { PrismaCitationView, QuotesView } from "../lib/api";
  import { contradictionNote } from "../lib/chat";
  import type { EvidenceCite } from "../lib/evidence";
  import { quotesDroppedNote } from "../lib/pico";

  let {
    citations,
    lost,
    citeKey,
    current,
    oncite,
  }: {
    citations: PrismaCitationView[];
    lost: QuotesView | null;
    citeKey: string;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
  } = $props();

  let dropped = $derived(lost ? quotesDroppedNote(lost) : null);
</script>

{#if citations.length}
  <ul class="citations">
    {#each citations as c, i (i)}
      <li>
        <button
          class="cite"
          class:current={current?.key === citeKey && current.index === i}
          aria-label={`Citation ${i + 1}: ${c.text}`}
          onclick={() => oncite({ key: citeKey, index: i })}>{i + 1}</button
        >
        <span class="wording">{c.text}</span>
      </li>
    {/each}
  </ul>
{/if}
{#if lost?.quotes_not_found.length}
  <p class="note">Not found in the paper:</p>
  <ul class="unlocated muted">
    {#each lost.quotes_not_found as q}<li>{q}</li>{/each}
  </ul>
{/if}
{#if lost?.quotes_contradicted.length}
  <p class="note">Contradicted by the paper:</p>
  <ul class="unlocated muted">
    {#each lost.quotes_contradicted as q}<li>{q.quote} <span class="note">({contradictionNote(q.contradiction)})</span></li>{/each}
  </ul>
{/if}
{#if dropped}<p class="note">{dropped}</p>{/if}

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
    margin: 0.3rem 0 0;
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
    margin: 0.3rem 0 0;
    font-size: 0.85rem;
    font-style: italic;
  }
</style>
