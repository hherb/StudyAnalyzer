<script lang="ts">
  /**
   * `email`: the stored contact email; `onsave` stores a new one.
   * `keepMismatched`: the stored debugging setting, `null` while unknown (the
   *   switch is then disabled); `onkeepmismatched` stores a new value.
   * `onback`: leaves Settings.
   */
  let {
    email,
    keepMismatched,
    onsave,
    onkeepmismatched,
    onback,
  }: {
    email: string;
    keepMismatched: boolean | null;
    onsave: (email: string) => void;
    onkeepmismatched: (keep: boolean) => void;
    onback: () => void;
  } = $props();

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
  <h3>Debugging</h3>
  <!-- The label text is KEEP_MISMATCHED_LABEL in Rust's config, which error messages quote (a test there checks it). -->
  <label>
    <input
      type="checkbox"
      checked={keepMismatched ?? false}
      disabled={keepMismatched === null}
      onchange={(e) => {
        // Show the stored value until the new one is stored, so a failed save is not shown as done.
        const want = e.currentTarget.checked;
        e.currentTarget.checked = keepMismatched ?? false;
        onkeepmismatched(want);
      }}
    />
    Open a mismatching document as a paper of its own
  </label>
  <p class="muted">
    A document whose identifiers contradict the stored paper it matches is normally refused. With
    this on, it opens as a separate paper, so you can see what was retrieved.
  </p>
</div>
