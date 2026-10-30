<script lang="ts">
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
