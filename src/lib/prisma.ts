// What the PRISMA 2020 tab shows and when it runs: labels, notes, counts, marks and captions.
// The lifecycle helpers PICO shares (auto-run, first run, caption, dropped quotes) are PICO's own.
import type {
  KeyGapsView,
  PRISMA_JUDGEMENTS,
  PrismaEvent,
  PrismaItemView,
  PrismaRunView,
  PrismaSectionView,
  PrismaView,
  Utf16Span,
} from "./api";
import { quotesDroppedNote, runningNote as picoRunningNote } from "./pico";

export { offerRun, shouldAutoRun } from "./pico";

/** One judgement string, as bmlib's `Judgement` serialises it. */
export type PrismaJudgement = (typeof PRISMA_JUDGEMENTS)[number];

/** Each judgement's label on its chip. */
export const JUDGEMENT_LABELS: Record<PrismaJudgement, string> = {
  reported: "Reported",
  partly_reported: "Partly reported",
  not_reported: "Not reported",
  reported_elsewhere: "Reported elsewhere, not checked",
  not_applicable: "Not applicable",
};

/** The chip of an item no part has assessed yet. */
export const NOT_ASSESSED_LABEL = "Not assessed";

/** A judgement's label; null is *Not assessed*, and one this version does not know shows as Rust sent it. */
export function judgementLabel(judgement: string | null): string {
  if (judgement === null) return NOT_ASSESSED_LABEL;
  return JUDGEMENT_LABELS[judgement as PrismaJudgement] ?? judgement;
}

/** Each checklist section's heading, by its wire string (`ChecklistSection::as_str`). */
export const SECTION_HEADINGS: Record<string, string> = {
  title: "Title",
  abstract: "Abstract",
  introduction: "Introduction",
  methods: "Methods",
  results: "Results",
  discussion: "Discussion",
  other_information: "Other information",
};

/** A section's heading; one this version does not know shows as Rust sent it. */
export function sectionHeading(section: string): string {
  return SECTION_HEADINGS[section] ?? section;
}

/** The sections open when a run is first shown (design §8). */
export const OPEN_SECTIONS: readonly string[] = ["methods", "results"];

/** Each count's word in a section line, in the line's order; `not_assessed` last. */
const COUNT_WORDS: Array<[string, string]> = [
  ["reported", "reported"],
  ["partly_reported", "partly"],
  ["not_reported", "not"],
  ["reported_elsewhere", "elsewhere"],
  ["not_applicable", "n/a"],
  ["not_assessed", "not assessed"],
];

/**
 * A section's heading with its counts, zeros left out, for example
 * *Methods: 9 reported · 4 partly · 2 not · 1 elsewhere · 1 n/a*. Items not
 * assessed yet are counted too, so an unfinished run never reads as finished.
 */
export function sectionLine(s: PrismaSectionView): string {
  const counts = COUNT_WORDS.filter(([key]) => (s.counts[key] ?? 0) > 0).map(
    ([key, word]) => `${s.counts[key]} ${word}`,
  );
  return `${sectionHeading(s.section)}: ${counts.join(" · ")}`;
}

/** The note on a claim shown as not reported for want of a located quote. */
const UNBACKED_NOTE = "Not reported: the model's claim could not be checked against the paper";
/** The note on an item judged from a condensed paper. */
const CONDENSED_NOTE = "Judged from a condensed paper";
/** The note on a key item checked again on a condensation of its own (§6.5). */
const RECHECKED_NOTE = "Checked again on a condensation for this item";

/**
 * Everything to know about how an item was judged, joined by " · ": its claim
 * unbacked, judged from a condensation (or re-checked on one of its own, which
 * says so in place of it), and quotes that were not usable text. Null when
 * none applies.
 */
export function itemNote(item: PrismaItemView): string | null {
  const notes: string[] = [];
  if (item.unbacked) notes.push(UNBACKED_NOTE);
  if (item.rechecked) notes.push(RECHECKED_NOTE);
  else if (item.condensed) notes.push(CONDENSED_NOTE);
  const dropped = quotesDroppedNote(item);
  if (dropped !== null) notes.push(dropped);
  return notes.length ? notes.join(" · ") : null;
}

