<script lang="ts">
  /**
   * The settings screen: the model that answers (provider, model — picked
   * from the provider's listing or typed — and the context window it is
   * given), each provider's API key (kept in the OS keychain; only whether
   * one is there is ever shown), the shared paper library
   * (`LibrarySetting`, which reads and stores its own setting), the contact
   * email for the full-text sources, and the debugging switch. The model
   * part reads and stores its settings itself and passes every failure to
   * `onerror`, already logged.
   * `email`: the stored contact email; `onsave` stores a new one.
   * `keepMismatched`: the stored debugging setting, `null` while unknown (the
   *   switch is then disabled); `onkeepmismatched` stores a new value.
   * `onerror`: shows a failure of the model or library settings.
   * `onback`: leaves Settings.
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import LibrarySetting from "./LibrarySetting.svelte";
  import { keyStatus, modelChoices, parseContextTokens } from "../lib/interrogator";

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
  /** Whether the Library setting is changing (Back waits for it). */
  let libraryBusy = $state(false);

  /** Why the stored model's maximum could not be read (shown by the model field), or null. */
  let windowNote = $state<string | null>(null);

  /** The models the chosen provider lists, which the model menu offers (see `choices`); null until listed, or when listing failed. */
  let models = $state<string[] | null>(null);
  /** Why the provider's models could not be listed (shown by the model field), or null. */
  let modelsNote = $state<string | null>(null);
  /** Whether a listing is under way (it can take a while: bmlib retries each page). */
  let modelsPending = $state(false);
  /** Counts listings, so an answer for a provider no longer chosen is dropped. */
  let listing = 0;
  /** Whether the model is typed although the provider lists models (*Other…* chosen). */
  let typing = $state(false);
  /** The model menu's entries, or null when the model is typed (see `modelChoices`). */
  let choices = $derived(typing ? null : modelChoices(models, model));
  /** The model menu's entry that switches to typing a name. */
  const OTHER = "\u0000other";

  /** Logs a failure and shows it in the banner. */
  function report(where: string, e: unknown) {
    const err = api.toCommandError(e);
    api.logFailure(where, err);
    onerror(err);
  }

  /** Rereads the stored settings; a failure is reported unless `quiet` (another failure is already shown). */
  async function reread(quiet: boolean) {
    try {
      llm = await api.llmSettings();
    } catch (e) {
      if (quiet) api.logFailure("model settings", api.toCommandError(e));
      else report("model settings", e);
    }
  }

  /**
   * Runs a model-settings command, then always rereads the settings (a failed
   * command may have stored part of its change); a failure is logged and shown.
   */
  async function act(where: string, fn: () => Promise<unknown>) {
    saving = true;
    let failed = false;
    try {
      await fn();
    } catch (e) {
      failed = true;
      report(where, e);
    }
    await reread(failed);
    saving = false;
  }

  /**
   * Asks `stored`'s provider for its maximum context window (nothing to ask
   * without a model). Never throws: a failure (often a missing key) is logged
   * and noted by the model field, so it never hides the banner's message.
   */
  async function readWindow(stored: string | null) {
    maxWindow = undefined;
    windowNote = null;
    if (!stored) return;
    try {
      maxWindow = await api.modelWindow(stored);
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure("model window", err);
      windowNote = `Its maximum context window could not be read: ${err.message}`;
    }
  }

  /**
   * Lists `name`'s models for the model menu, and goes back to the menu
   * (`typing = false`). Never throws: a failure (often a missing key) is
   * logged and noted, and the field stays a text input.
   */
  async function readModels(name: string) {
    const run = ++listing;
    models = null;
    typing = false;
    modelsNote = null;
    modelsPending = !!name;
    if (!name) return;
    try {
      const listed = await api.listModels(name);
      if (run === listing) models = listed;
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure("list models", err);
      if (run === listing) modelsNote = `${name}'s models could not be listed: ${err.message}`;
    } finally {
      if (run === listing) modelsPending = false;
    }
  }

  /** Stores the provider and model, then shows the model's maximum. */
  async function saveModel() {
    await act("set model", () => api.setModel(`${provider}:${model.trim()}`));
    await readWindow(llm?.model ?? null);
  }

  /** Stores the context-window limit, or removes it when the field is empty. */
  function saveTokens() {
    const t = parseContextTokens(tokens);
    tokensProblem = t.kind === "invalid" ? "Enter a whole number of tokens, or leave it empty for no limit." : null;
    if (t.kind === "set") act("set context tokens", () => api.setContextTokens(t.tokens));
    else if (t.kind === "clear") act("clear context tokens", api.clearContextTokens);
  }

  /** Saves the typed key for `name` in the keychain, clears the field and rereads the model's maximum and the provider's models (both may need the key). */
  async function saveKey(name: string) {
    await act("set api key", async () => {
      await api.setApiKey(name, keys[name]);
      keys[name] = "";
    });
    await Promise.all([readWindow(llm?.model ?? null), readModels(provider)]);
  }

  /** Removes `name`'s key from the keychain and rereads the model's maximum and the provider's models. */
  async function removeKey(name: string) {
    await act("delete api key", () => api.deleteApiKey(name));
    await Promise.all([readWindow(llm?.model ?? null), readModels(provider)]);
  }

  onMount(async () => {
    try {
      const s = await api.llmSettings();
      const at = s.model?.indexOf(":") ?? -1;
      provider = at > 0 ? s.model!.slice(0, at) : "";
      model = at > 0 ? s.model!.slice(at + 1) : "";
      tokens = s.context_tokens?.toString() ?? "";
      llm = s;
    } catch (e) {
      report("model settings", e);
    }
    await Promise.all([readWindow(llm?.model ?? null), readModels(provider)]);
  });
