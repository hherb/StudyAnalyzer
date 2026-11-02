<script lang="ts">
  /**
   * The offer to download the search model, shown when an answer needs passages
   * and the model is missing (or its file is broken): the question, then a
   * progress bar while it downloads. *Cancel* stops the download, or declines it.
   * `downloading`: whether the download is running (bindable, read by the parent).
   * `ondone`: called once the model is downloaded and loaded.
   * `oncancel`: called when the user declines or cancels.
   * `onerror`: called with any failure, already logged.
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { embedderPrompt } from "../lib/interrogator";

  let {
    downloading = $bindable(false),
    ondone,
    oncancel,
    onerror,
  }: {
    downloading?: boolean;
    ondone: () => void;
    oncancel: () => void;
    onerror: (err: api.CommandError) => void;
  } = $props();

  /** The download's size in bytes, once known. */
  let size = $state<number | null>(null);
  let progress = $state<api.DownloadProgress | null>(null);

  /** Logs and reports a failure. */
  function report(where: string, e: unknown) {
    const err = api.toCommandError(e);
    api.logFailure(where, err);
    onerror(err);
  }

  onMount(() => {
    api
      .embedderStatus()
      .then((s) => (size = s.size_bytes))
      .catch((e) => report("embedder status", e));
  });

  /** Downloads and loads the model; a cancel closes the offer, a failure leaves it open to try again. */
  async function download() {
    downloading = true;
    progress = null;
    // `downloading` is cleared before the callbacks: they may close this offer.
    try {
      await api.downloadEmbedder((p) => (progress = p));
      downloading = false;
      ondone();
    } catch (e) {
      downloading = false;
      if (api.toCommandError(e).kind === "cancelled") oncancel();
      else report("embedder download", e);
    }
  }

  /** Cancels a running download (it then rejects as cancelled), or declines the offer. */
  async function cancel() {
    if (!downloading) return oncancel();
    try {
      await api.cancelDownload();
    } catch (e) {
      report("cancel download", e);
    }
  }
</script>

<div class="offer" role="status">
  <span>{embedderPrompt(size)}</span>
  {#if downloading}
    <progress max={progress?.total || undefined} value={progress?.done} aria-label="Search model download"></progress>
    {#if progress?.total}<span>{Math.floor((progress.done / progress.total) * 100)} %</span>{/if}
  {:else}
    <button class="primary" onclick={download}>Download</button>
  {/if}
  <button onclick={cancel}>Cancel</button>
</div>

<style>
  .offer {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    align-items: center;
    background: var(--warn-bg);
    padding: 0.5rem 1rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
  progress {
    flex: 1;
    min-width: 8rem;
  }
</style>
