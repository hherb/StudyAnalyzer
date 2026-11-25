// Typed wrappers over the Tauri commands. The types mirror the Rust `Serialize` shapes.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** A half-open range of UTF-16 code units into `PaperView.text`. */
export interface Utf16Span {
  /** The first unit covered. */
  start: number;
  /** One past the last unit covered. */
  end: number;
}
/** The canonical identifiers of a paper; absent ones are null. */
export interface Identifiers {
  /** The PubMed ID (digits, no leading zeros) or a preprint accession (`PPR` + digits). */
  pmid: string | null;
  /** The DOI in lower case, without any `doi:` or URL prefix. */
  doi: string | null;
  /** The PubMed Central ID, always with its `PMC` prefix. */
  pmc_id: string | null;
}
/** One line of the recent-papers list. */
export interface PaperSummary {
  /** The paper's id. */
  id: string;
  /** The paper's title. */
  title: string;
  /** The identifiers known for the paper. */
  identifiers: Identifiers;
  /** When the paper was last opened (stored text, UTC). */
  opened_at: string;
}
/** How a request ended. */
export type OpenOutcome =
  /** The paper has a canonical text and can be read. */
  | {
      status: "ready";
      /** The paper's id. */
      paper_id: string;
      /** What the user should know about this open, one sentence each; usually empty. */
      notices: string[];
    }
  /** The paper is stored but has no full text yet; the user can add a file or paste. */
  | {
      status: "needs_full_text";
      /** The paper's id. */
      paper_id: string;
      /** Why there is no full text, for the user. */
      reason: string;
      /** True only when the sources could not be reached (try again later). */
      unreachable: boolean;
    };
