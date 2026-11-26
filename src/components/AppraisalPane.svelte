<script lang="ts">
  /**
   * The reader's own appraisal in the Evidence weight tab (design 2026-11-26,
   * §3–§6): the header says whether it is blind, the steps run across the top
   * (*1 Design → 2 Outcomes → 3 Your assessment → 4 Compare*), then the
   * machine's area and the reader's. The machine's area holds one neutral line
   * for its paired run (counts, a wait, or why it stopped, never a step's
   * name), *Stop* while it runs, *Continue* or *Run the machine again on these
   * outcomes* when it cannot go on, and only once the appraisal is revealed
   * the machine's verdict. The reader's area holds steps 1 and 2 while
   * confirming (`AppraisalConfirm`), then D1 and each confirmed outcome's
   * D2–D5 (`AppraisalDomain`), its escalation question and overall verdict
   * and the reader's ladder, then *Show the machine's verdict now* (asked
   * first on a blind appraisal) and *Save and compare*.
   * `view`: the appraisal to show.
   * `running` / `stopping`: its paired run goes / its stop was pressed.
   * `progress`: the paired run's latest counts, null before one arrives.
   * `idle`: nothing runs or loads here, so actions may start.
   * `notice`: why an automatic run did not start, or null (`onsettings` when Settings can fix it).
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   * `onconfirm`, `onedit`, `onsave`, `onreveal`: the reader's steps.
   * `oncontinue`, `onrunagain`, `onstop`: the paired run's.
   */
  import type { AppraisalEdit, AppraisalEvent, AppraisalView, ConfirmInput } from "../lib/api";
  import {
    APPRAISAL_LABELS,
    continueOffered,
    currentStep,
    machineLine,
    runAgainOffered,
    waitingForConfirmation,
  } from "../lib/appraisal";
  import { ANSWER_LABELS, ladderLine, notCoveredLine, resultLine, type EvidenceCite } from "../lib/evidence";
  import type { PicoNotice } from "../lib/pico";
  import AppraisalConfirm from "./AppraisalConfirm.svelte";
  import AppraisalDomain from "./AppraisalDomain.svelte";
  import AppraisalVerdict from "./AppraisalVerdict.svelte";
  import EvidenceCard from "./EvidenceCard.svelte";

  let {
    view,
    running,
    stopping,
    progress,
    idle,
    notice,
    current,
    oncite,
    onconfirm,
    onedit,
    onsave,
    onreveal,
    oncontinue,
    onrunagain,
    onstop,
    onsettings,
  }: {
    view: AppraisalView;
    running: boolean;
    stopping: boolean;
    progress: AppraisalEvent | null;
    idle: boolean;
    notice: PicoNotice | null;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
    onconfirm: (input: ConfirmInput) => void;
    onedit: (edit: AppraisalEdit) => void;
    onsave: () => void;
    onreveal: () => void;
    oncontinue: () => void;
    onrunagain: () => void;
    onstop: () => void;
    onsettings: () => void;
  } = $props();

  /** Whether the reveal's question is asked (a blind appraisal asks before it stops being blind). */
  let asking = $state(false);
  let step = $derived(currentStep(view));
  /** The machine's run, only once revealed: a view carrying it without a reveal shows nothing of it. */
  let revealed = $derived(view.revealed_at !== null ? view.machine.revealed : null);
  /** Whether the reader may answer: an editable version of an appraisal being answered. */
  let editable = $derived(view.state === "answering" && view.version.editable);

  /** *Show the machine's verdict now*: asked first while the appraisal is blind. */
  function reveal() {
    if (view.blind) asking = true;
    else onreveal();
  }
</script>

