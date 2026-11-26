<script lang="ts">
  /**
   * One domain of the reader's appraisal (design 2026-11-26, §5): its short
   * label with the verdict as the reader leaves it (or how many questions that
   * apply are answered), bmlib's title, then each signalling question. A
   * question that applies has bmlib's headline, the rest of its wording one
   * click away, a chip per answer it allows (pressed again clears it) and the
   * sources ticks; one that does not apply is greyed, its stored answer struck
   * through; a stored answer to a question these rules no longer have is
   * shown, with no chips. Then the rules' verdict with the reader's own over
   * it, and the reader's note, sent when it loses focus. Each change is one
   * edit.
   * `domain`: the domain to show.
   * `outcome`: the confirmed outcome's index, null for D1 (the trial's).
   * `editable`: whether the reader may change it; `idle`: nothing loads.
   * `onedit`: called with each edit.
   */
  import type { AppraisalEdit, QuestionView, ReaderDomainView } from "../lib/api";
  import { APPRAISAL_LABELS, SOURCES, UNKNOWN_QUESTION, answerChoices, domainProgress } from "../lib/appraisal";
  import { ANSWER_LABELS, DOMAIN_LABELS, judgementLabel, labelOf } from "../lib/evidence";
  import AppraisalVerdict from "./AppraisalVerdict.svelte";

  let {
    domain,
    outcome,
    editable,
    idle,
    onedit,
  }: {
    domain: ReaderDomainView;
    outcome: number | null;
    editable: boolean;
    idle: boolean;
    onedit: (edit: AppraisalEdit) => void;
  } = $props();

  let progress = $derived(domainProgress(domain));
  let label = $derived(labelOf(DOMAIN_LABELS, domain.domain));

  /** Where an edit of question `q` goes. */
  const at = (q: QuestionView) => ({ domain: domain.domain, outcome, question_id: q.question_id });

  /** Question `q`'s sources ticks with `field` set to `checked`. */
  function tick(q: QuestionView, field: (typeof SOURCES)[number]["field"], checked: boolean) {
    const ticks = { protocol: q.protocol, registry: q.registry, supplement: q.supplement, [field]: checked };
    onedit({ kind: "sources", ...at(q), ...ticks });
  }

  /** Sends the note as it left the box; an empty one clears it, an unchanged one sends nothing. */
  function note(text: string) {
    const next = text.trim() === "" ? null : text;
    if (next !== domain.note) onedit({ kind: "note", domain: domain.domain, outcome, note: next });
  }
</script>

<details class="domain" open>
  <summary>
    <span class="label">{label}</span>
    {#if domain.verdict !== null}
      <span class="prisma-chip {domain.verdict}">{judgementLabel(domain.verdict)}</span>
    {:else}
      <span class="count">{progress.answered} of {progress.applicable} answered</span>
    {/if}
  </summary>
  <p class="title">{domain.name}</p>
  <ol class="questions">
    {#each domain.questions as q (q.question_id)}
      <li data-question={q.question_id} class:na={!q.known || !q.applicable}>
        {#if !q.known}
          <p><span class="qid">{q.question_id}</span> {UNKNOWN_QUESTION}</p>
          {#if q.answer !== null}<p class="stored">Its answer: {labelOf(ANSWER_LABELS, q.answer)}</p>{/if}
        {:else}
          <p><span class="qid">{q.question_id}</span> {q.headline}</p>
          {#if q.details}
            <details class="more">
              <summary>Where to look, and what each answer means</summary>
              <p class="details">{q.details}</p>
            </details>
          {/if}
          {#if q.applicable}
            <div class="chips" role="group" aria-label="Answer to {q.question_id}">
              {#each answerChoices(q) as c (c.answer)}
                <button
                  class="chip"
                  aria-pressed={q.answer === c.answer}
                  disabled={!editable || !idle}
                  onclick={() => onedit({ kind: "answer", ...at(q), answer: q.answer === c.answer ? null : c.answer })}
                  >{c.label}</button
                >
              {/each}
            </div>
            <p class="sources">
              {APPRAISAL_LABELS.sources}
              {#each SOURCES as s (s.field)}
                <label
                  ><input
                    type="checkbox"
                    checked={q[s.field]}
                    disabled={!editable || !idle}
                    onchange={(e) => tick(q, s.field, e.currentTarget.checked)}
                  />
                  {s.label}</label
                >
              {/each}
            </p>
          {:else}
            <p class="stored">
              {ANSWER_LABELS.NA}{#if q.answer !== null}; your answer, not counted: <s>{labelOf(ANSWER_LABELS, q.answer)}</s>{/if}
            </p>
          {/if}
        {/if}
      </li>
    {/each}
  </ol>
  <AppraisalVerdict
    what={label}
    derived={domain.derived}
    override={domain.override}
    {editable}
    {idle}
    onoverride={(judgement) => onedit({ kind: "override", domain: domain.domain, outcome, judgement })}
  />
  {#if editable}
    <label class="note"
      >Your note <textarea
        rows="2"
        value={domain.note ?? ""}
        disabled={!idle}
        onblur={(e) => note(e.currentTarget.value)}
      ></textarea></label
    >
  {:else if domain.note}
    <p class="stored">Your note: {domain.note}</p>
  {/if}
</details>

<style>
  .domain {
    padding: 0.3rem 0;
    border-top: 1px solid var(--rule);
  }
  .domain > summary {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    cursor: pointer;
    font-weight: 700;
  }
  .label {
    flex: 1;
  }
  .count {
    font-size: 0.8rem;
    font-weight: normal;
  }
  .title {
    margin: 0.2rem 0;
    font-size: 0.8rem;
    font-style: italic;
  }
  .questions {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .questions > li {
    padding: 0.35rem 0;
  }
  .questions > li.na {
    opacity: 0.6;
  }
  p {
    margin: 0.15rem 0;
  }
  .qid {
    font-weight: 700;
  }
  .more {
    font-size: 0.8rem;
  }
  .details {
    white-space: pre-line;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin: 0.25rem 0;
  }
  .sources {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.75rem;
    font-size: 0.8rem;
  }
  .sources label {
    display: inline;
    font-weight: normal;
    margin: 0;
  }
  .stored {
    font-size: 0.85rem;
  }
  .note {
    margin-top: 0.3rem;
  }
</style>
