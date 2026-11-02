<script lang="ts">
  /**
   * The settings screen: the model that answers (provider, model and the context
   * window it is given), each provider's API key (kept in the OS keychain; only
   * whether one is there is ever shown), the contact email for the full-text
   * sources, and the debugging switch. The model part reads and stores its
   * settings itself and passes every failure to `onerror`, already logged.
   * `email`: the stored contact email; `onsave` stores a new one.
   * `keepMismatched`: the stored debugging setting, `null` while unknown (the
   *   switch is then disabled); `onkeepmismatched` stores a new value.
   * `onerror`: shows a failure of the model settings.
   * `onback`: leaves Settings.
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { keyStatus, parseContextTokens } from "../lib/interrogator";

  let {
    email,
    keepMismatched,
    onerror,
    onsave,
    onkeepmismatched,
    onback,
  }: {
    email: string;
    keepMismatched: boolean | null;
    onerror: (err: api.CommandError) => void;
    onsave: (email: string) => void;
    onkeepmismatched: (keep: boolean) => void;
    onback: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally -- the form is mounted fresh each time Settings opens
  let value = $state(email);

  /** The stored model settings; null until read. */
  let llm = $state<api.LlmSettings | null>(null);
  /** The provider and model being edited (stored together as `provider:model`). */
  let provider = $state("");
  let model = $state("");
  /** The stored model's maximum context window: a number, null when its provider states none, undefined while unknown. */
  let maxWindow = $state<number | null | undefined>(undefined);
  /** The context-window field, as typed. */
  let tokens = $state("");
  let tokensProblem = $state<string | null>(null);
  /** The key being typed for each provider; cleared once saved. */
  let keys = $state<Record<string, string>>({});
  let saving = $state(false);

  /** Runs a model-settings command, then rereads the settings; a failure is logged and shown. */
  async function act(where: string, fn: () => Promise<unknown>) {
    saving = true;
    try {
      await fn();
      llm = await api.llmSettings();
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure(where, err);
      onerror(err);
    } finally {
      saving = false;
    }
  }

  /** Asks `stored`'s provider for its maximum context window (nothing to ask without a model). */
  async function readWindow(stored: string | null) {
    maxWindow = undefined;
    if (stored) maxWindow = await api.modelWindow(stored);
  }

  /** Stores the provider and model, then shows the model's maximum. */
  const saveModel = () =>
    act("set model", async () => {
      const chosen = `${provider}:${model.trim()}`;
      await api.setModel(chosen);
      await readWindow(chosen);
    });

  /** Stores the context-window limit, or removes it when the field is empty. */
  function saveTokens() {
    const t = parseContextTokens(tokens);
    tokensProblem = t.kind === "invalid" ? "Enter a whole number of tokens, or leave it empty for no limit." : null;
    if (t.kind === "set") act("set context tokens", () => api.setContextTokens(t.tokens));
    else if (t.kind === "clear") act("clear context tokens", api.clearContextTokens);
  }

  /** Saves the typed key for `name` in the keychain and clears the field. */
  const saveKey = (name: string) =>
    act("set api key", async () => {
      await api.setApiKey(name, keys[name]);
      keys[name] = "";
    });

  onMount(() => {
    act("model settings", async () => {
      const s = await api.llmSettings();
      const at = s.model?.indexOf(":") ?? -1;
      provider = at > 0 ? s.model!.slice(0, at) : "";
      model = at > 0 ? s.model!.slice(at + 1) : "";
      tokens = s.context_tokens?.toString() ?? "";
      llm = s;
      await readWindow(s.model);
    });
  });
</script>

<div class="screen">
  <button onclick={onback}>← Back</button>
  <h2>Settings</h2>
  <h3>Model</h3>
  {#if llm?.keychain_error}<p class="warn" role="status">{llm.keychain_error}</p>{/if}
  <form
    onsubmit={(e) => {
      e.preventDefault();
      saveModel();
    }}
  >
    <label for="provider">Provider</label>
    <select
      id="provider"
      bind:value={provider}
      disabled={!llm}
      onchange={(e) => {
        const name = e.currentTarget.value;
        model = llm?.providers.find((p) => p.name === name)?.default_model ?? "";
      }}
    >
      {#if !provider}<option value="">Choose a provider</option>{/if}
      {#each llm?.providers ?? [] as p}<option value={p.name}>{p.name}</option>{/each}
    </select>
    <label for="model">Model</label>
    <input id="model" type="text" bind:value={model} autocomplete="off" />
    {#if llm?.model}
      <p class="muted">
        In use: {llm.model}.
        {#if typeof maxWindow === "number"}Its maximum context window is {maxWindow.toLocaleString("en-US")} tokens.
        {:else if maxWindow === null}Its provider states no maximum context window, so answers use passages unless a limit is set below.{/if}
      </p>
    {/if}
    <p><button class="primary" disabled={saving || !provider || !model.trim()}>Save model</button></p>
  </form>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      saveTokens();
    }}
  >
    <label for="tokens">Context window limit (tokens)</label>
    <p class="muted">Lower it when local hardware cannot run the model's full window. Empty: no limit.</p>
    <input id="tokens" type="text" inputmode="numeric" bind:value={tokens} />
    {#if tokensProblem}<p class="problem" role="alert">{tokensProblem}</p>{/if}
    <p><button class="primary" disabled={saving}>Save limit</button></p>
  </form>
  <h3>API keys</h3>
  <p class="muted">Kept in the system keychain, never in the app's files.</p>
  {#each llm?.providers.filter((p) => p.needs_key) ?? [] as p (p.name)}
    <form
      class="key"
      onsubmit={(e) => {
        e.preventDefault();
        saveKey(p.name);
      }}
    >
      <label for="key-{p.name}">{p.name}: {keyStatus(p)}</label>
      <div class="row">
        <input id="key-{p.name}" type="password" autocomplete="off" bind:value={keys[p.name]} />
        <button class="primary" disabled={saving || !keys[p.name]?.trim()}>Save key</button>
        <button type="button" disabled={saving || !p.has_key} onclick={() => act("delete api key", () => api.deleteApiKey(p.name))}>Remove key</button>
      </div>
    </form>
  {/each}
  <h3>Full-text sources</h3>
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

<style>
  select {
    font: 1rem var(--sans);
    padding: 0.4rem;
    margin-bottom: 0.5rem;
    background: transparent;
    color: var(--ink);
    border: 1px solid var(--rule);
    border-radius: 4px;
  }
  .row {
    display: flex;
    gap: 0.5rem;
  }
  .key {
    margin-bottom: 0.75rem;
  }
  input[type="password"] {
    flex: 1;
    min-width: 0;
    font: 1rem var(--sans);
    padding: 0.5rem;
    border: 1px solid var(--rule);
    border-radius: 4px;
    background: transparent;
    color: var(--ink);
  }
  .warn {
    background: var(--warn-bg);
    padding: 0.5rem;
  }
  .problem {
    color: var(--err-ink);
  }
</style>