</script>

<div class="screen">
  <!-- Held while the library changes: leaving would drop its report of what failed. -->
  <button onclick={onback} disabled={libraryBusy}>← Back</button>
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
        readModels(name);
      }}
    >
      {#if !provider}<option value="">Choose a provider</option>{/if}
      {#each llm?.providers ?? [] as p}<option value={p.name}>{p.name}</option>{/each}
    </select>
    <label for="model">Model</label>
    {#if choices}
      <select
        id="model"
        value={model.trim()}
        onchange={(e) => {
          const picked = e.currentTarget.value;
          if (picked === OTHER) typing = true;
          else model = picked;
        }}
      >
        {#if !model.trim()}<option value="">Choose a model</option>{/if}
        {#each choices as m (m)}<option value={m}>{m}{models?.includes(m) ? "" : ` (not listed by ${provider})`}</option>{/each}
        <option value={OTHER}>Other…</option>
      </select>
    {:else}
      <input id="model" type="text" bind:value={model} autocomplete="off" />
      {#if typing}<p>
          <button type="button" class="link" onclick={() => (typing = false)}>Choose from {provider}'s list</button>
        </p>{/if}
    {/if}
    {#if modelsPending}<p class="muted">Listing {provider}'s models…</p>
    {:else if modelsNote}<p class="muted">{modelsNote}</p>
    {:else if models?.length === 0}<p class="muted">
        {provider} lists no models. Type the model's name; a local provider such as Ollama may need one pulled
        first.
      </p>{/if}
    {#if llm?.model}
      <p class="muted">
        In use: {llm.model}.
        {#if typeof maxWindow === "number"}Its maximum context window is {maxWindow.toLocaleString("en-US")} tokens.
        {:else if maxWindow === null}Its provider states no maximum context window, so answers use passages unless a limit is set below.
        {:else if windowNote}{windowNote}{/if}
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
        <button type="button" disabled={saving || p.key_source !== "keychain"} onclick={() => removeKey(p.name)}>Remove key</button>
      </div>
    </form>
  {/each}
  <h3>Paper library</h3>
  <LibrarySetting {onerror} bind:busy={libraryBusy} />
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
  .link {
    border: none;
    padding: 0.15rem 0;
    text-align: left;
    text-decoration: underline;
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
