<script lang="ts">
  /**
   * The paper as its canonical text lays it out: title, headings, paragraphs and tables.
   * `view`: the paper to show; every span in it is a UTF-16 slice of `view.text`.
   * `marks`: optional citation highlights (spans of `view.text`); the earlier ones
   *   pale, the current one dark. Whenever the marks change, the current one is
   *   scrolled to the middle of the pane.
   * `outlined`: optional spans outlined, never filled (the reader's own marks,
   *   told apart from the machine's quotes even where they overlap); one
   *   `current` is scrolled to as a current mark is.
   * `onSelect`: when given, the pane waits for a selection (saying so) and
   *   calls it on mouseup or touchend with the span selected; a selection it
   *   cannot take (empty, outside one block) shows *Select text inside one
   *   paragraph.* instead, and a block whose display does not match the
   *   paper's text is logged and said so (`BLOCK_MISMATCH`). Each block
   *   carries its start (`BLOCK_START_ATTR`) for `selectionSpan`.
   */
  import { logFailure, type PaperView, type Utf16Span } from "../lib/api";
  import { highlightRuns } from "../lib/chat";
  import {
    BLOCK_MISMATCH,
    BLOCK_START_ATTR,
    SELECT_ONE_PARAGRAPH,
    SELECT_PROMPT,
    cellSpans,
    groupBlocks,
    selectionSpan,
    sliceSpan,
  } from "../lib/paper";

  type Marks = Array<{ span: Utf16Span; current: boolean }>;
  let {
    view,
    marks = [],
    outlined = [],
    onSelect,
  }: { view: PaperView; marks?: Marks; outlined?: Marks; onSelect?: (span: Utf16Span) => void } = $props();
  let root: HTMLElement;
  /** Why the last selection could not be taken (`SELECT_ONE_PARAGRAPH` or `BLOCK_MISMATCH`), or null. */
  let refused = $state<string | null>(null);

  $effect(() => {
    // Runs after the DOM shows the new marks (and after a phone's switch made the pane visible).
    if (marks.some((m) => m.current) || outlined.some((m) => m.current)) {
      root.querySelector("mark.current, .outlined.current")?.scrollIntoView({ block: "center" });
    }
  });
  // Listens only while armed, so a pane not waiting for a selection reads none.
  $effect(() => {
    refused = null;
    const take = onSelect;
    if (!take) return;
    /** Reads the selection once a drag ends; passes a span on, or says why it cannot. */
    const selected = () => {
      const sel = window.getSelection();
      let mismatch = false;
      const span = sel
        ? selectionSpan(root, view, sel, (why) => {
            mismatch = true;
            logFailure("paper selection", { kind: "internal", message: why });
          })
        : null;
      refused = span ? null : mismatch ? BLOCK_MISMATCH : SELECT_ONE_PARAGRAPH;
      if (span) take(span);
    };
    root.addEventListener("mouseup", selected);
    root.addEventListener("touchend", selected);
    return () => {
      root.removeEventListener("mouseup", selected);
      root.removeEventListener("touchend", selected);
    };
  });
</script>

<!-- Kept on one line: whitespace between runs would show inside pre-wrap paragraphs. -->
{#snippet runs(span: Utf16Span)}{#each highlightRuns(span, marks) as r}{#each highlightRuns(r.span, outlined) as o}{#if r.mark === "none"}{@render piece(o)}{:else}<mark class={r.mark}>{@render piece(o)}</mark>{/if}{/each}{/each}{/snippet}
{#snippet piece(o: { span: Utf16Span; mark: string })}{#if o.mark === "none"}{sliceSpan(view.text, o.span)}{:else}<span class={o.mark === "current" ? "outlined current" : "outlined"}>{sliceSpan(view.text, o.span)}</span>{/if}{/snippet}

{#if onSelect}
  <p class="select" role="status">{refused ?? SELECT_PROMPT}</p>
{/if}
<article bind:this={root}>
  <h1 {...view.title_span ? { [BLOCK_START_ATTR]: view.title_span.start } : {}}>{#if view.title_span}{@render runs(view.title_span)}{:else}{view.title}{/if}</h1>
  {#each view.sections as section}
    {#if section.heading}
      {@const level = section.level <= 1 ? "h2" : "h3"}
      <svelte:element this={level} {...{ [BLOCK_START_ATTR]: section.heading.start }}>{@render runs(section.heading)}</svelte:element>
    {/if}
    {#each groupBlocks(section.blocks) as g}
      {#if g.kind === "paragraph"}
        <p {...{ [BLOCK_START_ATTR]: g.span.start }}>{@render runs(g.span)}</p>
      {:else}
        <div class="scroll">
          <table>
            <tbody>
              {#each g.rows as r}
                <tr>
                  {#each cellSpans(view.text, r) as cell}<td {...{ [BLOCK_START_ATTR]: cell.start }}>{@render runs(cell)}</td>{/each}
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
  /* The reader's own mark: outlined, never filled, so it is told from a quote. */
  .outlined {
    outline: 2px solid var(--hl-current);
    border-radius: 2px;
  }
  .select {
    position: sticky;
    top: 0;
    margin: 0;
    padding: 0.4rem 0.75rem;
    background: var(--paper-bg);
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
    font-size: 0.85rem;
    font-weight: 700;
  }
</style>
