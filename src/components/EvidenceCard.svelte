<script lang="ts">
  /**
   * One primary outcome's card in the Evidence weight tab: its name and
   * timepoint with its overall RoB 2 chip; its reported result with the
   * passages naming it (or why the result was refused); the overall judgement
   * with the ladder's level and why the model did or did not escalate; the
   * ladder line; the outcome's number; then the domains, D1 (the trial's, the
   * same in every card) to D5, collapsed. A failure is never a score: an
   * outcome not judged reads *Not assessed* and has no number. A failed second
   * look of D5 is named beside the number, or under the name when there is no
   * number, so a number never stands without it.
   * `outcome`: the outcome to show.
   * `study`: the trial's D1, or null when it has none yet.
   * `condensed`: whether the run was judged from a condensed paper, said beside the number.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   */
  import type { DomainView, OutcomeView } from "../lib/api";
  import {
    CONDENSED_NOTE,
    LEVEL_LABELS,
    citeKeys,
    judgementLabel,
    labelOf,
    ladderLine,
    resultLine,
    secondLookNote,
    type EvidenceCite,
  } from "../lib/evidence";
  import EvidenceDomain from "./EvidenceDomain.svelte";
  import EvidenceQuotes from "./EvidenceQuotes.svelte";

  let {
    outcome,
    study,
    condensed,
    current,
    oncite,
  }: {
    outcome: OutcomeView;
    study: DomainView | null;
    condensed: boolean;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
  } = $props();

  let looks = $derived(outcome.second_look_failures.map(secondLookNote));
</script>

<details class="outcome" open>
  <summary>
    <span class="name">{outcome.name}{outcome.timepoint ? ` at ${outcome.timepoint}` : ""}</span>
    <span class="prisma-chip {outcome.overall ?? 'not_assessed'}">{judgementLabel(outcome.overall)}</span>
  </summary>
  {#if !outcome.ladder}
    {#each looks as n}<p class="failed">{n}</p>{/each}
  {/if}
  {#if outcome.result}
    <p class="result">{resultLine(outcome.result)}</p>
  {:else if outcome.result_error}
    <p class="failed">No result could be read: {outcome.result_error}</p>
  {/if}
  <EvidenceQuotes
    citations={outcome.quotes.citations}
    lost={outcome.quotes}
    citeKey={citeKeys.outcome(outcome.index)}
    {current}
    {oncite}
  />
  <p class="line">
    RoB 2: {judgementLabel(outcome.overall)}{#if outcome.ladder}<span class="gap">Ladder: {labelOf(LEVEL_LABELS, outcome.ladder.level)}</span>{/if}
  </p>
  {#if outcome.escalation}
    <p class="note">
      {outcome.escalation.escalate
        ? "The concerns add up to a high risk of bias"
        : "The concerns do not add up to a high risk of bias"}: {outcome.escalation.reasoning}
    </p>
  {/if}
  {#if outcome.escalation_error}<p class="failed">The overall judgement failed: {outcome.escalation_error}</p>{/if}
  {#if outcome.ladder}
    <p>{ladderLine(outcome.ladder)}</p>
    <p class="number">Number {outcome.ladder.weight}</p>
    {#if condensed}<p class="note">{CONDENSED_NOTE}</p>{/if}
    {#each looks as n}<p class="failed">{n}</p>{/each}
  {/if}
  {#if outcome.ladder_error}<p class="failed">No ladder: {outcome.ladder_error}</p>{/if}
  <div class="domains">
    {#if study}<EvidenceDomain domain={study} outcome={null} {current} {oncite} />{/if}
    {#each outcome.domains as d (d.domain)}
      <EvidenceDomain domain={d} outcome={outcome.index} {current} {oncite} />
    {/each}
  </div>
</details>

<style>
  .outcome {
    padding: 0.4rem 0 0.5rem;
    border-top: 1px solid var(--rule);
  }
  .outcome > summary {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    padding: 0.2rem 0;
    font-weight: 700;
    cursor: pointer;
  }
  .name {
    flex: 1;
    font-family: var(--serif);
    font-size: 1rem;
  }
  p {
    margin: 0.3rem 0 0;
  }
  .result {
    font-family: var(--serif);
  }
  .gap {
    margin-left: 1.5rem;
  }
  .number {
    font-weight: 700;
  }
  .domains {
    margin-top: 0.4rem;
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