/** Every kind of source a paper's text can come from, mirroring Rust's `SourceKind`. */
export const SOURCE_KINDS = ["pdf", "jats", "html", "markdown", "plain_text"] as const;
/** The kind of source a paper's text came from. */
export type SourceKind = (typeof SOURCE_KINDS)[number];
/** How a text file's bytes were decoded. */
export interface Decoded {
  /** The WHATWG encoding name, e.g. `"windows-1252"`. */
  encoding: string;
  /** True only when the encoding was guessed; false for a BOM, valid UTF-8, or a declaration that decoded cleanly. */
  guessed: boolean;
}
/** The full-text check's outcome. */
export interface FullTextStatus {
  /** Whether the check judged the text to be a whole paper. */
  passed: boolean;
  /** Whether the user confirmed "this is the whole paper". */
  confirmed: boolean;
  /** The words in the body. */
  body_words: number;
  /** The fewest body words the check accepts (bmlib's constant), so the banner can name it. */
  min_body_words: number;
  /** The headings the check requires and did not find, named for the banner; empty when it found them. */
  missing_headings: string[];
}
/** One block of a section. */
export interface BlockView {
  /** Whether the block is prose or a table row. */
  kind: "paragraph" | "table_row";
  /** Where the block sits in the text. */
  span: Utf16Span;
}
/** One section of the paper. */
export interface SectionView {
  /** The section's heading level (1 is top level). */
  level: number;
  /** Where the heading sits in the text, if the section has one. */
  heading: Utf16Span | null;
  /** The section's blocks, in order. */
  blocks: BlockView[];
}
/** The whole paper as rendered. */
export interface PaperView {
  /** The paper's id. */
  paper_id: string;
  /** The paper's title. */
  title: string;
  /** The stored canonical text shown, so a confirmation names the text the user saw, not one a re-derive stored since. */
  canonical_id: number;
  /** The full canonical text, never truncated. */
  text: string;
  /** Where the title sits in `text`, if the paper has one. */
  title_span: Utf16Span | null;
  /** The sections, in document order. */
  sections: SectionView[];
  /** The converter version that produced `text`. */
  converter_version: string;
  /** Whether the text passed the full-text check, and whether the user confirmed it. */
  full_text: FullTextStatus;
  /** What kind of source the text was derived from. */
  source_kind: SourceKind;
  /** How the source's bytes were decoded; null for a PDF. */
  decoded: Decoded | null;
  /** Whether the Original tab has a PDF to show: the source is one, or one was kept beside its JATS. */
  has_original_pdf: boolean;
}
/** One part of an answer as shown: text, or the `[n]` marker of a located quote. */
export type AnswerPiece = { kind: "text"; text: string } | { kind: "cite"; number: number };
/** A located quote of an answer. */
export interface CitationView {
  /** The number the answer shows as `[n]`. */
  number: number;
  /** Where the quote sits in the paper's text (UTF-16 units). */
  span: Utf16Span;
  /** The paper's wording at `span`, never the model's copy. */
  text: string;
}
/** A quote the model wrote that could not be found in the paper. */
export interface UnlocatedView {
  /** The quote as the model wrote it. */
  quote: string;
  /**
   * The similarity of its closest match in the paper (0 to 1; 0 for an unclosed or
   * empty quote, which was never looked up).
   */
  similarity: number;
  /**
   * Why bmlib's guard refused a match that reached the threshold, as bmlib names it
   * (`number`, `negation`, `direction`); null when it refused nothing (no match
   * reached the threshold, or the near match changed nothing it checks).
   */
  contradiction: string | null;
}
/** Every state a stored PRISMA run can be in, mirroring Rust's `RunStatus`. */
export const RUN_STATUSES = ["incomplete", "complete"] as const;
/** Every way a turn can end, mirroring Rust's `TurnStatus`. */
export const TURN_STATUSES = ["complete", "stopped", "failed"] as const;
/** What the model was given of the paper: all of it, or the passages found for the question. */
export const MODES = ["whole_paper", "retrieval"] as const;
/** One question and its answer. */
export interface TurnView {
  /** The turn's id. */
  id: number;
  /** The user's question. */
  question: string;
  /** The answer: text and numbered citation markers. */
  pieces: AnswerPiece[];
  /** The located quotes, in the order the answer cites them. */
  citations: CitationView[];
  /** The quotes that could not be found in the paper, in order. */
  unlocated: UnlocatedView[];
  /** How the turn ended; a stopped or failed answer's quotes were not checked. */
  status: (typeof TURN_STATUSES)[number];
  /**
   * Why it failed for a failed turn, or the cut-off notice for a complete
   * one. Read `status`, not this, to tell them apart.
   */
  error: string | null;
  /** The `provider:model` that answered. */
  model: string;
  /** What the model was given of the paper. */
  mode: (typeof MODES)[number];
}
/** A conversation with all its turns. */
export interface ConversationView {
  /** The conversation's id. */
  id: number;
  /** The paper text its spans point into (show it with `paperViewAt`). */
  canonical_id: number;
  /**
   * True unless this is the paper's current conversation (the newest on its latest text):
   * new questions always go there, so an earlier conversation (on this text or an older one)
   * is shown without a question box.
   */
  read_only: boolean;
  /** The turns, oldest first. */
  turns: TurnView[];
}
/** One line of a paper's list of conversations. */
export interface ConversationSummary {
  /** The conversation's id. */
  id: number;
  /** The paper text it is on. */
  canonical_id: number;
  /** How many turns it holds. */
  turns: number;
  /** When it was started (stored text, UTC). */
  created_at: string;
}
/** Everything the Interrogator tab opens with. */
export interface InterrogatorView {
  /** The paper's latest text, the one questions go to. */
  canonical_id: number;
  /** The newest conversation on that text, if there is one. */
  current: ConversationView | null;
  /** Every other conversation with turns, newest first. */
  earlier: ConversationSummary[];
}
/** The four PICO parts in order, mirroring bmlib's `PICO_PARTS`. */
export const PICO_PARTS = ["population", "intervention", "comparison", "outcome"] as const;
/** Every state a PICO part can be in, mirroring bmlib's `PicoPartState`. */
export const PICO_PART_STATES = ["verified", "unverified", "contradicted", "not_reported"] as const;
/** One PICO part as the card shows it. */
export interface PicoPartView {
  /** Which part. */
  part: (typeof PICO_PARTS)[number];
  /** The model's description, whatever the state. */
  description: string;
  /** Whether the paper was found to back it. */
  state: (typeof PICO_PART_STATES)[number];
  /** The model's confidence, 0–1, when it gave one. */
  confidence: number | null;
  /** The paper's spans backing the part, numbered across the run. */
  citations: CitationView[];
  /** The model's quotes that were not found in the paper, verbatim. */
  quotes_not_found: string[];
  /** The model's quotes the paper contradicts, with why (bmlib #647). */
  quotes_contradicted: ContradictedQuoteView[];
  /** How many of the model's quotes were not usable text at all. */
  quotes_dropped: number;
}
/** A model's quote the paper's passage contradicts. */
export interface ContradictedQuoteView {
  /** The model's quote, verbatim. */
  quote: string;
  /** Why, in bmlib's name for it (`number`, `negation`, `direction`); see `contradictionNote`. */
  contradiction: string;
}
/** An extraction as the card shows it. */
export interface PicoExtractionView {
  /** The four parts, in order. */
  parts: PicoPartView[];
  /** The design as the model named it; not checked against the paper. */
  study_type: string | null;
  /** The sample size as the model gave it; not checked against the paper. */
  sample_size: string | null;
  /** The mean confidence of the reported parts. */
  overall_confidence: number | null;
  /** The paper's length in characters when it was condensed first. */
  condensed_from_chars: number | null;
  /** How the condensation ended, when it did not complete cleanly. */
  condensation_status: string | null;
  /** bmlib's notes on how the extraction was made. */
  notes: string[];
}
/** A suitability check as the card shows it. */
export interface SuitabilityView {
  /** Whether the paper has a PICO to extract. */
  is_suitable: boolean;
  /** True when the check read the abstract, false for the whole text. */
  from_abstract: boolean;
  /** Whether the model found that the study tests an intervention. */
  is_intervention_study: boolean | null;
  /** Whether the model found a control or comparison group. */
  has_comparison: boolean | null;
  /** The design as the model named it. */
  study_type: string | null;
  /** The model's confidence, 0–1. */
  confidence: number | null;
  /** The model's explanation. */
  rationale: string | null;
  /** The text's length in characters when it was condensed first. */
  condensed_from_chars: number | null;
  /** How the condensation ended, when it did not complete cleanly. */
  condensation_status: string | null;
  /** bmlib's notes on how the check was made. */
  notes: string[];
}
/** A stored PICO run. */
export interface PicoRunView {
  /** The run's id. */
  id: number;
  /** The paper text its spans point into (show it with `paperViewAt`). */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was stored (stored text, UTC). */
  created_at: string;
  /** True unless this is the newest run on the paper's latest text. */
  read_only: boolean;
  /** The check, or null for a forced run. */
  suitability: SuitabilityView | null;
  /** The extraction, or null when the check found the paper unsuitable. */
  extraction: PicoExtractionView | null;
}
/** One line of a paper's list of PICO runs. */
export interface PicoRunSummary {
  /** The run's id. */
  id: number;
  /** The paper text it ran on. */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was stored (stored text, UTC). */
  created_at: string;
}
/** Everything the PICO tab opens with. */
export interface PicoView {
  /** The paper's latest text, the one a run goes to. */
  canonical_id: number;
  /** The newest run on that text, if there is one. */
  current: PicoRunView | null;
  /** Every other run, newest first. */
  earlier: PicoRunSummary[];
}
/** What arrives while a question is answered. */
export type AnswerEvent =
  /** The search model on disk is being checked and loaded (slow on the first retrieval of a launch). */
  | { kind: "checking_model" }
  /** The paper is being indexed for retrieval: `done` of `total` chunks. */
  | { kind: "indexing"; done: number; total: number }
  /** The model has been asked; its first text has not arrived yet. */
  | { kind: "waiting" }
  /** A piece of the answer's text, raw (quotes still in `<q>` marks). */
  | { kind: "delta"; text: string };
/** What arrives while a PICO run condenses a long paper. */
export type PicoEvent =
  /** Part `batch` of `batches` is being condensed; above `level` 0 the parts are condensed parts, condensed again. */
  | { kind: "condensing"; batch: number; batches: number; level: number }
  /** The condensation has ended, whether it succeeded or failed; the run's result says which. */
  | { kind: "condensed" }
  /** A stage of bmlib's this version does not know, in bmlib's words. */
  | { kind: "other"; stage: string; message: string };
