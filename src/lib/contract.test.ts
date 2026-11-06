// Reads the JSON contract Rust pins (`src-tauri/src/contract.rs`) through the TS types,
// and checks the argument keys `api.ts` sends against it.
import { describe, expect, test, vi } from "vitest";
import fixture from "./fixtures/ipc_contract.json";
import * as api from "./api";
import apiSource from "./api.ts?raw";
import {
  ERROR_KINDS,
  KEY_SOURCES,
  MODES,
  PICO_PART_STATES,
  PICO_PARTS,
  SOURCE_KINDS,
  TURN_STATUSES,
  type AnswerEvent,
  type PicoEvent,
  type BlockView,
  type CitationView,
  type CommandError,
  type ConversationSummary,
  type ConversationView,
  type Decoded,
  type DownloadProgress,
  type EmbedderStatus,
  type FullTextStatus,
  type GrantedFile,
  type HandoffSummary,
  type Identifiers,
  type InterrogatorView,
  type LlmSettings,
  type OpenOutcome,
  type PaperSummary,
  type PaperView,
  type PicoExtractionView,
  type PicoPartView,
  type PicoRunSummary,
  type PicoRunView,
  type PicoView,
  type ProviderInfo,
  type SectionView,
  type SuitabilityView,
  type TurnView,
  type UnlocatedView,
  type Utf16Span,
} from "./api";
import { bannerText, groupBlocks, sliceSpan, splitCells } from "./paper";

