<script lang="ts">
  /**
   * The paper as its canonical text lays it out: title, headings, paragraphs and tables.
   * `view`: the paper to show; every span in it is a UTF-16 slice of `view.text`.
   * `marks`: optional citation highlights (spans of `view.text`); the earlier ones
   *   pale, the current one dark. Whenever the marks change, the current one is
   *   scrolled to the middle of the pane.
   */
  import type { PaperView, Utf16Span } from "../lib/api";
  import { highlightRuns } from "../lib/chat";
  import { cellSpans, groupBlocks, sliceSpan } from "../lib/paper";

  let {
    view,
    marks = [],
  }: { view: PaperView; marks?: Array<{ span: Utf16Span; current: boolean }> } = $props();
  let root: HTMLElement;

  $effect(() => {
    // Runs after the DOM shows the new marks (and after a phone's switch made the pane visible).
    if (marks.some((m) => m.current)) {
      root.querySelector("mark.current")?.scrollIntoView({ block: "center" });
    }
  });
</script>

<!-- Kept on one line: whitespace between runs would show inside pre-wrap paragraphs. -->
{#snippet runs(span: Utf16Span)}{#each highlightRuns(span, marks) as r}{#if r.mark === "none"}{sliceSpan(view.text, r.span)}{:else}<mark class={r.mark}>{sliceSpan(view.text, r.span)}</mark>{/if}{/each}{/snippet}

<article bind:this={root}>
  <h1>{#if view.title_span}{@render runs(view.title_span)}{:else}{view.title}{/if}</h1>
  {#each view.sections as section}
    {#if section.heading}
      {@const level = section.level <= 1 ? "h2" : "h3"}
      <svelte:element this={level}>{@render runs(section.heading)}</svelte:element>
    {/if}
    {#each groupBlocks(section.blocks) as g}
      {#if g.kind === "paragraph"}
        <p>{@render runs(g.span)}</p>
      {:else}
        <div class="scroll">
          <table>
            <tbody>
              {#each g.rows as r}
                <tr>
                  {#each cellSpans(view.text, r) as cell}<td>{@render runs(cell)}</td>{/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/each}
  {/each}
</article>

<style>
  article {
    max-width: 46rem;
    margin: 0 auto;
    padding: 1rem;
    font-family: var(--serif);
    line-height: 1.6;
  }
  h2,
  h3 {
    color: var(--accent);
  }
  p {
    white-space: pre-wrap;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 0.9rem;
  }
  td {
    border: 1px solid var(--rule);
    padding: 0.25rem 0.5rem;
  }
  mark {
    color: var(--hl-ink);
    border-radius: 2px;
  }
  mark.past {
    background: var(--hl-past);
  }
  mark.current {
    background: var(--hl-current);
  }
</style>