<header class="head">
  <h2>{view.blind ? APPRAISAL_LABELS.blind : APPRAISAL_LABELS.notBlind}</h2>
  <ol class="steps">
    {#each APPRAISAL_LABELS.steps as s, i (s)}
      <li class:done={i + 1 < step} aria-current={i + 1 === step ? "step" : undefined}>
        {s}{#if i + 1 < step}<span class="tick"> ✓<span class="sr">(done)</span></span>{/if}
      </li>
    {/each}
  </ol>
</header>

<section class="machine" data-area="machine" aria-label="The machine">
  {#if running || view.machine.run_id !== null}
    <p class="line" aria-live="polite">{machineLine(view.machine, progress ?? undefined)}</p>
  {/if}
  <div class="actions">
    {#if running}<button onclick={onstop} disabled={stopping}>{stopping ? "Stopping…" : "Stop"}</button>{/if}
    {#if continueOffered(view, running)}<button onclick={oncontinue} disabled={!idle}>Continue</button>{/if}
    {#if runAgainOffered(view, running)}
      <button onclick={onrunagain} disabled={!idle}>{APPRAISAL_LABELS.runAgain}</button>
    {/if}
  </div>
  {#if notice}
    <p>{notice.text}</p>
    {#if notice.settings}<button onclick={onsettings}>Settings</button>{/if}
  {/if}
  {#if revealed}
    <h3>The machine's verdict</h3>
    {#each revealed.outcomes as o (o.index)}
      <EvidenceCard outcome={o} study={revealed.study} condensed={revealed.condensed} {current} {oncite} />
    {/each}
  {/if}
</section>

<section class="reader" data-area="reader" aria-label="Your appraisal">
  {#if view.version.earlier_rules}<p class="note">{APPRAISAL_LABELS.earlierRules}</p>{/if}
  {#if view.state === "confirming"}
    {#if view.proposals && waitingForConfirmation(view)}
      <AppraisalConfirm proposals={view.proposals} {idle} {current} {oncite} {onconfirm} />
    {:else}
      <p class="muted">The machine is proposing the design and the primary outcomes; you confirm them next.</p>
    {/if}
  {:else if view.state === "not_covered" && view.confirmed}
    <p class="verdict">{notCoveredLine(view.confirmed)}</p>
  {:else}
    <AppraisalDomain domain={view.version.study} outcome={null} {editable} {idle} {onedit} />
    {#each view.version.outcomes as o (o.index)}
      <article class="outcome">
        <h3>{o.name}{o.timepoint ? ` at ${o.timepoint}` : ""}</h3>
        {#if o.result}<p class="result">{resultLine(o.result)}</p>{/if}
        {#each o.domains as d (d.domain)}
          <AppraisalDomain domain={d} outcome={o.index} {editable} {idle} {onedit} />
        {/each}
        {#if o.needs_escalation}
          <div class="escalation">
            <p>{APPRAISAL_LABELS.escalation}</p>
            <div class="chips" role="group" aria-label="Escalation for {o.name}">
              {#each [true, false] as yes (yes)}
                <button
                  class="chip"
                  aria-pressed={o.escalate === yes}
                  disabled={!editable || !idle}
                  onclick={() => onedit({ kind: "escalate", outcome: o.index, escalate: o.escalate === yes ? null : yes })}
                  >{yes ? ANSWER_LABELS.Y : ANSWER_LABELS.N}</button
                >
              {/each}
            </div>
          </div>
        {/if}
        <p class="overall">Overall</p>
        <AppraisalVerdict
          what="the overall verdict on {o.name}"
          derived={o.derived_overall}
          override={o.override_overall}
          {editable}
          {idle}
          onoverride={(judgement) => onedit({ kind: "overall_override", outcome: o.index, judgement })}
        />
        {#if o.ladder}
          <p>{ladderLine(o.ladder)}</p>
          <p class="number">Your number {o.ladder.weight}</p>
        {:else if o.ladder_error}
          <p class="failed">No ladder: {o.ladder_error}</p>
        {/if}
      </article>
    {/each}
  {/if}

  <div class="actions">
    {#if view.saved_at === null && view.revealed_at === null}
      {#if asking}
        <div class="ask" role="group" aria-label={APPRAISAL_LABELS.revealNow}>
          <p>{APPRAISAL_LABELS.revealWarning}</p>
          <button
            onclick={() => {
              asking = false;
              onreveal();
            }}
            disabled={!idle}>{APPRAISAL_LABELS.revealYes}</button
          >
          <button onclick={() => (asking = false)}>{APPRAISAL_LABELS.revealNo}</button>
        </div>
      {:else}
        <button onclick={reveal} disabled={!idle}>{APPRAISAL_LABELS.revealNow}</button>
      {/if}
    {/if}
    {#if editable && view.saved_at === null && view.version.complete}
      <button class="primary" onclick={onsave} disabled={!idle}>{APPRAISAL_LABELS.save}</button>
    {/if}
  </div>
</section>

<style>
  .head {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    font-family: var(--sans);
  }
  h2 {
    margin: 0;
    font-family: var(--serif);
    font-size: 1.05rem;
  }
  .steps {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 1rem;
    margin: 0.3rem 0 0;
    padding: 0;
    list-style: none;
    font-size: 0.85rem;
  }
  .steps [aria-current="step"] {
    font-weight: 700;
    text-decoration: underline;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .machine,
  .reader {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.75rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
  .machine {
    border-bottom: 1px solid var(--rule);
  }
  .machine:empty {
    display: none;
  }
  p {
    margin: 0;
  }
  h3 {
    margin: 0.4rem 0 0;
    font-family: var(--serif);
    font-size: 1rem;
  }
  .line {
    font-style: italic;
  }
  .actions,
  .chips,
  .ask {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
  }
  .actions:empty {
    display: none;
  }
  .outcome {
    padding-top: 0.4rem;
    border-top: 2px solid var(--rule);
  }
  .result {
    font-family: var(--serif);
  }
  .escalation {
    margin: 0.4rem 0;
  }
  .overall,
  .number,
  .verdict {
    font-weight: 700;
  }
  .note {
    font-size: 0.85rem;
    font-style: italic;
  }
  .failed {
    color: var(--err-ink);
    font-size: 0.85rem;
  }
</style>
