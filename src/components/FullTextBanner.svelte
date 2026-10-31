<script lang="ts">
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
