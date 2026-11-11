<script lang="ts">
  /**
   * The paper's PDF (the one its text was derived from, or the one kept beside
   * its JATS), drawn page by page with pdf.js,
   * so the parsed text can be checked against it by eye.
   * `paperId`: the paper whose original to show.
   * `onerror`: called with any failure to load or draw it.
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";

  /** The widest a page is drawn, in CSS pixels; narrower windows draw it narrower. */
  const MAX_PAGE_WIDTH = 900;

  let { paperId, onerror }: { paperId: string; onerror: (err: api.CommandError) => void } = $props();
  let pages: HTMLDivElement;
  let loading = $state(true);

  onMount(() => {
    let closed = false;
    let destroy: (() => Promise<void>) | undefined;
    (async () => {
      // Loaded here, not at startup: pdf.js is large and only this tab needs it.
      const [pdfjs, worker, bytes] = await Promise.all([
        import("pdfjs-dist"),
        import("pdfjs-dist/build/pdf.worker.min.mjs?url"),
        api.originalPdf(paperId),
      ]);
      pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
      const task = pdfjs.getDocument({ data: new Uint8Array(bytes) });
      destroy = () => task.destroy();
      const doc = await task.promise;
      const width = Math.min(pages.clientWidth, MAX_PAGE_WIDTH);
      for (let n = 1; n <= doc.numPages && !closed; n++) {
        const page = await doc.getPage(n);
        const scale = width / page.getViewport({ scale: 1 }).width;
        // Drawn at the screen's pixel density, shown at `width` CSS pixels, so text stays sharp.
        const viewport = page.getViewport({ scale: scale * window.devicePixelRatio });
        const canvas = document.createElement("canvas");
        canvas.width = viewport.width;
        canvas.height = viewport.height;
        canvas.style.width = `${width}px`;
        canvas.setAttribute("aria-label", `Page ${n} of ${doc.numPages}`);
        pages.append(canvas);
        await page.render({ canvas, viewport }).promise;
        loading = false;
      }
    })()
      .catch((e) => {
        if (closed) return; // the tab was left mid-draw; destroying the task rejects it
        const err = api.toCommandError(e);
        api.logFailure("original PDF", err);
        onerror({ ...err, message: `The original PDF could not be shown: ${err.message}` });
      })
      .finally(() => (loading = false));
    return () => {
      closed = true;
      destroy?.();
    };
  });
</script>

{#if loading}<p class="loading">Loading the original…</p>{/if}
<div class="pages" bind:this={pages}></div>

<style>
  .pages {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
  }
  .pages :global(canvas) {
    max-width: 100%;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.25);
  }
  .loading {
    text-align: center;
    font-family: var(--sans);
  }
</style>
