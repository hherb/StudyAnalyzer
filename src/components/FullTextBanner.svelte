<script lang="ts">
  /**
   * `text`: what the full-text check found missing (`bannerText`), shown with a confirm button, or null.
   * `notes`: further notes in display order (the encoding warning, then the open's notices).
   * `onconfirm`: called when the user says this is the whole paper.
   */
  let {
    text,
    notes,
    onconfirm,
  }: { text: string | null; notes: string[]; onconfirm: () => void } = $props();
</script>

{#if text || notes.length}
  <div class="banner" role="status">
    {#if text}
      <span>{text}</span>
      <button onclick={onconfirm}>This is the whole paper</button>
    {/if}
    {#each notes as note}<span>{note}</span>{/each}
  </div>
{/if}

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    align-items: center;
    background: var(--warn-bg);
    padding: 0.5rem 1rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
</style>
