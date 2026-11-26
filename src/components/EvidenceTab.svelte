<script lang="ts">
  /**
   * The Evidence weight tab's side beside the paper (design 2026-11-24, §7).
   * At the top, the earlier runs and a run going with its step and *Stop*.
   * Then the run shown: the study design with its citations and the run's
   * caption; a run under earlier rules says so (Re-run only); a design with no
   * method yet, or a trial with no primary outcome, says so; otherwise the
   * paper-weight line, then one card per primary outcome (`EvidenceCard`).
   * Then the steps that failed with why, a run stopped as too long for its
   * model's window says so (Re-run only), and *Continue* (an incomplete run
   * under today's rules) or *Re-run*. Without a run it says why none can start
   * (no model, no key, a text not yet confirmed), or why the newest run cannot
   * be read. Its run state lives in the tab host, since this remounts on every
   * tab switch.
   * It routes by `mode` (plan 3, design 2026-11-26 §3): on a paper never shown,
   * only the choice, *Appraise it yourself first* or *Show the machine's
   * verdict*; with an open appraisal, the reader's appraisal (`AppraisalPane`);
   * otherwise the machine's runs as above, with *Appraise it yourself* (not
   * while an appraisal is open). Once the open appraisal is revealed, a
   * *Your appraisal | Machine runs* switch heads both sides; an appraisal set
   * aside here says so (*Set aside {date}*).
   * `mode`: which side shows; null while the tab's view loads (nothing to choose yet).
   * `appraisal`: the reader's appraisal state (the tab host's), which the pane drives.
   * `onmachine`: *Show the machine's verdict*; `onappraise`: start an appraisal;
   *   `onopenappraisal`: open the paper's appraisal again after a failed load.
   * `onmark`: arms the paper pane for a question's sentence (null cancels).
   * `onside`: the switch, true for *Machine runs*.
   * `run`: the run shown, or null when there is none; `readOnly` when it is an
   *   earlier one (`oncurrent` returns to the newest).
   * `latestCanonicalId`: the paper's latest text, for the caption.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   * `running` / `stopping`: a run is going / its stop was pressed.
   * `progress`: the latest event of the run going, null before one arrives.
   * `blocked`: why nothing can be run on this text, or null.
   * `needsModel`: no model is chosen, so the tab asks for one.
   * `chosenModel`: the model chosen in Settings, for *Continue*'s label.
   * `notice`: why a run did not start, or null; `settings` when Settings can fix it.
   * `offer`: the first run is offered (no readable run on the latest text, none going).
   * `currentError`: why the newest run on the latest text cannot be read, or null.
   * `offerContinue`: *Continue* is offered (`oncontinue` continues the run shown).
   * `earlier`: the paper's other runs; `onopen` shows one.
   * `busy`: a load is running; actions wait.
   * `navBusy`: something on the paper screen is running; leaving for Settings waits.
   * `onrun`: a new run; `onstop`, `onsettings`: stop the run, open Settings.
   */
  import type { EvidenceEvent, EvidenceRunSummary, EvidenceRunView } from "../lib/api";
  import { APPRAISAL_LABELS, setAsideLine, type QuestionAt, type TabMode } from "../lib/appraisal";
  import type { AppraisalTab } from "../lib/appraisalTab.svelte";
  import {
    CONDENSED_NOTE,
    NO_PRIMARY_OUTCOME,
    citeKeys,
    continueCaption,
    continueHint,
    designLine,
    currentErrorNote,
    earlierRulesNote,
    notCoveredLine,
    paperWeightLine,
    runningNote,
    tooLongNote,
    type EvidenceCite,
  } from "../lib/evidence";
  import { CREATED_AT_FORMAT, parseSqliteUtc } from "../lib/interrogator";
  import { runCaption, type PicoNotice } from "../lib/pico";
  import AppraisalPane from "./AppraisalPane.svelte";
  import EvidenceCard from "./EvidenceCard.svelte";
  import EvidenceQuotes from "./EvidenceQuotes.svelte";

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
    currentError,
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
    mode,
    appraisal,
    onmachine,
    onappraise,
    onopenappraisal,
    onmark,
    onside,
  }: {
    run: EvidenceRunView | null;
    readOnly: boolean;
    latestCanonicalId: number;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
    running: boolean;
    stopping: boolean;
    progress: EvidenceEvent | null;
    blocked: string | null;
    needsModel: boolean;
    chosenModel: string | null;
    notice: PicoNotice | null;
    offer: boolean;
    currentError: string | null;
    offerContinue: boolean;
    earlier: EvidenceRunSummary[];
    busy: boolean;
    navBusy: boolean;
    onrun: () => void;
    oncontinue: () => void;
    onstop: () => void;
    onopen: (runId: number) => void;
    oncurrent: () => void;
    onsettings: () => void;
    mode: TabMode | null;
    appraisal: AppraisalTab;
    onmachine: () => void;
    onappraise: () => void;
    onopenappraisal: () => void;
    onmark: (at: QuestionAt | null) => void;
    onside: (machine: boolean) => void;
  } = $props();

  /** One line of the earlier-runs list: when, by which model, whether finished, and whether on an older text. */
  function earlierLabel(s: EvidenceRunSummary): string {
    const when = parseSqliteUtc(s.created_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? s.created_at;
    const unfinished = s.status === "complete" ? "" : " · not complete";
    const older = s.canonical_id === latestCanonicalId ? "" : " · on an earlier version of the text";
    return `${when} · ${s.model}${unfinished}${older}`;
  }

  /** Whether nothing runs or loads, so the tab's actions may start. */
  let idle = $derived(!running && !busy && !appraisal.running && !appraisal.working);
  let weight = $derived(run ? paperWeightLine(run) : null);
  let rules = $derived(run ? earlierRulesNote(run) : null);
  let tooLong = $derived(run ? tooLongNote(run) : null);
  let hint = $derived(run && offerContinue ? continueHint(run, chosenModel) : null);
  /** Whether an appraisal of the paper is open here (not set aside), so no other is offered. */
  let open = $derived(appraisal.view !== null && appraisal.view.set_aside_at === null);
  /** Whether the open appraisal is revealed: then the switch to the machine's runs shows. */
  let revealed = $derived(open && appraisal.view?.revealed_at != null);
</script>

{#if revealed && mode !== "choice"}
  <div class="side-switch" role="group" aria-label="Show">
    <button aria-pressed={mode === "appraisal"} onclick={() => onside(false)}>{APPRAISAL_LABELS.yourAppraisal}</button>
    <button aria-pressed={mode === "machine"} onclick={() => onside(true)} disabled={!idle}
      >{APPRAISAL_LABELS.machineRuns}</button
    >
  </div>
{/if}
{#if !open && appraisal.view?.set_aside_at}
  <p class="aside-line">Your appraisal: {setAsideLine(appraisal.view.set_aside_at)}</p>
{/if}

{#if mode === "choice"}
  <div class="body">
    {#if blocked}
      <p class="muted">{blocked}</p>
    {:else}
      <p>Appraise this trial's risk of bias yourself before you see the machine's verdict, or see it now.</p>
      <div class="actions">
        <button class="primary" onclick={onappraise} disabled={!idle}>{APPRAISAL_LABELS.appraiseFirst}</button>
        <button onclick={onmachine} disabled={!idle}>{APPRAISAL_LABELS.showMachine}</button>
      </div>
    {/if}
  </div>
{:else if mode === "appraisal"}
  {#if appraisal.view}
    <div class="body appraisal">
      <AppraisalPane
        view={appraisal.view}
        running={appraisal.running}
        stopping={appraisal.stopping}
        progress={appraisal.progress}
        idle={!appraisal.working && !appraisal.settingAside}
        notice={appraisal.notice}
        {current}
        {oncite}
        onconfirm={(input) => appraisal.confirm(input)}
        onedit={(edit) => appraisal.edit(edit)}
        onsave={() => appraisal.save()}
        onreveal={() => appraisal.reveal()}
        oncontinue={() => appraisal.run(false)}
        onrunagain={() => appraisal.run(true)}
        onstop={() => appraisal.stop()}
        {onsettings}
        marking={appraisal.marking}
        {onmark}
        onversion={(n) => appraisal.showVersion(n)}
        onclose={() => appraisal.closeRevision()}
        onsetaside={() => appraisal.setAside()}
        settingAside={appraisal.settingAside}
      />
    </div>
  {:else if !appraisal.working}
    <div class="body">
      <p>Your appraisal could not be opened.</p>
      <button onclick={onopenappraisal}>Open your appraisal</button>
    </div>
  {/if}
{:else}
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
      <p>Choose a model in Settings to assess the evidence weight.</p>
      <button onclick={onsettings} disabled={navBusy}>Settings</button>
    {:else if notice}
      <p>{notice.text}</p>
      {#if notice.settings}<button onclick={onsettings} disabled={navBusy}>Settings</button>{/if}
    {/if}
    {#if currentError}
      <p class="failed">{currentErrorNote(currentError)}</p>
    {/if}
    {#if offer && !blocked && !needsModel}
      <button class="primary" onclick={onrun} disabled={!idle}>Assess the evidence weight</button>
    {/if}
    {#if run}
      <div class="run">
        {#if run.design}
          <p class="type">{designLine(run.design.design, run.design.rct_layout)}</p>
          <EvidenceQuotes
            citations={run.design.quotes.citations}
            lost={run.design.quotes}
            citeKey={citeKeys.design}
            {current}
            {oncite}
          />
          {#if run.design.reasoning}<p class="note">{run.design.reasoning}</p>{/if}
        {/if}
        <p class="caption muted">{runCaption(run, latestCanonicalId)}</p>
        {#if rules}<p class="note">{rules}</p>{/if}
        {#if run.kind === "not_covered" && run.design}
          <p class="verdict">{notCoveredLine(run.design)}</p>
        {:else if run.kind === "no_primary_outcome"}
          <p class="verdict">{NO_PRIMARY_OUTCOME}</p>
        {/if}
        {#if weight}
          <p class="weight">{weight}</p>
          {#if run.condensed}<p class="note">{CONDENSED_NOTE}</p>{/if}
        {/if}
        {#each run.outcomes as o (o.index)}
          <EvidenceCard outcome={o} study={run.study} condensed={run.condensed} {current} {oncite} />
        {/each}
        {#each run.failed_steps as f}
          <p class="failed">{f.label} failed: {f.error}</p>
        {/each}
        {#if tooLong}<p class="note">{tooLong}</p>{/if}
        <!-- Continue runs on the run's own model and text, so an earlier run shown can be continued too. -->
        {#if offerContinue || (!readOnly && !blocked)}
          <div class="actions">
            {#if offerContinue}
              <button class="primary" onclick={oncontinue} disabled={!idle}>{continueCaption(run, chosenModel)}</button>
            {/if}
            {#if !readOnly && !blocked}
              <button onclick={onrun} disabled={!idle}>Re-run</button>
              {#if hint}<span class="note">{hint}</span>{/if}
            {/if}
          </div>
        {/if}
      </div>
    {/if}
    {#if mode === "machine" && !blocked && !open}
      <button onclick={onappraise} disabled={!idle}>{APPRAISAL_LABELS.appraise}</button>
    {/if}
  </div>
{/if}

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
  .side-switch {
    display: flex;
    justify-content: center;
    gap: 0.25rem;
    padding: 0.5rem;
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
  }
  .side-switch button[aria-pressed="true"] {
    background: var(--accent);
    color: var(--paper-bg);
  }
  .aside-line {
    margin: 0;
    padding: 0.5rem 0.75rem 0;
    font-family: var(--sans);
    font-size: 0.85rem;
    font-style: italic;
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
  .body.appraisal {
    padding: 0;
    align-items: stretch;
    gap: 0;
  }
  .run {
    align-self: stretch;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .type {
    font-family: var(--serif);
    font-size: 1rem;
  }
  .verdict {
    font-weight: 700;
  }
  .weight {
    font-size: 0.8rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
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
</style>