/** Every judgement a PRISMA item can be shown with, mirroring bmlib's `Judgement`. */
export const PRISMA_JUDGEMENTS = [
  "reported",
  "partly_reported",
  "not_reported",
  "reported_elsewhere",
  "not_applicable",
] as const;
/** The count key of the items with no shown judgement (*Not assessed*). */
export const PRISMA_NOT_ASSESSED = "not_assessed";
/** A located quote of a PRISMA item. */
export interface PrismaCitationView {
  /** Where the quote sits in the paper's text (UTF-16 units). */
  span: Utf16Span;
  /** The paper's wording at `span`, never the model's copy. */
  text: string;
  /** `find_passage`'s similarity, 0-1. */
  similarity: number;
  /** Whether the match was exact after normalisation. */
  exact: boolean;
}
/** One checklist item as the tab shows it. */
export interface PrismaItemView {
  /** The checklist id, for example `"13d"`. */
  id: string;
  /** The section's wire string (`title`, `abstract`, `introduction`, `methods`, `results`, `discussion`, `other_information`). */
  section: string;
  /** The item's short name. */
  name: string;
  /** Whether it is one of the ten key items. */
  key: boolean;
  /** The judgement shown; null means *Not assessed*. */
  judgement: string | null;
  /** What the model said; differs from `judgement` only for an unbacked claim; null with it. */
  model_judgement: string | null;
  /** True when the claim was shown as `not_reported` for want of a located quote. */
  unbacked: boolean;
  /**
   * Whether the paper was condensed for the part that first assessed the item; for a re-checked item,
   * `recheck_condensed` says how the shown answer was judged.
   */
  condensed: boolean;
  /** Whether a key-item re-check replaced the first answer. */
  rechecked: boolean;
  /** Whether the latest re-check's paper was condensed for it; false when there was none. */
  recheck_condensed: boolean;
  /** The judgement shown before the first re-check; null when there was none. */
  first_judgement: string | null;
  /** The model's explanation. */
  explanation: string | null;
  /** The paper's spans backing the item. */
  citations: PrismaCitationView[];
  /** The model's quotes not found in the paper, verbatim. */
  quotes_not_found: string[];
  /** The model's quotes the paper contradicts. */
  quotes_contradicted: ContradictedQuoteView[];
  /** How many of the model's quotes were not usable text. */
  quotes_dropped: number;
}
/** One section's tally of its items. */
export interface PrismaSectionView {
  /** The section's wire string. */
  section: string;
  /** Items per shown judgement plus `not_assessed`, zeros included. */
  counts: Record<string, number>;
}
/** How many of the ten key items are gaps. */
export interface KeyGapsView {
  /** The key item ids, in order. */
  items: string[];
  /** Key items `not_reported` (unbacked included) or `partly_reported`. */
  gaps: number;
  /** Key items `reported_elsewhere`. */
  elsewhere: number;
  /** The number of key items. */
  total: number;
  /** Whether the run is complete. */
  complete: boolean;
}
/** A part that failed (and has not succeeded since). */
export interface FailedPartView {
  /** The part's items, as `"Methods 5–13c"`. */
  label: string;
  /** The error's kind. A part Continue made pending again keeps its last error until it is saved. */
  error_kind: string;
  /** The error's message. */
  error_message: string;
}
/** A PRISMA suitability check as the tab shows it. */
export interface PrismaSuitabilityView {
  /** Whether the model found the paper a systematic review. */
  is_systematic_review: boolean;
  /** Whether the model found the paper a meta-analysis. */
  is_meta_analysis: boolean;
  /** Whether the checklist applies. */
  is_suitable: boolean;
  /** The document type as the model named it. */
  document_type: string;
  /** The review variant as the model named it. */
  variant: string | null;
  /** The model's explanation. */
  rationale: string;
  /** The model's confidence, 0-1. */
  confidence: number;
  /** True when the check read the abstract, false for the whole text. */
  from_abstract: boolean;
  /** The text's length in characters when it was condensed first. */
  condensed_from_chars: number | null;
  /** How the condensation ended, when it did not complete cleanly. */
  condensation_status: string | null;
}
/** A stored PRISMA run. */
export interface PrismaRunView {
  /** The run's id. */
  id: number;
  /** The paper text its spans point into. */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was stored (stored text, UTC). */
  created_at: string;
  /** True when the user forced the run past the check. */
  forced: boolean;
  /** `incomplete` or `complete`. */
  status: (typeof RUN_STATUSES)[number];
  /** The check, or null for a forced run with none. */
  suitability: PrismaSuitabilityView | null;
  /** All 42 items, in checklist order. */
  items: PrismaItemView[];
  /** A tally per section. */
  sections: PrismaSectionView[];
  /** The key-item gap count. */
  key_gaps: KeyGapsView;
  /** Every failed part. */
  failed_parts: FailedPartView[];
  /** The ids a reply left out, so a part of their own asks them again, in checklist order, each once. */
  left_out: string[];
  /** The labels of the parts cut off and asked in halves, as `"Methods 5–13c"`. */
  halved: string[];
  /** The ids replies answered unasked (set aside), in checklist order, each once; one not on the checklist comes last. */
  unasked: string[];
  /** Every note the user should see, each once. */
  notes: string[];
}
/** One line of a paper's list of PRISMA runs. */
export interface PrismaRunSummary {
  /** The run's id. */
  id: number;
  /** The paper text it ran on. */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was stored (stored text, UTC). */
  created_at: string;
  /** `incomplete` or `complete`. */
  status: (typeof RUN_STATUSES)[number];
}
/** Everything the PRISMA tab opens with. */
export interface PrismaView {
  /** The paper's latest text, the one a run goes to. */
  canonical_id: number;
  /** The newest run on that text, if there is one. */
  current: PrismaRunView | null;
  /** Every other run, newest first. */
  earlier: PrismaRunSummary[];
}
/** What arrives while a PRISMA run works. */
export type PrismaEvent =
  /** Part `index` of `of` has begun, its items named by `label`. */
  | { kind: "part"; index: number; of: number; label: string }
  /** Batch `batch` of `batches` of a condensation is being condensed. */
  | { kind: "condensing"; batch: number; batches: number; level: number }
  /** The condensation has ended, whether it succeeded or failed. */
  | { kind: "condensed" }
  /** The part just begun checks key item `item` again (sent after its `part` event). */
  | { kind: "rechecking"; item: string }
  /** A stage of bmlib's this version does not know, in bmlib's words. */
  | { kind: "other"; stage: string; message: string };
