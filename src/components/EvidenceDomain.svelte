<script lang="ts">
  /**
   * One RoB 2 domain in an outcome card, collapsed: its short label and its
   * judgement chip (text, never colour alone; *Not assessed* while pending or
   * failed), with why a failed step failed. Opened, bmlib's title for it, then
   * its questions in bmlib's wording, each answer with its reasoning and
   * citations. D5 opens to its facts first (each with its quotes and the check
   * it passed or failed), the other measurements of the outcome (a failed
   * second look named, as D5 may then be too lenient), and the analysis check,
   * then the answers they gave.
   * `domain`: the domain to show.
   * `outcome`: the outcome's index, or null for the trial's D1.
   * `current`: the citation in focus, or null; `oncite` is called when one is clicked.
   */
  import type { DomainView } from "../lib/api";
  import {
    ANSWER_LABELS,
    CONDENSED_NOTE,
    DOMAIN_LABELS,
    VERDICT_LABELS,
    citeKeys,
    judgementLabel,
    labelOf,
    secondLookNote,
    type EvidenceCite,
  } from "../lib/evidence";
  import EvidenceQuotes from "./EvidenceQuotes.svelte";

  let {
    domain,
    outcome,
    current,
    oncite,
  }: {
    domain: DomainView;
    outcome: number | null;
    current: EvidenceCite | null;
    oncite: (cite: EvidenceCite) => void;
  } = $props();

  let key = $derived(citeKeys.domain(outcome, domain.domain));

  /** A question's first line (the question itself) and the guidance after it. */
  function split(question: string): { head: string; rest: string } {
    const at = question.indexOf("\n");
    return at < 0 ? { head: question, rest: "" } : { head: question.slice(0, at), rest: question.slice(at + 1) };
  }
</script>

<details class="domain">
  <summary>
    <span class="label">{labelOf(DOMAIN_LABELS, domain.domain)}</span>
    <span class="prisma-chip {domain.judgement ?? 'not_assessed'}">{judgementLabel(domain.judgement)}</span>
  </summary>
  <p class="title muted">{domain.name}</p>
  {#if domain.error}<p class="failed">{labelOf(DOMAIN_LABELS, domain.domain)} failed: {domain.error}</p>{/if}
  {#if domain.condensed}<p class="note">{CONDENSED_NOTE}</p>{/if}
  {#if domain.facts}
    <h5>Facts</h5>
    {#each domain.facts as f (f.fact)}
      <div class="entry">
        <p><strong>{f.meaning}</strong> — {labelOf(VERDICT_LABELS, f.verdict)}</p>
        <EvidenceQuotes citations={f.quotes.citations} lost={f.quotes} citeKey={citeKeys.fact(key, f.fact)} {current} {oncite} />
      </div>
    {/each}
  {/if}
  {#each domain.measurements as m, j (j)}
    <div class="entry">
      <p><strong>Other measurement: {m.label}</strong></p>
      {#if m.named.length}
        <p class="note">Where it is named:</p>
        <EvidenceQuotes citations={m.named} lost={null} citeKey={citeKeys.measurement(key, j, "named")} {current} {oncite} />
      {/if}
      {#if m.result.length}
        <p class="note">Where its result is given:</p>
        <EvidenceQuotes citations={m.result} lost={null} citeKey={citeKeys.measurement(key, j, "result")} {current} {oncite} />
      {/if}
      {#if m.second_look_error !== null}
        <p class="failed">{secondLookNote({ label: m.label, error: m.second_look_error })}</p>
      {:else if m.second_look !== null}
        <p class="note">{m.second_look.length ? "A second look found:" : "A second look found nothing more."}</p>
        <EvidenceQuotes citations={m.second_look} lost={null} citeKey={citeKeys.measurement(key, j, "look")} {current} {oncite} />
      {/if}
      <EvidenceQuotes citations={[]} lost={m.quotes} citeKey="" {current} {oncite} />
    </div>
  {/each}
  {#if domain.analysis}
    <div class="entry">
      <p>
        <strong>The analysis reported</strong> —
        {domain.analysis.differs ? "differs from the one planned, with no reason given" : "is the one planned"}
      </p>
      {#if domain.analysis.planned.length}
        <p class="note">The planned analysis:</p>
        <EvidenceQuotes citations={domain.analysis.planned} lost={null} citeKey={citeKeys.analysis(key, "planned")} {current} {oncite} />
      {/if}
      {#if domain.analysis.reported.length}
        <p class="note">The reported analysis:</p>
        <EvidenceQuotes citations={domain.analysis.reported} lost={null} citeKey={citeKeys.analysis(key, "reported")} {current} {oncite} />
      {/if}
      <EvidenceQuotes citations={[]} lost={domain.analysis.quotes} citeKey="" {current} {oncite} />
    </div>
  {/if}
  {#if domain.facts && domain.answers.length}<h5>The answers they gave</h5>{/if}
  {#each domain.answers as a (a.question_id)}
    {@const q = split(a.question)}
    <div class="entry">
      <p><strong>{a.question_id} {labelOf(ANSWER_LABELS, a.answer)}</strong> {q.head}</p>
      {#if q.rest}
        <details class="guidance">
          <summary>How it is answered</summary>
          <p class="muted">{q.rest}</p>
        </details>
      {/if}
      {#if a.reasoning}<p class="explanation">{a.reasoning}</p>{/if}
      <EvidenceQuotes citations={a.quotes.citations} lost={a.quotes} citeKey={citeKeys.answer(key, a.question_id)} {current} {oncite} />
    </div>
  {/each}
  {#if domain.ignored.length}
    <p class="note">Answered although they did not apply: {domain.ignored.join(", ")}</p>
  {/if}
</details>

<style>
  .domain summary {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    padding: 0.2rem 0;
    cursor: pointer;
  }
  .label {
    flex: 1;
  }
  .domain[open] {
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--rule);
  }
  .domain > :not(summary) {
    margin-left: 1rem;
  }
  p {
    margin: 0.3rem 0 0;
  }
  h5 {
    margin: 0.5rem 0 0 1rem;
    font-size: 0.75rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .title {
    font-size: 0.8rem;
  }
  .entry {
    padding-top: 0.3rem;
  }
  .guidance {
    font-size: 0.8rem;
  }
  .guidance p {
    white-space: pre-line;
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
