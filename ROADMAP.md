# ROADMAP

Planned work, decided unless an item says otherwise, and not started. Where to
start on an item is in `HANDOVER.md`; remove an item here once it has landed.
The argument for every item is in the design,
`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md` (§ numbers below).

---

## Phase 1: bmlib additions (spec §7)

Work in `~/src/bmlib`, one bmlib PR per item, **Rust and Python on par in the
same PR**, held together by shared contract fixtures. Each item gets its own
plan in bmlib's `docs/superpowers/plans/` (as `2026-10-21-passage-match.md`
and `2026-10-22-embedder.md` did).

| # | Item | Blocks phase 2 | Notes |
|---|---|---|---|
| 9 | **Retrieval returns its source**: `FullTextService` also hands back the raw JATS XML or PDF bytes, not only `FullTextResult.html` | yes | **In review: bmlib PR #545** (`FullTextResult.jats_xml`, cache `jats/`). Without it §5's sections cannot be built from a fetched paper |
| 4 | **Canonical text** from JATS (new text renderer over `JATSArticle`), PDF (segmenter), HTML (new extraction), Markdown, plain text, with section spans and a converter version | yes | §5. One offset unit (code points); JATS tables as plain rows |
| 3 | **RAG chunker**: sentence-aware, section-labelled chunks with offsets into the canonical text | yes | ~1,000 chars, 100 overlap, as bmlibrarian's `chunk_embedder.py`. Not the existing 10k `TextChunker` |
| 6 | **Streaming chat** in `LlmClient` for Anthropic, OpenAI-compatible, Ollama | yes | Callback-per-delta over the blocking `ureq` client; no async |
| 5 | **Metadata by identifier** (PMID / DOI / PMC ID) | no | `parse_article_set` exists; a single lookup call does not |
| 7 | **PICO agent** port, every part carrying verified quotes | no | `bmlibrarian/src/bmlibrarian/agents/pico_agent.py` and its 27 tests |
| 8 | **Vector storage** for chunks + embeddings in SQLite | no | Brute-force cosine is enough for one paper; sqlite-vec is the builder's call |

Open bmlib issues on landed items: **#528** and **#529** (`find_passage`,
both are decisions for the maintainer).

## Phase 2: StudyAnalyzer on macOS (+ Linux, Windows)

Needs phase 1's items 1–4, 6, 9, 10. Scope (§9): start screen (recent, open
file, paste, identifier), full-text retrieval with upload/paste fallback,
canonical text, Interrogator (whole-paper and retrieval modes, streamed
answers, verified citations, dark/pale highlights), PICO tab, settings
(provider/model, context-limit override, keychain), SQLite storage, the
`.studyanalyzer` hand-off file and `studyanalyzer://` link, style A light and
dark. Built sandbox-ready even before submission.

Builder's calls to settle in its plan (§11): repository layout and how bmlib is
depended on, hand-off container and versioning, retrieval parameters, the
quote-marking format, the find_passage threshold (by measurement), HTML
extraction, converter-version handling, test strategy.

## Phase 3: "Open in StudyAnalyzer" in MedicalFactChecker (macOS)

On the paper card and in the full-text viewer: write a hand-off file
(identifiers + PDF or stored full text) and open it with StudyAnalyzer; the
link alone without full text. First full round trip. MedicalFactChecker is not
sandboxed today; the hand-off must keep working once it is (§10).

## Phase 4: iPhone/iPad, then Android

Starts with a **measurement spike**: embedder speed and memory on a real phone
(F16 ≈ 1.1 GB, Q8_0 ≈ 0.6 GB). The plan then decides between Q8_0, embedding
only when retrieval mode is needed, or both. Then StudyAnalyzer on each
platform and MedicalFactChecker's sending side.

## Later, outside the design

Further analysis tabs (study assessment, PRISMA 2020, transparency, risk of
bias, retraction status, paper weight), each its own tab on the PICO pattern;
store submission; fine-tuning the embedder; other senders (bmlibrarian_lite's
desktop app).
