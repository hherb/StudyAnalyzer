# ROADMAP

Planned work, decided unless an item says otherwise, and not started. Where to
start on an item is in `HANDOVER.md`; remove an item here once it has landed.
The argument for every item is in the design,
`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md` (§ numbers below).

---

## Phase 1: bmlib additions (spec §7) — done

Every item merged in bmlib by 2026-10-31 (last: PR #586), all in bmlib 0.12.0
(merged, publication pending). What
each gives phase 2 is in `HANDOVER.md` under *Landed*.

Open bmlib issues on landed items that phase 2 may meet: **#528**, **#529**
(`find_passage`), **#560** (`assess_full_text` strictness) and **#561**
(Rust's `chat()` retries nothing), all maintainer decisions; **#563** (a
deeply nested page parses in quadratic time; blocks 2d); **#565** (a typed
exhaustion error; 2a's `shortcut:` waits on it); **#564** (Rust
`normalise_pmc_id` takes Unicode digits); **#566**, **#567** (context
window); **#572**, **#573**, **#574**, **#576**, **#577**, **#578** (PICO
agent, `chat_json`); **#581**–**#583**, **#585** (record lookup); **#599**,
**#605** (MuPDF's silent warnings and page stops); **#604** (the PDF error
kind never reaches `FullTextService`); **#603** (a
PDF tier's result drops earlier tier faults); and minor
canonical-text issues **#548**, **#549**, **#552**, **#553**, **#557**,
**#558**.

## Phase 2: StudyAnalyzer on macOS (+ Linux, Windows)

Scope (§9): start screen, full-text retrieval with upload/paste fallback,
canonical text, Interrogator, PICO tab, settings, SQLite storage, the hand-off
file and link, style A light and dark, built sandbox-ready. Four plans, each in
`docs/superpowers/plans/`:

| Plan | Scope | Waits on |
|---|---|---|
| **2a** | Scaffold, storage, start screen, file/paste/identifier intake, retrieval, canonical text, full-text check, paper pane, contact email | **landed**, PR #10 |
| 2b | Interrogator: chunks + embeddings and model download, whole-paper vs retrieval mode, streamed answers, quote marking, `find_passage` threshold by measurement, citations and highlights, provider/model settings, keychain | 2a |
| 2c | PICO tab | 2b |
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
store submission (blocked on #20, MuPDF's licence); fine-tuning the embedder; other senders (bmlibrarian_lite's
desktop app).