/** The quotes backing one evidence claim: the paper's spans and what was lost. */
export interface QuotesView {
  /** The paper's spans, in order; each `text` is the paper's wording, never the model's copy. */
  citations: PrismaCitationView[];
  /** The model's quotes not found in the paper, verbatim. */
  quotes_not_found: string[];
  /** The model's quotes the paper contradicts. */
  quotes_contradicted: ContradictedQuoteView[];
  /** How many of the model's quotes were not usable text. */
  quotes_dropped: number;
}
/** The study design as the Evidence weight tab shows it. */
export interface DesignView {
  /** bmlib's name for the design (`rct`, `cohort_prospective`, …). */
  design: string;
  /** A randomised trial's layout (`parallel`, `cluster`, `crossover`, `other`). */
  rct_layout: string | null;
  /** The model's explanation. */
  reasoning: string;
  /** The quotes backing it. */
  quotes: QuotesView;
}
/** An outcome's reported effect. */
export interface ResultView {
  /** The measure's name, as the paper gives it. */
  measure: string;
  /** `ratio` or `difference`. */
  scale: string;
  /** The point estimate. */
  estimate: number;
  /** The interval's lower bound. */
  ci_low: number | null;
  /** The interval's upper bound. */
  ci_high: number | null;
  /** The interval's level, as a fraction (`0.95`). */
  ci_level: number | null;
  /** How many were analysed. */
  n_analysed: number | null;
}
/** One answer to a RoB 2 signalling question. */
export interface AnswerView {
  /** The question's id (`2.3`). */
  question_id: string;
  /** The question's wording, or the id itself for one today's bmlib does not ask (earlier rules). */
  question: string;
  /** The answer (`Y`, `PY`, `PN`, `N`, `NI`). */
  answer: string;
  /** The model's explanation. */
  reasoning: string;
  /** The quotes backing it. */
  quotes: QuotesView;
}
/** One D5 fact. */
export interface FactView {
  /** The fact's name (`plan_cited`, …). */
  fact: string;
  /** What it means, or the name for a fact today's bmlib does not know. */
  meaning: string;
  /** Whether the model said it is present. */
  claimed: boolean;
  /** Whether it counts, and if not, why (`present`, `not_claimed`, …). */
  verdict: string;
  /** The quotes backing it. */
  quotes: QuotesView;
}
/** One other measurement of D5's outcome. */
export interface MeasurementView {
  /** The model's short name for it. */
  label: string;
  /** The passages naming it. */
  named: PrismaCitationView[];
  /** The passages giving its result. */
  result: PrismaCitationView[];
  /** What a second look found; null when none was made. */
  second_look: PrismaCitationView[] | null;
  /** Why the second look failed, when it did. */
  second_look_error: string | null;
  /** Its lost quotes (`citations` always empty). */
  quotes: QuotesView;
}
/** D5's check of the analysis reported against the one planned. */
export interface AnalysisView {
  /** Whether the model says they differ with no reason given. */
  differs: boolean;
  /** The passages naming the planned main analysis. */
  planned: PrismaCitationView[];
  /** The passages giving the result with its analysis. */
  reported: PrismaCitationView[];
  /** Its lost quotes (`citations` always empty). */
  quotes: QuotesView;
}
/** Where an evidence step stands, mirroring Rust's `StepStatus`. */
export const STEP_STATUSES = ["pending", "done", "failed"] as const;
/** What an evidence run found, mirroring Rust's `RunKind`. */
export const RUN_KINDS = ["not_covered", "no_primary_outcome", "assessed"] as const;
/** One RoB 2 domain, whatever its step's status. */
export interface DomainView {
  /** The domain (`D1` … `D5`). */
  domain: string;
  /** Its title, in bmlib's words. */
  name: string;
  /** Where its step stands. */
  status: (typeof STEP_STATUSES)[number];
  /** Why its step failed; kept on one set back to pending until it is saved. */
  error: string | null;
  /** Its judgement (`low`, `some_concerns`, `high`); null means *Not assessed*. */
  judgement: string | null;
  /** Whether it was judged from a condensed paper. */
  condensed: boolean;
  /** The answers, by question id. */
  answers: AnswerView[];
  /** Question ids answered although they did not apply. */
  ignored: string[];
  /** D5's facts; null for every other domain and a D5 not done. */
  facts: FactView[] | null;
  /** D5's other measurements; empty otherwise. */
  measurements: MeasurementView[];
  /** D5's analysis check; null otherwise. */
  analysis: AnalysisView | null;
}
/** One step down an outcome's ladder. */
export interface LadderStepView {
  /** Why (`risk_of_bias`, `crosses_no_effect`, `small_trial`, `no_interval`, `sample_size_unknown`). */
  reason: string;
  /** The overall judgement, for `risk_of_bias`. */
  judgement: string | null;
  /** How many were analysed, for `small_trial`. */
  n: number | null;
  /** The threshold it fell under, for `small_trial`. */
  threshold: number | null;
  /** Levels lost. */
  levels: number;
}
/** An outcome's ladder. */
export interface LadderView {
  /** Where it started (`high`, `moderate`, `low`, `very_low`). */
  start: string;
  /** Where it ended. */
  level: string;
  /** The end level's weight: the outcome's number. */
  weight: number;
  /** Every step, risk of bias first. */
  steps: LadderStepView[];
}
/** An escalation the model answered. */
export interface EscalationView {
  /** Whether the concerns add up to a high risk of bias. */
  escalate: boolean;
  /** The model's explanation. */
  reasoning: string;
}
/** Something that failed, named for the user: a step by its progress label, or a second look by its measurement. */
export interface NamedErrorView {
  /** What failed. */
  label: string;
  /** Why, as stored. */
  error: string;
}
/** One primary outcome as its card shows it. */
export interface OutcomeView {
  /** Its index among the run's outcomes, from 0. */
  index: number;
  /** The outcome as the paper names it. */
  name: string;
  /** When it was measured. */
  timepoint: string | null;
  /** Its reported effect. */
  result: ResultView | null;
  /** Why its reported effect was refused. */
  result_error: string | null;
  /** The quotes naming it. */
  quotes: QuotesView;
  /** Its domains, always D2–D5 in order. */
  domains: DomainView[];
  /** The escalation, when the model answered one. */
  escalation: EscalationView | null;
  /** Why its escalation step failed. */
  escalation_error: string | null;
  /** The overall judgement; null means *Not assessed*. */
  overall: string | null;
  /** Its ladder, when one was built. */
  ladder: LadderView | null;
  /** Why there is no ladder, when its escalation stored one. */
  ladder_error: string | null;
  /** D5's other measurements whose second look failed, named beside the number. */
  second_look_failures: NamedErrorView[];
}
/** A stored evidence run. */
export interface EvidenceRunView {
  /** The run's id. */
  id: number;
  /** The paper text its spans point into. */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was started (stored text, UTC). */
  created_at: string;
  /** When the tab first showed it with a RoB 2 result. */
  first_shown_at: string | null;
  /** `incomplete` or `complete`. */
  status: (typeof RUN_STATUSES)[number];
  /** What it found, once known. */
  kind: (typeof RUN_KINDS)[number] | null;
  /** Whether it was made under earlier rules: shown in full, Re-run only. */
  earlier_rules: boolean;
  /** Whether any step was judged from a condensed paper. */
  condensed: boolean;
  /** The study design, once its step is done. */
  design: DesignView | null;
  /** The primary outcomes, in order. */
  outcomes: OutcomeView[];
  /** D1, judged once for the trial; null until the run is assessed. */
  study: DomainView | null;
  /** How many outcomes have a ladder. */
  assessed_outcomes: number;
  /** The largest ladder weight, only for a complete, assessed run; null otherwise. */
  paper_weight: number | null;
  /** Every step that failed (or was set back with its error kept), by its progress label. */
  failed_steps: NamedErrorView[];
  /** Whether a domain step is done: a RoB 2 result is on show. */
  has_rob2_result: boolean;
}
/** One line of a paper's list of evidence runs. */
export interface EvidenceRunSummary {
  /** The run's id. */
  id: number;
  /** The paper text it ran on. */
  canonical_id: number;
  /** The `provider:model` that ran it. */
  model: string;
  /** When it was started (stored text, UTC). */
  created_at: string;
  /** `incomplete` or `complete`. */
  status: (typeof RUN_STATUSES)[number];
}
/** Everything the Evidence weight tab opens with. */
export interface EvidenceView {
  /** The paper's latest text, the one a run goes to. */
  canonical_id: number;
  /** The newest run on that text, if there is one. */
  current: EvidenceRunView | null;
  /** Every other run, newest first. */
  earlier: EvidenceRunSummary[];
}
/** What arrives while an evidence weight run works. */
export type EvidenceEvent =
  /** A step has begun, named by its progress label. */
  | { kind: "step"; label: string }
  /** Batch `batch` of `batches` of a condensation is being condensed. */
  | { kind: "condensing"; batch: number; batches: number; level: number }
  /** The condensation has ended, whether it succeeded or failed. */
  | { kind: "condensed" }
  /** A stage of bmlib's this version does not know, in bmlib's words. */
  | { kind: "other"; stage: string; message: string };
