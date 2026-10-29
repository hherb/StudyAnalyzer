<script lang="ts">
  let {
    reason,
    unreachable,
    busy,
    onretry,
    onpickfile,
    onpaste,
    onback,
  }: {
    reason: string;
    unreachable: boolean;
    busy: boolean;
    onretry: () => void;
    onpickfile: () => void;
    onpaste: (text: string) => void;
    onback: () => void;
  } = $props();

  let pasted = $state("");
</script>

<div class="screen">
  <button onclick={onback}>← Papers</button>
  <h2>
    {unreachable
      ? "The sources could not be reached — check the connection and try again"
      : "No free full text was found"}
  </h2>
  <p class="muted">{reason}</p>
  {#if unreachable}
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
