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
export const PICO_PART_STATES = ["verified", "unverified", "not_reported"] as const;
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
  /** How many of the model's quotes were not usable text at all. */
  quotes_dropped: number;
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
/** The PDF the paper's text was derived from, as raw bytes (rejects when it came from no PDF). */
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
 * `not_full_text`, `invalid_input` (the provider has no such model), `llm`,
 * `cancelled`, ….
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
/** The models `provider` offers, sorted, for Settings to suggest; every page read, nothing filtered out. */
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