/** Every place a provider's key can come from, mirroring Rust's `KeySource`. */
export const KEY_SOURCES = ["keychain", "env"] as const;
/** One model provider as Settings shows it; whether a key exists, never the key. */
export interface ProviderInfo {
  /** The provider's name (the part before the first `:` of a model string). */
  name: string;
  /** The model used when none is named. */
  default_model: string;
  /** Whether the provider cannot be used without a key. */
  needs_key: boolean;
  /** Whether a key is available, from the keychain or the environment. */
  has_key: boolean;
  /** Where that key comes from (the keychain wins); null when there is none. */
  key_source: (typeof KEY_SOURCES)[number] | null;
  /** The environment variable that can supply the key. */
  key_env: string | null;
}
/** Everything Settings shows about the model. */
export interface LlmSettings {
  /** The chosen `provider:model`, if any. */
  model: string | null;
  /** The user's context-window limit in tokens, if set. */
  context_tokens: number | null;
  /** Every provider bmlib supports. */
  providers: ProviderInfo[];
  /** Set when the keychain could not be read (then `has_key` reflects the environment only). */
  keychain_error: string | null;
}
/** Whether the search model is on this device. */
export interface EmbedderStatus {
  /** True when it is downloaded (and checked). */
  downloaded: boolean;
  /** The download's size in bytes. */
  size_bytes: number;
}
/** How far the search-model download has got, in bytes. */
export interface DownloadProgress {
  /** Bytes received so far. */
  done: number;
  /** Bytes in all. */
  total: number;
}
/** Every `CommandError` kind, mirroring Rust's `ErrorKind` (`contract.test.ts` checks it against the fixture). */
export const ERROR_KINDS = [
  "invalid_input",
  "too_large",
  "unsupported_format",
  "undecodable",
  "needs_email",
  "needs_model",
  "needs_api_key",
  "needs_embedder",
  "not_full_text",
  "llm",
  "cancelled",
  "storage",
  "internal",
] as const;
/** What a rejected command carries. */
export interface CommandError {
  /** A stable machine-readable category. */
  kind: (typeof ERROR_KINDS)[number];
  /** A message the user can read. */
  message: string;
}

/** Whether `k` is one of Rust's error kinds. */
function isErrorKind(k: unknown): k is CommandError["kind"] {
  return (ERROR_KINDS as readonly unknown[]).includes(k);
}

/** A rejection's text: an object as JSON (never "[object Object]"), anything else as `String` gives it. */
function rejectionText(e: unknown): string {
  if (e === null || typeof e !== "object" || e instanceof Error) return String(e);
  try {
    return JSON.stringify(e);
  } catch {
    return String(e); // circular or BigInt: the plain form is all there is
  }
}

/**
 * Turns anything a rejected `invoke` threw into a `CommandError` to show.
 * Only a known kind with a string message passes as Rust's; an unknown kind
 * becomes `internal` with its name kept in the message.
 */
export function toCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "kind" in e && "message" in e && typeof e.message === "string") {
    if (isErrorKind(e.kind)) return e as CommandError;
    return { kind: "internal", message: `${e.message} (unknown error kind ${JSON.stringify(e.kind)})` };
  }
  return { kind: "internal", message: rejectionText(e) };
}

/**
 * Writes a failure to the app's log file. Rust already logs the errors it
 * raises; logging every failure here also catches the rest (IPC, dialog and
 * import failures). Never throws.
 */