/** `true` only when A and B have exactly the same keys. */
type SameKeys<A, B> = [keyof A] extends [keyof B] ? ([keyof B] extends [keyof A] ? true : false) : false;
/** T with its string literals widened to `string`, as a JSON import types them. */
type Widen<T> = T extends string ? string : T extends object ? { [K in keyof T]: Widen<T[K]> } : T;
/** One section of the fixture's paper view. */
type Section = (typeof fixture.paper_view.sections)[number];
/** One section of the fixture's bare paper view, whose optional fields are null (no title, heading or decoding). */
type BareSection = (typeof fixture.paper_view_bare.sections)[number];
// Checked by `check:web`, not by vitest (which strips types): a field added,
// renamed or dropped on one side fails to compile.
const keysMatch: [
  SameKeys<typeof fixture.open_ready, Extract<OpenOutcome, { status: "ready" }>>,
  SameKeys<typeof fixture.open_needs_full_text, Extract<OpenOutcome, { status: "needs_full_text" }>>,
  SameKeys<typeof fixture.paper_summary, PaperSummary>,
  SameKeys<typeof fixture.paper_summary.identifiers, Identifiers>,
  SameKeys<typeof fixture.paper_view, PaperView>,
  SameKeys<typeof fixture.paper_view.title_span, Utf16Span>,
  SameKeys<typeof fixture.paper_view.full_text, FullTextStatus>,
  SameKeys<typeof fixture.paper_view.decoded, Decoded>,
  SameKeys<Section, SectionView>,
  SameKeys<Section["blocks"][number], BlockView>,
  SameKeys<(typeof fixture.command_errors)[number], CommandError>,
  SameKeys<typeof fixture.paper_view_bare, PaperView>,
  SameKeys<BareSection, SectionView>,
  SameKeys<typeof fixture.turn_view, TurnView>,
  SameKeys<(typeof fixture.turn_view.citations)[number], CitationView>,
  SameKeys<(typeof fixture.turn_view.unlocated)[number], UnlocatedView>,
  SameKeys<typeof fixture.turn_view_stopped, TurnView>,
  SameKeys<typeof fixture.turn_view_failed, TurnView>,
  SameKeys<typeof fixture.conversation_view, ConversationView>,
  SameKeys<typeof fixture.interrogator_view, InterrogatorView>,
  SameKeys<(typeof fixture.interrogator_view.earlier)[number], ConversationSummary>,
  SameKeys<typeof fixture.llm_settings, LlmSettings>,
  SameKeys<(typeof fixture.llm_settings.providers)[number], ProviderInfo>,
  SameKeys<typeof fixture.embedder_status, EmbedderStatus>,
  SameKeys<typeof fixture.granted_file, GrantedFile>,
  SameKeys<typeof fixture.handoff_summary, HandoffSummary>,
  SameKeys<typeof fixture.handoff_summary.identifiers, Identifiers>,
  SameKeys<typeof fixture.handoff_summary_refused, HandoffSummary>,
  SameKeys<typeof fixture.download_progress, DownloadProgress>,
  SameKeys<typeof fixture.answer_indexing, Extract<AnswerEvent, { kind: "indexing" }>>,
  SameKeys<typeof fixture.answer_delta, Extract<AnswerEvent, { kind: "delta" }>>,
  SameKeys<typeof fixture.answer_checking_model, Extract<AnswerEvent, { kind: "checking_model" }>>,
  SameKeys<typeof fixture.answer_waiting, Extract<AnswerEvent, { kind: "waiting" }>>,
  SameKeys<typeof fixture.pico_condensing, Extract<PicoEvent, { kind: "condensing" }>>,
  SameKeys<typeof fixture.pico_condensed, Extract<PicoEvent, { kind: "condensed" }>>,
  SameKeys<typeof fixture.pico_other, Extract<PicoEvent, { kind: "other" }>>,
  SameKeys<typeof fixture.pico_view, PicoView>,
  SameKeys<NonNullable<typeof fixture.pico_view.current>, PicoRunView>,
  SameKeys<NonNullable<typeof fixture.pico_view.current.extraction>, PicoExtractionView>,
  SameKeys<(typeof fixture.pico_view.current.extraction.parts)[number], PicoPartView>,
  SameKeys<NonNullable<typeof fixture.pico_view.current.suitability>, SuitabilityView>,
  SameKeys<(typeof fixture.pico_view.earlier)[number], PicoRunSummary>,
  SameKeys<typeof fixture.pico_run_not_suitable, PicoRunView>,
  SameKeys<typeof fixture.pico_run_forced, PicoRunView>,
  SameKeys<typeof fixture.pico_view_empty, PicoView>,
] = [
  true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
  true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
  true, true, true, true, true, true,
];
// Also `check:web` only: a field whose type changed fails to compile. The tag
// values the JSON widens to `string` are pinned at run time below.
const valuesMatch: [
  Widen<OpenOutcome>,
  Widen<OpenOutcome>,
  Widen<PaperSummary>,
  Widen<PaperView>,
  Widen<PaperView>,
  Widen<CommandError>[],
  Widen<TurnView>,
  Widen<TurnView>,
  Widen<TurnView>,
  Widen<ConversationView>,
  Widen<InterrogatorView>,
  Widen<LlmSettings>,
  Widen<EmbedderStatus>,
  Widen<GrantedFile>,
  Widen<HandoffSummary>,
  Widen<HandoffSummary>,
  Widen<DownloadProgress>,
  Widen<AnswerEvent>,
  Widen<AnswerEvent>,
  Widen<AnswerEvent>,
  Widen<AnswerEvent>,
  Widen<PicoEvent>,
  Widen<PicoEvent>,
  Widen<PicoEvent>,
  Widen<PicoView>,
  Widen<PicoRunView>,
  Widen<PicoRunView>,
  Widen<PicoView>,
] = [
  fixture.open_ready,
  fixture.open_needs_full_text,
  fixture.paper_summary,
  fixture.paper_view,
  fixture.paper_view_bare,
  fixture.command_errors,
  fixture.turn_view,
  fixture.turn_view_stopped,
  fixture.turn_view_failed,
  fixture.conversation_view,
  fixture.interrogator_view,
  fixture.llm_settings,
  fixture.embedder_status,
  fixture.granted_file,
  fixture.handoff_summary,
  fixture.handoff_summary_refused,
  fixture.download_progress,
  fixture.answer_indexing,
  fixture.answer_delta,
  fixture.answer_checking_model,
  fixture.answer_waiting,
  fixture.pico_condensing,
  fixture.pico_condensed,
  fixture.pico_other,
  fixture.pico_view,
  fixture.pico_run_not_suitable,
  fixture.pico_run_forced,
  fixture.pico_view_empty,
];

const view = fixture.paper_view as PaperView;

/** What `invoke` was called with: each command's argument keys and their JSON types (`"channel"` for a `Channel`). */
const sent: Record<string, Record<string, string>> = {};
/** The channels each command was given, to check they reach the caller's handler. */
const channels: Record<string, { onmessage: unknown }> = {};
/** Stands in for Tauri's `Channel`, which needs the Tauri runtime. */
const { MockChannel } = vi.hoisted(() => ({
  MockChannel: class {
    constructor(public onmessage?: (message: unknown) => void) {}
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: MockChannel,
  invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
    sent[cmd] = Object.fromEntries(
      Object.entries(args).map(([k, v]) => {
        if (!(v instanceof MockChannel)) return [k, typeof v];
        channels[cmd] = v as { onmessage: unknown };
        return [k, "channel"];
      }),
    );
  },
}));

