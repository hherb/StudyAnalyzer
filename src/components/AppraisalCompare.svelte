<script lang="ts">
  /**
   * Step 4, *Compare* (design 2026-11-26, §6–§7), shown only once the
   * appraisal is revealed: per confirmed outcome, each signalling question
   * with the reader's answer and the machine's side by side, the differences
   * marked in words (*Differs*), a side with no answer said as such, never a
   * difference; the reader's marked sentence one click from the paper
   * (outlined there). Then each domain's verdicts, the reader's as they left
   * it beside the machine's (*Not assessed*
   * for a domain it has not done), each departure from the rules under its
   * verdict, once (*Not checked (earlier rules).* for a rater under earlier
   * rules), and the machine's reasoning and quotes on demand through plan 2's
   * `EvidenceDomain` (D5's facts with their checks). Then both overall
   * verdicts, both escalation answers, both ladders and both numbers.
   * `view`: the revealed appraisal, its `compare` set.
   * `machine`: the paired run's own view (`view.machine.revealed`).
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   */
  import type { AppraisalView, DepartureView, EvidenceRunView, ReaderDomainView } from "../lib/api";
  import {
    APPRAISAL_LABELS,
    DIFFERS,
    compareRows,
    departureLine,
    markKey,
  } from "../lib/appraisal";
  import { ANSWER_LABELS, DOMAIN_LABELS, judgementLabel, labelOf, ladderLine, type EvidenceCite } from "../lib/evidence";
  import EvidenceDomain from "./EvidenceDomain.svelte";

  let {
    view,
    machine,
    current,
    oncite,
  }: {
    view: AppraisalView;
    machine: EvidenceRunView;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
  } = $props();

  /** The machine's departures on domain `domain` (null: the overall verdict) of outcome `outcome` (null: D1). */
  const machineDepartures = (domain: string | null, outcome: number | null): DepartureView[] =>
    view.machine.departures.filter((d) => d.domain === domain && d.outcome === outcome);

  /** An escalation answer in words; null when it was not asked or answered. */
  const yesNo = (b: boolean | null | undefined) => (b === null || b === undefined ? null : b ? ANSWER_LABELS.Y : ANSWER_LABELS.N);
</script>