export function logFailure(where: string, err: CommandError): void {
  import("@tauri-apps/plugin-log")
    .then(({ error }) => error(`frontend ${where} (${err.kind}): ${err.message}`))
    .catch((e) => console.error("could not write to the log:", e, err));
}

/** A file the user chose or dropped, as Rust granted it: the webview can only pass the grant back, never a path. */
export interface GrantedFile {
  /** The grant to pass to `openFile` / `attachFile`; good once. */
  id: number;
  /** The file's name, for display. */
  name: string;
}
/** Opens a file chosen in the dialog or dropped on the window, by its grant. */
export const openFile = (grant: number) => invoke<OpenOutcome>("open_file", { grant });
/** Opens pasted text as a new paper. */
export const openPaste = (text: string) => invoke<OpenOutcome>("open_paste", { text });
/** Opens a PMID, DOI or PMC ID as typed (Rust validates it). */
export const openIdentifier = (input: string) => invoke<OpenOutcome>("open_identifier", { input });
/**
 * Stops the running identifier lookup; it then rejects with kind `cancelled`,
 * unless it was reopening a stored paper or already saving the result.
 * Resolves false when no lookup was registered (send it with `stopWhilePending`).
 */
export const cancelLookup = () => invoke<boolean>("cancel_lookup");
/** Gives a stored paper its full text from a file. */
export const attachFile = (paperId: string, grant: number) =>
  invoke<OpenOutcome>("attach_file", { paperId, grant });
/** Gives a stored paper its full text from pasted text. */
export const attachPaste = (paperId: string, text: string) =>
  invoke<OpenOutcome>("attach_paste", { paperId, text });
/** Reopens a stored paper from the recent list (offline). */
export const reopenPaper = (paperId: string) => invoke<OpenOutcome>("reopen_paper", { paperId });
/** The papers opened so far, most recent first. */
export const recentPapers = () => invoke<PaperSummary[]>("recent_papers");
/** The paper's text and spans, ready to render. */
export const paperView = (paperId: string) => invoke<PaperView>("paper_view", { paperId });
/** The PDF for the Original tab, as raw bytes: the text's own, else the one kept beside its JATS (rejects when there is neither). */
export const originalPdf = (paperId: string) => invoke<ArrayBuffer>("original_pdf", { paperId });
/** Records that the user confirmed the text they saw (`PaperView.canonical_id`) is the whole paper. */
export const confirmFullText = (paperId: string, canonicalId: number) =>
  invoke<void>("confirm_full_text", { paperId, canonicalId });
/** The stored contact email, or null if none was saved. */
export const getContactEmail = () => invoke<string | null>("get_contact_email");
/** Saves the contact email the full-text services ask for. */
export const setContactEmail = (email: string) => invoke<void>("set_contact_email", { email });
/** Whether a document contradicting its stored paper opens as a paper of its own (a debugging setting). */
export const getKeepMismatched = () => invoke<boolean>("get_keep_mismatched");
/** Turns that debugging setting on or off. */
export const setKeepMismatched = (keep: boolean) => invoke<void>("set_keep_mismatched", { keep });

/**
 * Asks a question of the paper with the model chosen in Settings; indexing
 * progress and the answer's text arrive at `onEvent` as they come. Resolves
 * to the conversation with the new turn; rejects with `needs_model`,
 * `needs_api_key` (no key, or one the provider refused), `needs_embedder`,
 * `not_full_text`, `invalid_input` (the provider has no such model, or the
 * paper's text was replaced while it asked: asking again uses the new one),
 * `llm`, `cancelled`, ….
 */
export const ask = (paperId: string, question: string, onEvent: (event: AnswerEvent) => void) =>
  invoke<ConversationView>("ask", { paperId, question, onEvent: new Channel(onEvent) });
/**
 * Stops the running answer; it is kept as a `stopped` turn, or, with no text yet, `ask` rejects with
 * `cancelled`. Resolves to whether an answer was running (see `stopWhilePending`).
 */
export const stopAnswer = () => invoke<boolean>("stop_answer");
/** Starts a new conversation on the paper's latest text. */
export const newConversation = (paperId: string) =>
  invoke<ConversationView>("new_conversation", { paperId });
/** What the Interrogator tab opens with. */
export const interrogator = (paperId: string) => invoke<InterrogatorView>("interrogator", { paperId });
/** One stored conversation with its turns. */
export const conversation = (conversationId: number) =>
  invoke<ConversationView>("conversation", { conversationId });
/** The paper view of one of the paper's texts (an earlier conversation's `canonical_id`). */
export const paperViewAt = (paperId: string, canonicalId: number) =>
  invoke<PaperView>("paper_view_at", { paperId, canonicalId });
/** What the PICO tab opens with; never runs anything. */
export const pico = (paperId: string) => invoke<PicoView>("pico", { paperId });
/** One stored PICO run (an earlier one, from the tab's list). */
export const picoRun = (runId: number) => invoke<PicoRunView>("pico_run", { runId });
/**
 * Runs PICO on the paper with the model chosen in Settings (`force` skips the
 * suitability check). Resolves to the tab's view; rejects with `needs_model`,
 * `needs_api_key`, `not_full_text`, `invalid_input` (a window too small, or a
 * run already going), `llm`, `cancelled`, …. A long paper's condensation reports to `onEvent`.
 */
export const runPico = (paperId: string, force: boolean, onEvent: (event: PicoEvent) => void) =>
  invoke<PicoView>("run_pico", { paperId, force, onEvent: new Channel(onEvent) });
/**
 * Stops the running PICO extraction before its next model call; `runPico` then rejects with
 * `cancelled`. Resolves to whether a run was going (see `stopWhilePending`).
 */
export const stopPico = () => invoke<boolean>("stop_pico");
/** What the PRISMA tab opens with; never runs anything. */
export const prisma = (paperId: string) => invoke<PrismaView>("prisma", { paperId });
/** One stored PRISMA run (an earlier one, from the tab's list). */
export const prismaRun = (runId: number) => invoke<PrismaRunView>("prisma_run", { runId });
/**
 * Runs PRISMA 2020 on the paper and resolves to the tab's view. With
 * `continueRun` null, a new run with the model chosen in Settings (`force`
 * skips the suitability check); with a run's id, the parts that run has left,
 * on its own model and text. Rejects with `needs_model`, `needs_api_key`,
 * `not_full_text`, `invalid_input` (a window or reply room too small, a run not
 * stored, or a run already going), `llm`, `cancelled`, …; the parts finished
 * before a stop or failure are kept. Parts and condensations report to `onEvent`.
 */
