<script lang="ts">
  /**
   * The Library setting (design 2026-11-13, §4.1, §4.6, §4.7): the folder
   * set (a label only: Rust shows the folder dialog and never passes the
   * path), *Choose…* with the offer to bring this app's papers along, *No
   * library* with the offer to copy its files back, the move of the files
   * the app keeps, and the sources still waiting for the library. One change
   * runs at a time; what it did is shown below the buttons.
   * `onerror`: shows a failure, already logged.
   * `busy`: bound; true while a change runs (Settings then keeps *Back*
   *   disabled, so the change's report is not lost with the screen).
   */
  import { onMount } from "svelte";
  import * as api from "../lib/api";
  import { KIND_NAMES, bringLabel, changeNote, libraryCaption, moveLabel } from "../lib/library";

  let { onerror, busy = $bindable(false) }: { onerror: (err: api.CommandError) => void; busy?: boolean } =
    $props();

  /** The setting as Rust reads it; null until read. */
  let status = $state<api.LibraryStatus | null>(null);
  /** Whether *Choose…* brings this app's papers along. */
  let bring = $state(false);
  /** Whether *No library* copies this app's files back into the app. */
  let bringBack = $state(false);
  /** What a running change is doing, or null when none runs. */
  let running = $state<string | null>(null);
  /** What the last change did, or null. */
  let note = $state<string | null>(null);
  /** The bring checkbox's label, or null when there is nothing to bring (no checkbox). */
  let offer = $derived(status ? bringLabel(status) : null);

  /** Logs a failure and shows it in the banner. */
  function report(where: string, e: unknown) {
    const err = api.toCommandError(e);
    api.logFailure(where, err);
    onerror(err);
  }

  /** Rereads the setting. */
  async function reread() {
    try {
      status = await api.libraryStatus();
    } catch (e) {
      report("library status", e);
    }
  }

  /** Runs one change (`doing` is shown meanwhile), shows what it did, then rereads the setting. */
  async function change(doing: string, fn: () => Promise<api.Changed | null>) {
    running = doing;
    busy = true;
    note = null;
    try {
      const changed = await fn();
      if (changed) {
        note = changeNote(changed);
        bring = false;
        bringBack = false;
      }
    } catch (e) {
      report("library setting", e);
    }
    await reread();
    running = null;
    busy = false;
  }

  onMount(reread);
</script>

<p>{status ? libraryCaption(status) : "Reading the library setting…"}</p>
{#if status}
  <p class="muted">
    One folder of papers the bmlib apps share. Nothing in it is ever deleted by the app; the apps write it, so do not
    add your own files.
  </p>
  <p>
    <button
      class="primary"
      disabled={running !== null}
      onclick={() => change("Setting the library folder…", () => api.chooseLibrary(bring && offer !== null))}
      >{status.path === null ? "Choose…" : "Choose another…"}</button
    >
  </p>
  {#if offer}<label><input type="checkbox" bind:checked={bring} disabled={running !== null} /> {offer}</label>{/if}
  {#if status.path !== null}
    {#if status.unreachable === null && status.counts.app > 0}
      <p>
        <button disabled={running !== null} onclick={() => change("Moving files into the library…", api.moveIntoLibrary)}
          >{moveLabel(status)}</button
        >
      </p>
    {/if}
    <p>
      <button
        disabled={running !== null}
        onclick={() => change("Setting no library…", () => api.clearLibrary(bringBack && status!.counts.library > 0 && status!.unreachable === null))}
        >No library</button
      >
    </p>
    {#if status.counts.library > 0 && status.unreachable === null}
      <label
        ><input type="checkbox" bind:checked={bringBack} disabled={running !== null} /> Copy this app's files back into
        the app</label
      >
    {/if}
  {/if}
  {#if running}<p class="muted" role="status">{running}</p>{/if}
  {#if note}<p role="status">{note}</p>{/if}
  {#if status.waiting.length > 0}
    <h4>Waiting for the library</h4>
    <ul>
      {#each status.waiting as w, i (i)}
        <li>{w.title} ({KIND_NAMES[w.kind]}){#if w.note}: {w.note}{/if}</li>
      {/each}
    </ul>
  {/if}
{/if}