/**
 * The lines under the key-gaps panel: how many key items are gaps (a count,
 * never a score), and how many rely on what we cannot read when any do. An
 * incomplete run says *Not complete* in place of both, as its counts are partial.
 */
export function keyGapsLine(k: KeyGapsView): string[] {
  if (!k.complete) return ["Not complete"];
  const lines = [`${k.gaps} of ${k.total} key items are not or only partly reported`];
  if (k.elsewhere > 0) {
    const rely = k.elsewhere === 1 ? "1 key item relies" : `${k.elsewhere} key items rely`;
    lines.push(`${rely} on material we cannot read (supplements, registries, figures)`);
  }
  return lines;
}

/** The run's key items in the panel's order (`key_gaps.items`); an id the run lacks is left out. */
export function keyItems(run: PrismaRunView): PrismaItemView[] {
  return run.key_gaps.items.flatMap((id) => run.items.filter((i) => i.id === id));
}

/**
 * What the tab says while a run goes: the part begun, how far a condensation
 * has got (PICO's words), a key item checked again, or that the stop was
 * pressed. `event` is null before any arrives.
 */
export function runningNote(event: PrismaEvent | null, stopping: boolean): string {
  if (stopping) return "Stopping…";
  if (event === null) return "Assessing PRISMA 2020… A review is assessed part by part and can take several minutes.";
  if (event.kind === "part") return `Part ${event.index} of ${event.of}: ${event.label}`;
  if (event.kind === "rechecking") return `Checking key item ${event.item} again on a condensation of its own…`;
  return picoRunningNote(event, false);
}

/** A PRISMA citation in focus: its item's id and its place among the item's citations. */
export type PrismaCite = { item: string; index: number };

/**
 * The run's citations as marks on the paper. With one in focus, its item's
 * citations are marked, that one current (design §8: the current one dark, the
 * item's others pale); with none, every citation of the run, none current.
 */
export function prismaMarks(
  run: PrismaRunView,
  current: PrismaCite | null,
): Array<{ span: Utf16Span; current: boolean }> {
  const items = current === null ? run.items : run.items.filter((i) => i.id === current.item);
  return items.flatMap((i) =>
    i.citations.map((c, index) => ({ span: c.span, current: current !== null && index === current.index })),
  );
}

/** The extension that fits a variant better, by bmlib's variant token. */
const VARIANT_NOTES: Record<string, string> = {
  scoping_review: "A scoping review: PRISMA-ScR fits it better",
  network_meta_analysis: "A network meta-analysis: PRISMA-NMA fits it better",
};

/**
 * The line naming a specialised extension for the review's variant (design
 * §3), or null for a variant with none (umbrella, rapid, living) or none at
 * all. The variant is the model's own string, so case, spaces and hyphens are
 * read as bmlib's `scoping_review` / `network_meta_analysis` tokens.
 */
export function variantNote(variant: string | null): string | null {
  if (variant === null) return null;
  return VARIANT_NOTES[variant.trim().toLowerCase().replace(/[\s-]+/g, "_")] ?? null;
}

/** Whether *Run anyway* is offered: the run's check found the paper not suitable. */
export function offerRunAnyway(run: PrismaRunView | null): boolean {
  return run?.suitability?.is_suitable === false;
}

/** Whether *Continue* is offered: the current run is incomplete and no run goes. */
export function continueOffered(view: PrismaView, running: boolean): boolean {
  return view.current?.status === "incomplete" && !running;
}

/**
 * *Continue*'s label: plain when the run's model is the one chosen, else naming
 * the run's model, since Continue always uses it (spec §7).
 */
export function continueCaption(run: PrismaRunView, chosenModel: string | null): string {
  return run.model === chosenModel ? "Continue" : `Continue with ${run.model}`;
}

/** The hint beside *Continue* when another model is chosen: *Re-run* uses that one. Null otherwise. */
export function continueHint(run: PrismaRunView, chosenModel: string | null): string | null {
  return chosenModel !== null && run.model !== chosenModel ? `Re-run uses ${chosenModel}` : null;
}
