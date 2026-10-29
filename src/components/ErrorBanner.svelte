<script lang="ts">
  let { message, ondismiss }: { message: string; ondismiss: () => void } = $props();
</script>

<div class="err" role="alert">
  <span>{message}</span>
  <button onclick={ondismiss} aria-label="Dismiss error">Dismiss</button>
</div>

<style>
  .err {
    display: flex;
    gap: 1rem;
    justify-content: space-between;
    align-items: center;
    background: var(--err-bg);
    color: var(--err-ink);
    padding: 0.6rem 1rem;
    font-family: var(--sans);
  }
  button {
    color: inherit;
    border-color: currentColor;
  }
</style>
