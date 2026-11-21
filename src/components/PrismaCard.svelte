<script lang="ts">
  /**
   * One PRISMA 2020 checklist item: its id, name and judgement chip (text, never
   * colour alone; *Not assessed* when no part has judged it yet), the model's
   * explanation, how it was judged (`itemNote`: an unbacked claim, a
   * condensation, a re-check, unusable quotes), what the model said when it was
   * shown otherwise, and the judgement a re-check replaced. Then what backs it:
   * `[n]` buttons with the paper's wording, the quotes not found (listed, never
   * dropped) and those the paper contradicts, with why.
   * `item`: the item to show.
   * `current`: the place of this item's citation in focus, or null; `oncite`
   *   is called with a citation's place when it is clicked.
   */
  import type { PrismaItemView } from "../lib/api";
  import { contradictionNote } from "../lib/chat";
  import { itemNote, judgementLabel } from "../lib/prisma";

  let {
    item,
    current,
    oncite,
  }: {
    item: PrismaItemView;
    current: number | null;
    oncite: (index: number) => void;
  } = $props();

  let note = $derived(itemNote(item));
</script>

<article class="item" id={`prisma-item-${item.id}`}>
  <h4>
    <span class="id">{item.id}</span>
    <span class="name">{item.name}</span>
    <span class="prisma-chip {item.judgement ?? 'not_assessed'}">{judgementLabel(item.judgement)}</span>
  </h4>
  {#if item.explanation}<p class="explanation">{item.explanation}</p>{/if}
  {#if note}<p class="note">{note}</p>{/if}
  {#if item.model_judgement !== null && item.model_judgement !== item.judgement}
    <p class="note">The model said: {judgementLabel(item.model_judgement)}</p>
  {/if}
  {#if item.rechecked && item.first_judgement !== null}
    <p class="note">First judged: {judgementLabel(item.first_judgement)}</p>
  {/if}
  {#if item.citations.length}
    <ul class="citations">
      {#each item.citations as c, i (i)}
        <li>
          <button
            class="cite"
            class:current={current === i}
            aria-label={`Citation ${i + 1}: ${c.text}`}
            onclick={() => oncite(i)}>{i + 1}</button
          >
          <span class="wording">{c.text}</span>
        </li>
      {/each}
    </ul>
  {/if}
  {#if item.quotes_not_found.length}
    <p class="note">Not found in the paper:</p>
    <ul class="unlocated muted">
      {#each item.quotes_not_found as q}<li>{q}</li>{/each}
    </ul>
  {/if}
  {#if item.quotes_contradicted.length}
    <p class="note">Contradicted by the paper:</p>
    <ul class="unlocated muted">
      {#each item.quotes_contradicted as q}<li>{q.quote} <span class="note">({contradictionNote(q.contradiction)})</span></li>{/each}
    </ul>
  {/if}
</article>

<style>
  .item {
    padding: 0.4rem 0 0.5rem;
    border-top: 1px solid var(--rule);
    scroll-margin-top: 0.5rem;
  }
  h4 {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: baseline;
    margin: 0;
    font-size: 0.9rem;
  }
  .id {
    min-width: 2rem;
    font-weight: 700;
  }
  .name {
    flex: 1;
    font-weight: normal;
  }
  .explanation {
    margin: 0.3rem 0 0;
  }
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
    margin: 0.4rem 0 0;
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