/** Does nothing; a handler whose identity the channel test checks. */
const onEvent = () => {};
/** Does nothing; a handler whose identity the channel test checks. */
const onProgress = () => {};
/** Does nothing; a handler whose identity the channel test checks. */
const onPicoEvent = () => {};

describe("the Rust↔TS contract", () => {
  test("the open outcomes carry the tags the frontend branches on", () => {
    const tags: OpenOutcome["status"][] = ["ready", "needs_full_text"];
    expect([fixture.open_ready.status, fixture.open_needs_full_text.status]).toEqual(tags);
  });

  test("the source kinds are Rust's names", () => {
    const kinds: string[] = [fixture.paper_view.source_kind, fixture.paper_view_bare.source_kind];
    expect(kinds).toEqual(["plain_text", "pdf"]);
    expect(kinds.every((k) => (SOURCE_KINDS as readonly string[]).includes(k))).toBe(true);
  });

  test("an absent title, heading or decoding is null, not missing", () => {
    const bare = fixture.paper_view_bare as PaperView;
    expect(bare.title_span).toBeNull();
    expect(bare.sections[0].heading).toBeNull();
    expect(bare.decoded).toBeNull();
  });

  test("turns, modes, pieces and answer events carry Rust's tags", () => {
    const turns = fixture.conversation_view.turns;
    const distinct = (values: string[]) => [...new Set(values)].sort();
    // The fixture holds every status and mode, so the hand-written lists must equal its values.
    expect(distinct(turns.map((t) => t.status))).toEqual([...TURN_STATUSES].sort());
    expect(distinct(turns.map((t) => t.mode))).toEqual([...MODES].sort());
    expect(fixture.turn_view_failed.error).toBe("the stream broke off");
    expect(fixture.turn_view.pieces.map((p) => p.kind)).toEqual(["text", "cite", "text"]);
    expect([
      fixture.answer_indexing.kind,
      fixture.answer_delta.kind,
      fixture.answer_checking_model.kind,
      fixture.answer_waiting.kind,
    ]).toEqual(["indexing", "delta", "checking_model", "waiting"]);
    expect([fixture.pico_condensing.kind, fixture.pico_condensed.kind, fixture.pico_other.kind]).toEqual([
      "condensing",
      "condensed",
      "other",
    ]);
  });

  test("a citation's span slices the paper's text to the citation's text, astral characters included", () => {
    const [c] = (fixture.turn_view as TurnView).citations;
    expect(sliceSpan(view.text, c.span)).toBe(c.text);
    expect(c.text).toBe("Body 😀 text.");
    expect((fixture.turn_view as TurnView).unlocated).toEqual([
      { quote: "not in it", similarity: 0.2, contradiction: "number" },
    ]);
  });

  test("a stopped turn shows its open quote as text; no current conversation is null", () => {
    const stopped = fixture.turn_view_stopped as TurnView;
    expect(stopped.pieces).toEqual([{ kind: "text", text: "So far \u201cAft\u201d" }]);
    expect(stopped.citations).toEqual([]);
    expect(fixture.interrogator_view.current).toBeNull();
    expect(fixture.llm_settings.providers[0].key_env).toBe("ANTHROPIC_API_KEY");
  });

  test("PICO parts and states carry Rust's tags, all of them, and a citation slices to the paper's wording", () => {
    const run = fixture.pico_view.current as PicoRunView;
    const parts = run.extraction!.parts;
    expect(parts.map((p) => p.part)).toEqual([...PICO_PARTS]);
    // The fixture holds every state, so the hand-written list must equal its values.
    expect([...new Set(parts.map((p) => p.state))].sort()).toEqual([...PICO_PART_STATES].sort());
    const [c] = parts[0].citations;
    expect(sliceSpan(view.text, c.span)).toBe(c.text);
    expect(c.text).toBe("Body 😀 text.");
    expect(fixture.pico_run_not_suitable.extraction).toBeNull();
    // A forced run has no check: `suitability` is present and null, not omitted.
    expect(fixture.pico_run_forced).toHaveProperty("suitability", null);
    expect(fixture.pico_run_forced.extraction).not.toBeNull();
    expect(fixture.pico_view_empty.current).toBeNull();
  });

  test("key sources are Rust's, all of them, and no key is null", () => {
    // The fixture has one provider per source, so the hand-written list must equal its values.
    const sources = fixture.llm_settings.providers.map((p) => p.key_source);
    expect(sources.filter((s) => s !== null).sort()).toEqual([...KEY_SOURCES].sort());
    expect(sources).toContain(null);
  });

  test("the dropped-files and hand-off events are Rust's", () => {
    expect(api.FILES_DROPPED_EVENT).toBe(fixture.files_dropped_event);
    expect(api.HANDOFF_EVENT).toBe(fixture.handoff_event);
  });

  test("a refused hand-off holds only its refusal; absent fields are null, not missing", () => {
    const refused = fixture.handoff_summary_refused as HandoffSummary;
    expect(refused.refusal).not.toBeNull();
    expect([refused.sender, refused.title, refused.identifiers.pmid]).toEqual([null, null, null]);
    expect(refused.has_full_text).toBe(false);
    expect(fixture.handoff_summary.refusal).toBeNull();
  });

  test("the error kinds are Rust's, all of them", () => {
    expect(fixture.command_errors.map((e) => e.kind)).toEqual([...ERROR_KINDS]);
  });

  test("every span slices to the text Rust cut by code point", () => {
    const spans = [view.title_span!, ...view.sections.flatMap((s) => [s.heading!, ...s.blocks.map((b) => b.span)])];
    expect(spans.map((s) => sliceSpan(view.text, s))).toEqual(fixture.span_texts);
  });

  test("table rows group into one table", () => {
    const groups = groupBlocks(view.sections[0].blocks);
    expect(groups.map((g) => g.kind)).toEqual(["paragraph", "table", "paragraph"]);
    const table = groups[1];
    if (table.kind !== "table") throw new Error("not a table");
    expect(table.rows.map((r) => splitCells(sliceSpan(view.text, r)))).toEqual([
      ["a", "b😀"],
      ["1", "2"],
    ]);
  });

  test("the banner names what Rust found missing", () => {
    const text = bannerText(view.full_text);
    expect(text).toContain("no Methods heading");
    expect(text).toContain("no Results or Discussion heading");
    expect(text).toContain("fewer than 1,000 words (8)");
  });

  test("every command is invoked with the argument keys and types Rust decodes", async () => {
    await Promise.all([
      api.pickFile(),
      api.openFile(1),
      api.openPaste("t"),
      api.openIdentifier("i"),
      api.cancelLookup(),
      api.attachFile("p1", 1),
      api.attachPaste("p1", "t"),
      api.reopenPaper("p1"),
      api.recentPapers(),
      api.paperView("p1"),
      api.originalPdf("p1"),
      api.confirmFullText("p1", 1),
      api.getContactEmail(),
      api.setContactEmail("e"),
      api.getKeepMismatched(),
      api.setKeepMismatched(true),
      api.ask("p1", "q", onEvent),
      api.stopAnswer(),
      api.newConversation("p1"),
      api.interrogator("p1"),
      api.conversation(1),
      api.paperViewAt("p1", 1),
      api.pico("p1"),
      api.picoRun(1),
      api.runPico("p1", false, onPicoEvent),
      api.stopPico(),
      api.llmSettings(),
      api.setModel("m"),
      api.setContextTokens(8192),
      api.clearContextTokens(),
      api.setApiKey("anthropic", "k"),
      api.deleteApiKey("anthropic"),
      api.modelWindow("m"),
      api.listModels("ollama"),
      api.embedderStatus(),
      api.downloadEmbedder(onProgress),
      api.cancelDownload(),
      api.pendingHandoffs(),
      api.openHandoff(1),
      api.discardHandoff(1),
    ]);
    expect(sent).toEqual(fixture.commands);
    expect(channels.ask.onmessage).toBe(onEvent);
    expect(channels.download_embedder.onmessage).toBe(onProgress);
    expect(channels.run_pico.onmessage).toBe(onPicoEvent);
  });

  test("the fixture lists every command api.ts invokes, so the call list above misses none", () => {
    const names = [...apiSource.matchAll(/invoke(?:<[^>]*>)?\("(\w+)"/g)].map((m) => m[1]);
    expect(names.sort()).toEqual(Object.keys(fixture.commands).sort());
  });

  test("every invoke names the type its Rust handler returns", () => {
    const declared = Object.fromEntries(
      [...apiSource.matchAll(/invoke<([^>]+)>\("(\w+)"/g)].map((m) => [m[2], m[1]]),
    );
    expect(declared).toEqual(fixture.returns);
  });
});
