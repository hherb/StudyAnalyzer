<script lang="ts">
  /**
   * Steps 1 and 2 of the reader's appraisal (design 2026-11-26, §4): the
   * machine's proposed design with its quotes, which the reader keeps or
   * replaces from bmlib's taxonomy; then each proposed primary outcome with its
   * result and quotes, which the reader keeps, edits (name, time point,
   * result) or removes, and any outcome the machine missed, added. *Confirm*
   * checks the draft first (`confirmProblems`) and says what stops it; the
   * core checks again. Confirming is final for the appraisal.
   * `proposals`: the machine's proposals; the draft starts from them once, on mount.
   * `idle`: nothing loads, so *Confirm* may be pressed.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   * `onconfirm`: called with what the reader confirms.
   */
  import type { ConfirmInput, ProposalsView } from "../lib/api";
  import {
    APPRAISAL_LABELS,
    SCALE_LABELS,
    confirmDraft,
    confirmInput,
    confirmProblems,
    designChoices,
    outcomeDraft,
  } from "../lib/appraisal";
  import { NO_PRIMARY_OUTCOME, citeKeys, designLine, resultLine, type EvidenceCite } from "../lib/evidence";
  import EvidenceQuotes from "./EvidenceQuotes.svelte";

  let {
    proposals,
    idle,
    current,
    oncite,
    onconfirm,
  }: {
    proposals: ProposalsView;
    idle: boolean;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
    onconfirm: (input: ConfirmInput) => void;
  } = $props();

  /** The draft as it starts: the proposals, read once (later views of the same proposals change nothing). */
  const start = () => confirmDraft(proposals);
  let draft = $state(start());
  /** Whether *Confirm* was pressed, so the problems show only then. */
  let tried = $state(false);
  let problems = $derived(confirmProblems(draft));

  const choices = designChoices();
  /** A design's key in the picker. */
  const key = (d: { design: string; rct_layout: string | null }) => `${d.design}:${d.rct_layout ?? ""}`;

  /** Takes the design picked under key `k`. */
  function pick(k: string) {
    const c = choices.find((x) => key(x) === k);
    if (c) [draft.design, draft.rct_layout] = [c.design, c.rct_layout];
  }

  /** Sends the draft once nothing stops it. */
  function confirm() {
    tried = true;
    if (problems.length === 0) onconfirm(confirmInput(draft));
  }
</script>

<section class="step">
  <h3>{APPRAISAL_LABELS.steps[0]}</h3>
  <p>The machine proposes: <strong>{designLine(proposals.design.design, proposals.design.rct_layout)}</strong></p>
  <EvidenceQuotes
    citations={proposals.design.quotes.citations}
    lost={proposals.design.quotes}
    citeKey={citeKeys.design}
    {current}
    {oncite}
  />
  {#if proposals.design.reasoning}<p class="note">{proposals.design.reasoning}</p>{/if}
  <label
    >The design
    <select value={key(draft)} onchange={(e) => pick(e.currentTarget.value)}>
      {#each choices as c (key(c))}<option value={key(c)}>{c.label}</option>{/each}
    </select></label
  >
</section>

<section class="step">
  <h3>{APPRAISAL_LABELS.steps[1]}</h3>
  {#if proposals.outcomes.length === 0}<p class="note">{NO_PRIMARY_OUTCOME}</p>{/if}
  {#each draft.outcomes as o, i (o)}
    {@const proposed = o.from_proposed === null ? null : proposals.outcomes[o.from_proposed]}
    <fieldset>
      <legend>Outcome {i + 1}{proposed ? "" : " (added)"}</legend>
      {#if proposed}
        <p class="note">
          The machine proposes: {proposed.name}{proposed.timepoint ? ` at ${proposed.timepoint}` : ""}{proposed.result
            ? `, ${resultLine(proposed.result)}`
            : ""}
        </p>
        {#if proposed.result_error}<p class="failed">No result could be read: {proposed.result_error}</p>{/if}
        <EvidenceQuotes
          citations={proposed.quotes.citations}
          lost={proposed.quotes}
          citeKey={citeKeys.outcome(o.from_proposed ?? 0)}
          {current}
          {oncite}
        />
      {/if}
      <div class="fields">
        <label>Name <input bind:value={o.name} /></label>
        <label>Time point <input bind:value={o.timepoint} /></label>
        <label>Measure <input bind:value={o.result.measure} /></label>
        <label
          >Scale
          <select bind:value={o.result.scale}>
            {#each Object.entries(SCALE_LABELS) as [value, text] (value)}<option {value}>{text}</option>{/each}
          </select></label
        >
        <label>Estimate <input inputmode="decimal" bind:value={o.result.estimate} /></label>
        <label>Interval from <input inputmode="decimal" bind:value={o.result.ci_low} /></label>
        <label>Interval to <input inputmode="decimal" bind:value={o.result.ci_high} /></label>
        <label>Interval level (%) <input inputmode="decimal" bind:value={o.result.ci_level} /></label>
        <label>Number analysed <input inputmode="numeric" bind:value={o.result.n_analysed} /></label>
      </div>
      <button onclick={() => draft.outcomes.splice(i, 1)}>Remove this outcome</button>
    </fieldset>
  {/each}
  <button onclick={() => draft.outcomes.push(outcomeDraft(null, null))}>Add an outcome the machine missed</button>
</section>

{#if tried && problems.length}
  <ul class="problems" role="alert">
    {#each problems as p (p)}<li>{p}</li>{/each}
  </ul>
{/if}
<button class="primary" onclick={confirm} disabled={!idle}>Confirm the design and outcomes</button>
<p class="note">Confirming is final for this appraisal: the machine and you both assess these outcomes.</p>

<style>
  .step {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  h3 {
    margin: 0.5rem 0 0;
    font-family: var(--serif);
    font-size: 1rem;
  }
  p {
    margin: 0;
  }
  fieldset {
    border: 1px solid var(--rule);
    padding: 0.5rem;
  }
  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
    gap: 0.4rem 0.75rem;
    margin: 0.4rem 0;
  }
  .fields input,
  .fields select {
    width: 100%;
    box-sizing: border-box;
  }
  .note {
    font-size: 0.85rem;
    font-style: italic;
  }
  .failed,
  .problems {
    color: var(--err-ink);
    font-size: 0.85rem;
  }
</style>
