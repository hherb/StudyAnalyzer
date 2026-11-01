<script lang="ts">
  /**
   * The start screen: the recent papers and the three ways to open one (file, paste, identifier).
   * `recent`: the stored papers, most recently opened first.
   * `busyText`: what is running, shown with the controls disabled, or null when idle.
   * `oncancel`: cancels the running lookup; Cancel is offered only when it is given.
   * `onreopen`: opens a stored paper by its id.
   * `onpickfile`: asks for a file to open; `onpaste` opens the pasted text; `onidentifier` looks up a PMID, DOI or PMC ID.
   */
  import type { PaperSummary } from "../lib/api";

  let {
    recent,
    busyText,
    oncancel,
    onreopen,
    onpickfile,
    onpaste,
    onidentifier,
  }: {
    recent: PaperSummary[];
    busyText: string | null;
    oncancel?: () => void;
    onreopen: (id: string) => void;
    onpickfile: () => void;
    onpaste: (text: string) => void;
    onidentifier: (input: string) => void;
  } = $props();

  let busy = $derived(busyText !== null);
  let pasted = $state("");
  let ident = $state("");

  /** The identifiers of a paper as one line. */
  function idLine(p: PaperSummary): string {
    const i = p.identifiers;
    return [i.pmid && `PMID ${i.pmid}`, i.pmc_id, i.doi && `DOI ${i.doi}`].filter(Boolean).join(" · ");
  }
</script>

<div class="screen">
  {#if busy}
    <p class="spin" role="status">
      <span class="dot"></span> {busyText}
      {#if oncancel}<button onclick={oncancel}>Cancel</button>{/if}
    </p>
  {/if}

  <section>
    <h2>Open a paper</h2>
    <button class="primary" onclick={onpickfile} disabled={busy}>Open a file…</button>
    <span class="muted">or drop a PDF, XML, HTML, Markdown or text file anywhere on this window</span>
  </section>

  <section>
    <label for="paste">Paste text</label>
    <textarea id="paste" rows="6" bind:value={pasted}></textarea>
    <button onclick={() => onpaste(pasted)} disabled={busy || !pasted.trim()}>Open</button>
  </section>

  <section>
    <label for="ident">PMID, DOI or PMC ID</label>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        onidentifier(ident);
      }}
    >
      <input id="ident" type="text" bind:value={ident} />
      <button disabled={busy || !ident.trim()}>Find</button>
    </form>
  </section>

  {#if recent.length}
    <section>
      <h2>Recent papers</h2>
      <ul>
        {#each recent as p (p.id)}
          <li>
            <button class="link" onclick={() => onreopen(p.id)} disabled={busy}>{p.title}</button>
            <div class="muted">{idLine(p)} {idLine(p) ? "·" : ""} opened {p.opened_at}</div>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  ul {
    list-style: none;
    padding: 0;
  }
  li {
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--rule);
  }
  button.link {
    border: none;
    padding: 0;
    text-align: left;
    font-size: 1rem;
  }
  form {
    display: flex;
    gap: 0.5rem;
  }
  h2 {
    font-size: 1rem;
    color: var(--accent);
  }
  .dot {
    display: inline-block;
    width: 0.8em;
    height: 0.8em;
    border: 2px solid var(--accent);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
