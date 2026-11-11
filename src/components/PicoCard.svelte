<script lang="ts">
  /**
   * One PICO run as a card. An extraction shows the four parts in order, each
   * with its description, what backs it (`[n]` buttons with the paper's wording),
   * the quotes not found (listed, never dropped), those the paper contradicts
   * with why, and those not usable as text;
   * then study type and sample size, marked as the model's. A run whose check
   * found the paper unsuitable says so plainly, with what the check read, the
   * study type (marked as the model's) and the model's rationale; an extraction
   * after a check that read the abstract says so. The check's findings and
   * confidence, each part's confidence and the run's mean confidence are
   * shown too, as the model gave them. Every run ends with its
   * caption and every note bmlib made on how it went.
   * `run`: the run to show.
   * `latestCanonicalId`: the paper's latest text, for the caption.
   * `current`: the number of the citation in focus, or null; `oncite` is called
   *   with a citation's number when it is clicked.
   */
  import type { PicoRunView } from "../lib/api";
  import { contradictionNote } from "../lib/chat";
  import {
    PART_LABELS,
    checkNote,
    findingsNote,
    partNote,
    percent,
    quotesDroppedNote,
    runCaption,
    runNotes,
    sampleSizeNote,
  } from "../lib/pico";

  let {
    run,
    latestCanonicalId,
    current,
    oncite,
  }: {
    run: PicoRunView;
    latestCanonicalId: number;
    current: number | null;
    oncite: (number: number) => void;
  } = $props();
</script>

<div class="card">
  {#if run.extraction}
    {#each run.extraction.parts as p (p.part)}
      <section class="part">
        <h3>
          {PART_LABELS[p.part]}
          <span class="muted confidence">{percent(p.confidence) ?? "confidence not given"}</span>
        </h3>
        <p class="description">{p.description}</p>
        {#if partNote(p)}<p class="note">{partNote(p)}</p>{/if}
        {#if p.citations.length}
          <ul class="citations">
            {#each p.citations as c (c.number)}
              <li>
                <button
                  class="cite"
                  class:current={current === c.number}
                  aria-label={`Citation ${c.number}: ${c.text}`}
                  onclick={() => oncite(c.number)}>{c.number}</button
                >
                <span class="wording">{c.text}</span>
              </li>
            {/each}
          </ul>
        {/if}
        {#if p.quotes_not_found.length}
          <p class="note">Not found in the paper:</p>
          <ul class="unlocated muted">
            {#each p.quotes_not_found as q}<li>{q}</li>{/each}
          </ul>
        {/if}
        {#if p.quotes_contradicted.length}
          <p class="note">Contradicted by the paper:</p>
          <ul class="unlocated muted">
            {#each p.quotes_contradicted as q}<li>{q.quote} <span class="note">({contradictionNote(q.contradiction)})</span></li>{/each}
          </ul>
        {/if}
        {#if quotesDroppedNote(p)}<p class="note">{quotesDroppedNote(p)}</p>{/if}
      </section>
    {/each}
    <section class="part">
      <dl>
        <dt>Study type</dt>
        <dd>{run.extraction.study_type ?? "Not given"}</dd>
        <dt>Sample size</dt>
        <dd>{run.extraction.sample_size ?? "Not given"}</dd>
        <dt>Mean confidence</dt>
        <dd>{percent(run.extraction.overall_confidence) ?? "Not given"}</dd>
      </dl>
      <p class="note">{sampleSizeNote()}</p>
    </section>
  {:else if run.suitability}
    <section class="part">
      <p><strong>This paper does not look like an intervention study, so it may have no PICO.</strong></p>
      {#if checkNote(run.suitability)}<p class="note">{checkNote(run.suitability)}</p>{/if}
      <p class="note">{findingsNote(run.suitability)}</p>
      {#if run.suitability.study_type}
        <p>Study type: {run.suitability.study_type}</p>
        <p class="note">{sampleSizeNote()}</p>
      {/if}
      {#if run.suitability.rationale}<p class="description">{run.suitability.rationale}</p>{/if}
    </section>
  {/if}
  {#if run.extraction && run.suitability && checkNote(run.suitability)}
    <p class="note">Its suitability was {checkNote(run.suitability)?.toLowerCase()}.</p>
  {/if}
  {#if run.extraction && run.suitability}<p class="note">{findingsNote(run.suitability)}</p>{/if}
  {#each runNotes(run) as n}<p class="note">{n}</p>{/each}
  <p class="caption muted">{runCaption(run, latestCanonicalId)}</p>
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    font-family: var(--sans);
    font-size: 0.9rem;
  }
  .part {
    padding: 0.5rem 0.7rem;
    border: 1px solid var(--rule);
    border-radius: 10px;
  }
  h3 {
    margin: 0;
    font-size: 0.95rem;
  }
  .confidence {
    font-weight: normal;
    font-size: 0.8rem;
  }
  .description {
    margin: 0.3rem 0 0;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.2rem 0.75rem;
    margin: 0;
  }
  dt {
    font-weight: 700;
  }
  dd {
    margin: 0;
  }
  .cite {
    font: 700 0.7rem var(--sans);
    padding: 0 0.35rem;
    margin: 0 0.1rem;
    border: none;
    border-radius: 4px;
    background: var(--hl-past);
    color: var(--hl-ink);
    vertical-align: 0.1em;
  }
  .cite.current {
    background: var(--hl-current);
  }
  .citations,
  .unlocated {
    margin: 0.4rem 0 0;
    padding: 0;
    list-style: none;
    font-size: 0.85rem;
  }
  .citations li {
    margin-top: 0.25rem;
  }
  .wording {
    font-family: var(--serif);
  }
  .unlocated li {
    margin-top: 0.2rem;
    padding-left: 0.5rem;
    border-left: 2px solid var(--rule);
  }
  .note {
    margin: 0.4rem 0 0;
    font-size: 0.85rem;
    font-style: italic;
  }
  .caption {
    margin: 0;
    font-size: 0.75rem;
  }
</style>
