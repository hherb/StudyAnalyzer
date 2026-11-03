<script lang="ts">
  /**
   * The PICO tab's side beside the paper: the run shown (see `PicoCard`), what
   * can be done with it (*Re-run*, *Run anyway*), the earlier runs, a run going
   * with *Stop*, and why no run can start (no model, no key, a text not yet
   * confirmed). It holds no state of its own: the tab host keeps the run's state,
   * since this remounts on every tab switch.
   * `run`: the run shown, or null when there is none.
   * `latestCanonicalId`: the paper's latest text, for captions.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   * `running` / `stopping`: a run is going / its stop was pressed.
   * `blocked`: why nothing can be run on this text, or null.
   * `needsModel`: no model is chosen, so the tab asks for one.
   * `notice`: why an automatic run did not start (a missing key), or null.
   * `offer`: *Extract PICO* is offered (an attempt left no run).
   * `earlier`: the paper's other runs; `onopen` shows one, `oncurrent` returns to the newest.
   * `busy`: a load is running; actions wait.
   * `onrun(force)`, `onstop`, `onsettings`: start a run, stop it, open Settings.
   */
  import type { PicoRunSummary, PicoRunView } from "../lib/api";
  import { CREATED_AT_FORMAT, parseSqliteUtc } from "../lib/interrogator";
  import PicoCard from "./PicoCard.svelte";

  let {
    run,
    latestCanonicalId,
    current,
    oncite,
    running,
    stopping,
    blocked,
    needsModel,
    notice,
    offer,
    earlier,
    busy,
    onrun,
    onstop,
    onopen,
    oncurrent,
    onsettings,
  }: {
    run: PicoRunView | null;
    latestCanonicalId: number;
    current: number | null;
    oncite: (number: number) => void;
    running: boolean;
    stopping: boolean;
    blocked: string | null;
    needsModel: boolean;
    notice: string | null;
    offer: boolean;
    earlier: PicoRunSummary[];
    busy: boolean;
    onrun: (force: boolean) => void;
    onstop: () => void;
    onopen: (runId: number) => void;
    oncurrent: () => void;
    onsettings: () => void;
  } = $props();

  /** One line of the earlier-runs list: when, by which model, and whether on an older text. */
  function earlierLabel(s: PicoRunSummary): string {
    const when = parseSqliteUtc(s.created_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? s.created_at;
    const older = s.canonical_id === latestCanonicalId ? "" : " · on an earlier version of the text";
    return `${when} · ${s.model}${older}`;
  }

  let idle = $derived(!running && !busy);
</script>

<div class="head">
  {#if run?.read_only}
    <span class="caption">earlier run</span>
    <button onclick={oncurrent} disabled={!idle}>← Current run</button>
  {/if}
  {#if earlier.length}
    <details>
      <summary>Earlier runs ({earlier.length})</summary>
      <ul>
        {#each earlier as s (s.id)}
          <li><button class="link" onclick={() => onopen(s.id)} disabled={!idle}>{earlierLabel(s)}</button></li>
        {/each}
      </ul>
    </details>
  {/if}
</div>

<div class="body">
  {#if running}
    <div class="status" aria-live="polite">
      <p>
        {stopping ? "Stopping…" : "Extracting PICO… A long paper is condensed first and can take several minutes."}
      </p>
      <button onclick={onstop} disabled={stopping}>Stop</button>
    </div>
  {/if}
  {#if blocked && !run}
    <p class="muted">{blocked}</p>
  {:else if needsModel && !run}
    <p>Choose a model in Settings to extract PICO.</p>
    <button onclick={onsettings}>Settings</button>
  {:else if notice}
    <p>{notice}</p>
    <button onclick={onsettings}>Settings</button>
  {/if}
  {#if offer && !blocked && !needsModel}
    <button class="primary" onclick={() => onrun(false)} disabled={!idle}>Extract PICO</button>
  {/if}
  {#if run}
    <PicoCard {run} {latestCanonicalId} {current} {oncite} />
    {#if !run.read_only && !blocked}
      <div class="actions">
        {#if run.extraction}
          <button onclick={() => onrun(run.suitability === null)} disabled={!idle}>Re-run</button>
        {:else}
          <button onclick={() => onrun(true)} disabled={!idle}>Run anyway</button>
          <button onclick={() => onrun(false)} disabled={!idle}>Re-run</button>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .head {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
  }
  .head:empty {
    display: none;
  }
  .caption {
    font-size: 0.85rem;
    font-style: italic;
  }
  details {
    font-size: 0.85rem;
  }
  details ul {
    margin: 0.25rem 0 0;
    padding-left: 1rem;
  }
  .link {
    border: none;
    padding: 0.15rem 0;
    text-align: left;
    text-decoration: underline;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.75rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
  .body > :global(.card) {
    align-self: stretch;
  }
  .body p {
    margin: 0;
  }
  .status {
    display: flex;
    gap: 0.75rem;
    align-items: center;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
</style>