export const runPrisma = (
  paperId: string,
  force: boolean,
  continueRun: number | null,
  onEvent: (event: PrismaEvent) => void,
) => invoke<PrismaView>("run_prisma", { paperId, force, continueRun, onEvent: new Channel(onEvent) });
/**
 * Stops the running PRISMA assessment before its next model call; `runPrisma`
 * then rejects with `cancelled`. Resolves to whether a run was going (see `stopWhilePending`).
 */
export const stopPrisma = () => invoke<boolean>("stop_prisma");
/** What the Evidence weight tab opens with; never runs anything. Serving a run with a RoB 2 result records when it was first shown. */
export const evidence = (paperId: string) => invoke<EvidenceView>("evidence", { paperId });
/** One stored evidence run (an earlier one, from the tab's list); recorded as shown, as `evidence`. */
export const evidenceRun = (runId: number) => invoke<EvidenceRunView>("evidence_run", { runId });
/**
 * Assesses the paper's evidence weight and resolves to the tab's view. With
 * `continueRun` null, a new run with the model chosen in Settings; with a run's
 * id, the steps that run has left, on its own model and text. Rejects with
 * `needs_model`, `needs_api_key`, `not_full_text`, `invalid_input` (a window
 * too small, a run made under earlier rules, a run not stored, or a run already
 * going), `llm`, `cancelled`, …; the steps finished before a stop or failure are
 * kept. Steps and condensations report to `onEvent`.
 */
export const runEvidence = (paperId: string, continueRun: number | null, onEvent: (event: EvidenceEvent) => void) =>
  invoke<EvidenceView>("run_evidence", { paperId, continueRun, onEvent: new Channel(onEvent) });
/**
 * Stops the running evidence weight assessment before its next model call;
 * `runEvidence` then rejects with `cancelled`. Resolves to whether a run was going (see `stopWhilePending`).
 */
export const stopEvidence = () => invoke<boolean>("stop_evidence");
/** The model settings and each provider's key status (never a key). */
export const llmSettings = () => invoke<LlmSettings>("llm_settings");
/** Chooses the `provider:model` that answers. */
export const setModel = (model: string) => invoke<void>("set_model", { model });
/** Limits the context window to `tokens`. */
export const setContextTokens = (tokens: number) => invoke<void>("set_context_tokens", { tokens });
/** Removes the context-window limit. */
export const clearContextTokens = () => invoke<void>("clear_context_tokens");
/** Saves a provider's API key in the OS keychain. */
export const setApiKey = (provider: string, key: string) => invoke<void>("set_api_key", { provider, key });
/** Removes a provider's API key from the OS keychain. */
export const deleteApiKey = (provider: string) => invoke<void>("delete_api_key", { provider });
/** The model's context window as its provider states it, or null when it states none. */
export const modelWindow = (model: string) => invoke<number | null>("model_window", { model });
/** The models `provider` offers, sorted, for Settings' model menu; every page read, nothing filtered out. */
export const listModels = (provider: string) => invoke<string[]>("list_models", { provider });
/** Whether the search model is downloaded, and its size. */
export const embedderStatus = () => invoke<EmbedderStatus>("embedder_status");
/** Downloads (once) and loads the search model; progress arrives at `onProgress`. */
export const downloadEmbedder = (onProgress: (progress: DownloadProgress) => void) =>
  invoke<void>("download_embedder", { onProgress: new Channel(onProgress) });
/** Cancels the running download; it then rejects with kind `cancelled`. Resolves to whether one was running. */
export const cancelDownload = () => invoke<boolean>("cancel_download");

/** Milliseconds before a stop that found no run is sent again (`stopWhilePending`). */
export const STOP_RESEND_MS = 50;

/**
 * Sends `stop` until it reaches a run or `pending()` turns false. Rust may handle a stop before
 * the command it is meant for has registered its run (#61); such a stop finds nothing, and
 * the run would go on. Rejects as `stop` does.
 */
export async function stopWhilePending(
  stop: () => Promise<boolean>,
  pending: () => boolean,
  waitMs = STOP_RESEND_MS,
): Promise<void> {
  while (!(await stop())) {
    await new Promise((r) => setTimeout(r, waitMs));
    // Checked after the wait: a later run may have begun during it, and must not get this stop.
    if (!pending()) return;
  }
}

/** The event Rust emits with the grants of dropped files (`FILES_DROPPED_EVENT` in handoff.rs). */
export const FILES_DROPPED_EVENT = "files-dropped";

/** Shows the system file dialog (in Rust); the granted file, or null if cancelled. */
export const pickFile = () => invoke<GrantedFile | null>("pick_file");

/** Calls `cb` with the files dropped on the window; resolves to the function that stops listening. */
export const onFilesDropped = (cb: (files: GrantedFile[]) => void): Promise<() => void> =>
  listen<GrantedFile[]>(FILES_DROPPED_EVENT, (ev) => cb(ev.payload));

/** A hand-off another app sent, stored on arrival and waiting to be opened or discarded. */
export interface HandoffSummary {
  /** The hand-off's id, for `openHandoff` / `discardHandoff`. */
  id: number;
  /** When it arrived (stored text, UTC). */
  received_at: string;
  /** The sending app, or null (a link names none). */
  sender: string | null;
  /** The title the sender had, or null. */
  title: string | null;
  /** The identifiers that passed their checks. */
  identifiers: Identifiers;
  /** True when the hand-off carried the full text. */
  has_full_text: boolean;
  /** Why the hand-off was refused, or null; a refused one holds nothing else. */
  refusal: string | null;
}

/** The event Rust emits (no payload) once a received hand-off is stored (`HANDOFF_EVENT` in handoff.rs). */
export const HANDOFF_EVENT = "handoff-received";

/** The hand-offs waiting to be opened, refusals included, newest first. */
export const pendingHandoffs = () => invoke<HandoffSummary[]>("pending_handoffs");
/**
 * Opens a waiting hand-off (a lookup by its identifiers when it carries no full text, which
 * `cancelLookup` stops). Rejects with `needs_email` for such a lookup without a contact email; the
 * hand-off then stays waiting, as it does on `storage`, `internal` and a full text that contradicts
 * a stored paper (an `invalid_input` pointing to Keep mismatched, after which Open stores it). Any
 * other end removes it.
 */
