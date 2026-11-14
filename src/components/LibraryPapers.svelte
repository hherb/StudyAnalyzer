<script lang="ts">
  /**
   * The manual clear (design 2026-11-13, §4.8): on request, the library's
   * papers, each with its size, its files and the apps that use them. The
   * user removes a paper (its folder) or one file after a confirmation, shown
   * here, that names the other apps still using it. Nothing is removed
   * otherwise. After each removal the list is read again.
   * `onerror`: shows a failure, already logged.
   * `busy`: bound; true while the list is read or a removal runs.
   */
  import * as api from "../lib/api";
  import { paperSize, paperUsers, problemsNote, removeQuestion, sizeLabel, usersNote } from "../lib/library";

  let { onerror, busy = $bindable(false) }: { onerror: (err: api.CommandError) => void; busy?: boolean } =
    $props();

  /** The library's papers, or null until asked for. */
  let listing = $state<api.LibraryListing | null>(null);
  /** The removal awaiting the user's answer: what it removes, its question and what it runs. */
  let pending = $state<{ what: string; question: string; run: () => Promise<void> } | null>(null);
  /** What the last removal did, or null. */
  let note = $state<string | null>(null);

  /** Logs a failure and shows it in the banner. */
  function report(where: string, e: unknown) {
    const err = api.toCommandError(e);
    api.logFailure(where, err);
    onerror(err);
  }

  /** Reads the library's papers; on failure the list shown goes, since it may be out of date. */
  async function load() {
    busy = true;
    try {
      listing = await api.libraryPapers();
    } catch (e) {
      listing = null;
      report("library papers", e);
    } finally {
      busy = false;
    }
  }

  /**
   * Shows the confirmation for removing `what` (the label shown), naming `usedBy` and whether this
   * app uses it; `run` performs the removal once the user confirms.
   */
  function ask(what: string, usedBy: string[], usedHere: boolean, run: () => Promise<void>) {
    note = null;
    pending = { what, question: removeQuestion(what, usedBy, usedHere), run };
  }

  /**
   * Runs the confirmed removal, says so, then reads the list again (after a failed one too: the
   * list may be stale). A failed removal is reported after the reread, so a failed reread does not
   * replace it in the banner.
   */
  async function confirm() {
    const { what, run } = pending!;
    pending = null;
    busy = true;
    let failure: unknown = null;
    try {
      await run();
      note = `“${what}” was removed from the paper library.`;
    } catch (e) {
      failure = e;
    } finally {
      busy = false;
    }
    await load();
    if (failure !== null) report("library removal", failure);
  }
</script>

<h4>Papers in the library</h4>
{#if note}<p role="status">{note}</p>{/if}
{#if listing === null}
  <p><button disabled={busy} onclick={load}>Show the library's papers</button></p>
{:else}
  <p><button disabled={busy} onclick={load}>Read the list again</button></p>
  {#if pending}
    <div class="confirm" role="alertdialog" aria-label="Confirm removal">
      <p>{pending.question}</p>
      <button class="primary" onclick={confirm}>Remove</button>
      <button onclick={() => (pending = null)}>Keep it</button>
    </div>
  {/if}
  {#if listing.papers.length === 0}
    <p class="muted">The library holds no papers.</p>
  {/if}
  <ul>
    {#each listing.papers as p (p.folder)}
      {@const users = paperUsers(p)}
      <li>
        <strong>{p.title ?? p.folder}</strong>
        <span class="muted">({paperSize(p)}, {usersNote(users.usedBy, users.usedHere)})</span>
        <button
          disabled={busy || pending !== null}
          onclick={() =>
            ask(p.title ?? p.folder, users.usedBy, users.usedHere, () => api.removeLibraryPaper(p.folder))}
          >Remove paper</button
        >
        <ul>
          {#each p.files as f (f.name)}
            <li>
              {f.name} <span class="muted">({sizeLabel(f.size)}, {usersNote(f.used_by, f.used_here)})</span>
              <button
                disabled={busy || pending !== null}
                onclick={() => ask(f.name, f.used_by, f.used_here, () => api.removeLibraryFile(p.folder, f.name))}
                >Remove file</button
              >
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ul>
  {#if problemsNote(listing.problems)}<p class="muted">{problemsNote(listing.problems)}</p>{/if}
{/if}

<style>
  .confirm {
    border: 1px solid currentColor;
    padding: 0.5em 1em;
    margin-block: 0.5em;
  }
</style>
