<script lang="ts">
  import type { PaperView } from "../lib/api";
  import { groupBlocks, sliceSpan, splitCells } from "../lib/paper";

  let { view }: { view: PaperView } = $props();
</script>

<article>
  <h1>{view.title_span ? sliceSpan(view.text, view.title_span) : view.title}</h1>
  {#each view.sections as section}
    {#if section.heading}
      {@const level = section.level <= 1 ? "h2" : "h3"}
      <svelte:element this={level}>{sliceSpan(view.text, section.heading)}</svelte:element>
    {/if}
    {#each groupBlocks(section.blocks) as g}
      {#if g.kind === "paragraph"}
        <p>{sliceSpan(view.text, g.span)}</p>
      {:else}
        <div class="scroll">
          <table>
            <tbody>
              {#each g.rows as r}
                <tr>
                  {#each splitCells(sliceSpan(view.text, r)) as cell}<td>{cell}</td>{/each}
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
</style>
