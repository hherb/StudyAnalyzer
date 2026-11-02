<script lang="ts">
  /**
   * The screen for a paper with no full text yet: why, a retry, and an upload or paste of the text.
   * `reason`: why there is no full text, as the user should read it.
   * `unreachable`: whether the sources could not be reached (so the heading says to check the connection).
   * `busyText`: what is running, shown with the buttons disabled, or null when idle.
   * `oncancel`: cancels the running lookup; Cancel is offered only when it is given.
   * `canRetry`: whether the paper has an identifier to look up again with `onretry`.
   * `onpickfile`: asks for a file to attach; `onpaste` attaches the pasted text.
   * `onback`: goes back to the papers.
   */
  let {
    reason,
    unreachable,
    busyText,
    oncancel,
    canRetry,
    onretry,
    onpickfile,
    onpaste,
    onback,
  }: {
    reason: string;
    unreachable: boolean;
    busyText: string | null;
    oncancel?: () => void;
    canRetry: boolean;
    onretry: () => void;
    onpickfile: () => void;
    onpaste: (text: string) => void;
    onback: () => void;
  } = $props();

  import { needsFullTextHeading } from "../lib/paper";

  let busy = $derived(busyText !== null);
  let pasted = $state("");
</script>

<div class="screen">
  <button onclick={onback} disabled={busy}>← Papers</button>
  <h2>{needsFullTextHeading(unreachable)}</h2>
  <p class="muted">{reason}</p>
  {#if busy}
    <p role="status">
      {busyText}
      {#if oncancel}<button onclick={oncancel}>Cancel</button>{/if}
    </p>
  {/if}
  {#if canRetry}
    <p><button class="primary" onclick={onretry} disabled={busy}>Try again</button></p>
  {/if}
  <section>
    <button onclick={onpickfile} disabled={busy}>Upload a file</button>
  </section>
  <section>
    <label for="nf-paste">Paste the text</label>
    <textarea id="nf-paste" rows="8" bind:value={pasted}></textarea>
    <button onclick={() => onpaste(pasted)} disabled={busy || !pasted.trim()}>Use this text</button>
  </section>
</div>
