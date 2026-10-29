<script lang="ts">
  let {
    text,
    note,
    onconfirm,
  }: { text: string | null; note: string | null; onconfirm: () => void } = $props();
</script>

{#if text || note}
  <div class="banner" role="status">
    {#if text}
      <span>{text}</span>
      <button onclick={onconfirm}>This is the whole paper</button>
    {/if}
    {#if note}<span>{note}</span>{/if}
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
