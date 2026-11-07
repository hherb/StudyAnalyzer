// Pure helpers for the Interrogator screen and the model settings: captions and parsing. No Tauri, no DOM.
import type { ConversationSummary, ConversationView, ProviderInfo, TurnView } from "./api";

/** Bytes per "GB" in the download size: a binary gigabyte (GiB, 2^30), which names bmlib's model as the brief's 1.1 GB (Finder, counting decimal, says 1.2 GB). */
const BYTES_PER_GB = 1024 ** 3;

/**
 * The caption above a read-only conversation: *on an earlier version of the text*
 * when it is on another text than the paper's latest, else *earlier conversation*;
 * null for the conversation questions go to.
 */
export function conversationCaption(conv: ConversationView, latestCanonicalId: number): string | null {
  if (!conv.read_only) return null;
  return conv.canonical_id === latestCanonicalId ? "earlier conversation" : "on an earlier version of the text";
}

/** How a conversation's start is shown: the date and time in the user's locale and time zone. */
export const CREATED_AT_FORMAT: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** SQLite's `datetime('now')` text, `YYYY-MM-DD HH:MM:SS`, which is UTC though it names no zone. */
const SQLITE_UTC = /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2})$/;

/** Reads SQLite's `datetime('now')` text as the UTC time it is; null for any other text or an impossible date. */
export function parseSqliteUtc(text: string): Date | null {
  const m = SQLITE_UTC.exec(text);
  if (!m) return null;
  const date = new Date(`${m[1]}T${m[2]}Z`);
  return Number.isNaN(date.getTime()) ? null : date;
}

/**
 * One line of the earlier-conversations list: when it started (in local time;
 * a time that cannot be read is shown as stored), how many questions, and
 * whether it is on an older text.
 */
export function earlierLabel(s: ConversationSummary, latestCanonicalId: number): string {
  const questions = `${s.turns} ${s.turns === 1 ? "question" : "questions"}`;
  const older = s.canonical_id === latestCanonicalId ? "" : " · on an earlier version of the text";
  const when = parseSqliteUtc(s.created_at)?.toLocaleString(undefined, CREATED_AT_FORMAT) ?? s.created_at;
  return `${when} · ${questions}${older}`;
}

/** The small caption under an answer: the model that answered and what it was given of the paper. */
export function turnCaption(turn: TurnView): string {
  return `${turn.model} · ${turn.mode === "whole_paper" ? "whole paper" : "passages"}`;
}

/** The question offering the search-model download, naming its size when known. */
export function embedderPrompt(sizeBytes: number | null): string {
  const size = sizeBytes === null ? "" : `${(sizeBytes / BYTES_PER_GB).toFixed(1)} GB, `;
  return `Answering from this paper needs the search model (${size}downloaded once). Download it?`;
}

/** Where a provider's key comes from, as Settings shows it (never the key): *saved*, *from VAR*, or *none*. */
export function keyStatus(p: ProviderInfo): string {
  if (p.key_source === "keychain") return "saved";
  if (p.key_source === "env") return `from ${p.key_env}`;
  return "none";
}

/**
 * The models Settings' model menu offers: the provider's listing, with the
 * model being edited first when the listing lacks it (so a stored or typed
 * name stays chosen). `null` when there is no list to choose from (not yet
 * listed, listing failed, or the provider lists none): the field is typed.
 */
export function modelChoices(listed: string[] | null, current: string): string[] | null {
  if (!listed || listed.length === 0) return null;
  const name = current.trim();
  return name && !listed.includes(name) ? [name, ...listed] : listed;
}

/** What the context-window field asks for: clear the limit (empty), set it (a positive whole number), or nothing valid. */
export type TokensInput = { kind: "clear" } | { kind: "set"; tokens: number } | { kind: "invalid" };

/** Reads the context-window field: empty clears the limit, digits set it, anything else (0, decimals, signs) is invalid. */
export function parseContextTokens(input: string): TokensInput {
  const s = input.trim().replace(/[\s,_]/g, "");
  if (input.trim() === "") return { kind: "clear" };
  if (!/^\d+$/.test(s)) return { kind: "invalid" };
  const tokens = Number(s);
  return tokens > 0 && Number.isSafeInteger(tokens) ? { kind: "set", tokens } : { kind: "invalid" };
}