export const openHandoff = (id: number) => invoke<OpenOutcome>("open_handoff", { id });
/** Removes a waiting hand-off (discarded, or its refusal shown); one already gone is fine. */
export const discardHandoff = (id: number) => invoke<void>("discard_handoff", { id });

/** Calls `cb` each time a hand-off arrives; resolves to the function that stops listening. */
export const onHandoff = (cb: () => void): Promise<() => void> => listen(HANDOFF_EVENT, () => cb());

/** How many sources are kept at each place. */
export interface PlaceCounts {
  /** In the app for good, not waiting for the library (a library may still be set: `moveIntoLibrary` moves these). */
  app: number;
  /** In the app, to be moved into the library. */
  waiting: number;
  /** In the library only. */
  library: number;
}

/** A source the Library setting lists: one waiting for the library, or one whose file was left in another library folder. */
export interface WaitingSource {
  /** Its paper. */
  paper_id: string;
  /** Its paper's title. */
  title: string;
  /** What kind of source it is. */
  kind: SourceKind;
  /** Why it is not in the library yet, once a move failed (null before); for one left behind, the folder its file is in. */
  note: string | null;
}

/** What the Library setting shows. */
export interface LibraryStatus {
  /** The library folder set, as a label (it is never passed back), or null for none. */
  path: string | null;
  /** Why the library set cannot be used now, or null. */
  unreachable: string | null;
  counts: PlaceCounts;
  /** Every source waiting for the library, oldest first. */
  waiting: WaitingSource[];
  /** Every library source whose file a change of the setting left in another folder, oldest first. */
  elsewhere: WaitingSource[];
  /** What the launch learnt of the folder but could not keep or reach (a stale bookmark not renewed, a lost bookmark), while the library is reachable; else null (an unreachable reason ends with it). */
  launch_note: string | null;
}

/** A paper whose file a change of the setting could not copy or move, and why. */
export interface PaperProblem {
  /** The paper. */
  paper_id: string;
  /** Its title. */
  title: string;
  /** Why, as a phrase for the user. */
  why: string;
}

/** What a change of the Library setting did; the two counts never overlap. */
export interface Changed {
  /** Files read back from the old library (into the new one, or the app). */
  copied: number;
  /** Files kept in the app (waiting, or moved on purpose) that went into the library. */
  moved: number;
  /** One entry per file not copied or moved. */
  failed: PaperProblem[];
}

/** What a settle of the waiting sources did (`LibrarySettled` in lib.rs). */
export interface LibrarySettled {
  /** How many moved into the library. */
  moved: number;
  /** How many still wait; after a failure (`error` set), only those noted before it. */
  left: number;
  /** Why settling failed (logged in Rust), or null. */
  error: string | null;
}

/**
 * The event Rust emits after waiting sources were settled at launch or on activation, only when a
 * source moved, one still waits, or settling failed, or, as 0/0 with no error, when a library lost
 * at launch changed status on activation, so the status line rereads (`LIBRARY_SETTLED_EVENT` in
 * lib.rs).
 */
export const LIBRARY_SETTLED_EVENT = "library-settled";

/** What the Library setting shows. */
export const libraryStatus = () => invoke<LibraryStatus>("library_status");
/** Shows the folder dialog (in Rust) and makes the chosen folder the library, bringing this app's papers along when `bring`; null if cancelled. */
export const chooseLibrary = (bring: boolean) => invoke<Changed | null>("choose_library", { bring });
/** Sets no library; with `bring`, this app's files are copied back into the app first. */
export const clearLibrary = (bring: boolean) => invoke<Changed>("clear_library", { bring });
/** Moves every source kept in the app into the library set. */
export const moveIntoLibrary = () => invoke<Changed>("move_into_library");

/** One source file of a library paper, as the manual clear lists it (`ListedFile` in library_clear.rs). */
export interface ListedFile {
  /** Its name in the folder (its kind and a prefix of its digest), what `removeLibraryFile` takes back. */
  name: string;
  /** Its size in bytes, or null when the folder cannot say. */
  size: number | null;
  /** The other installations whose records list it, by label. */
  used_by: string[];
  /** Whether this installation's record lists it. */
  used_here: boolean;
}

/** One paper folder of the library, as the manual clear lists it (`ListedPaper` in library_clear.rs). */
export interface ListedPaper {
  /** The folder's name under `papers/`, what `removeLibraryPaper` takes back. */
  folder: string;
  /** The title a record gives, or null. */
  title: string | null;
  /** Its source files, ordered by name (one only online is not listed). */
  files: ListedFile[];
  /** The other installations with a record in it (one each), by label (files on disk or not). */
  used_by: string[];
  /** Whether this installation has a record in it. */
  used_here: boolean;
  /** Whether some of its records could not be read, so who else uses it is not known. */
  users_unknown: boolean;
}

/** What the manual clear shows (`Listing` in library_clear.rs). */
export interface LibraryListing {
  /** Every paper folder, ordered by name. */
  papers: ListedPaper[];
  /** What the library scan found odd and did not use, each in the `Debug` form of bmlib's `Problem`. */
  problems: string[];
}

/** The library's papers for the manual clear (one scan of the folder). */
export const libraryPapers = () => invoke<LibraryListing>("library_papers");
/** Deletes a paper's folder from the library, records and all; refused when a fresh scan no longer lists the folder. */
export const removeLibraryPaper = (folder: string) => invoke<void>("remove_library_paper", { folder });
/** Deletes one file of a paper's folder from the library; refused when a fresh scan no longer lists the file. */
export const removeLibraryFile = (folder: string, name: string) =>
  invoke<void>("remove_library_file", { folder, name });

/** Says the webview now listens for `LIBRARY_SETTLED_EVENT`; what settles told before it did (usually the launch's), once, or null. */
export const librarySettleListening = () => invoke<LibrarySettled | null>("library_settle_listening");

/** Calls `cb` each time waiting sources are settled; resolves to the function that stops listening. */
export const onLibrarySettled = (cb: (s: LibrarySettled) => void): Promise<() => void> =>
  listen<LibrarySettled>(LIBRARY_SETTLED_EVENT, (ev) => cb(ev.payload));
