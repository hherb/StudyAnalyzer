<script lang="ts">
  /**
   * The PRISMA 2020 tab's side beside the paper (design §8). At the top, the
   * earlier runs and a run going with *Stop*. Then the run shown: what the
   * check found (a paper not suitable says so, with its type and the model's
   * rationale, and offers *Run anyway*), the key-gaps panel (each key item with
   * its chip; clicking one opens its section, scrolls to it and highlights its
   * first citation) with its count lines, then the checklist by section with
   * its counts (Methods and Results open first, see `PrismaCard`), the parts
   * that failed with why, every note, *Continue* for an incomplete run and
   * *Re-run*, the caption, and the checklist's licence. Without a run it says
   * why none can start (no model, no key, a text not yet confirmed). Its run
   * state lives in the tab host, since this remounts on every tab switch; only
   * which sections are open is its own.
   * `run`: the run shown, or null when there is none; `readOnly` when it is an
   *   earlier one (`oncurrent` returns to the newest).
   * `latestCanonicalId`: the paper's latest text, for the caption.
   * `current`: the citation in focus, or null; `oncite` is called when one is
   *   clicked, `toPaper` false for a key row (the phone stays on the checklist).
   * `running` / `stopping`: a run is going / its stop was pressed.
   * `progress`: the latest event of the run going, null before one arrives.
   * `blocked`: why nothing can be run on this text, or null.
   * `needsModel`: no model is chosen, so the tab asks for one.
   * `chosenModel`: the model chosen in Settings, for *Continue*'s label.
   * `notice`: why a run did not start, or null; `settings` when Settings can fix it.
   * `offer`: the first run is offered (no run on the latest text, none going).
   * `offerContinue`: *Continue* is offered (the run shown is incomplete, an
   *   earlier one included; `oncontinue` continues the run shown).
   * `earlier`: the paper's other runs; `onopen` shows one.
   * `busy`: a load is running; actions wait.
   * `navBusy`: something on the paper screen is running; leaving for Settings waits.
   * `onrun(force)`: a new run; `oncontinue`: the shown run's remaining parts;
   *   `onstop`, `onsettings`: stop the run, open Settings.
   */
  import { tick } from "svelte";
  import type { PrismaEvent, PrismaRunSummary, PrismaRunView } from "../lib/api";
  import { CREATED_AT_FORMAT, parseSqliteUtc } from "../lib/interrogator";
  import { checkNote, percent, runCaption, type PicoNotice } from "../lib/pico";
  import {
    OPEN_SECTIONS,
    continueCaption,
    continueHint,
    judgementLabel,
    keyGapsLine,
    keyItems,
    offerRunAnyway,
    runningNote,
    sectionLine,
    variantNote,
    type PrismaCite,
  } from "../lib/prisma";
  import PrismaCard from "./PrismaCard.svelte";

  let {
    run,
    readOnly,
    latestCanonicalId,
    current,
    oncite,
    running,
    stopping,
    progress,
    blocked,
    needsModel,
    chosenModel,
    notice,
    offer,
    offerContinue,
    earlier,
    busy,
    navBusy,
    onrun,
    oncontinue,
    onstop,
    onopen,
    oncurrent,
    onsettings,
  }: {
    run: PrismaRunView | null;
    readOnly: boolean;
    latestCanonicalId: number;
    current: PrismaCite | null;
    oncite: (cite: PrismaCite, toPaper?: boolean) => void;
    running: boolean;
    stopping: boolean;
    progress: PrismaEvent | null;
    blocked: string | null;
    needsModel: boolean;
    chosenModel: string | null;
    notice: PicoNotice | null;
    offer: boolean;
    offerContinue: boolean;
    earlier: PrismaRunSummary[];
    busy: boolean;
    navBusy: boolean;
    onrun: (force: boolean) => void;
    oncontinue: () => void;
    onstop: () => void;
    onopen: (runId: number) => void;
    oncurrent: () => void;
    onsettings: () => void;
  } = $props();

  /** The sections the user opened or closed, by wire string; the rest follow `OPEN_SECTIONS`. */
  let toggled = $state<Record<string, boolean>>({});

  /** Whether `section` shows its items. */
  const isOpen = (section: string) => toggled[section] ?? OPEN_SECTIONS.includes(section);

  /** One line of the earlier-runs list: when, by which model, whether finished, and whether on an older text. */
  function earlierLabel(s: PrismaRunSummary): string {
    const when = parseSqliteUtc(s.created_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? s.created_at;
    const unfinished = s.status === "complete" ? "" : " · not complete";
    const older = s.canonical_id === latestCanonicalId ? "" : " · on an earlier version of the text";
    return `${when} · ${s.model}${unfinished}${older}`;
  }

  /** Opens key item `id`'s section, scrolls the checklist to it, and highlights its first citation. */
  async function focusItem(id: string) {
    const item = run?.items.find((i) => i.id === id);
    if (!item) return;
    toggled[item.section] = true;
    await tick();
    document.getElementById(`prisma-item-${id}`)?.scrollIntoView({ block: "start" });
    if (item.citations.length) oncite({ item: id, index: 0 }, false);
  }

  /** Whether nothing runs or loads, so the tab's actions may start. */
  let idle = $derived(!running && !busy);
  let variant = $derived(run ? variantNote(run.variant) : null);
  let hint = $derived(run && offerContinue ? continueHint(run, chosenModel) : null);
</script>

<div class="head">
  {#if readOnly}
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
      <p>{runningNote(progress, stopping)}</p>
      <button onclick={onstop} disabled={stopping}>Stop</button>
    </div>
  {/if}
  {#if blocked && !run}
    <p class="muted">{blocked}</p>
  {:else if needsModel && !run}
    <p>Choose a model in Settings to assess PRISMA 2020.</p>
    <button onclick={onsettings} disabled={navBusy}>Settings</button>
  {:else if notice}
    <p>{notice.text}</p>
    {#if notice.settings}<button onclick={onsettings} disabled={navBusy}>Settings</button>{/if}
  {/if}
  {#if offer && !blocked && !needsModel}
    <button class="primary" onclick={() => onrun(false)} disabled={!idle}>Assess PRISMA 2020</button>
  {/if}
  {#if run}
    <div class="run">
      {#if run.suitability && !run.suitability.is_suitable}
        <section class="box">
          <p><strong>This paper does not look like a systematic review, so PRISMA 2020 may not apply.</strong></p>
          <p>Document type: {run.suitability.document_type} (as the model gave it)</p>
          {#if run.suitability.rationale}<p class="explanation">{run.suitability.rationale}</p>{/if}
        </section>
      {:else}
        <p class="type">{run.suitability?.document_type ?? "Assessed without the suitability check"}</p>
        {#if variant}<p class="note">{variant}</p>{/if}
      {/if}
      {#if run.suitability}
        <p class="note">{[checkNote(run), `confidence ${percent(run.suitability.confidence)}`].filter((n) => n).join(" · ")}</p>
      {/if}
      {#if !run.suitability || run.suitability.is_suitable}
        <section class="box keys" aria-label="Key gaps">
          <h3>Key gaps</h3>
          {#each keyItems(run) as item (item.id)}
            <button class="key" onclick={() => focusItem(item.id)}>
              <span class="id">{item.id}</span>
              <span class="name">{item.name}</span>
              <span class="prisma-chip {item.judgement ?? 'not_assessed'}">{judgementLabel(item.judgement)}</span>
            </button>
          {/each}
          {#each keyGapsLine(run.key_gaps) as line}<p class="count">{line}</p>{/each}
        </section>
        {#each run.sections as s (s.section)}
          <details
            class="section"
            open={isOpen(s.section)}
            ontoggle={(e) => (toggled[s.section] = e.currentTarget.open)}
          >
            <summary>{sectionLine(s)}</summary>
            {#each run.items.filter((i) => i.section === s.section) as item (item.id)}
              <PrismaCard
                {item}
                current={current?.item === item.id ? current.index : null}
                oncite={(index) => oncite({ item: item.id, index })}
              />
            {/each}
          </details>
        {/each}
      {/if}
      {#each run.failed_parts as f}
        <p class="failed">Part {f.label} failed: {f.error_message ?? f.error_kind ?? "no reason was stored"}</p>
      {/each}
      {#each run.notes as n}<p class="note">{n}</p>{/each}
      <!-- Continue runs on the run's own model and text, so an earlier run shown can be continued too (spec §7). -->
      {#if offerContinue || (!readOnly && !blocked)}
        <div class="actions">
          {#if offerContinue}
            <button class="primary" onclick={oncontinue} disabled={!idle}>{continueCaption(run, chosenModel)}</button>
          {/if}
          {#if !readOnly && !blocked}
            {#if offerRunAnyway(run)}
              <button onclick={() => onrun(true)} disabled={!idle}>Run anyway</button>
            {/if}
            <button onclick={() => onrun(run.forced)} disabled={!idle}>Re-run</button>
            {#if hint}<span class="note">{hint}</span>{/if}
          {/if}
        </div>
      {/if}
      <p class="caption muted">{runCaption(run, latestCanonicalId)}</p>
    </div>
  {/if}
  <p class="caption muted licence">PRISMA 2020 checklist © Page MJ et al., BMJ 2021;372:n71, CC BY 4.0.</p>
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
    margin: 0;
    font-size: 0.75rem;
  }
  .head .caption {
    font-size: 0.85rem;
    font-style: italic;
  }
  details {
    font-size: 0.85rem;
  }
  .head details ul {
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
  .body p {
    margin: 0;
  }
  .run {
    align-self: stretch;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .type {
    font-family: var(--serif);
    font-size: 1rem;
  }
  .box {
    padding: 0.5rem 0.7rem;
    border: 1px solid var(--rule);
    border-radius: 10px;
  }
  h3 {
    margin: 0 0 0.3rem;
    font-size: 0.8rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .key {
    display: flex;
    width: 100%;
    gap: 0.4rem;
    align-items: baseline;
    padding: 0.2rem 0;
    border: none;
    color: var(--ink);
    text-align: left;
  }
  .id {
    min-width: 2rem;
    font-weight: 700;
  }
  .name {
    flex: 1;
  }
  .count {
    margin-top: 0.3rem;
    font-weight: 700;
  }
  .section {
    font-size: 0.9rem;
  }
  .section summary {
    padding: 0.3rem 0;
    font-weight: 700;
    cursor: pointer;
  }
  .explanation {
    margin-top: 0.3rem;
  }
  .note {
    font-size: 0.85rem;
    font-style: italic;
  }
  .failed {
    color: var(--err-ink);
    font-size: 0.85rem;
  }
  .status {
    display: flex;
    gap: 0.75rem;
    align-items: center;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .licence {
    margin-top: auto;
  }
</style>
