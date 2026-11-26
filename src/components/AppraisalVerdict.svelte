<script lang="ts">
  /**
   * A verdict of the reader's (a domain's or an outcome's overall): the one
   * RoB 2's rules give their answers, then their own over it as chips (Low,
   * Some concerns, High; the one pressed again clears it). The chips show once
   * the rules give a verdict, since an override is over it. Text, never colour
   * alone.
   * `what`: what the verdict is of, for the chips' group name.
   * `derived`: the rules' verdict, null until every question that applies is answered.
   * `override`: the reader's own, or null.
   * `editable`: whether the reader may change it; `idle`: nothing loads.
   * `onoverride`: called with the judgement picked, or null to clear it.
   */
  import { JUDGEMENT_LABELS, judgementLabel } from "../lib/evidence";

  let {
    what,
    derived,
    override,
    editable,
    idle,
    onoverride,
  }: {
    what: string;
    derived: string | null;
    override: string | null;
    editable: boolean;
    idle: boolean;
    onoverride: (judgement: string | null) => void;
  } = $props();
</script>

{#if derived !== null}
  <div class="verdict">
    <span>By the rules: <span class="prisma-chip {derived}">{judgementLabel(derived)}</span></span>
    {#if editable}
      <span class="chips" role="group" aria-label="Your verdict on {what}">
        Your verdict:
        {#each Object.keys(JUDGEMENT_LABELS) as j (j)}
          <button
            class="chip"
            aria-pressed={override === j}
            disabled={!idle}
            onclick={() => onoverride(override === j ? null : j)}>{judgementLabel(j)}</button
          >
        {/each}
      </span>
    {:else if override !== null}
      <span>Your verdict: <span class="prisma-chip {override}">{judgementLabel(override)}</span></span>
    {/if}
  </div>
{/if}

<style>
  .verdict {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
    align-items: center;
    margin: 0.4rem 0;
    font-size: 0.85rem;
  }
  .chips {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    align-items: center;
  }
</style>
