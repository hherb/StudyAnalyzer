// Typed wrappers over the Tauri commands. The types mirror the Rust `Serialize` shapes.
import { invoke } from "@tauri-apps/api/core";

/** A half-open range of UTF-16 code units into `PaperView.text`. */
export interface Utf16Span {
  start: number;
  end: number;
}
/** The canonical identifiers of a paper; absent ones are null. */
export interface Identifiers {
  pmid: string | null;
  doi: string | null;
  pmc_id: string | null;
}
/** One line of the recent-papers list. */
export interface PaperSummary {
  id: string;
  title: string;
  identifiers: Identifiers;
  opened_at: string;
}
/** How a request ended. */
export type OpenOutcome =
  | { status: "ready"; paper_id: string }
  | { status: "needs_full_text"; paper_id: string; reason: string; unreachable: boolean };
/** How a text file's bytes were decoded. */
export interface Decoded {
  encoding: string;
  guessed: boolean;
}
/** The full-text check's outcome. */
export interface FullTextStatus {
  passed: boolean;
  confirmed: boolean;
  body_words: number;
  min_body_words: number;
  section_types: string[];
}
/** One block of a section. */
export interface BlockView {
  kind: "paragraph" | "table_row";
  span: Utf16Span;
}
/** One section of the paper. */
export interface SectionView {
  level: number;
  heading: Utf16Span | null;
  blocks: BlockView[];
}
/** The whole paper as rendered. */
export interface PaperView {
  paper_id: string;
  title: string;
  text: string;
  title_span: Utf16Span | null;
  sections: SectionView[];
  converter_version: string;
  full_text: FullTextStatus;
  decoded: Decoded | null;
}
/** What a rejected command carries. */
export interface CommandError {
  kind:
    | "invalid_input"
    | "too_large"
    | "unsupported_format"
    | "undecodable"
    | "needs_email"
    | "cancelled"
    | "storage"
    | "internal";
  message: string;
}

/** Turns anything a rejected `invoke` threw into a `CommandError` to show. */
export function toCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "kind" in e && "message" in e) return e as CommandError;
  return { kind: "internal", message: String(e) };
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
/** Stops the running identifier lookup; it then rejects with kind `cancelled`. */
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
/** Records that the user confirmed the paper's text is the whole paper. */
export const confirmFullText = (paperId: string) =>
  invoke<void>("confirm_full_text", { paperId });
/** The stored contact email, or null if none was saved. */
export const getContactEmail = () => invoke<string | null>("get_contact_email");
/** Saves the contact email the full-text services ask for. */
export const setContactEmail = (email: string) => invoke<void>("set_contact_email", { email });

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