{#snippet departures(reader: DepartureView | null, domain: string | null, outcome: number | null)}
  {#if view.version.earlier_rules}
    <p class="note">You: {APPRAISAL_LABELS.notChecked}</p>
  {:else if reader}
    <p class="note">{departureLine(reader, "reader")}</p>
  {/if}
  {#if view.machine.earlier_rules}
    <p class="note">The machine: {APPRAISAL_LABELS.notChecked}</p>
  {:else}
    {#each machineDepartures(domain, outcome) as d}<p class="failed">{departureLine(d, "machine")}</p>{/each}
  {/if}
{/snippet}

{#snippet domainVerdicts(d: ReaderDomainView, outcome: number | null)}
  {@const m =
    outcome === null
      ? machine.study
      : (machine.outcomes.find((x) => x.index === outcome)?.domains.find((x) => x.domain === d.domain) ?? null)}
  <div class="domain">
    <p class="verdicts">
      <strong>{labelOf(DOMAIN_LABELS, d.domain)}</strong>
      <span
        >You: <span class="prisma-chip {d.verdict ?? 'not_assessed'}">{judgementLabel(d.verdict)}</span></span
      >
      <span
        >The machine: <span class="prisma-chip {m?.judgement ?? 'not_assessed'}">{judgementLabel(m?.judgement ?? null)}</span
        ></span
      >
    </p>
    {@render departures(d.departure, d.domain, outcome)}
    {#if m}<EvidenceDomain domain={m} {outcome} {current} {oncite} />{/if}
  </div>
{/snippet}

<section class="compare" aria-label="Compare">
  <h3>{APPRAISAL_LABELS.steps[3]}</h3>
  {@render domainVerdicts(view.version.study, null)}
  {#each view.version.outcomes as o (o.index)}
    {@const mo = machine.outcomes.find((x) => x.index === o.index) ?? null}
    {@const rows = view.compare ? compareRows(view.compare, view.version, o.index) : []}
    <article class="outcome">
      <h4>{o.name}{o.timepoint ? ` at ${o.timepoint}` : ""}</h4>
      <div class="scroll">
        <table>
          <thead><tr><th>Question</th><th>You</th><th>The machine</th><th></th></tr></thead>
          <tbody>
            {#each rows as r (`${r.domain}.${r.question_id}`)}
              <tr class:differs={r.differs}>
                <td>{r.question_id}</td>
                <td>
                  {r.reader}
                  {#if r.mark}
                    {@const key = markKey({ domain: r.domain, outcome: r.domain === "D1" ? null : o.index, question_id: r.question_id })}
                    <button class="link" aria-pressed={current?.key === key} onclick={() => oncite({ key, index: 0 })}
                      >{APPRAISAL_LABELS.showMark}</button
                    >
                  {/if}
                </td>
                <td>{r.machine}</td>
                <td>{#if r.differs}<strong>{DIFFERS}</strong>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#each o.domains as d (d.domain)}
        {@render domainVerdicts(d, o.index)}
      {/each}
      <div class="domain">
        <p class="verdicts">
          <strong>Overall</strong>
          <span
            >You: <span class="prisma-chip {o.overall ?? 'not_assessed'}">{judgementLabel(o.overall)}</span></span
          >
          <span
            >The machine: <span class="prisma-chip {mo?.overall ?? 'not_assessed'}">{judgementLabel(mo?.overall ?? null)}</span
            ></span
          >
        </p>
        {#if (o.needs_escalation && yesNo(o.escalate)) || mo?.escalation}
          <p class="note">{APPRAISAL_LABELS.escalation}</p>
          <p>
            {#if yesNo(o.escalate)}You: {yesNo(o.escalate)}.{/if}
            {#if mo?.escalation}The machine: {yesNo(mo.escalation.escalate)}.{/if}
          </p>
        {/if}
        {@render departures(o.overall_departure, null, o.index)}
      </div>
      {#if o.ladder}
        <p>Your ladder: {ladderLine(o.ladder)}</p>
        <p class="number">Your number {o.ladder.weight}</p>
      {:else if o.ladder_error}
        <p class="failed">No ladder of yours: {o.ladder_error}</p>
      {/if}
      {#if mo?.ladder}
        <p>The machine's ladder: {ladderLine(mo.ladder)}</p>
        <p class="number">The machine's number {mo.ladder.weight}</p>
      {:else if mo?.ladder_error}
        <p class="failed">No ladder of the machine's: {mo.ladder_error}</p>
      {/if}
    </article>
  {/each}
</section>

<style>
  .compare {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  h3,
  h4 {
    margin: 0.4rem 0 0;
    font-family: var(--serif);
  }
  h4 {
    font-size: 0.95rem;
  }
  p {
    margin: 0.15rem 0;
  }
  .outcome {
    padding-top: 0.4rem;
    border-top: 2px solid var(--rule);
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th,
  td {
    padding: 0.15rem 0.5rem;
    border-bottom: 1px solid var(--rule);
    text-align: left;
    vertical-align: top;
  }
  tr.differs {
    background: var(--warn-bg);
  }
  .domain {
    padding: 0.3rem 0;
    border-top: 1px solid var(--rule);
  }
  .verdicts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 1rem;
    align-items: baseline;
  }
  .note {
    font-size: 0.85rem;
    font-style: italic;
  }
  .number {
    font-weight: 700;
  }
  .link {
    border: none;
    padding: 0;
    font-size: 0.8rem;
    text-decoration: underline;
  }
  .failed {
    color: var(--err-ink);
    font-size: 0.85rem;
  }
</style>
