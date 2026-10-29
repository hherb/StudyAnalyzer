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

| # | Item | Blocks | Notes |
|---|---|---|---|
| 5 | **Metadata by identifier** (PMID / DOI / PMC ID) | no (2a shows the title only) | `parse_article_set` exists; a single lookup call does not |
| 7 | **PICO agent** port, every part carrying verified quotes | 2c | `bmlibrarian/src/bmlibrarian/agents/pico_agent.py` and its 27 tests |
| 8 | **Vector storage** for chunks + embeddings in SQLite | no | Brute-force cosine is enough for one paper; sqlite-vec is the builder's call |
| 11 | **Model context-window table** (provider metadata where available, else a known table), plus Ollama `num_ctx` | 2b | **In review, bmlib PR #568**; filed beside it: **#566**, **#567**. Spec §6.2's whole-paper vs retrieval decision |

Open bmlib issues on landed items: **#528** and **#529** (`find_passage`,
both are decisions for the maintainer); **#548**, **#549**, **#552**, **#553**
(canonical text, minor); **#557** (heading classification parity),
**#558** (scraper 0.29), **#560** (`assess_full_text` strictness, a
maintainer decision) and **#563** (a deeply nested page parses in quadratic
time; a size cap does not bound it), from 4c; **#561** (Rust's `chat()` retries nothing),
from 6; **#564** (Rust `normalise_pmc_id` takes Unicode digits) and **#565**
(a typed exhaustion error), from 2a.

## Phase 2: StudyAnalyzer on macOS (+ Linux, Windows)

Scope (§9): start screen, full-text retrieval with upload/paste fallback,
canonical text, Interrogator, PICO tab, settings, SQLite storage, the hand-off
file and link, style A light and dark, built sandbox-ready. Four plans, each in
`docs/superpowers/plans/`:

| Plan | Scope | Waits on |
|---|---|---|
| **2a** | Scaffold, storage, start screen, file/paste/identifier intake, retrieval, canonical text, full-text check, paper pane, contact email | **landed**, PR #10 |
| 2b | Interrogator (and StudyAnalyzer #7, #8 first): chunks + embeddings and model download, whole-paper vs retrieval mode, streamed answers, quote marking, `find_passage` threshold by measurement, citations and highlights, provider/model settings, keychain | 2a, bmlib item 11 (PR #568) |
| 2c | PICO tab | 2b, bmlib item 7 |
| 2d | `.studyanalyzer` hand-off file (container, versioning) and `studyanalyzer://` link, single instance | 2a; bmlib **#563** (HTML nesting guard: a hand-off file is untrusted) |

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
