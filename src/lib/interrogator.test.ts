import { describe, expect, test } from "vitest";
import type { ConversationView, ProviderInfo, TurnView } from "./api";
import {
  conversationCaption,
  earlierLabel,
  embedderPrompt,
  keyStatus,
  parseContextTokens,
  turnCaption,
} from "./interrogator";

/** A conversation on text `canonical_id`, read-only or not. */
const conv = (canonical_id: number, read_only: boolean): ConversationView => ({
  id: 1,
  canonical_id,
  read_only,
  turns: [],
});
/** A provider whose key comes from `key_source`. */
const provider = (key_source: ProviderInfo["key_source"]): ProviderInfo => ({
  name: "anthropic",
  default_model: "m",
  needs_key: true,
  has_key: key_source !== null,
  key_source,
  key_env: "ANTHROPIC_API_KEY",
});

describe("interrogator helpers", () => {
  test("conversationCaption", () => {
    expect(conversationCaption(conv(7, false), 7)).toBeNull();
    expect(conversationCaption(conv(7, true), 7)).toBe("earlier conversation");
    expect(conversationCaption(conv(5, true), 7)).toBe("on an earlier version of the text");
  });
  test("earlierLabel", () => {
    const s = { id: 1, canonical_id: 7, turns: 1, created_at: "2026-11-02 10:00:00" };
    expect(earlierLabel(s, 7)).toBe("2026-11-02 10:00:00 · 1 question");
    expect(earlierLabel({ ...s, turns: 3, canonical_id: 5 }, 7)).toBe(
      "2026-11-02 10:00:00 · 3 questions · on an earlier version of the text",
    );
  });
  test("turnCaption", () => {
    const t = { model: "ollama:qwen3", mode: "whole_paper" } as TurnView;
    expect(turnCaption(t)).toBe("ollama:qwen3 · whole paper");
    expect(turnCaption({ ...t, mode: "retrieval" })).toBe("ollama:qwen3 · passages");
  });
  test("embedderPrompt names bmlib's model size as the brief does", () => {
    expect(embedderPrompt(1_157_672_160)).toBe(
      "Answering from this paper needs the search model (1.1 GB, downloaded once). Download it?",
    );
    expect(embedderPrompt(null)).toBe(
      "Answering from this paper needs the search model (downloaded once). Download it?",
    );
  });
  test("keyStatus names where the key comes from", () => {
    expect(keyStatus(provider(null))).toBe("none");
    expect(keyStatus(provider("keychain"))).toBe("saved");
    expect(keyStatus(provider("env"))).toBe("from ANTHROPIC_API_KEY");
  });
  test("parseContextTokens", () => {
    expect(parseContextTokens("  ")).toEqual({ kind: "clear" });
    expect(parseContextTokens("8192")).toEqual({ kind: "set", tokens: 8192 });
    expect(parseContextTokens("32,768")).toEqual({ kind: "set", tokens: 32768 });
    for (const bad of ["0", "-5", "1.5", "1e4", "abc", "99999999999999999999"]) {
      expect(parseContextTokens(bad)).toEqual({ kind: "invalid" });
    }
  });
});
