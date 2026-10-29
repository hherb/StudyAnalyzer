<script lang="ts">
  let {
    email,
    onsave,
    onback,
  }: { email: string; onsave: (email: string) => void; onback: () => void } = $props();

  // svelte-ignore state_referenced_locally -- the form is mounted fresh each time Settings opens
  let value = $state(email);
</script>

<div class="screen">
  <button onclick={onback}>← Back</button>
  <h2>Settings</h2>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onsave(value);
    }}
  >
    <label for="email">Contact email</label>
    <p class="muted">Europe PMC and Unpaywall ask for an address with each request.</p>
    <input id="email" type="email" bind:value />
    <p><button class="primary">Save</button></p>
  </form>
</div>
