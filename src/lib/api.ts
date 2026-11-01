// Typed wrappers over the Tauri commands. The types mirror the Rust `Serialize` shapes.
import { invoke } from "@tauri-apps/api/core";

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
/** Every `CommandError` kind, mirroring Rust's `ErrorKind` (`contract.test.ts` checks it against the fixture). */
export const ERROR_KINDS = [
  "invalid_input",
  "too_large",
  "unsupported_format",
  "undecodable",
  "needs_email",
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

/** Opens a file chosen in the dialog or dropped on the window, by its path. */
export const openFile = (path: string) => invoke<OpenOutcome>("open_file", { path });
/** Opens pasted text as a new paper. */
export const openPaste = (text: string) => invoke<OpenOutcome>("open_paste", { text });
/** Opens a PMID, DOI or PMC ID as typed (Rust validates it). */
export const openIdentifier = (input: string) => invoke<OpenOutcome>("open_identifier", { input });
/**
 * Stops the running identifier lookup; it then rejects with kind `cancelled`,
 * unless it was reopening a stored paper or already saving the result.
 */
export const cancelLookup = () => invoke<void>("cancel_lookup");
/** Gives a stored paper its full text from a file. */
export const attachFile = (paperId: string, path: string) =>
  invoke<OpenOutcome>("attach_file", { paperId, path });
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

/** File extensions the app can read. */
export const FILE_EXTENSIONS = ["pdf", "xml", "nxml", "html", "htm", "md", "markdown", "txt"];

/** Shows the system file dialog; the chosen path, or null if cancelled. */
export async function pickFile(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const chosen = await open({
    multiple: false,
    filters: [{ name: "Papers", extensions: FILE_EXTENSIONS }],
  });
  return typeof chosen === "string" ? chosen : null;
}
